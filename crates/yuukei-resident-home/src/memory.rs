use super::*;

impl ResidentHome {
    pub async fn list_memories(
        &self,
        episode_limit: Option<usize>,
        episode_offset: Option<usize>,
    ) -> Result<MemoryListOutput> {
        let input = MemoryListInput {
            resident_id: self.resident_id()?,
            world_pack_id: self.world_pack.id.clone(),
            episode_limit,
            episode_offset,
        };
        self.invoke_memory_admin(MEMORY_LIST_CAPABILITY, "list", input)
            .await
    }

    pub async fn update_memory(
        &self,
        kind: MemoryEntryKind,
        id: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<MemoryUpdateOutput> {
        let input = MemoryUpdateInput {
            resident_id: self.resident_id()?,
            world_pack_id: self.world_pack.id.clone(),
            kind,
            id: id.into(),
            text: text.into(),
        };
        self.invoke_memory_admin(MEMORY_UPDATE_CAPABILITY, "update", input)
            .await
    }

    pub async fn forget_memories(
        &self,
        entries: Vec<MemoryForgetEntry>,
        all: bool,
    ) -> Result<MemoryForgetOutput> {
        let input = MemoryForgetInput {
            resident_id: self.resident_id()?,
            world_pack_id: self.world_pack.id.clone(),
            entries,
            all,
        };
        self.invoke_memory_admin(MEMORY_FORGET_CAPABILITY, "forget", input)
            .await
    }

    pub(crate) async fn maybe_index_memory_for_trigger(
        &self,
        trigger_event: &RuntimeEvent,
    ) -> Result<()> {
        if !matches!(
            trigger_event.kind.as_str(),
            "app.startup" | "device.sleep.before"
        ) {
            return Ok(());
        }
        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        if !router.has_healthy_provider(MEMORY_INDEX_CAPABILITY) {
            return Ok(());
        }

        let trigger_date =
            event_record_date(&trigger_event.timestamp).unwrap_or_else(|| Utc::now().date_naive());
        let records = self
            .event_log
            .read(EventLogQuery {
                resident_id: Some(trigger_event.resident_id.clone()),
                kind: None,
                after_sequence: None,
                limit: None,
                extension_readable_only: false,
            })?
            .records;
        let indexed_dates = indexed_memory_dates(&records);
        let mut events_by_date: BTreeMap<NaiveDate, Vec<MemoryIndexEvent>> = BTreeMap::new();
        for record in records {
            let Some(date) = event_record_date(&record.timestamp) else {
                continue;
            };
            if date >= trigger_date
                || indexed_dates.contains(&date)
                || !is_memory_index_event_kind(&record.kind)
            {
                continue;
            }
            events_by_date
                .entry(date)
                .or_default()
                .push(MemoryIndexEvent {
                    kind: record.kind,
                    timestamp: record.timestamp,
                    payload: major_payload(record.payload),
                });
        }

        let mut targets = events_by_date.into_iter().collect::<Vec<_>>();
        targets.reverse();
        targets.truncate(MAX_MEMORY_INDEX_DAYS_PER_TRIGGER);
        targets.reverse();

        for (date, events) in targets {
            if events.is_empty() {
                continue;
            }
            let input = MemoryIndexInput {
                resident_id: trigger_event.resident_id.clone(),
                world_pack_id: self.world_pack.id.clone(),
                date: date.to_string(),
                events,
            };
            let invocation = CapabilityInvocation {
                id: new_id("cap"),
                capability: MEMORY_INDEX_CAPABILITY.to_string(),
                method: "index".to_string(),
                resident_id: trigger_event.resident_id.clone(),
                actor_id: None,
                input: json_map_from_value(serde_json::to_value(input)?),
                context: None,
            };
            self.record_capability_request(&invocation, trigger_event, None)?;
            let Some(result) = self
                .invoke_capability_with_timeout(
                    router.clone(),
                    invocation.clone(),
                    self.runtime_settings.llm_timeout,
                    trigger_event,
                )
                .await?
            else {
                return Ok(());
            };
            self.record_capability_result(CapabilityResultRecord {
                invocation_id: result.invocation_id,
                extension_id: result.extension_id,
                capability: result.capability,
                output: result.output.clone(),
                metadata: result.metadata,
                source_event: trigger_event,
                source_command_id: None,
                actor_id: None,
            })?;
            let output_value = Value::Object(result.output.into_iter().collect());
            let Ok(output) = serde_json::from_value::<MemoryIndexOutput>(output_value) else {
                return Ok(());
            };
            if !output.indexed {
                return Ok(());
            }
        }
        Ok(())
    }

    async fn invoke_memory_admin<TInput, TOutput>(
        &self,
        capability: &str,
        method: &str,
        input: TInput,
    ) -> Result<TOutput>
    where
        TInput: Serialize,
        TOutput: DeserializeOwned,
    {
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: capability.to_string(),
            method: method.to_string(),
            resident_id: self.resident_id()?,
            actor_id: None,
            input: json_map_omitting_null_values(serde_json::to_value(input)?),
            context: None,
        };
        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        let result = timeout(MEMORY_RETRIEVE_TIMEOUT, router.invoke(invocation)).await;
        let result = match result {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => return Err(ResidentHomeError::Capability(error)),
            Err(_) => {
                return Err(ResidentHomeError::Capability(CapabilityError::Extension(
                    format!("{capability} timed out"),
                )))
            }
        };
        let output_value = Value::Object(result.output.into_iter().collect());
        Ok(serde_json::from_value(output_value)?)
    }

