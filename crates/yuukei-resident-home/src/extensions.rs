use super::*;

impl ResidentHome {
    pub(crate) async fn notify_extensions_event_appended(
        &self,
        record: &EventLogRecord,
    ) -> Result<Vec<RuntimeEvent>> {
        let registry = self
            .extensions
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        let result = registry
            .notify_event_appended(
                record.clone(),
                ExtensionEventContext {
                    world_pack_id: self.world_pack.id.clone(),
                },
            )
            .await?;

        let mut proposed_events = Vec::new();
        for report in result.reports {
            if let Some(failure) = &report.process_failure {
                self.handle_process_failure_report(
                    failure,
                    &record.id,
                    &record.resident_id,
                    record
                        .causality
                        .as_ref()
                        .and_then(|causality| causality.trace_id.clone()),
                )
                .await?;
            }
            for proposed in &report.result.proposed_events {
                match self.normalize_extension_event(&registry, &report, proposed, record) {
                    Ok(event) => proposed_events.push(event),
                    Err(reason) => {
                        self.record_extension_event_rejection(&report, record, reason)?
                    }
                }
            }
        }
        Ok(proposed_events)
    }

    pub(crate) fn normalize_extension_event(
        &self,
        registry: &ExtensionRegistry,
        report: &ExtensionEventReport,
        proposed: &RuntimeEvent,
        source_record: &EventLogRecord,
    ) -> std::result::Result<RuntimeEvent, String> {
        let extension_id = &report.invocation.extension_id;
        let required_prefix = format!("ext.{extension_id}.");
        if !proposed.kind.starts_with(&required_prefix) {
            return Err(format!(
                "extension event type must start with {required_prefix}: {}",
                proposed.kind
            ));
        }
        if !registry.can_emit_event(extension_id, &proposed.kind) {
            return Err(format!(
                "extension did not declare emitted event type: {}",
                proposed.kind
            ));
        }

        let hop_count = extension_event_hop_count(source_record) + 1;
        if hop_count > MAX_EXTENSION_EVENT_HOPS {
            return Err(format!(
                "extension event hop count exceeded {MAX_EXTENSION_EVENT_HOPS}: {hop_count}"
            ));
        }

        let mut event = proposed.clone();
        event.id = new_id("evt");
        event.timestamp = yuukei_protocol::now_timestamp();
        event.source = "extension".to_string();
        event.resident_id = source_record.resident_id.clone();
        event.device_id = source_record.device_id.clone();
        event.surface_id = source_record.surface_id.clone();
        event.actor_id = source_record.actor_id.clone();
        event.causality = Some(Causality {
            source_event_id: Some(source_record.id.clone()),
            source_command_id: None,
            trace_id: source_record
                .causality
                .as_ref()
                .and_then(|causality| causality.trace_id.clone()),
        });
        event.payload.insert(
            "yuukeiExtension".to_string(),
            serde_json::json!({
                "extensionId": extension_id,
                "hopCount": hop_count,
            }),
        );
        Ok(event)
    }

    pub(crate) fn record_extension_event_rejection(
        &self,
        report: &ExtensionEventReport,
        source_record: &EventLogRecord,
        reason: String,
    ) -> Result<()> {
        let record = NewEventLogRecord {
            id: new_id("evt"),
            kind: "extension.event.rejected".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id: source_record.resident_id.clone(),
            source: "extension".to_string(),
            device_id: source_record.device_id.clone(),
            surface_id: source_record.surface_id.clone(),
            actor_id: source_record.actor_id.clone(),
            payload: JsonMap::from([
                (
                    "invocationId".to_string(),
                    Value::String(report.invocation.id.clone()),
                ),
                (
                    "extensionId".to_string(),
                    Value::String(report.invocation.extension_id.clone()),
                ),
                ("reason".to_string(), Value::String(reason)),
            ]),
            causality: Some(Causality {
                source_event_id: Some(source_record.id.clone()),
                source_command_id: None,
                trace_id: source_record
                    .causality
                    .as_ref()
                    .and_then(|causality| causality.trace_id.clone()),
            }),
            privacy: None,
        };
        let appended = self.event_log.append(record)?;
        self.set_cursor(appended.sequence)?;
        Ok(())
    }

