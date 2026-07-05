use std::collections::BTreeMap;

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use uuid::Uuid;

pub type JsonMap = BTreeMap<String, Value>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardSignalDefinition {
    pub canonical_id: &'static str,
    pub daihon_alias: &'static str,
    pub display_label: &'static str,
}

const STANDARD_SIGNAL_DEFINITIONS: &[StandardSignalDefinition] = &[
    StandardSignalDefinition {
        canonical_id: "conversation.text",
        daihon_alias: "会話_入力",
        display_label: "会話入力",
    },
    StandardSignalDefinition {
        canonical_id: "surface.attach",
        daihon_alias: "画面_接続",
        display_label: "画面接続",
    },
    StandardSignalDefinition {
        canonical_id: "app.startup",
        daihon_alias: "アプリ_起動",
        display_label: "アプリ起動",
    },
    StandardSignalDefinition {
        canonical_id: "presence.life_tick",
        daihon_alias: "生活_定期",
        display_label: "生活定期",
    },
    StandardSignalDefinition {
        canonical_id: "presence.time_period",
        daihon_alias: "時間帯_変化",
        display_label: "時間帯変化",
    },
    StandardSignalDefinition {
        canonical_id: "device.sleep.before",
        daihon_alias: "端末_スリープ前",
        display_label: "端末スリープ前",
    },
    StandardSignalDefinition {
        canonical_id: "device.wake",
        daihon_alias: "端末_復帰",
        display_label: "端末復帰",
    },
    StandardSignalDefinition {
        canonical_id: "avatar.gesture.poke",
        daihon_alias: "住人_つつく",
        display_label: "住人つつき",
    },
    StandardSignalDefinition {
        canonical_id: "avatar.gesture.pat",
        daihon_alias: "住人_なでる",
        display_label: "住人なで",
    },
];

pub fn standard_signal_definitions() -> &'static [StandardSignalDefinition] {
    STANDARD_SIGNAL_DEFINITIONS
}

pub fn canonical_signal_id(signal: &str) -> &str {
    let trimmed = signal.trim();
    standard_signal_definitions()
        .iter()
        .find(|definition| definition.canonical_id == trimmed || definition.daihon_alias == trimmed)
        .map(|definition| definition.canonical_id)
        .unwrap_or(trimmed)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SignalAliasTable {
    aliases: BTreeMap<String, String>,
}

impl SignalAliasTable {
    pub fn new(aliases: impl IntoIterator<Item = (String, String)>) -> Self {
        let aliases = aliases
            .into_iter()
            .map(|(alias, canonical_id)| {
                (alias.trim().to_string(), canonical_id.trim().to_string())
            })
            .filter(|(alias, canonical_id)| !alias.is_empty() && !canonical_id.is_empty())
            .collect();
        Self { aliases }
    }

    pub fn with_standard_and_donated(
        aliases: impl IntoIterator<Item = ExtensionSignalAlias>,
    ) -> Self {
        let mut table = BTreeMap::new();
        for alias in aliases {
            table.insert(
                alias.alias.trim().to_string(),
                alias.signal.trim().to_string(),
            );
        }
        Self::new(table)
    }

    pub fn canonicalize<'a>(&'a self, signal: &'a str) -> String {
        let standard = canonical_signal_id(signal);
        if standard != signal.trim() {
            return standard.to_string();
        }
        self.aliases
            .get(signal.trim())
            .cloned()
            .unwrap_or_else(|| signal.trim().to_string())
    }
}

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4())
}

