use std::collections::BTreeMap;

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use uuid::Uuid;

pub type JsonMap = BTreeMap<String, Value>;

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4())
}

pub fn now_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct Causality {
    #[ts(optional)]
    pub source_event_id: Option<String>,
    #[ts(optional)]
    pub source_command_id: Option<String>,
    #[ts(optional)]
    pub trace_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub enum RetentionPolicy {
    Session,
    Short,
    Long,
    Manual,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct Privacy {
    pub category: String,
    pub retention: RetentionPolicy,
    pub provider_readable: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ContentReference {
    pub uri: String,
    #[ts(optional)]
    pub content_type: Option<String>,
    #[ts(optional)]
    pub digest: Option<String>,
    #[ts(optional)]
    pub size_bytes: Option<u64>,
    #[ts(optional)]
    pub permission_ref: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct RuntimeEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub timestamp: String,
    pub source: String,
    pub resident_id: String,
    #[ts(type = "{ [key: string]: unknown }")]
    pub payload: JsonMap,
    #[ts(optional)]
    pub causality: Option<Causality>,
    #[ts(optional)]
    pub device_id: Option<String>,
    #[ts(optional)]
    pub surface_id: Option<String>,
    #[ts(optional)]
    pub actor_id: Option<String>,
}

impl RuntimeEvent {
    pub fn new(
        kind: impl Into<String>,
        source: impl Into<String>,
        resident_id: impl Into<String>,
    ) -> Self {
        Self {
            id: new_id("evt"),
            kind: kind.into(),
            timestamp: now_timestamp(),
            source: source.into(),
            resident_id: resident_id.into(),
            payload: JsonMap::new(),
            causality: None,
            device_id: None,
            surface_id: None,
            actor_id: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct CommandTarget {
    #[ts(optional)]
    pub device_id: Option<String>,
    #[ts(optional)]
    pub surface_id: Option<String>,
    #[ts(optional)]
    pub actor_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct RuntimeCommand {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub timestamp: String,
    pub source: String,
    pub resident_id: String,
    #[ts(type = "{ [key: string]: unknown }")]
    pub payload: JsonMap,
    #[ts(optional)]
    pub causality: Option<Causality>,
    #[ts(optional)]
    pub target: Option<CommandTarget>,
}

impl RuntimeCommand {
    pub fn new(
        kind: impl Into<String>,
        source: impl Into<String>,
        resident_id: impl Into<String>,
    ) -> Self {
        Self {
            id: new_id("cmd"),
            kind: kind.into(),
            timestamp: now_timestamp(),
            source: source.into(),
            resident_id: resident_id.into(),
            payload: JsonMap::new(),
            causality: None,
            target: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub enum SurfaceKind {
    Desktop,
    Mobile,
    Widget,
    Overlay,
    Effect,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub enum SurfaceRenderer {
    Vrm,
    Live2d,
    Sprite,
    Html,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct SurfacePresentation {
    #[ts(optional)]
    pub renderer: Option<SurfaceRenderer>,
    #[ts(optional)]
    pub transparent: Option<bool>,
    #[ts(optional)]
    pub accepts_input: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct SurfaceSession {
    pub surface_id: String,
    pub device_id: String,
    pub kind: SurfaceKind,
    pub active: bool,
    pub capabilities: Vec<String>,
    pub presentation: SurfacePresentation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub enum ProviderHealth {
    Unknown,
    Ready,
    Degraded,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub enum ExecutionLocation {
    ResidentHome,
    DeviceHost,
    Remote,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct CapabilityProviderSummary {
    pub provider_id: String,
    pub capabilities: Vec<String>,
    pub location: ExecutionLocation,
    pub health: ProviderHealth,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct CapabilityContext {
    #[serde(default)]
    pub event_ids: Vec<String>,
    #[ts(type = "unknown", optional)]
    pub memory_hints: Option<Value>,
    #[ts(type = "unknown", optional)]
    pub actor_profile: Option<Value>,
    #[ts(type = "unknown", optional)]
    pub device: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct CapabilityInvocation {
    pub id: String,
    pub capability: String,
    pub method: String,
    pub resident_id: String,
    #[ts(optional)]
    pub actor_id: Option<String>,
    #[ts(type = "{ [key: string]: unknown }")]
    pub input: JsonMap,
    #[ts(optional)]
    pub context: Option<CapabilityContext>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ActorSnapshot {
    pub display_name: String,
    pub expression: String,
    pub motion: String,
    pub location: String,
    #[ts(optional)]
    pub speaking: Option<bool>,
    #[ts(optional)]
    pub bubble: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ResidentSnapshot {
    pub resident_id: String,
    pub world_pack_id: String,
    #[ts(optional)]
    pub active_surface_id: Option<String>,
    pub actors: BTreeMap<String, ActorSnapshot>,
    pub surfaces: BTreeMap<String, SurfaceSession>,
    pub capabilities: BTreeMap<String, CapabilityProviderSummary>,
    pub recent_event_cursor: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct EventLogRecord {
    pub sequence: i64,
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub timestamp: String,
    pub resident_id: String,
    pub source: String,
    #[ts(optional)]
    pub device_id: Option<String>,
    #[ts(optional)]
    pub surface_id: Option<String>,
    #[ts(optional)]
    pub actor_id: Option<String>,
    #[ts(type = "{ [key: string]: unknown }")]
    pub payload: JsonMap,
    #[ts(optional)]
    pub causality: Option<Causality>,
    #[ts(optional)]
    pub privacy: Option<Privacy>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewEventLogRecord {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub timestamp: String,
    pub resident_id: String,
    pub source: String,
    pub device_id: Option<String>,
    pub surface_id: Option<String>,
    pub actor_id: Option<String>,
    pub payload: JsonMap,
    pub causality: Option<Causality>,
    pub privacy: Option<Privacy>,
}

impl From<RuntimeEvent> for NewEventLogRecord {
    fn from(event: RuntimeEvent) -> Self {
        Self {
            id: event.id,
            kind: event.kind,
            timestamp: event.timestamp,
            resident_id: event.resident_id,
            source: event.source,
            device_id: event.device_id,
            surface_id: event.surface_id,
            actor_id: event.actor_id,
            payload: event.payload,
            causality: event.causality,
            privacy: None,
        }
    }
}

impl From<RuntimeCommand> for NewEventLogRecord {
    fn from(command: RuntimeCommand) -> Self {
        let (device_id, surface_id, actor_id) = command
            .target
            .map(|target| (target.device_id, target.surface_id, target.actor_id))
            .unwrap_or_default();
        Self {
            id: command.id,
            kind: command.kind,
            timestamp: command.timestamp,
            resident_id: command.resident_id,
            source: command.source,
            device_id,
            surface_id,
            actor_id,
            payload: command.payload,
            causality: command.causality,
            privacy: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn runtime_event_uses_protocol_field_names() -> anyhow::Result<()> {
        let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
        event.id = "evt_1".to_string();
        event.timestamp = "2026-06-25T00:00:00.000Z".to_string();
        event.device_id = Some("device-local".to_string());
        event.surface_id = Some("surface-main".to_string());
        event.payload.insert("text".to_string(), json!("hello"));

        let value = serde_json::to_value(&event)?;
        assert_eq!(value["type"], "conversation.text");
        assert_eq!(value["residentId"], "resident-default");
        assert_eq!(value["deviceId"], "device-local");
        assert!(value.get("kind").is_none());
        Ok(())
    }

    #[test]
    fn event_log_record_keeps_required_fields() -> anyhow::Result<()> {
        let record = EventLogRecord {
            sequence: 7,
            id: "cmd_1".to_string(),
            kind: "dialogue.say".to_string(),
            timestamp: "2026-06-25T00:00:00.000Z".to_string(),
            resident_id: "resident-default".to_string(),
            source: "daihon".to_string(),
            device_id: None,
            surface_id: Some("surface-main".to_string()),
            actor_id: Some("actor-yuukei".to_string()),
            payload: JsonMap::from([("text".to_string(), json!("hi"))]),
            causality: Some(Causality {
                source_event_id: Some("evt_1".to_string()),
                source_command_id: None,
                trace_id: Some("trace_1".to_string()),
            }),
            privacy: None,
        };

        let value = serde_json::to_value(&record)?;
        assert_eq!(value["sequence"], 7);
        assert_eq!(value["type"], "dialogue.say");
        assert_eq!(value["causality"]["sourceEventId"], "evt_1");
        Ok(())
    }
}