    pub(crate) async fn reload_daihon_signal_aliases(&self) -> Result<()> {
        let aliases = self.extension_signal_alias_table()?;
        match self
            .daihon
            .load_world_with_signal_aliases(&self.world_pack, &aliases)
            .await
        {
            Ok(()) => Ok(()),
            Err(error) => {
                if let Some(report) = error.daihon_report() {
                    self.record_daihon_report(report)?;
                }
                Err(error.into())
            }
        }
    }

    pub(crate) fn extension_signal_alias_table(&self) -> Result<SignalAliasTable> {
        let aliases = self
            .extensions
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .signal_aliases();
        Ok(SignalAliasTable::with_standard_and_donated(aliases))
    }

    pub(crate) async fn apply_extensions_before_command_emit(
        &self,
        command: RuntimeCommand,
        source_event: &RuntimeEvent,
    ) -> Result<RuntimeCommand> {
        let registry = self
            .extensions
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        let result = registry
            .apply_before_command_emit(
                command,
                ExtensionCommandContext {
                    world_pack_id: self.world_pack.id.clone(),
                },
            )
            .await?;
        for report in &result.reports {
            self.record_extension_hook_result(report, source_event)?;
            if let Some(failure) = &report.process_failure {
                self.handle_process_failure_report(
                    failure,
                    &source_event.id,
                    &source_event.resident_id,
                    source_event
                        .causality
                        .as_ref()
                        .and_then(|causality| causality.trace_id.clone()),
                )
                .await?;
            }
        }
        Ok(result.command)
    }

    pub(crate) fn record_extension_hook_result(
        &self,
        report: &ExtensionHookReport,
        source_event: &RuntimeEvent,
    ) -> Result<()> {
        let result_value = serde_json::to_value(&report.result)?;
        let output_command_value = serde_json::to_value(&report.output_command)?;
        let mut payload = JsonMap::from([
            (
                "invocationId".to_string(),
                Value::String(report.invocation.id.clone()),
            ),
            (
                "extensionId".to_string(),
                Value::String(report.invocation.extension_id.clone()),
            ),
            (
                "hookPoint".to_string(),
                serde_json::to_value(&report.invocation.hook_point)?,
            ),
            (
                "inputCommandId".to_string(),
                Value::String(report.input_command.id.clone()),
            ),
            (
                "outputCommandId".to_string(),
                Value::String(report.output_command.id.clone()),
            ),
            (
                "commandType".to_string(),
                Value::String(report.output_command.kind.clone()),
            ),
            ("changed".to_string(), Value::Bool(report.changed)),
            ("result".to_string(), result_value),
            ("outputCommand".to_string(), output_command_value),
        ]);
        if let Some(error) = &report.error {
            payload.insert("error".to_string(), Value::String(error.clone()));
        }
        let record = NewEventLogRecord {
            id: new_id("evt"),
            kind: "extension.hook.result".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id: source_event.resident_id.clone(),
            source: "extension".to_string(),
            device_id: source_event.device_id.clone(),
            surface_id: source_event.surface_id.clone(),
            actor_id: report
                .output_command
                .target
                .as_ref()
                .and_then(|target| target.actor_id.clone()),
            payload,
            causality: Some(Causality {
                source_event_id: Some(source_event.id.clone()),
                source_command_id: Some(report.input_command.id.clone()),
                trace_id: source_event
                    .causality
                    .as_ref()
                    .and_then(|causality| causality.trace_id.clone()),
            }),
            privacy: None,
        };
        let appended = self.event_log.append(record)?;
        self.set_cursor(appended.sequence)?;
        Ok(())
    }