pub fn now_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
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
    pub extension_readable: bool,
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
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

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub enum ExtensionHookPoint {
    BeforeCommandEmit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ExtensionHookSubscription {
    pub hook_point: ExtensionHookPoint,
    #[serde(default)]
    pub command_types: Vec<String>,
}

impl ExtensionHookSubscription {
    pub fn matches_command(&self, hook_point: &ExtensionHookPoint, command_kind: &str) -> bool {
        &self.hook_point == hook_point
            && (self.command_types.is_empty()
                || self
                    .command_types
                    .iter()
                    .any(|declared| declared == command_kind))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub enum ExtensionRuntimeKind {
    Process,
    Bundled,
    Wasm,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ExtensionPermissions {
    #[serde(default)]
    pub broad_event_subscription: bool,
    #[serde(default)]
    pub event_log_read: Option<ExtensionEventLogReadPermission>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ExtensionEventLogReadPermission {
    #[serde(default)]
    pub event_types: Vec<String>,
    #[serde(default)]
    pub privacy_categories: Vec<String>,
    pub allow_payloads: bool,
    pub allow_references: bool,
    pub max_records: usize,
    pub purpose: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ExtensionEventSubscription {
    #[serde(default)]
    pub event_types: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ExtensionCapabilityDeclaration {
    pub capability: String,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub required_permissions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ExtensionSignalAlias {
    pub alias: String,
    pub signal: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ExtensionSummary {
    pub extension_id: String,
    pub display_name: String,
    pub runtime: ExtensionRuntimeKind,
    pub permissions: ExtensionPermissions,
    pub hooks: Vec<ExtensionHookSubscription>,
    pub event_subscriptions: Vec<ExtensionEventSubscription>,
    pub emitted_events: Vec<String>,
    pub capabilities: Vec<ExtensionCapabilityDeclaration>,
    pub signal_aliases: Vec<ExtensionSignalAlias>,
    pub location: ExecutionLocation,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ExtensionHookInvocation {
    pub id: String,
    pub hook_point: ExtensionHookPoint,
    pub extension_id: String,
    pub resident_id: String,
    pub world_pack_id: String,
    pub command: RuntimeCommand,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub enum ExtensionHookAction {
    Unchanged,
    ReplaceCommand,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ExtensionHookResult {
    pub action: ExtensionHookAction,
    #[ts(optional)]
    pub command: Option<RuntimeCommand>,
    #[ts(type = "{ [key: string]: unknown }", optional)]
    pub metadata: Option<JsonMap>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ExtensionEventInvocation {
    pub id: String,
    pub extension_id: String,
    pub resident_id: String,
    pub world_pack_id: String,
    pub event: EventLogRecord,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct ExtensionEventResult {
    #[serde(default)]
    pub proposed_events: Vec<RuntimeEvent>,
    #[ts(type = "{ [key: string]: unknown }", optional)]
    pub metadata: Option<JsonMap>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub enum SurfaceKind {
    Cli,
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
    Terminal,
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
pub enum ExtensionHealth {
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
pub struct CapabilityRouteSummary {
    pub extension_id: String,
    pub capabilities: Vec<String>,
    pub location: ExecutionLocation,
    pub health: ExtensionHealth,
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
pub struct DialogueGenerateInput {
    pub event: DialogueGenerateEvent,
    pub persona: DialogueGeneratePersona,
    #[serde(default)]
    pub recent_context: Vec<DialogueGenerateRecentContext>,
    pub constraints: DialogueGenerateConstraints,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct DialogueGenerateEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[ts(type = "{ [key: string]: unknown }")]
    pub payload: JsonMap,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct DialogueGeneratePersona {
    pub actor_id: String,
    pub display_name: String,
    #[ts(type = "{ [key: string]: unknown }")]
    pub profile: JsonMap,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct DialogueGenerateRecentContext {
    pub kind: String,
    pub timestamp: String,
    #[ts(type = "{ [key: string]: unknown }")]
    pub payload: JsonMap,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct DialogueGenerateConstraints {
    pub max_length: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct DialogueGenerateOutput {
    pub speak: bool,
    #[ts(optional)]
    pub text: Option<String>,
    #[ts(optional)]
    pub expression: Option<String>,
    #[ts(optional)]
    pub motion: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct DialogueInterpretInput {
    pub question: String,
    pub choices: Vec<String>,
    pub input: DialogueInterpretTextInput,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct DialogueInterpretTextInput {
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../packages/yuukei-protocol/src/generated/")]
pub struct DialogueInterpretOutput {
    pub choice: String,
    #[ts(optional)]
    pub confidence: Option<f64>,
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
    pub capabilities: BTreeMap<String, CapabilityRouteSummary>,
    pub extensions: BTreeMap<String, ExtensionSummary>,
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
    fn standard_signals_resolve_daihon_aliases_to_canonical_ids() {
        assert_eq!(canonical_signal_id("会話_入力"), "conversation.text");
        assert_eq!(canonical_signal_id("生活_定期"), "presence.life_tick");
        assert_eq!(canonical_signal_id("端末_復帰"), "device.wake");
        assert_eq!(canonical_signal_id("住人_つつく"), "avatar.gesture.poke");
        assert_eq!(canonical_signal_id("住人_なでる"), "avatar.gesture.pat");
        assert_eq!(canonical_signal_id(" device.wake "), "device.wake");
        assert_eq!(canonical_signal_id("pack.custom"), "pack.custom");
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

    #[test]
    fn extension_hook_uses_protocol_field_names() -> anyhow::Result<()> {
        let command = RuntimeCommand::new("dialogue.say", "daihon", "resident-default");
        let invocation = ExtensionHookInvocation {
            id: "hook_1".to_string(),
            hook_point: ExtensionHookPoint::BeforeCommandEmit,
            extension_id: "nya-suffix".to_string(),
            resident_id: "resident-default".to_string(),
            world_pack_id: "default-yuukei".to_string(),
            command,
        };

        let value = serde_json::to_value(&invocation)?;
        assert_eq!(value["hookPoint"], "beforeCommandEmit");
        assert_eq!(value["extensionId"], "nya-suffix");
        assert_eq!(value["command"]["type"], "dialogue.say");
        Ok(())
    }
}
