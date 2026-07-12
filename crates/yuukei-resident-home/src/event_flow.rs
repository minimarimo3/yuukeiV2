use super::*;

impl ResidentHome {
    pub async fn ingest_event(&self, event: RuntimeEvent) -> Result<Vec<RuntimeCommand>> {
        let appended_event = self
            .event_log
            .append(NewEventLogRecord::from(event.clone()))?;
        self.set_cursor(appended_event.sequence)?;
        self.apply_runtime_event_to_snapshot(&event)?;
        if self.resolve_pending_choice_event(&event)? {
            return Ok(Vec::new());
        }
        if self.defer_event_while_interpreting(event.clone(), appended_event.clone())? {
            return Ok(Vec::new());
        }
        let mut emitted = self
            .process_appended_runtime_event(event, appended_event)
            .await?;
        emitted.extend(self.drain_interpretation_queue().await?);
        Ok(emitted)
    }

    pub(crate) fn defer_event_while_interpreting(
        &self,
        event: RuntimeEvent,
        record: EventLogRecord,
    ) -> Result<bool> {
        let dropped = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?;
            if !state.interpretation.in_flight {
                return Ok(false);
            }
            if !event.kind.starts_with("conversation.") {
                return Ok(true);
            }
            let dropped = if state.interpretation.queued_events.len()
                >= MAX_QUEUED_CONVERSATION_EVENTS_DURING_INTERPRET
            {
                state.interpretation.queued_events.pop_front()
            } else {
                None
            };
            state
                .interpretation
                .queued_events
                .push_back((event, record));
            dropped
        };
        if let Some((dropped_event, dropped_record)) = dropped {
            self.record_interpretation_queue_drop(&dropped_event, &dropped_record)?;
        }
        Ok(true)
    }

    pub(crate) async fn drain_interpretation_queue(&self) -> Result<Vec<RuntimeCommand>> {
        let mut emitted = Vec::new();
        loop {
            let next = {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| ResidentHomeError::PoisonedLock)?;
                if state.interpretation.in_flight {
                    None
                } else {
                    state.interpretation.queued_events.pop_front()
                }
            };
            let Some((event, record)) = next else {
                break;
            };
            emitted.extend(self.process_appended_runtime_event(event, record).await?);
        }
        Ok(emitted)
    }

    pub(crate) fn record_interpretation_queue_drop(
        &self,
        dropped_event: &RuntimeEvent,
        dropped_record: &EventLogRecord,
    ) -> Result<()> {
        let record = NewEventLogRecord {
            id: new_id("evt"),
            kind: "daihon.interpretation.queue.dropped".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id: dropped_event.resident_id.clone(),
            source: "resident-home".to_string(),
            device_id: dropped_event.device_id.clone(),
            surface_id: dropped_event.surface_id.clone(),
            actor_id: dropped_event.actor_id.clone(),
            payload: JsonMap::from([
                (
                    "droppedEventId".to_string(),
                    Value::String(dropped_event.id.clone()),
                ),
                (
                    "droppedEventType".to_string(),
                    Value::String(dropped_event.kind.clone()),
                ),
                (
                    "droppedSequence".to_string(),
                    Value::Number(dropped_record.sequence.into()),
                ),
                (
                    "reason".to_string(),
                    Value::String("interpretation queue overflow".to_string()),
                ),
            ]),
            causality: Some(Causality {
                source_event_id: Some(dropped_event.id.clone()),
                source_command_id: None,
                trace_id: dropped_event
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

    pub(crate) async fn process_appended_runtime_event(
        &self,
        event: RuntimeEvent,
        record: EventLogRecord,
    ) -> Result<Vec<RuntimeCommand>> {
        let mut queue = VecDeque::from([(event, record)]);
        let mut emitted_commands = Vec::new();

        while let Some((event, record)) = queue.pop_front() {
            let proposed_events = self.notify_extensions_event_appended(&record).await?;
            for proposed in proposed_events {
                let appended = self
                    .event_log
                    .append(NewEventLogRecord::from(proposed.clone()))?;
                self.set_cursor(appended.sequence)?;
                queue.push_back((proposed, appended));
            }
            let outcome = self.dispatch_recorded_event(event, &record).await?;
            emitted_commands.extend(outcome.commands);
            for internal_event in outcome.events {
                let appended = self
                    .event_log
                    .append(NewEventLogRecord::from(internal_event.clone()))?;
                self.set_cursor(appended.sequence)?;
                queue.push_back((internal_event, appended));
            }
        }

        Ok(emitted_commands)
    }

    pub(crate) async fn dispatch_recorded_event(
        &self,
        event: RuntimeEvent,
        record: &EventLogRecord,
    ) -> Result<DispatchOutcome> {
        self.maybe_index_memory_for_trigger(&event).await?;
        let mut internal_events = Vec::new();
        if event.kind == "presence.life_tick" {
            if let Some(mood_event) = self.maybe_evaluate_mood(&event).await? {
                internal_events.push(mood_event);
            }
        }
        if event.kind == MOOD_CHANGED_EVENT {
            if let Some(talk_event) = self.apply_mood_changed_event(&event, record)? {
                internal_events.push(talk_event);
            }
            return Ok(DispatchOutcome {
                commands: Vec::new(),
                events: internal_events,
            });
        }

        let event = if event.kind == TALK_IMPULSE_EVENT {
            match self.moderate_talk_impulse_event(event)? {
                TalkImpulseModeration::Dispatch(event) => event,
                TalkImpulseModeration::Skip { source_event } => {
                    self.record_talk_impulse_skip(&source_event)?;
                    return Ok(DispatchOutcome {
                        commands: Vec::new(),
                        events: internal_events,
                    });
                }
            }
        } else {
            event
        };
        let event = self.enrich_event_for_daihon_dispatch(event)?;
        let aliases = self.extension_signal_alias_table()?;
        if !self
            .world_pack
            .allows_signal_with_aliases(&event.kind, &aliases)
        {
            return Ok(DispatchOutcome {
                commands: Vec::new(),
                events: internal_events,
            });
        }

        let mut interpret_handler = ResidentHomeInterpretHandler {
            home: self.clone(),
            source_event: event.clone(),
        };
        let result = match self
            .daihon
            .dispatch_with_interpret(&event, &self.world_pack, &mut interpret_handler)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                if let Some(report) = error.daihon_report() {
                    self.record_daihon_report(report)?;
                }
                return Err(error.into());
            }
        };
        if !result.is_empty() {
            let result_payload = serde_json::to_value(&result)?;
            let result_record = NewEventLogRecord {
                id: new_id("evt"),
                kind: "daihon.dispatch.result".to_string(),
                timestamp: yuukei_protocol::now_timestamp(),
                resident_id: event.resident_id.clone(),
                source: "daihon".to_string(),
                device_id: event.device_id.clone(),
                surface_id: event.surface_id.clone(),
                actor_id: event.actor_id.clone(),
                payload: json_map_from_value(result_payload),
                causality: Some(Causality {
                    source_event_id: Some(event.id.clone()),
                    source_command_id: None,
                    trace_id: event
                        .causality
                        .as_ref()
                        .and_then(|causality| causality.trace_id.clone()),
                }),
                privacy: None,
            };
            let appended_result = self.event_log.append(result_record)?;
            self.set_cursor(appended_result.sequence)?;
        }
        let commands = if result.is_empty() {
            self.maybe_generate_dialogue_fallback(&event, &aliases)
                .await?
        } else {
            result.commands
        };
        let mut emitted_commands = Vec::with_capacity(commands.len());
        for command in commands {
            emitted_commands.push(self.emit_command_for_event(command, &event).await?);
        }
        Ok(DispatchOutcome {
            commands: emitted_commands,
            events: internal_events,
        })
    }

    pub(crate) fn enrich_event_for_daihon_dispatch(
        &self,
        mut event: RuntimeEvent,
    ) -> Result<RuntimeEvent> {
        let ai_connected = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .has_healthy_provider(DIALOGUE_GENERATE_CAPABILITY);
        event
            .payload
            .insert("aiConnected".to_string(), json!(ai_connected));
        if event.kind == "desktop.folder.opened" {
            let (file_name, file_category) = self.recent_download_for_folder_event(&event)?;
            event.payload.insert(
                "recentDownloadFileName".to_string(),
                Value::String(file_name),
            );
            event.payload.insert(
                "recentDownloadCategory".to_string(),
                Value::String(file_category),
            );
        }
        Ok(event)
    }

    pub(crate) fn recent_download_for_folder_event(
        &self,
        event: &RuntimeEvent,
    ) -> Result<(String, String)> {
        let dispatch_at = event_timestamp_or_now(event);
        let cutoff = dispatch_at - chrono::Duration::days(7);
        let records = self
            .event_log
            .read(EventLogQuery {
                resident_id: Some(event.resident_id.clone()),
                kind: Some("desktop.download.completed".to_string()),
                after_sequence: None,
                limit: None,
                extension_readable_only: false,
            })?
            .records;
        for record in records.into_iter().rev() {
            let Some(timestamp) = event_record_timestamp(&record.timestamp) else {
                continue;
            };
            if timestamp < cutoff || timestamp > dispatch_at {
                continue;
            }
            let file_name = record
                .payload
                .get("fileName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let file_category = record
                .payload
                .get("fileCategory")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            return Ok((file_name, file_category));
        }
        Ok((String::new(), String::new()))
    }

    pub(crate) async fn emit_command_for_event(
        &self,
        command: RuntimeCommand,
        source_event: &RuntimeEvent,
    ) -> Result<RuntimeCommand> {
        let mut command = self
            .apply_extensions_before_command_emit(command, source_event)
            .await?;
        let should_mark_speech_pending = command.kind == "dialogue.say"
            && command
                .payload
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.trim().is_empty())
            && self
                .capabilities
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?
                .has_healthy_provider(SPEECH_SYNTHESIS_CAPABILITY);
        if should_mark_speech_pending {
            command
                .payload
                .insert("speechPending".to_string(), Value::Bool(true));
        }
        let appended_command = self
            .event_log
            .append(NewEventLogRecord::from(command.clone()))?;
        self.set_cursor(appended_command.sequence)?;
        self.apply_command_to_snapshot(&command)?;
        let _ = self.command_tx.send(command.clone());
        self.spawn_speech_synthesis_if_needed(command.clone(), source_event.clone())?;
        Ok(command)
    }

    pub(crate) fn emit_internal_command_without_extensions(
        &self,
        command: RuntimeCommand,
    ) -> Result<RuntimeCommand> {
        let appended_command = self
            .event_log
            .append(NewEventLogRecord::from(command.clone()))?;
        self.set_cursor(appended_command.sequence)?;
        self.apply_command_to_snapshot(&command)?;
        let _ = self.command_tx.send(command.clone());
        Ok(command)
    }

    pub(crate) fn set_cursor(&self, sequence: i64) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        state.recent_event_cursor = sequence;
        Ok(())
    }

    pub(crate) fn record_daihon_report(&self, report: &DaihonDiagnosticReport) -> Result<()> {
        let occurred_at = yuukei_protocol::now_timestamp();
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        state.daihon_diagnostics.extend(
            report
                .diagnostics
                .iter()
                .cloned()
                .map(|entry| entry.with_occurred_at(occurred_at.clone())),
        );
        Ok(())
    }

    pub(crate) fn apply_command_to_snapshot(&self, command: &RuntimeCommand) -> Result<()> {
        let actor_id = command
            .target
            .as_ref()
            .and_then(|target| target.actor_id.clone())
            .or_else(|| {
                command
                    .payload
                    .get("speakerId")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            });
        let Some(actor_id) = actor_id else {
            return Ok(());
        };

        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        let Some(actor) = state.actors.get_mut(&actor_id) else {
            return Ok(());
        };
        match command.kind.as_str() {
            "dialogue.say" => {
                if let Some(text) = command
                    .payload
                    .get("text")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                {
                    actor.speaking = Some(true);
                    actor.bubble = Some(text);
                }
            }
            "avatar.expression" => {
                if let Some(expression) = command.payload.get("expression").and_then(Value::as_str)
                {
                    actor.expression = expression.to_string();
                }
            }
            "avatar.motion" => {
                if let Some(motion) = command.payload.get("motion").and_then(Value::as_str) {
                    actor.motion = motion.to_string();
                }
            }
            "stage.walk" => {
                if let Some(motion) = command.payload.get("motion").and_then(Value::as_str) {
                    actor.motion = motion.to_string();
                }
                actor.heading = match command.payload.get("destination").and_then(Value::as_str) {
                    Some("right-edge") => "right".to_string(),
                    Some("left-edge") => "left".to_string(),
                    _ => String::new(),
                };
                state
                    .active_walk_commands
                    .insert(actor_id.clone(), command.id.clone());
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn apply_runtime_event_to_snapshot(&self, event: &RuntimeEvent) -> Result<()> {
        if event.kind != "stage.walk.ended" {
            return Ok(());
        }
        let actor_id = event
            .actor_id
            .as_deref()
            .or_else(|| event.payload.get("speakerId").and_then(Value::as_str));
        let Some(actor_id) = actor_id else {
            return Ok(());
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        if let Some(ended_command_id) = event
            .causality
            .as_ref()
            .and_then(|causality| causality.source_command_id.as_deref())
        {
            if state
                .active_walk_commands
                .get(actor_id)
                .is_some_and(|active_command_id| active_command_id != ended_command_id)
            {
                return Ok(());
            }
        }
        let Some(actor) = state.actors.get_mut(actor_id) else {
            return Ok(());
        };
        actor.motion.clear();
        actor.heading.clear();
        state.active_walk_commands.remove(actor_id);
        Ok(())
    }
}

pub(crate) fn event_timestamp_or_now(event: &RuntimeEvent) -> DateTime<Utc> {
    event_record_timestamp(&event.timestamp).unwrap_or_else(Utc::now)
}