    pub(crate) async fn handle_process_failure_report(
        &self,
        failure: &ProcessFailureReport,
        source_event_id: &str,
        resident_id: &str,
        trace_id: Option<String>,
    ) -> Result<()> {
        self.record_extension_process_failure(
            "extension.process.failed",
            failure,
            source_event_id,
            resident_id,
            trace_id.clone(),
        )?;
        if !failure.suspension_started {
            return Ok(());
        }
        self.record_extension_process_failure(
            "extension.process.suspended",
            failure,
            source_event_id,
            resident_id,
            trace_id.clone(),
        )?;
        let mut command = RuntimeCommand::new("ui.notification", "resident-home", resident_id);
        command.payload = JsonMap::from([
            (
                "extensionId".to_string(),
                Value::String(failure.extension_id.clone()),
            ),
            (
                "text".to_string(),
                Value::String(format!(
                    "{}が応答しないため、いったん休止しました。設定画面から再起動できます",
                    failure.display_name
                )),
            ),
        ]);
        command.causality = Some(Causality {
            source_event_id: Some(source_event_id.to_string()),
            source_command_id: None,
            trace_id: trace_id.clone(),
        });
        self.emit_internal_command_without_extensions(command)?;
        Ok(())
    }

    pub(crate) async fn handle_capability_error(
        &self,
        error: &CapabilityError,
        source_event: &RuntimeEvent,
    ) -> Result<()> {
        if let CapabilityError::ExtensionProcessSuspended {
            extension_id,
            display_name,
            message,
            suspension_started,
        } = error
        {
            let failure = ProcessFailureReport {
                extension_id: extension_id.clone(),
                display_name: display_name.clone(),
                kind: ProcessFailureKind::Crash,
                message: message.clone(),
                suspended: true,
                suspension_started: *suspension_started,
            };
            self.handle_process_failure_report(
                &failure,
                &source_event.id,
                &source_event.resident_id,
                source_event
                    .causality
                    .as_ref()
                    .and_then(|causality| causality.trace_id.clone()),
            )
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn invoke_capability_with_timeout(
        &self,
        router: CapabilityRouter,
        invocation: CapabilityInvocation,
        timeout_duration: Duration,
        source_event: &RuntimeEvent,
    ) -> Result<Option<CapabilityResult>> {
        match timeout(timeout_duration, router.invoke(invocation)).await {
            Ok(Ok(result)) => Ok(Some(result)),
            Ok(Err(error)) => {
                self.handle_capability_error(&error, source_event).await?;
                Ok(None)
            }
            Err(_) => Ok(None),
        }
    }

    pub(crate) fn record_extension_process_failure(
        &self,
        kind: &str,
        failure: &ProcessFailureReport,
        source_event_id: &str,
        resident_id: &str,
        trace_id: Option<String>,
    ) -> Result<()> {
        let record = NewEventLogRecord {
            id: new_id("evt"),
            kind: kind.to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id: resident_id.to_string(),
            source: "resident-home".to_string(),
            device_id: None,
            surface_id: None,
            actor_id: None,
            payload: JsonMap::from([
                (
                    "extensionId".to_string(),
                    Value::String(failure.extension_id.clone()),
                ),
                (
                    "displayName".to_string(),
                    Value::String(failure.display_name.clone()),
                ),
                (
                    "failureKind".to_string(),
                    serde_json::to_value(&failure.kind)?,
                ),
                (
                    "message".to_string(),
                    Value::String(failure.message.clone()),
                ),
                ("suspended".to_string(), Value::Bool(failure.suspended)),
            ]),
            causality: Some(Causality {
                source_event_id: Some(source_event_id.to_string()),
                source_command_id: None,
                trace_id,
            }),
            privacy: None,
        };
        let appended = self.event_log.append(record)?;
        self.set_cursor(appended.sequence)?;
        Ok(())
    }

}

fn extension_event_hop_count(record: &EventLogRecord) -> u32 {
    record
        .payload
        .get("yuukeiExtension")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("hopCount"))
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0)
}

