use super::*;

impl ResidentHome {
    pub(crate) fn record_capability_request(
        &self,
        invocation: &CapabilityInvocation,
        source_event: &RuntimeEvent,
        source_command_id: Option<&str>,
    ) -> Result<()> {
        let request_payload = serde_json::to_value(invocation)?;
        let request = NewEventLogRecord {
            id: new_id("evt"),
            kind: "capability.invocation.request".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id: invocation.resident_id.clone(),
            source: "resident-home".to_string(),
            device_id: source_event.device_id.clone(),
            surface_id: source_event.surface_id.clone(),
            actor_id: invocation.actor_id.clone(),
            payload: json_map_from_value(request_payload),
            causality: Some(Causality {
                source_event_id: Some(source_event.id.clone()),
                source_command_id: source_command_id.map(ToOwned::to_owned),
                trace_id: source_event
                    .causality
                    .as_ref()
                    .and_then(|causality| causality.trace_id.clone()),
            }),
            privacy: None,
        };
        let appended_request = self.event_log.append(request)?;
        self.set_cursor(appended_request.sequence)?;
        Ok(())
    }

    pub(crate) fn record_capability_result(
        &self,
        record: CapabilityResultRecord<'_>,
    ) -> Result<()> {
        let result_payload = JsonMap::from([
            (
                "invocationId".to_string(),
                Value::String(record.invocation_id),
            ),
            (
                "extensionId".to_string(),
                Value::String(record.extension_id),
            ),
            ("capability".to_string(), Value::String(record.capability)),
            (
                "output".to_string(),
                Value::Object(record.output.into_iter().collect()),
            ),
            (
                "metadata".to_string(),
                Value::Object(record.metadata.into_iter().collect()),
            ),
        ]);
        let source_event = record.source_event;
        let result_record = NewEventLogRecord {
            id: new_id("evt"),
            kind: "capability.invocation.result".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id: source_event.resident_id.clone(),
            source: "capability".to_string(),
            device_id: source_event.device_id.clone(),
            surface_id: source_event.surface_id.clone(),
            actor_id: record.actor_id.map(ToOwned::to_owned),
            payload: result_payload,
            causality: Some(Causality {
                source_event_id: Some(source_event.id.clone()),
                source_command_id: record.source_command_id.map(ToOwned::to_owned),
                trace_id: source_event
                    .causality
                    .as_ref()
                    .and_then(|causality| causality.trace_id.clone()),
            }),
            privacy: None,
        };
        let appended_result = self.event_log.append(result_record)?;
        self.set_cursor(appended_result.sequence)?;
        Ok(())
    }
}

pub(crate) struct CapabilityResultRecord<'a> {
    pub(crate) invocation_id: String,
    pub(crate) extension_id: String,
    pub(crate) capability: String,
    pub(crate) output: JsonMap,
    pub(crate) metadata: JsonMap,
    pub(crate) source_event: &'a RuntimeEvent,
    pub(crate) source_command_id: Option<&'a str>,
    pub(crate) actor_id: Option<&'a str>,
}

pub(crate) fn major_payload(payload: JsonMap) -> JsonMap {
    const KEYS: &[&str] = &[
        "text",
        "speakerId",
        "emotion",
        "expression",
        "motion",
        "anchor",
        "button",
        "hitZoneId",
        "hitZoneLabel",
        "hitSurface",
        "movedDistance",
        "timePeriod",
        "localHour",
        "localMinute",
        "sourceCapability",
    ];
    payload
        .into_iter()
        .filter(|(key, value)| KEYS.contains(&key.as_str()) && is_small_context_value(value))
        .collect()
}

fn is_small_context_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}