    pub(crate) async fn retrieve_memories_for_dialogue_generate(
        &self,
        source_event: &RuntimeEvent,
    ) -> Result<Option<Vec<String>>> {
        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        if !router.has_healthy_provider(MEMORY_RETRIEVE_CAPABILITY) {
            return Ok(None);
        }
        let query_text = source_event
            .payload
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .unwrap_or(&source_event.kind)
            .to_string();
        let input = MemoryRetrieveInput {
            resident_id: source_event.resident_id.clone(),
            world_pack_id: self.world_pack.id.clone(),
            query: MemoryRetrieveQuery { text: query_text },
            limits: MemoryRetrieveLimits {
                facts: MEMORY_RETRIEVE_FACT_LIMIT,
                episodes: MEMORY_RETRIEVE_EPISODE_LIMIT,
            },
        };
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: MEMORY_RETRIEVE_CAPABILITY.to_string(),
            method: "retrieve".to_string(),
            resident_id: source_event.resident_id.clone(),
            actor_id: None,
            input: json_map_omitting_null_values(serde_json::to_value(input)?),
            context: None,
        };
        self.record_capability_request(&invocation, source_event, None)?;
        let Some(result) = self
            .invoke_capability_with_timeout(
                router,
                invocation.clone(),
                MEMORY_RETRIEVE_TIMEOUT,
                source_event,
            )
            .await?
        else {
            return Ok(None);
        };
        self.record_capability_result(CapabilityResultRecord {
            invocation_id: result.invocation_id,
            extension_id: result.extension_id,
            capability: result.capability,
            output: result.output.clone(),
            metadata: result.metadata,
            source_event,
            source_command_id: None,
            actor_id: None,
        })?;
        let output_value = Value::Object(result.output.into_iter().collect());
        let Ok(output) = serde_json::from_value::<MemoryRetrieveOutput>(output_value) else {
            return Ok(None);
        };
        let memories = output
            .memories
            .into_iter()
            .map(|memory| memory.text)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>();
        Ok((!memories.is_empty()).then_some(memories))
    }
}

fn indexed_memory_dates(records: &[EventLogRecord]) -> BTreeSet<NaiveDate> {
    let successful_invocations = records
        .iter()
        .filter(|record| {
            record.kind == "capability.invocation.result"
                && record.payload.get("capability").and_then(Value::as_str)
                    == Some(MEMORY_INDEX_CAPABILITY)
                && record
                    .payload
                    .get("output")
                    .and_then(Value::as_object)
                    .and_then(|output| output.get("indexed"))
                    .and_then(Value::as_bool)
                    == Some(true)
        })
        .filter_map(|record| {
            record
                .payload
                .get("invocationId")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect::<BTreeSet<_>>();

    records
        .iter()
        .filter(|record| {
            record.kind == "capability.invocation.request"
                && record.payload.get("capability").and_then(Value::as_str)
                    == Some(MEMORY_INDEX_CAPABILITY)
                && record
                    .payload
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| successful_invocations.contains(id))
        })
        .filter_map(|record| {
            record
                .payload
                .get("input")
                .and_then(Value::as_object)
                .and_then(|input| input.get("date"))
                .and_then(Value::as_str)
                .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
        })
        .collect()
}

fn is_memory_index_event_kind(kind: &str) -> bool {
    kind.starts_with("conversation.")
        || kind == "dialogue.say"
        || kind == "app.startup"
        || kind.starts_with("device.")
        || kind.starts_with("avatar.gesture.")
}

pub(crate) fn strip_references_from_payload(payload: &mut JsonMap) {
    for value in payload.values_mut() {
        strip_references_from_value(value);
    }
}

fn strip_references_from_value(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if object.contains_key("uri") || object.contains_key("permissionRef") {
                *value = Value::Null;
                return;
            }
            for nested in object.values_mut() {
                strip_references_from_value(nested);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_references_from_value(item);
            }
        }
        _ => {}
    }
}
