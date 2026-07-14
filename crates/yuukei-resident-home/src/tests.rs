use std::{
    fs,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use tempfile::tempdir;
use yuukei_capability::{CapabilityError, CapabilityResult, ProviderRegistration};
use yuukei_event_log::EventLogQuery;
use yuukei_extension::{
    DialogueSuffixExtension, ProcessCommandSpec, ProcessExtensionManifest, ProcessHookExtension,
    YuukeiExtension,
};
use yuukei_protocol::{
    ExecutionLocation, ExtensionEventInvocation, ExtensionEventLogReadPermission,
    ExtensionEventResult, ExtensionEventSubscription, ExtensionHookAction, ExtensionHookInvocation,
    ExtensionHookPoint, ExtensionHookResult, ExtensionHookSubscription, ExtensionPermissions,
    ExtensionRuntimeKind, ExtensionSignalAlias, ExtensionSummary, Privacy, RetentionPolicy,
    SurfaceKind, SurfacePresentation, SurfaceRenderer,
};
use yuukei_world::{
    ActorDefinition, CapabilityDeclarations, DaihonConfig, DaihonScriptSource, LlmDelegation,
    LlmDelegationSignal, SignalAllowlist,
};

use super::*;

fn world_pack() -> WorldPack {
    WorldPack {
        schema_version: 1,
        id: "default-yuukei".to_string(),
        display_name: "Default Yuukei".to_string(),
        default_actor_id: "yuukei".to_string(),
        actors: vec![ActorDefinition {
            id: "yuukei".to_string(),
            display_name: "Yuukei".to_string(),
            speaker_aliases: Vec::new(),
            profile: JsonMap::new(),
            renderer: None,
        }],
        signals: SignalAllowlist {
            allow: vec![
                "conversation.text".to_string(),
                "surface.attach".to_string(),
            ],
        },
        capabilities: CapabilityDeclarations {
            required: Vec::new(),
            optional: vec!["speech.synthesis".to_string()],
        },
        llm_delegation: LlmDelegation::default(),
        daihon: DaihonConfig {
            scripts: vec!["scripts/reactions.daihon".to_string()],
            loaded_scripts: vec![DaihonScriptSource {
                path: "scripts/reactions.daihon".to_string(),
                source: r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: yuukei
ユーザー発言=入力#ユーザー発言
＜表情 笑顔＞
「聞こえています。＜ユーザー発言＞」
"#
                .to_string(),
            }],
        },
        initial_variables: JsonMap::new(),
        ui_space: JsonMap::new(),
    }
}

fn future_timestamp() -> String {
    (Utc::now() + Duration::days(1)).to_rfc3339()
}

#[derive(Clone)]
struct EventEmitterExtension {
    extension_id: String,
    subscriptions: Vec<String>,
    emitted_events: Vec<String>,
    proposed_kind: Option<String>,
    proposed_event: Option<RuntimeEvent>,
    broad_event_subscription: bool,
    event_log_read: Option<ExtensionEventLogReadPermission>,
    signal_aliases: Vec<ExtensionSignalAlias>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl EventEmitterExtension {
    fn new(extension_id: &str) -> Self {
        Self {
            extension_id: extension_id.to_string(),
            subscriptions: Vec::new(),
            emitted_events: Vec::new(),
            proposed_kind: None,
            proposed_event: None,
            broad_event_subscription: false,
            event_log_read: None,
            signal_aliases: Vec::new(),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn subscribed_to(mut self, event_types: impl IntoIterator<Item = &'static str>) -> Self {
        self.subscriptions = event_types.into_iter().map(ToOwned::to_owned).collect();
        self
    }

    fn emits(mut self, event_types: impl IntoIterator<Item = &'static str>) -> Self {
        self.emitted_events = event_types.into_iter().map(ToOwned::to_owned).collect();
        self
    }

    fn proposes(mut self, event_type: &str) -> Self {
        self.proposed_kind = Some(event_type.to_string());
        self
    }

    fn proposes_event(mut self, event: RuntimeEvent) -> Self {
        self.proposed_event = Some(event);
        self
    }

    fn with_broad_event_subscription(mut self) -> Self {
        self.broad_event_subscription = true;
        self
    }

    fn with_event_log_read(mut self, permission: ExtensionEventLogReadPermission) -> Self {
        self.event_log_read = Some(permission);
        self
    }

    fn with_signal_alias(mut self, alias: &str, signal: &str) -> Self {
        self.signal_aliases.push(ExtensionSignalAlias {
            alias: alias.to_string(),
            signal: signal.to_string(),
        });
        self
    }

    fn calls(&self) -> Arc<Mutex<Vec<String>>> {
        self.calls.clone()
    }
}

#[async_trait]
impl YuukeiExtension for EventEmitterExtension {
    fn registration(&self) -> ExtensionSummary {
        ExtensionSummary {
            extension_id: self.extension_id.clone(),
            display_name: self.extension_id.clone(),
            runtime: ExtensionRuntimeKind::Bundled,
            permissions: ExtensionPermissions {
                broad_event_subscription: self.broad_event_subscription,
                event_log_read: self.event_log_read.clone(),
            },
            hooks: Vec::new(),
            event_subscriptions: if self.subscriptions.is_empty() {
                Vec::new()
            } else {
                vec![ExtensionEventSubscription {
                    event_types: self.subscriptions.clone(),
                }]
            },
            emitted_events: self.emitted_events.clone(),
            capabilities: Vec::new(),
            signal_aliases: self.signal_aliases.clone(),
            location: ExecutionLocation::ResidentHome,
            enabled: true,
        }
    }

    async fn invoke(
        &self,
        _invocation: ExtensionHookInvocation,
    ) -> yuukei_extension::Result<ExtensionHookResult> {
        Ok(ExtensionHookResult {
            action: ExtensionHookAction::Unchanged,
            command: None,
            metadata: None,
        })
    }

    async fn on_event_appended(
        &self,
        invocation: ExtensionEventInvocation,
    ) -> yuukei_extension::Result<ExtensionEventResult> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(invocation.event.kind.clone());
        let mut proposed_events = Vec::new();
        if let Some(event) = &self.proposed_event {
            proposed_events.push(event.clone());
        } else if let Some(kind) = &self.proposed_kind {
            proposed_events.push(RuntimeEvent::new(
                kind,
                "extension",
                invocation.resident_id.clone(),
            ));
        }
        Ok(ExtensionEventResult {
            proposed_events,
            metadata: None,
        })
    }
}

#[derive(Clone)]
struct DialogueGenerateProvider {
    output: JsonMap,
    calls: Arc<Mutex<Vec<CapabilityInvocation>>>,
    delay: Option<std::time::Duration>,
}

impl DialogueGenerateProvider {
    fn new(output: JsonMap) -> Self {
        Self {
            output,
            calls: Arc::new(Mutex::new(Vec::new())),
            delay: None,
        }
    }

    fn delayed(mut self, delay: std::time::Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    fn calls(&self) -> Arc<Mutex<Vec<CapabilityInvocation>>> {
        self.calls.clone()
    }
}

#[async_trait]
impl CapabilityProvider for DialogueGenerateProvider {
    fn registration(&self) -> ProviderRegistration {
        ProviderRegistration {
            extension_id: "fake-dialogue".to_string(),
            capabilities: vec![DIALOGUE_GENERATE_CAPABILITY.to_string()],
            methods: vec!["generate".to_string()],
            required_permissions: Vec::new(),
            location: ExecutionLocation::ResidentHome,
            health: yuukei_protocol::ExtensionHealth::Ready,
            enabled: true,
            config_schema: JsonMap::new(),
            runtime_settings: JsonMap::new(),
        }
    }

    async fn invoke(
        &self,
        invocation: CapabilityInvocation,
    ) -> yuukei_capability::Result<CapabilityResult> {
        self.calls
            .lock()
            .expect("dialogue generate calls lock")
            .push(invocation.clone());
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        Ok(CapabilityResult {
            invocation_id: invocation.id,
            extension_id: "fake-dialogue".to_string(),
            capability: DIALOGUE_GENERATE_CAPABILITY.to_string(),
            output: self.output.clone(),
            metadata: JsonMap::new(),
        })
    }
}

#[derive(Clone)]
struct SpeechSynthesisProvider {
    output: JsonMap,
    calls: Arc<Mutex<Vec<CapabilityInvocation>>>,
    fail: bool,
}

impl SpeechSynthesisProvider {
    fn new(output: JsonMap) -> Self {
        Self {
            output,
            calls: Arc::new(Mutex::new(Vec::new())),
            fail: false,
        }
    }

    fn failing(mut self) -> Self {
        self.fail = true;
        self
    }

    fn calls(&self) -> Arc<Mutex<Vec<CapabilityInvocation>>> {
        self.calls.clone()
    }
}

#[async_trait]
impl CapabilityProvider for SpeechSynthesisProvider {
    fn registration(&self) -> ProviderRegistration {
        ProviderRegistration {
            extension_id: "fake-speech".to_string(),
            capabilities: vec![SPEECH_SYNTHESIS_CAPABILITY.to_string()],
            methods: vec!["synthesize".to_string()],
            required_permissions: Vec::new(),
            location: ExecutionLocation::ResidentHome,
            health: yuukei_protocol::ExtensionHealth::Ready,
            enabled: true,
            config_schema: JsonMap::new(),
            runtime_settings: JsonMap::new(),
        }
    }

    async fn invoke(
        &self,
        invocation: CapabilityInvocation,
    ) -> yuukei_capability::Result<CapabilityResult> {
        self.calls
            .lock()
            .expect("speech calls lock")
            .push(invocation.clone());
        if self.fail {
            return Err(CapabilityError::Extension("speech failed".to_string()));
        }
        Ok(CapabilityResult {
            invocation_id: invocation.id,
            extension_id: "fake-speech".to_string(),
            capability: SPEECH_SYNTHESIS_CAPABILITY.to_string(),
            output: self.output.clone(),
            metadata: JsonMap::new(),
        })
    }
}

#[derive(Clone)]
struct DialogueInterpretProvider {
    output: JsonMap,
    calls: Arc<Mutex<Vec<CapabilityInvocation>>>,
    delay: Option<std::time::Duration>,
}

impl DialogueInterpretProvider {
    fn new(output: JsonMap) -> Self {
        Self {
            output,
            calls: Arc::new(Mutex::new(Vec::new())),
            delay: None,
        }
    }

    fn delayed(mut self, delay: std::time::Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    fn calls(&self) -> Arc<Mutex<Vec<CapabilityInvocation>>> {
        self.calls.clone()
    }
}

#[async_trait]
impl CapabilityProvider for DialogueInterpretProvider {
    fn registration(&self) -> ProviderRegistration {
        ProviderRegistration {
            extension_id: "fake-interpret".to_string(),
            capabilities: vec![DIALOGUE_INTERPRET_CAPABILITY.to_string()],
            methods: vec!["interpret".to_string()],
            required_permissions: Vec::new(),
            location: ExecutionLocation::ResidentHome,
            health: yuukei_protocol::ExtensionHealth::Ready,
            enabled: true,
            config_schema: JsonMap::new(),
            runtime_settings: JsonMap::new(),
        }
    }

    async fn invoke(
        &self,
        invocation: CapabilityInvocation,
    ) -> yuukei_capability::Result<CapabilityResult> {
        self.calls
            .lock()
            .expect("dialogue interpret calls lock")
            .push(invocation.clone());
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        Ok(CapabilityResult {
            invocation_id: invocation.id,
            extension_id: "fake-interpret".to_string(),
            capability: DIALOGUE_INTERPRET_CAPABILITY.to_string(),
            output: self.output.clone(),
            metadata: JsonMap::new(),
        })
    }
}

#[derive(Clone)]
struct MemoryProvider {
    retrieve_output: JsonMap,
    calls: Arc<Mutex<Vec<CapabilityInvocation>>>,
    fail_retrieve: bool,
}

impl MemoryProvider {
    fn new(retrieve_output: JsonMap) -> Self {
        Self {
            retrieve_output,
            calls: Arc::new(Mutex::new(Vec::new())),
            fail_retrieve: false,
        }
    }

    fn failing_retrieve(mut self) -> Self {
        self.fail_retrieve = true;
        self
    }

    fn calls(&self) -> Arc<Mutex<Vec<CapabilityInvocation>>> {
        self.calls.clone()
    }
}

#[async_trait]
impl CapabilityProvider for MemoryProvider {
    fn registration(&self) -> ProviderRegistration {
        ProviderRegistration {
            extension_id: "fake-memory".to_string(),
            capabilities: vec![
                MEMORY_INDEX_CAPABILITY.to_string(),
                MEMORY_LIST_CAPABILITY.to_string(),
                MEMORY_RETRIEVE_CAPABILITY.to_string(),
                MEMORY_UPDATE_CAPABILITY.to_string(),
                MEMORY_FORGET_CAPABILITY.to_string(),
            ],
            methods: vec![
                "index".to_string(),
                "list".to_string(),
                "retrieve".to_string(),
                "update".to_string(),
                "forget".to_string(),
            ],
            required_permissions: Vec::new(),
            location: ExecutionLocation::ResidentHome,
            health: yuukei_protocol::ExtensionHealth::Ready,
            enabled: true,
            config_schema: JsonMap::new(),
            runtime_settings: JsonMap::new(),
        }
    }

    async fn invoke(
        &self,
        invocation: CapabilityInvocation,
    ) -> yuukei_capability::Result<CapabilityResult> {
        self.calls
            .lock()
            .expect("memory calls lock")
            .push(invocation.clone());
        if invocation.capability == MEMORY_RETRIEVE_CAPABILITY && self.fail_retrieve {
            return Err(CapabilityError::Extension("retrieve failed".to_string()));
        }
        let output = match invocation.capability.as_str() {
            MEMORY_INDEX_CAPABILITY => JsonMap::from([
                ("indexed".to_string(), json!(true)),
                ("noteCount".to_string(), json!(1)),
            ]),
            MEMORY_UPDATE_CAPABILITY => JsonMap::from([("updated".to_string(), json!(true))]),
            MEMORY_FORGET_CAPABILITY => JsonMap::from([
                ("removedFacts".to_string(), json!(1)),
                ("removedEpisodes".to_string(), json!(1)),
            ]),
            _ => self.retrieve_output.clone(),
        };
        Ok(CapabilityResult {
            invocation_id: invocation.id,
            extension_id: "fake-memory".to_string(),
            capability: invocation.capability,
            output,
            metadata: JsonMap::new(),
        })
    }
}

#[derive(Clone)]
struct MoodProvider {
    output: JsonMap,
    calls: Arc<Mutex<Vec<CapabilityInvocation>>>,
    runtime_settings: JsonMap,
    fail: bool,
}

impl MoodProvider {
    fn new(output: JsonMap) -> Self {
        Self {
            output,
            calls: Arc::new(Mutex::new(Vec::new())),
            runtime_settings: JsonMap::new(),
            fail: false,
        }
    }

    fn with_runtime_settings(mut self, runtime_settings: JsonMap) -> Self {
        self.runtime_settings = runtime_settings;
        self
    }

    fn failing(mut self) -> Self {
        self.fail = true;
        self
    }

    fn calls(&self) -> Arc<Mutex<Vec<CapabilityInvocation>>> {
        self.calls.clone()
    }
}

#[async_trait]
impl CapabilityProvider for MoodProvider {
    fn registration(&self) -> ProviderRegistration {
        ProviderRegistration {
            extension_id: "yuukei-intelligence".to_string(),
            capabilities: vec![MOOD_EVALUATE_CAPABILITY.to_string()],
            methods: vec!["evaluate".to_string()],
            required_permissions: Vec::new(),
            location: ExecutionLocation::ResidentHome,
            health: yuukei_protocol::ExtensionHealth::Ready,
            enabled: true,
            config_schema: JsonMap::new(),
            runtime_settings: self.runtime_settings.clone(),
        }
    }

    async fn invoke(
        &self,
        invocation: CapabilityInvocation,
    ) -> yuukei_capability::Result<CapabilityResult> {
        self.calls
            .lock()
            .expect("mood calls lock")
            .push(invocation.clone());
        if self.fail {
            return Err(CapabilityError::Extension("mood failed".to_string()));
        }
        Ok(CapabilityResult {
            invocation_id: invocation.id,
            extension_id: "yuukei-intelligence".to_string(),
            capability: MOOD_EVALUATE_CAPABILITY.to_string(),
            output: self.output.clone(),
            metadata: JsonMap::new(),
        })
    }
}

fn llm_fallback_world() -> WorldPack {
    let mut world = world_pack();
    world.daihon.loaded_scripts[0].source = r#"
## desktop reactions
### attach
合図: ＠surface.attach
話者: yuukei
「ここにいます。」
"#
    .to_string();
    world.llm_delegation = LlmDelegation {
        signals: vec![LlmDelegationSignal {
            signal: "conversation.text".to_string(),
            cooldown_seconds: Some(60),
        }],
        daily_budget: None,
    };
    world
}

fn conversation_ai_connected_world() -> WorldPack {
    let mut world = world_pack();
    world.actors[0].speaker_aliases = vec!["ゆ".to_string()];
    world.actors.push(ActorDefinition {
        id: "partner".to_string(),
        display_name: "Partner".to_string(),
        speaker_aliases: vec!["パ".to_string()],
        profile: JsonMap::new(),
        renderer: None,
    });
    world.daihon.loaded_scripts[0].source = r#"
## 会話_入力

### AIなしの相槌1
合図: ＠会話_入力
条件:（入力#AI接続 = いいえ）
頻度: 30秒に1回
話者: ゆ
「ん、聞いてます。……いまは、うまく言葉が出ないんですけど。」

### AIなしの相槌2
合図: ＠会話_入力
条件:（入力#AI接続 = いいえ）
頻度: 30秒に1回
話者: パ
「……(こくり)」
"#
    .to_string();
    world.llm_delegation = LlmDelegation {
        signals: vec![LlmDelegationSignal {
            signal: "conversation.text".to_string(),
            cooldown_seconds: Some(60),
        }],
        daily_budget: None,
    };
    world
}

fn random_talk_world() -> WorldPack {
    let mut world = world_pack();
    world.signals.allow = vec![
        TALK_IMPULSE_EVENT.to_string(),
        "presence.life_tick".to_string(),
    ];
    world.daihon.loaded_scripts[0].source = r#"
## 雑談
### normal
合図: ＠雑談_定期
条件:（入力#気分 = 「ふつう」）
話者: yuukei
「ふつうに話します。」

### lonely
合図: ＠雑談_定期
条件:（入力#気分 = 「さみしい」）
話者: yuukei
「少し静かですね。」
"#
    .to_string();
    world
}

fn folder_download_world() -> WorldPack {
    let mut world = world_pack();
    world.signals.allow = vec!["desktop.folder.opened".to_string()];
    world.daihon.loaded_scripts[0].source = r#"
## desktop folders
### recent download
合図: ＠フォルダ_開いた
条件:（入力#最近のダウンロード = 「photo.png」）
話者: yuukei
「さっきのphoto.pngだね。」

### no recent download
合図: ＠フォルダ_開いた
条件:（入力#最近のダウンロード = 「」）
話者: yuukei
「最近のダウンロードはありません。」
"#
    .to_string();
    world
}

fn yuukei_intelligence_events_extension() -> EventEmitterExtension {
    EventEmitterExtension::new("yuukei-intelligence")
        .emits([MOOD_CHANGED_EVENT])
        .with_signal_alias("気分_変化", MOOD_CHANGED_EVENT)
}

fn interpret_world() -> WorldPack {
    let mut world = world_pack();
    world.daihon.loaded_scripts[0].source = r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: yuukei
判定=＜解釈 (入力#ユーザー発言) 「返事は肯定ですか？」 「はい/いいえ」＞
※（判定 = 「はい」）なら:
「はい枝」
※あるいは（判定 = 「不明」）なら:
「不明枝」
※それ以外:
「いいえ枝」
おわり
### device
合図: ＠device.wake
話者: yuukei
「起きました」
"#
    .to_string();
    world.signals.allow.push("device.wake".to_string());
    world
}

fn choice_world(timeout_seconds: u64) -> WorldPack {
    let mut world = world_pack();
    world.daihon.loaded_scripts[0].source = format!(
        r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: yuukei
「見る？」
返事=＜選択 「見る」 「あとで」 秒数={timeout_seconds}＞
※（返事 = 「見る」）なら:
「見る枝」
※あるいは（返事 = 「不明」）なら:
「不明枝」
※それ以外:
「あとで枝」
おわり
"#
    );
    world
}

fn choice_queue_world() -> WorldPack {
    let mut world = world_pack();
    world.daihon.loaded_scripts[0].source = r#"
## desktop reactions
### choice
合図: ＠conversation.text
条件:（入力#ユーザー発言 = 「start」）
話者: yuukei
返事=＜選択 「見る」 「あとで」 秒数=30＞
※（返事 = 「見る」）なら:
「見る枝」
※あるいは（返事 = 「不明」）なら:
「不明枝」
※それ以外:
「あとで枝」
おわり
### queued
合図: ＠conversation.text
条件:（入力#ユーザー発言 = 「queued」）
話者: yuukei
「queued handled」
"#
    .to_string();
    world
}

async fn next_command_of_kind(
    receiver: &mut broadcast::Receiver<RuntimeCommand>,
    kind: &str,
) -> RuntimeCommand {
    loop {
        let command = tokio::time::timeout(std::time::Duration::from_secs(10), receiver.recv())
            .await
            .expect("command broadcast timed out")
            .expect("command broadcast closed");
        if command.kind == kind {
            return command;
        }
    }
}

fn generate_world(script: &str) -> WorldPack {
    let mut world = world_pack();
    world.daihon.loaded_scripts[0].source = script.to_string();
    world
}

#[tokio::test]
async fn headless_text_event_is_logged_and_broadcasts_dialogue() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    let mut receiver = home.subscribe_commands();
    home.attach_surface(SurfaceSession {
        surface_id: "surface-main".to_string(),
        device_id: "device-local".to_string(),
        kind: SurfaceKind::Desktop,
        active: true,
        capabilities: vec!["dialogue.say".to_string()],
        presentation: SurfacePresentation {
            renderer: Some(SurfaceRenderer::Html),
            transparent: Some(false),
            accepts_input: Some(true),
        },
    })
    .await?;

    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.id = "evt_text".to_string();
    event.device_id = Some("device-local".to_string());
    event.surface_id = Some("surface-main".to_string());
    event
        .payload
        .insert("text".to_string(), json!("こんにちは"));

    let commands = home.ingest_event(event).await?;
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].kind, "avatar.expression");
    assert_eq!(commands[1].kind, "dialogue.say");
    assert!(!commands[1].payload.contains_key("speechRef"));
    let expression = receiver.recv().await.expect("expression broadcast");
    assert_eq!(expression.kind, "avatar.expression");
    let dialogue = receiver.recv().await.expect("dialogue broadcast");
    assert_eq!(dialogue.kind, "dialogue.say");

    let records = home
        .event_log()
        .read(EventLogQuery::default())?
        .records
        .into_iter()
        .map(|record| record.kind)
        .collect::<Vec<_>>();
    assert!(records.contains(&"conversation.text".to_string()));
    assert!(records.contains(&"daihon.dispatch.result".to_string()));
    assert!(records.contains(&"avatar.expression".to_string()));
    assert!(records.contains(&"dialogue.say".to_string()));
    assert!(!records.contains(&"audio.play".to_string()));
    assert!(!records.contains(&"capability.invocation.request".to_string()));
    assert!(!records.contains(&"capability.invocation.result".to_string()));

    let snapshot = home.snapshot()?;
    assert_eq!(snapshot.active_surface_id.as_deref(), Some("surface-main"));
    assert_eq!(snapshot.actors["yuukei"].expression, "笑顔");
    assert_eq!(
        snapshot.actors["yuukei"].bubble.as_deref(),
        Some("聞こえています。こんにちは")
    );
    Ok(())
}

#[tokio::test]
async fn stage_walk_command_and_ended_event_update_actor_snapshot() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    let mut command = RuntimeCommand::new("stage.walk", "daihon", "resident-default");
    command.target = Some(CommandTarget {
        device_id: None,
        surface_id: None,
        actor_id: Some("yuukei".to_string()),
    });
    command.payload = JsonMap::from([
        ("destination".to_string(), json!("right-edge")),
        ("motion".to_string(), json!("walk")),
    ]);

    home.emit_internal_command_without_extensions(command)?;

    let walking = home.snapshot()?;
    assert_eq!(walking.actors["yuukei"].motion, "walk");
    assert_eq!(walking.actors["yuukei"].heading, "right");

    let mut ended = RuntimeEvent::new("stage.walk.ended", "device", "resident-default");
    ended.actor_id = Some("yuukei".to_string());
    ended.payload.insert("reason".to_string(), json!("arrived"));
    home.ingest_event(ended).await?;

    let stopped = home.snapshot()?;
    assert_eq!(stopped.actors["yuukei"].motion, "");
    assert_eq!(stopped.actors["yuukei"].heading, "");
    Ok(())
}

#[tokio::test]
async fn actor_location_exit_and_enter_commands_update_snapshot_atomically() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    let actor_command = |kind: &str, location: Option<&str>| {
        let mut command = RuntimeCommand::new(kind, "daihon", "resident-default");
        command.target = Some(CommandTarget {
            device_id: None,
            surface_id: None,
            actor_id: Some("yuukei".to_string()),
        });
        if let Some(location) = location {
            command
                .payload
                .insert("location".to_string(), json!(location));
        }
        command
    };

    home.emit_internal_command_without_extensions(actor_command(
        "actor.location.set",
        Some("pictures"),
    ))?;
    let located = home.snapshot()?;
    assert_eq!(located.actors["yuukei"].location, "pictures");
    assert_eq!(located.actors["yuukei"].presence, ActorPresence::Present);

    home.emit_internal_command_without_extensions(actor_command("actor.exit", Some("downloads")))?;
    let away = home.snapshot()?;
    assert_eq!(away.actors["yuukei"].location, "downloads");
    assert_eq!(away.actors["yuukei"].presence, ActorPresence::Away);

    home.emit_internal_command_without_extensions(actor_command("actor.enter", None))?;
    let returned = home.snapshot()?;
    assert_eq!(returned.actors["yuukei"].location, "downloads");
    assert_eq!(returned.actors["yuukei"].presence, ActorPresence::Present);
    Ok(())
}

#[tokio::test]
async fn actor_location_and_presence_restore_from_canonical_event_log() -> Result<()> {
    let event_log = EventLog::in_memory()?;
    let home = ResidentHome::new("resident-default", world_pack(), event_log.clone()).await?;
    let mut command = RuntimeCommand::new("actor.exit", "daihon", "resident-default");
    command.target = Some(CommandTarget {
        device_id: None,
        surface_id: None,
        actor_id: Some("yuukei".to_string()),
    });
    command
        .payload
        .insert("location".to_string(), json!("downloads"));
    home.emit_internal_command_without_extensions(command)?;
    drop(home);

    let reopened = ResidentHome::new("resident-default", world_pack(), event_log).await?;
    let snapshot = reopened.snapshot()?;
    assert_eq!(snapshot.actors["yuukei"].location, "downloads");
    assert_eq!(snapshot.actors["yuukei"].presence, ActorPresence::Away);
    Ok(())
}

#[tokio::test]
async fn daihon_dispatch_context_includes_target_actor_location_and_presence() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    let mut exit = RuntimeCommand::new("actor.exit", "daihon", "resident-default");
    exit.target = Some(CommandTarget {
        device_id: None,
        surface_id: None,
        actor_id: Some("yuukei".to_string()),
    });
    exit.payload
        .insert("location".to_string(), json!("downloads"));
    home.emit_internal_command_without_extensions(exit)?;

    let event = RuntimeEvent::new("desktop.folder.opened", "device", "resident-default");
    let enriched = home.enrich_event_for_daihon_dispatch(event)?;

    assert_eq!(enriched.payload["actorLocation"], "downloads");
    assert_eq!(enriched.payload["actorPresence"], "away");
    Ok(())
}

#[tokio::test]
async fn replaced_walk_end_does_not_clear_newer_walk_snapshot() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    for (id, destination, motion) in [
        ("walk-1", "right-edge", "walk"),
        ("walk-2", "left-edge", "歩く"),
    ] {
        let mut command = RuntimeCommand::new("stage.walk", "daihon", "resident-default");
        command.id = id.to_string();
        command.target = Some(CommandTarget {
            device_id: None,
            surface_id: None,
            actor_id: Some("yuukei".to_string()),
        });
        command.payload = JsonMap::from([
            ("destination".to_string(), json!(destination)),
            ("motion".to_string(), json!(motion)),
        ]);
        home.emit_internal_command_without_extensions(command)?;
    }
    let mut ended = RuntimeEvent::new("stage.walk.ended", "device", "resident-default");
    ended.actor_id = Some("yuukei".to_string());
    ended
        .payload
        .insert("reason".to_string(), json!("replaced"));
    ended.causality = Some(Causality {
        source_event_id: None,
        source_command_id: Some("walk-1".to_string()),
        trace_id: None,
    });

    home.ingest_event(ended).await?;

    let snapshot = home.snapshot()?;
    assert_eq!(snapshot.actors["yuukei"].motion, "歩く");
    assert_eq!(snapshot.actors["yuukei"].heading, "left");
    Ok(())
}

#[tokio::test]
async fn speech_synthesis_success_emits_audio_play_after_dialogue() -> Result<()> {
    let provider = SpeechSynthesisProvider::new(JsonMap::from([
        (
            "audioPath".to_string(),
            json!("/tmp/yuukei-voicevox/cmd_1.wav"),
        ),
        ("durationMs".to_string(), json!(1234)),
        ("format".to_string(), json!("wav")),
    ]));
    let calls = provider.calls();
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(provider)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        world_pack(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;
    let mut receiver = home.subscribe_commands();
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.id = "evt_speech".to_string();
    event
        .payload
        .insert("text".to_string(), json!("こんにちは"));

    let commands = home.ingest_event(event).await?;
    let dialogue = commands
        .iter()
        .find(|command| command.kind == "dialogue.say")
        .expect("dialogue command");
    assert_eq!(dialogue.payload["text"], "聞こえています。こんにちは");
    assert_eq!(dialogue.payload["speechPending"], true);
    let broadcast_dialogue = next_command_of_kind(&mut receiver, "dialogue.say").await;
    assert_eq!(broadcast_dialogue.id, dialogue.id);
    assert_eq!(broadcast_dialogue.payload["speechPending"], true);
    let audio = next_command_of_kind(&mut receiver, "audio.play").await;
    assert_eq!(audio.source, "capability");
    assert_eq!(audio.payload["audioPath"], "/tmp/yuukei-voicevox/cmd_1.wav");
    assert_eq!(audio.payload["durationMs"], 1234);
    assert_eq!(
        audio
            .causality
            .as_ref()
            .and_then(|causality| causality.source_command_id.as_deref()),
        Some(dialogue.id.as_str())
    );

    let calls = calls.lock().expect("speech calls lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].capability, SPEECH_SYNTHESIS_CAPABILITY);
    assert_eq!(calls[0].method, "synthesize");
    assert_eq!(calls[0].input["text"], "聞こえています。こんにちは");
    assert_eq!(calls[0].input["speakerId"], "yuukei");
    assert_eq!(calls[0].input["displayCommandId"], dialogue.id);

    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert!(records.iter().any(|record| record.kind == "audio.play"));
    assert!(records.iter().any(|record| {
        record.kind == "dialogue.say" && record.payload["speechPending"] == true
    }));
    assert!(records
        .iter()
        .any(|record| record.kind == "capability.invocation.request"
            && record.payload["capability"] == SPEECH_SYNTHESIS_CAPABILITY));
    assert!(records
        .iter()
        .any(|record| record.kind == "capability.invocation.result"
            && record.payload["capability"] == SPEECH_SYNTHESIS_CAPABILITY));
    Ok(())
}

#[tokio::test]
async fn speech_synthesis_route_absent_keeps_dialogue_without_audio() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    let mut receiver = home.subscribe_commands();
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event
        .payload
        .insert("text".to_string(), json!("こんにちは"));

    let commands = home.ingest_event(event).await?;
    assert!(commands
        .iter()
        .any(|command| command.kind == "dialogue.say"));
    let dialogue = next_command_of_kind(&mut receiver, "dialogue.say").await;
    assert_eq!(dialogue.payload["text"], "聞こえています。こんにちは");
    assert!(!dialogue.payload.contains_key("speechPending"));
    let no_audio = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        next_command_of_kind(&mut receiver, "audio.play"),
    )
    .await;
    assert!(no_audio.is_err());

    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert!(!records.iter().any(|record| record.kind == "audio.play"));
    assert!(!records
        .iter()
        .any(|record| record.kind == "capability.invocation.request"
            && record.payload["capability"] == SPEECH_SYNTHESIS_CAPABILITY));
    Ok(())
}

#[tokio::test]
async fn speech_synthesis_does_not_mark_empty_dialogue_as_pending() -> Result<()> {
    let provider = SpeechSynthesisProvider::new(JsonMap::new());
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(provider)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        world_pack(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;
    let mut command = RuntimeCommand::new("dialogue.say", "daihon", "resident-default");
    command.payload.insert("text".to_string(), json!(" \t "));
    let event = RuntimeEvent::new("conversation.text", "surface", "resident-default");

    let emitted = home.emit_command_for_event(command, &event).await?;

    assert!(!emitted.payload.contains_key("speechPending"));
    Ok(())
}

#[tokio::test]
async fn speech_synthesis_failure_keeps_dialogue_without_audio() -> Result<()> {
    let provider = SpeechSynthesisProvider::new(JsonMap::new()).failing();
    let calls = provider.calls();
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(provider)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        world_pack(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;
    let mut receiver = home.subscribe_commands();
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event
        .payload
        .insert("text".to_string(), json!("こんにちは"));

    let commands = home.ingest_event(event).await?;
    assert!(commands
        .iter()
        .any(|command| command.kind == "dialogue.say"));
    let _dialogue = next_command_of_kind(&mut receiver, "dialogue.say").await;
    let no_audio = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        next_command_of_kind(&mut receiver, "audio.play"),
    )
    .await;
    assert!(no_audio.is_err());
    assert_eq!(calls.lock().expect("speech calls lock").len(), 1);

    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert!(!records.iter().any(|record| record.kind == "audio.play"));
    assert!(records
        .iter()
        .any(|record| record.kind == "capability.invocation.request"
            && record.payload["capability"] == SPEECH_SYNTHESIS_CAPABILITY));
    assert!(!records
        .iter()
        .any(|record| record.kind == "capability.invocation.result"
            && record.payload["capability"] == SPEECH_SYNTHESIS_CAPABILITY));
    Ok(())
}

#[tokio::test]
async fn dialogue_interpret_choice_drives_daihon_branch() -> Result<()> {
    let provider =
        DialogueInterpretProvider::new(JsonMap::from([("choice".to_string(), json!("はい"))]));
    let calls = provider.calls();
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(provider)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        interpret_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event
        .payload
        .insert("text".to_string(), json!("うん、いいよ"));

    let commands = home.ingest_event(event).await?;
    let dialogue = commands
        .iter()
        .find(|command| command.kind == "dialogue.say")
        .expect("dialogue command");
    assert_eq!(dialogue.payload["text"], "はい枝");
    let calls = calls.lock().expect("interpret calls lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].capability, DIALOGUE_INTERPRET_CAPABILITY);
    assert_eq!(calls[0].input["question"], "返事は肯定ですか？");
    assert_eq!(calls[0].input["choices"], json!(["はい", "いいえ"]));
    assert_eq!(calls[0].input["input"]["text"], "うん、いいよ");
    Ok(())
}

#[tokio::test]
async fn missing_dialogue_interpret_provider_falls_to_unknown_branch() -> Result<()> {
    let home = ResidentHome::new(
        "resident-default",
        interpret_world(),
        EventLog::in_memory()?,
    )
    .await?;
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.payload.insert("text".to_string(), json!("曖昧"));

    let commands = home.ingest_event(event).await?;
    let dialogue = commands
        .iter()
        .find(|command| command.kind == "dialogue.say")
        .expect("dialogue command");
    assert_eq!(dialogue.payload["text"], "不明枝");
    Ok(())
}

#[tokio::test]
async fn dialogue_interpret_out_of_choice_output_is_normalized_to_unknown() -> Result<()> {
    let provider =
        DialogueInterpretProvider::new(JsonMap::from([("choice".to_string(), json!("maybe"))]));
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(provider)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        interpret_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.payload.insert("text".to_string(), json!("曖昧"));

    let commands = home.ingest_event(event).await?;
    let dialogue = commands
        .iter()
        .find(|command| command.kind == "dialogue.say")
        .expect("dialogue command");
    assert_eq!(dialogue.payload["text"], "不明枝");
    Ok(())
}

#[tokio::test]
async fn conversation_events_are_queued_while_dialogue_interpret_is_in_flight() -> Result<()> {
    let provider =
        DialogueInterpretProvider::new(JsonMap::from([("choice".to_string(), json!("はい"))]))
            .delayed(std::time::Duration::from_millis(80));
    let calls = provider.calls();
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(provider)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        interpret_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;

    let mut first = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    first.id = "evt_first".to_string();
    first.payload.insert("text".to_string(), json!("うん"));
    let first_home = home.clone();
    let first_task = tokio::spawn(async move { first_home.ingest_event(first).await });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let mut second = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    second.id = "evt_second".to_string();
    second.payload.insert("text".to_string(), json!("うん2"));
    let second_commands = home.ingest_event(second).await?;
    assert!(second_commands.is_empty());

    let first_commands = first_task.await.expect("first ingest task")?;
    let dialogue_texts = first_commands
        .iter()
        .filter(|command| command.kind == "dialogue.say")
        .map(|command| command.payload["text"].clone())
        .collect::<Vec<_>>();
    assert_eq!(dialogue_texts, vec![json!("はい枝"), json!("はい枝")]);
    assert_eq!(calls.lock().expect("interpret calls lock").len(), 2);
    Ok(())
}

#[tokio::test]
async fn non_conversation_events_are_record_only_while_dialogue_interpret_is_in_flight(
) -> Result<()> {
    let provider =
        DialogueInterpretProvider::new(JsonMap::from([("choice".to_string(), json!("はい"))]))
            .delayed(std::time::Duration::from_millis(80));
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(provider)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        interpret_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;

    let mut conversation = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    conversation
        .payload
        .insert("text".to_string(), json!("うん"));
    let first_home = home.clone();
    let first_task = tokio::spawn(async move { first_home.ingest_event(conversation).await });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let wake = RuntimeEvent::new("device.wake", "device", "resident-default");
    let wake_commands = home.ingest_event(wake).await?;
    assert!(wake_commands.is_empty());

    let first_commands = first_task.await.expect("first ingest task")?;
    assert!(first_commands.iter().all(|command| {
        command.kind != "dialogue.say" || command.payload["text"] != json!("起きました")
    }));
    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert!(records.iter().any(|record| record.kind == "device.wake"));
    assert!(!records.iter().any(|record| {
        record.kind == "dialogue.say" && record.payload["text"] == json!("起きました")
    }));
    Ok(())
}

#[tokio::test]
async fn memory_index_runs_for_unindexed_previous_day_on_app_startup() -> Result<()> {
    let memory = MemoryProvider::new(JsonMap::from([("memories".to_string(), json!([]))]));
    let calls = memory.calls();
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(memory)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        world_pack(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;

    let mut yesterday = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    yesterday.timestamp = (Utc::now() - Duration::days(1)).to_rfc3339();
    yesterday
        .payload
        .insert("text".to_string(), json!("昨日の話"));
    home.ingest_event(yesterday).await?;

    let startup = RuntimeEvent::new("app.startup", "device", "resident-default");
    home.ingest_event(startup).await?;

    let calls = calls.lock().expect("memory calls lock");
    let index_calls = calls
        .iter()
        .filter(|call| call.capability == MEMORY_INDEX_CAPABILITY)
        .collect::<Vec<_>>();
    assert_eq!(index_calls.len(), 1);
    assert_eq!(
        index_calls[0].input["date"],
        (Utc::now() - Duration::days(1)).date_naive().to_string()
    );
    assert_eq!(index_calls[0].input["residentId"], "resident-default");
    assert_eq!(index_calls[0].input["worldPackId"], "default-yuukei");
    assert!(index_calls[0].input["events"]
        .as_array()
        .expect("events array")
        .iter()
        .any(|event| event["type"] == json!("conversation.text")
            || event["kind"] == json!("conversation.text")));
    assert!(index_calls[0].input["events"]
        .as_array()
        .expect("events array")
        .iter()
        .any(|event| event["payload"]["text"] == json!("昨日の話")));
    Ok(())
}

#[tokio::test]
async fn memory_index_does_not_repeat_after_successful_result() -> Result<()> {
    let memory = MemoryProvider::new(JsonMap::from([("memories".to_string(), json!([]))]));
    let calls = memory.calls();
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(memory)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        world_pack(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;

    let mut yesterday = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    yesterday.timestamp = (Utc::now() - Duration::days(1)).to_rfc3339();
    yesterday
        .payload
        .insert("text".to_string(), json!("昨日の話"));
    home.ingest_event(yesterday).await?;

    home.ingest_event(RuntimeEvent::new(
        "app.startup",
        "device",
        "resident-default",
    ))
    .await?;
    home.ingest_event(RuntimeEvent::new(
        "app.startup",
        "device",
        "resident-default",
    ))
    .await?;

    let calls = calls.lock().expect("memory calls lock");
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.capability == MEMORY_INDEX_CAPABILITY)
            .count(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn dialogue_generate_statement_sends_instruction_and_emits_generated_commands() -> Result<()>
{
    let provider = DialogueGenerateProvider::new(JsonMap::from([
        ("speak".to_string(), json!(true)),
        ("text".to_string(), json!("早く行きたいな。")),
        ("expression".to_string(), json!("sparkle")),
        ("motion".to_string(), json!("bounce")),
    ]));
    let calls = provider.calls();
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(provider)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        generate_world(
            r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: yuukei
「やった、楽しみにしてるね。」
＜生成 「お出かけの日の楽しみを一言」 「楽しみだなあ」＞
"#,
        ),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.id = "evt_scene_generate".to_string();
    event.payload.insert("text".to_string(), json!("うん"));

    let commands = home.ingest_event(event).await?;

    assert_eq!(commands.len(), 4);
    assert_eq!(commands[0].kind, "dialogue.say");
    assert_eq!(commands[0].payload["text"], "やった、楽しみにしてるね。");
    assert_eq!(commands[1].kind, "avatar.expression");
    assert_eq!(commands[1].payload["expression"], "sparkle");
    assert_eq!(commands[2].kind, "avatar.motion");
    assert_eq!(commands[2].payload["motion"], "bounce");
    assert_eq!(commands[3].kind, "dialogue.say");
    assert_eq!(commands[3].payload["text"], "早く行きたいな。");
    assert_eq!(commands[3].source, "capability.dialogue.generate");
    assert_eq!(commands[3].payload["sourceFunction"], "生成");
    assert_eq!(
        commands[3].payload["generationInstruction"],
        "お出かけの日の楽しみを一言"
    );

    let calls = calls.lock().expect("generate calls lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].capability, DIALOGUE_GENERATE_CAPABILITY);
    assert_eq!(calls[0].input["instruction"], "お出かけの日の楽しみを一言");
    assert_eq!(calls[0].input["persona"]["actorId"], "yuukei");

    let records = home.event_log().read(EventLogQuery::default())?.records;
    let generated = records
        .iter()
        .find(|record| {
            record.kind == "dialogue.say"
                && record.source == "capability.dialogue.generate"
                && record.payload["text"] == json!("早く行きたいな。")
        })
        .expect("generated dialogue record");
    assert_eq!(generated.payload["sourceFunction"], "生成");
    assert_eq!(
        generated
            .causality
            .as_ref()
            .and_then(|causality| causality.source_event_id.as_deref()),
        Some("evt_scene_generate")
    );
    Ok(())
}

#[tokio::test]
async fn dialogue_generate_statement_speak_false_uses_fallback_or_skips() -> Result<()> {
    let provider =
        DialogueGenerateProvider::new(JsonMap::from([("speak".to_string(), json!(false))]));
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(provider)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        generate_world(
            r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: yuukei
＜生成 「一言目」 「フォールバック」＞
＜生成 「二言目」＞
「続き」
"#,
        ),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.payload.insert("text".to_string(), json!("うん"));

    let commands = home.ingest_event(event).await?;
    let dialogue_texts = commands
        .iter()
        .filter(|command| command.kind == "dialogue.say")
        .map(|command| command.payload["text"].clone())
        .collect::<Vec<_>>();
    assert_eq!(dialogue_texts, vec![json!("フォールバック"), json!("続き")]);
    Ok(())
}

#[tokio::test]
async fn conversation_events_are_queued_while_dialogue_generate_is_in_flight() -> Result<()> {
    let provider = DialogueGenerateProvider::new(JsonMap::from([
        ("speak".to_string(), json!(true)),
        ("text".to_string(), json!("生成応答")),
    ]))
    .delayed(std::time::Duration::from_millis(80));
    let calls = provider.calls();
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(provider)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        generate_world(
            r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: yuukei
＜生成 「短く返す」 「fallback」＞
"#,
        ),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;

    let mut first = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    first.id = "evt_generate_first".to_string();
    first.payload.insert("text".to_string(), json!("一つ目"));
    let first_home = home.clone();
    let first_task = tokio::spawn(async move { first_home.ingest_event(first).await });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let mut second = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    second.id = "evt_generate_second".to_string();
    second.payload.insert("text".to_string(), json!("二つ目"));
    let second_commands = home.ingest_event(second).await?;
    assert!(second_commands.is_empty());

    let first_commands = first_task.await.expect("first ingest task")?;
    let dialogue_texts = first_commands
        .iter()
        .filter(|command| command.kind == "dialogue.say")
        .map(|command| command.payload["text"].clone())
        .collect::<Vec<_>>();
    assert_eq!(dialogue_texts, vec![json!("生成応答"), json!("生成応答")]);
    assert_eq!(calls.lock().expect("generate calls lock").len(), 2);
    Ok(())
}

#[tokio::test]
async fn speaker_alias_dialogue_updates_canonical_actor_snapshot() -> Result<()> {
    let mut world = world_pack();
    world.actors[0].speaker_aliases = vec!["ゆ".to_string()];
    world.actors.push(ActorDefinition {
        id: "partner".to_string(),
        display_name: "Partner".to_string(),
        speaker_aliases: vec!["パ".to_string()],
        profile: JsonMap::new(),
        renderer: None,
    });
    world.daihon.loaded_scripts[0].source = r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: ゆ
パ: 「短い名で話します。」
"#
    .to_string();
    let home = ResidentHome::new("resident-default", world, EventLog::in_memory()?).await?;
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.id = "evt_speaker_alias".to_string();
    event
        .payload
        .insert("text".to_string(), json!("こんにちは"));

    let commands = home.ingest_event(event).await?;
    let dialogue = commands
        .iter()
        .find(|command| command.kind == "dialogue.say")
        .expect("dialogue command");
    assert_eq!(
        dialogue
            .target
            .as_ref()
            .and_then(|target| target.actor_id.as_deref()),
        Some("partner")
    );
    assert_eq!(dialogue.payload["speakerId"], "partner");

    let snapshot = home.snapshot()?;
    assert_eq!(
        snapshot.actors["partner"].bubble.as_deref(),
        Some("短い名で話します。")
    );
    assert!(snapshot.actors["yuukei"].bubble.is_none());
    Ok(())
}

#[tokio::test]
async fn disallowed_signal_is_logged_but_not_dispatched() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    let event = RuntimeEvent::new("os.file_browser.focused", "device", "resident-default");
    let commands = home.ingest_event(event).await?;
    assert!(commands.is_empty());
    assert_eq!(
        home.event_log()
            .read(EventLogQuery::default())?
            .records
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn declared_signal_without_daihon_result_generates_dialogue() -> Result<()> {
    let home = ResidentHome::new(
        "resident-default",
        llm_fallback_world(),
        EventLog::in_memory()?,
    )
    .await?;
    let provider = DialogueGenerateProvider::new(JsonMap::from([
        ("speak".to_string(), json!(true)),
        ("text".to_string(), json!("少しだけ返します。")),
        ("expression".to_string(), json!("smile")),
        ("motion".to_string(), json!("nod")),
    ]));
    let calls = provider.calls();
    home.register_provider(provider)?;
    let mut receiver = home.subscribe_commands();

    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.id = "evt_generate".to_string();
    event.surface_id = Some("surface-main".to_string());
    event.payload.insert("text".to_string(), json!("ねえ"));
    let commands = home.ingest_event(event).await?;

    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0].kind, "avatar.expression");
    assert_eq!(commands[1].kind, "avatar.motion");
    assert_eq!(commands[2].kind, "dialogue.say");
    assert_eq!(commands[2].payload["text"], "少しだけ返します。");
    assert_eq!(commands[2].source, "capability.dialogue.generate");
    assert_eq!(
        calls.lock().expect("calls lock")[0].input["persona"]["displayName"],
        "Yuukei"
    );
    assert_eq!(
        receiver.recv().await.expect("expression broadcast").kind,
        "avatar.expression"
    );
    assert_eq!(
        receiver.recv().await.expect("motion broadcast").kind,
        "avatar.motion"
    );
    assert_eq!(
        receiver.recv().await.expect("dialogue broadcast").kind,
        "dialogue.say"
    );

    let records = home.event_log().read(EventLogQuery::default())?.records;
    let dialogue = records
        .iter()
        .find(|record| record.kind == "dialogue.say")
        .expect("generated dialogue record");
    assert_eq!(dialogue.source, "capability.dialogue.generate");
    assert_eq!(
        dialogue
            .causality
            .as_ref()
            .and_then(|causality| causality.source_event_id.as_deref()),
        Some("evt_generate")
    );
    assert!(records
        .iter()
        .any(|record| record.kind == "capability.invocation.request"
            && record.payload["capability"] == DIALOGUE_GENERATE_CAPABILITY));
    assert!(records
        .iter()
        .any(|record| record.kind == "capability.invocation.result"
            && record.payload["capability"] == DIALOGUE_GENERATE_CAPABILITY));
    Ok(())
}

#[tokio::test]
async fn conversation_without_ai_route_uses_daihon_acknowledgement() -> Result<()> {
    let home = ResidentHome::new(
        "resident-default",
        conversation_ai_connected_world(),
        EventLog::in_memory()?,
    )
    .await?;

    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.id = "evt_no_ai_ack".to_string();
    event.payload.insert("text".to_string(), json!("ねえ"));
    let commands = home.ingest_event(event).await?;

    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].kind, "dialogue.say");
    let text = commands[0].payload["text"].as_str().unwrap_or_default();
    assert!(
        text == "ん、聞いてます。……いまは、うまく言葉が出ないんですけど。" || text == "……(こくり)"
    );
    let records = home.event_log().read(EventLogQuery::default())?.records;
    let dispatch = records
        .iter()
        .find(|record| record.kind == "daihon.dispatch.result")
        .expect("daihon dispatch result");
    assert!(dispatch.payload["executedScenes"]
        .as_array()
        .is_some_and(|scenes| !scenes.is_empty()));
    Ok(())
}

#[tokio::test]
async fn conversation_with_ai_route_skips_acknowledgement_and_delegates() -> Result<()> {
    let provider = DialogueGenerateProvider::new(JsonMap::from([
        ("speak".to_string(), json!(true)),
        ("text".to_string(), json!("AIで返します。")),
    ]));
    let calls = provider.calls();
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(provider)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        conversation_ai_connected_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;

    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.id = "evt_ai_connected".to_string();
    event.payload.insert("text".to_string(), json!("ねえ"));
    let commands = home.ingest_event(event).await?;

    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].kind, "dialogue.say");
    assert_eq!(commands[0].payload["text"], "AIで返します。");
    assert_eq!(calls.lock().expect("calls lock").len(), 1);
    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert!(!records
        .iter()
        .any(|record| record.kind == "daihon.dispatch.result"));
    Ok(())
}

#[tokio::test]
async fn dialogue_generate_uses_configured_recent_context_count() -> Result<()> {
    let provider = DialogueGenerateProvider::new(JsonMap::from([
        ("speak".to_string(), json!(true)),
        ("text".to_string(), json!("文脈つきです。")),
    ]));
    let calls = provider.calls();
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(provider)?;
    let home = ResidentHome::with_parts_and_runtime_settings(
        "resident-default",
        llm_fallback_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
        ResidentRuntimeSettings {
            llm_timeout: std::time::Duration::from_secs(30),
            recent_context_count: 2,
            talk_desire_low: 30,
            talk_desire_high: 80,
            mood_state_path: None,
        },
    )
    .await?;

    for index in 0..3 {
        let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
        event.id = format!("evt_context_{index}");
        event.payload.insert("text".to_string(), json!(index));
        home.event_log().append(event.into())?;
    }
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.id = "evt_generate_recent_context".to_string();
    event.payload.insert("text".to_string(), json!("ねえ"));
    home.ingest_event(event).await?;

    let calls = calls.lock().expect("calls lock");
    let recent_context = calls[0].input["recentContext"]
        .as_array()
        .expect("recent context array");
    assert_eq!(recent_context.len(), 2);
    assert_eq!(recent_context[0]["payload"]["text"], json!(2));
    assert_eq!(recent_context[1]["payload"]["text"], json!("ねえ"));
    Ok(())
}

#[tokio::test]
async fn dialogue_generate_input_includes_retrieved_memories() -> Result<()> {
    let memory = MemoryProvider::new(JsonMap::from([(
        "memories".to_string(),
        json!([
            { "text": "唐揚げが好き。", "kind": "fact" },
            { "text": "昨日は公園へ行った。", "kind": "episode", "date": "2026-01-01" }
        ]),
    )]));
    let memory_calls = memory.calls();
    let dialogue = DialogueGenerateProvider::new(JsonMap::from([
        ("speak".to_string(), json!(true)),
        ("text".to_string(), json!("覚えています。")),
    ]));
    let dialogue_calls = dialogue.calls();
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(memory)?;
    capabilities.register(dialogue)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        llm_fallback_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;

    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event
        .payload
        .insert("text".to_string(), json!("唐揚げの話"));
    let commands = home.ingest_event(event).await?;

    assert!(commands
        .iter()
        .any(|command| command.payload["text"] == json!("覚えています。")));
    let memory_calls = memory_calls.lock().expect("memory calls lock");
    let retrieve = memory_calls
        .iter()
        .find(|call| call.capability == MEMORY_RETRIEVE_CAPABILITY)
        .expect("memory retrieve call");
    assert_eq!(retrieve.input["query"]["text"], "唐揚げの話");
    assert_eq!(retrieve.input["limits"]["facts"], 10);
    assert_eq!(retrieve.input["limits"]["episodes"], 5);
    let dialogue_calls = dialogue_calls.lock().expect("dialogue calls lock");
    assert_eq!(
        dialogue_calls[0].input["memories"],
        json!(["唐揚げが好き。", "昨日は公園へ行った。"])
    );
    Ok(())
}

#[tokio::test]
async fn memory_retrieve_failure_does_not_block_dialogue_generate() -> Result<()> {
    let memory = MemoryProvider::new(JsonMap::new()).failing_retrieve();
    let dialogue = DialogueGenerateProvider::new(JsonMap::from([
        ("speak".to_string(), json!(true)),
        ("text".to_string(), json!("記憶なしでも返します。")),
    ]));
    let dialogue_calls = dialogue.calls();
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(memory)?;
    capabilities.register(dialogue)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        llm_fallback_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;

    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event
        .payload
        .insert("text".to_string(), json!("覚えてる？"));
    let commands = home.ingest_event(event).await?;

    assert!(commands
        .iter()
        .any(|command| command.payload["text"] == json!("記憶なしでも返します。")));
    let dialogue_calls = dialogue_calls.lock().expect("dialogue calls lock");
    assert!(!dialogue_calls[0].input.contains_key("memories"));
    Ok(())
}

#[tokio::test]
async fn memory_admin_invokes_router_round_trip() -> Result<()> {
    let memory = MemoryProvider::new(JsonMap::from([
        (
            "facts".to_string(),
            json!([
                {
                    "id": "fact-1",
                    "text": "唐揚げが好き。",
                    "createdAt": "2026-06-25T00:00:00.000Z",
                    "updatedAt": "2026-06-25T00:00:00.000Z"
                }
            ]),
        ),
        (
            "episodes".to_string(),
            json!([
                {
                    "id": "episode-1",
                    "text": "公園へ行った。",
                    "timestamp": "2026-06-25T00:00:00.000Z"
                }
            ]),
        ),
        ("episodeTotal".to_string(), json!(1)),
    ]));
    let calls = memory.calls();
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(memory)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        llm_fallback_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;

    let listed = home.list_memories(Some(10), Some(0)).await?;
    assert_eq!(listed.facts[0].id, "fact-1");
    assert_eq!(listed.episodes[0].id, "episode-1");
    assert!(
        home.update_memory(MemoryEntryKind::Fact, "fact-1", "唐揚げがとても好き。")
            .await?
            .updated
    );
    let forgotten = home
        .forget_memories(
            vec![MemoryForgetEntry {
                kind: MemoryEntryKind::Episode,
                id: "episode-1".to_string(),
            }],
            false,
        )
        .await?;
    assert_eq!(forgotten.removed_facts, 1);
    assert_eq!(forgotten.removed_episodes, 1);

    let calls = calls.lock().expect("memory calls lock");
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].capability, MEMORY_LIST_CAPABILITY);
    assert_eq!(calls[0].input["episodeLimit"], 10);
    assert_eq!(calls[0].input["episodeOffset"], 0);
    assert_eq!(calls[1].capability, MEMORY_UPDATE_CAPABILITY);
    assert_eq!(calls[1].input["kind"], "fact");
    assert_eq!(calls[2].capability, MEMORY_FORGET_CAPABILITY);
    assert_eq!(calls[2].input["entries"][0]["kind"], "episode");
    Ok(())
}

#[tokio::test]
async fn memory_admin_returns_error_when_extension_missing() -> Result<()> {
    let home = ResidentHome::with_parts(
        "resident-default",
        llm_fallback_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        CapabilityRouter::new(),
    )
    .await?;

    let error = home
        .list_memories(Some(10), Some(0))
        .await
        .expect_err("memory provider should be missing");
    assert!(matches!(error, ResidentHomeError::Capability(_)));
    Ok(())
}

#[tokio::test]
async fn talk_impulse_without_mood_dispatches_with_default_inputs() -> Result<()> {
    let home = ResidentHome::new(
        "resident-default",
        random_talk_world(),
        EventLog::in_memory()?,
    )
    .await?;

    let commands = home
        .ingest_event(RuntimeEvent::new(
            TALK_IMPULSE_EVENT,
            "device",
            "resident-default",
        ))
        .await?;

    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].payload["text"], "ふつうに話します。");
    let records = home.event_log().read(EventLogQuery::default())?.records;
    let dispatch = records
        .iter()
        .find(|record| record.kind == "daihon.dispatch.result")
        .expect("dispatch result");
    assert_eq!(
        dispatch.payload["commands"][0]["payload"]["text"],
        "ふつうに話します。"
    );
    Ok(())
}

#[tokio::test]
async fn desktop_folder_opened_enriches_recent_download_inputs_and_ignores_old() -> Result<()> {
    let now = Utc::now();
    let home = ResidentHome::new(
        "resident-default",
        folder_download_world(),
        EventLog::in_memory()?,
    )
    .await?;
    let mut download =
        RuntimeEvent::new("desktop.download.completed", "device", "resident-default");
    download.id = "evt_download_recent".to_string();
    download.timestamp = (now - Duration::days(2)).to_rfc3339();
    download
        .payload
        .insert("fileName".to_string(), json!("photo.png"));
    download
        .payload
        .insert("fileCategory".to_string(), json!("image"));
    home.event_log().append(NewEventLogRecord::from(download))?;

    let mut folder = RuntimeEvent::new("desktop.folder.opened", "device", "resident-default");
    folder.id = "evt_folder_recent".to_string();
    folder.timestamp = now.to_rfc3339();
    folder
        .payload
        .insert("category".to_string(), json!("downloads"));
    folder.payload.insert("app".to_string(), json!("finder"));
    let commands = home.ingest_event(folder).await?;
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].payload["text"], "さっきのphoto.pngだね。");
    let records = home.event_log().read(EventLogQuery {
        kind: Some("desktop.folder.opened".to_string()),
        ..EventLogQuery::default()
    })?;
    let folder_record = records.records.first().expect("folder record");
    assert!(!folder_record.payload.contains_key("recentDownloadFileName"));
    assert!(!folder_record.payload.contains_key("recentDownloadCategory"));

    let old_home = ResidentHome::new(
        "resident-default",
        folder_download_world(),
        EventLog::in_memory()?,
    )
    .await?;
    let mut old_download =
        RuntimeEvent::new("desktop.download.completed", "device", "resident-default");
    old_download.id = "evt_download_old".to_string();
    old_download.timestamp = (now - Duration::days(8)).to_rfc3339();
    old_download
        .payload
        .insert("fileName".to_string(), json!("photo.png"));
    old_download
        .payload
        .insert("fileCategory".to_string(), json!("image"));
    old_home
        .event_log()
        .append(NewEventLogRecord::from(old_download))?;

    let mut folder = RuntimeEvent::new("desktop.folder.opened", "device", "resident-default");
    folder.id = "evt_folder_old".to_string();
    folder.timestamp = now.to_rfc3339();
    folder
        .payload
        .insert("category".to_string(), json!("downloads"));
    folder.payload.insert("app".to_string(), json!("finder"));
    let commands = old_home.ingest_event(folder).await?;
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].payload["text"],
        "最近のダウンロードはありません。"
    );
    Ok(())
}

#[tokio::test]
async fn event_log_trim_records_audit_event() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    for index in 0..12 {
        let mut event = RuntimeEvent::new("conversation.text", "user", "resident-default");
        event.id = format!("evt_trim_{index}");
        event.timestamp = format!("2026-07-{day:02}T00:00:00.000Z", day = index + 1);
        home.event_log().append(NewEventLogRecord::from(event))?;
    }

    let summary = home.trim_event_log_to_record_limit(10, 10)?;

    assert_eq!(summary.deleted, 1);
    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert_eq!(records.len(), 12);
    assert_eq!(records[0].id, "evt_trim_1");
    let audit = records.last().expect("trim audit record");
    assert_eq!(audit.kind, "event_log.trimmed");
    assert_eq!(audit.payload["deleted"], json!(1));
    assert_eq!(
        audit.payload["oldestTimestamp"],
        json!("2026-07-01T00:00:00.000Z")
    );
    Ok(())
}

#[tokio::test]
async fn process_extension_suspension_records_events_and_notifies_once() -> Result<()> {
    let dir = tempdir().map_err(ExtensionError::from)?;
    fs::write(
        dir.path().join("invalid.js"),
        r#"process.stdout.write("{bad");"#,
    )
    .map_err(ExtensionError::from)?;
    let manifest = ProcessExtensionManifest {
        schema_version: 1,
        id: "bad-process".to_string(),
        display_name: "Bad Process".to_string(),
        runtime: None,
        permissions: ExtensionPermissions::default(),
        hooks: vec![ExtensionHookSubscription {
            hook_point: ExtensionHookPoint::BeforeCommandEmit,
            command_types: vec!["dialogue.say".to_string()],
        }],
        event_subscriptions: Vec::new(),
        emitted_events: Vec::new(),
        capabilities: Vec::new(),
        signal_aliases: Vec::new(),
        settings: None,
        process: ProcessCommandSpec {
            command: "node".to_string(),
            args: vec!["invalid.js".to_string()],
            cwd: None,
            timeout_ms: Some(1_000),
        },
    };
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    home.register_extension(ProcessHookExtension::from_installed_manifest(
        manifest,
        dir.path(),
        true,
    ))
    .await?;
    home.set_extension_hook_order(
        ExtensionHookPoint::BeforeCommandEmit,
        vec!["bad-process".to_string()],
    )?;
    let mut receiver = home.subscribe_commands();
    let source_event = RuntimeEvent::new("conversation.text", "user", "resident-default");

    for _ in 0..3 {
        let mut command = RuntimeCommand::new("dialogue.say", "daihon", "resident-default");
        command.payload.insert("text".to_string(), json!("hello"));
        home.emit_command_for_event(command, &source_event).await?;
    }

    let mut notifications = Vec::new();
    for _ in 0..6 {
        let command = tokio::time::timeout(std::time::Duration::from_millis(200), receiver.recv())
            .await
            .ok()
            .and_then(std::result::Result::ok);
        let Some(command) = command else {
            break;
        };
        if command.kind == "ui.notification" {
            notifications.push(command);
        }
    }
    assert_eq!(notifications.len(), 1);
    assert_eq!(
        notifications[0].payload["extensionId"],
        json!("bad-process")
    );
    assert!(notifications[0].payload["text"]
        .as_str()
        .unwrap_or_default()
        .contains("いったん休止しました"));

    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert_eq!(
        records
            .iter()
            .filter(|record| record.kind == "extension.process.suspended")
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.kind == "extension.process.failed")
            .count(),
        3
    );
    Ok(())
}

#[tokio::test]
async fn low_talk_desire_skips_talk_impulse() -> Result<()> {
    let home = ResidentHome::new(
        "resident-default",
        random_talk_world(),
        EventLog::in_memory()?,
    )
    .await?;

    let mut mood_event = RuntimeEvent::new(MOOD_CHANGED_EVENT, "extension", "resident-default");
    mood_event.payload = JsonMap::from([
        ("mood".to_string(), json!("さみしい")),
        ("talkDesire".to_string(), json!(12)),
        ("topic".to_string(), json!("静かな画面")),
    ]);
    assert!(home.ingest_event(mood_event).await?.is_empty());

    let commands = home
        .ingest_event(RuntimeEvent::new(
            TALK_IMPULSE_EVENT,
            "device",
            "resident-default",
        ))
        .await?;
    assert!(commands.is_empty());
    let records = home.event_log().read(EventLogQuery::default())?.records;
    let skipped = records
        .iter()
        .find(|record| record.kind == "presence.talk_impulse.skipped")
        .expect("skip record");
    assert_eq!(skipped.payload["reason"], "low-talk-desire");
    assert_eq!(skipped.payload["mood"], "さみしい");
    assert_eq!(skipped.payload["talkDesire"], 12);
    Ok(())
}

#[tokio::test]
async fn high_talk_desire_mood_changed_interrupts_with_random_talk() -> Result<()> {
    let home = ResidentHome::new(
        "resident-default",
        random_talk_world(),
        EventLog::in_memory()?,
    )
    .await?;

    let mut mood_event = RuntimeEvent::new(MOOD_CHANGED_EVENT, "extension", "resident-default");
    mood_event.payload = JsonMap::from([
        ("mood".to_string(), json!("さみしい")),
        ("talkDesire".to_string(), json!(92)),
        ("topic".to_string(), json!("静かな画面")),
    ]);
    let commands = home.ingest_event(mood_event).await?;

    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].payload["text"], "少し静かですね。");
    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert!(records.iter().any(|record| {
        record.kind == TALK_IMPULSE_EVENT
            && record.source == "resident-home"
            && record.payload["trigger"] == "mood.changed"
    }));
    Ok(())
}

#[tokio::test]
async fn configured_high_talk_desire_threshold_suppresses_mood_interrupt() -> Result<()> {
    let home = ResidentHome::with_parts_and_runtime_settings(
        "resident-default",
        random_talk_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        CapabilityRouter::new(),
        ResidentRuntimeSettings {
            talk_desire_high: 95,
            ..ResidentRuntimeSettings::default()
        },
    )
    .await?;
    let mut mood_event = RuntimeEvent::new(MOOD_CHANGED_EVENT, "extension", "resident-default");
    mood_event.payload = JsonMap::from([
        ("mood".to_string(), json!("さみしい")),
        ("talkDesire".to_string(), json!(92)),
        ("topic".to_string(), json!("静かな画面")),
    ]);
    assert!(home.ingest_event(mood_event).await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn mood_state_persists_restores_and_expires_after_one_hour() -> Result<()> {
    let dir = tempdir().expect("tempdir");
    let mood_path = dir.path().join("mood.json");
    let runtime_settings = ResidentRuntimeSettings {
        mood_state_path: Some(mood_path.clone()),
        ..ResidentRuntimeSettings::default()
    };
    let home = ResidentHome::with_parts_and_runtime_settings(
        "resident-default",
        random_talk_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        CapabilityRouter::new(),
        runtime_settings.clone(),
    )
    .await?;
    let mut mood_event = RuntimeEvent::new(MOOD_CHANGED_EVENT, "extension", "resident-default");
    mood_event.payload = JsonMap::from([
        ("mood".to_string(), json!("さみしい")),
        ("talkDesire".to_string(), json!(50)),
        ("topic".to_string(), json!("静かな画面")),
    ]);
    home.ingest_event(mood_event).await?;
    assert!(mood_path.exists());

    let restored = ResidentHome::with_parts_and_runtime_settings(
        "resident-default",
        random_talk_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        CapabilityRouter::new(),
        runtime_settings.clone(),
    )
    .await?;
    let restored_commands = restored
        .ingest_event(RuntimeEvent::new(
            TALK_IMPULSE_EVENT,
            "device",
            "resident-default",
        ))
        .await?;
    assert_eq!(restored_commands[0].payload["text"], "少し静かですね。");

    let stale_state = MoodState {
        last_evaluated_at: Some(Utc::now() - chrono::Duration::minutes(61)),
        current: Some(MoodSnapshot {
            mood: "さみしい".to_string(),
            talk_desire: 50,
            topic: "古い画面".to_string(),
        }),
    };
    std::fs::write(
        &mood_path,
        serde_json::to_vec_pretty(&stale_state).expect("stale mood json"),
    )
    .expect("write stale mood state");
    let expired = ResidentHome::with_parts_and_runtime_settings(
        "resident-default",
        random_talk_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        CapabilityRouter::new(),
        runtime_settings,
    )
    .await?;
    let expired_commands = expired
        .ingest_event(RuntimeEvent::new(
            TALK_IMPULSE_EVENT,
            "device",
            "resident-default",
        ))
        .await?;
    assert_eq!(expired_commands[0].payload["text"], "ふつうに話します。");
    Ok(())
}

#[tokio::test]
async fn life_tick_evaluates_mood_and_records_changed_event() -> Result<()> {
    let mut router = CapabilityRouter::new();
    let mood = MoodProvider::new(JsonMap::from([
        ("mood".to_string(), json!("うれしい")),
        ("talkDesire".to_string(), json!(45)),
        ("topic".to_string(), json!("机の上")),
    ]));
    let calls = mood.calls();
    router.register(mood)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        random_talk_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        router,
    )
    .await?;
    home.register_extension(yuukei_intelligence_events_extension())
        .await?;

    let mut tick = RuntimeEvent::new("presence.life_tick", "device", "resident-default");
    tick.payload = JsonMap::from([("timePeriod".to_string(), json!("昼"))]);
    let commands = home.ingest_event(tick).await?;

    assert!(commands.is_empty());
    assert_eq!(calls.lock().expect("mood calls lock").len(), 1);
    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert!(records.iter().any(|record| {
        record.kind == "capability.invocation.result"
            && record.payload["capability"] == MOOD_EVALUATE_CAPABILITY
    }));
    let changed = records
        .iter()
        .find(|record| record.kind == MOOD_CHANGED_EVENT)
        .expect("mood changed event");
    assert_eq!(changed.payload["mood"], "うれしい");
    assert_eq!(changed.payload["talkDesire"], 45);
    assert_eq!(changed.payload["topic"], "机の上");
    Ok(())
}

#[tokio::test]
async fn mood_interval_zero_disables_evaluation() -> Result<()> {
    let mut router = CapabilityRouter::new();
    let mood = MoodProvider::new(JsonMap::from([
        ("mood".to_string(), json!("うれしい")),
        ("talkDesire".to_string(), json!(45)),
        ("topic".to_string(), json!("机の上")),
    ]))
    .with_runtime_settings(JsonMap::from([(
        "mood.intervalMinutes".to_string(),
        json!(0),
    )]));
    let calls = mood.calls();
    router.register(mood)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        random_talk_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        router,
    )
    .await?;
    home.register_extension(yuukei_intelligence_events_extension())
        .await?;

    home.ingest_event(RuntimeEvent::new(
        "presence.life_tick",
        "device",
        "resident-default",
    ))
    .await?;

    assert!(calls.lock().expect("mood calls lock").is_empty());
    Ok(())
}

#[tokio::test]
async fn mood_evaluate_failure_keeps_previous_mood() -> Result<()> {
    let mut router = CapabilityRouter::new();
    let mood = MoodProvider::new(JsonMap::new()).failing();
    let calls = mood.calls();
    router.register(mood)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        random_talk_world(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        router,
    )
    .await?;
    home.register_extension(yuukei_intelligence_events_extension())
        .await?;

    {
        let mut state = home
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        state.mood.current = Some(MoodSnapshot {
            mood: "さみしい".to_string(),
            talk_desire: 10,
            topic: "静かな画面".to_string(),
        });
    }
    home.ingest_event(RuntimeEvent::new(
        "presence.life_tick",
        "device",
        "resident-default",
    ))
    .await?;
    let commands = home
        .ingest_event(RuntimeEvent::new(
            TALK_IMPULSE_EVENT,
            "device",
            "resident-default",
        ))
        .await?;

    assert!(commands.is_empty());
    assert_eq!(calls.lock().expect("mood calls lock").len(), 1);
    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert_eq!(
        records
            .iter()
            .filter(|record| record.kind == MOOD_CHANGED_EVENT)
            .count(),
        0
    );
    assert!(records
        .iter()
        .any(|record| record.kind == "presence.talk_impulse.skipped"
            && record.payload["mood"] == "さみしい"));
    Ok(())
}

#[tokio::test]
async fn undeclared_signal_does_not_call_dialogue_generate() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    let provider = DialogueGenerateProvider::new(JsonMap::from([
        ("speak".to_string(), json!(true)),
        ("text".to_string(), json!("呼ばれません。")),
    ]));
    let calls = provider.calls();
    home.register_provider(provider)?;

    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event
        .payload
        .insert("text".to_string(), json!("こんにちは"));
    let commands = home.ingest_event(event).await?;

    assert!(commands
        .iter()
        .any(|command| command.kind == "dialogue.say"));
    assert!(calls.lock().expect("calls lock").is_empty());
    Ok(())
}

#[tokio::test]
async fn dialogue_generate_cooldown_suppresses_second_call() -> Result<()> {
    let home = ResidentHome::new(
        "resident-default",
        llm_fallback_world(),
        EventLog::in_memory()?,
    )
    .await?;
    let provider = DialogueGenerateProvider::new(JsonMap::from([
        ("speak".to_string(), json!(true)),
        ("text".to_string(), json!("一度だけ。")),
    ]));
    let calls = provider.calls();
    home.register_provider(provider)?;

    let mut first = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    first.payload.insert("text".to_string(), json!("一つ目"));
    let mut second = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    second.payload.insert("text".to_string(), json!("二つ目"));

    assert_eq!(home.ingest_event(first).await?.len(), 1);
    assert!(home.ingest_event(second).await?.is_empty());
    assert_eq!(calls.lock().expect("calls lock").len(), 1);
    Ok(())
}

#[tokio::test]
async fn dialogue_generate_without_cooldown_invokes_every_time() -> Result<()> {
    let mut world = llm_fallback_world();
    world.llm_delegation.signals[0].cooldown_seconds = None;
    let home = ResidentHome::new("resident-default", world, EventLog::in_memory()?).await?;
    let provider = DialogueGenerateProvider::new(JsonMap::from([
        ("speak".to_string(), json!(true)),
        ("text".to_string(), json!("毎回応える。")),
    ]));
    let calls = provider.calls();
    home.register_provider(provider)?;

    let mut first = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    first.payload.insert("text".to_string(), json!("一つ目"));
    let mut second = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    second.payload.insert("text".to_string(), json!("二つ目"));

    assert_eq!(home.ingest_event(first).await?.len(), 1);
    assert_eq!(home.ingest_event(second).await?.len(), 1);
    assert_eq!(calls.lock().expect("calls lock").len(), 2);
    Ok(())
}

#[tokio::test]
async fn dialogue_generate_daily_budget_suppresses_after_limit() -> Result<()> {
    let mut world = llm_fallback_world();
    world.llm_delegation.signals[0].cooldown_seconds = None;
    world.llm_delegation.daily_budget = Some(1);
    let home = ResidentHome::new("resident-default", world, EventLog::in_memory()?).await?;
    let provider = DialogueGenerateProvider::new(JsonMap::from([
        ("speak".to_string(), json!(true)),
        ("text".to_string(), json!("一日一度だけ。")),
    ]));
    let calls = provider.calls();
    home.register_provider(provider)?;

    let mut first = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    first.payload.insert("text".to_string(), json!("一つ目"));
    let mut second = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    second.payload.insert("text".to_string(), json!("二つ目"));

    assert_eq!(home.ingest_event(first).await?.len(), 1);
    assert!(home.ingest_event(second).await?.is_empty());
    assert_eq!(calls.lock().expect("calls lock").len(), 1);
    Ok(())
}

#[tokio::test]
async fn dialogue_generate_speak_false_is_silent() -> Result<()> {
    let home = ResidentHome::new(
        "resident-default",
        llm_fallback_world(),
        EventLog::in_memory()?,
    )
    .await?;
    let provider =
        DialogueGenerateProvider::new(JsonMap::from([("speak".to_string(), json!(false))]));
    let calls = provider.calls();
    home.register_provider(provider)?;

    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.payload.insert("text".to_string(), json!("今？"));
    let commands = home.ingest_event(event).await?;

    assert!(commands.is_empty());
    assert_eq!(calls.lock().expect("calls lock").len(), 1);
    assert!(!home
        .event_log()
        .read(EventLogQuery::default())?
        .records
        .iter()
        .any(|record| record.kind == "dialogue.say"));
    Ok(())
}

#[tokio::test]
async fn missing_dialogue_generate_provider_is_silent() -> Result<()> {
    let home = ResidentHome::new(
        "resident-default",
        llm_fallback_world(),
        EventLog::in_memory()?,
    )
    .await?;
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.payload.insert("text".to_string(), json!("いる？"));

    let commands = home.ingest_event(event).await?;

    assert!(commands.is_empty());
    assert_eq!(
        home.event_log()
            .read(EventLogQuery::default())?
            .records
            .iter()
            .filter(|record| record.kind == "dialogue.say")
            .count(),
        0
    );
    Ok(())
}

#[tokio::test]
async fn surface_attach_is_logged_and_dispatched() -> Result<()> {
    let mut world = world_pack();
    world.daihon.loaded_scripts[0].source = r#"
## desktop reactions
### attach
合図: ＠画面_接続
話者: yuukei
「ここにいます。」
"#
    .to_string();
    let home = ResidentHome::new("resident-default", world, EventLog::in_memory()?).await?;
    let mut receiver = home.subscribe_commands();

    let snapshot = home
        .attach_surface(SurfaceSession {
            surface_id: "surface-main".to_string(),
            device_id: "device-local".to_string(),
            kind: SurfaceKind::Desktop,
            active: true,
            capabilities: vec!["dialogue.say".to_string()],
            presentation: SurfacePresentation {
                renderer: Some(SurfaceRenderer::Html),
                transparent: Some(false),
                accepts_input: Some(true),
            },
        })
        .await?;

    assert_eq!(snapshot.active_surface_id.as_deref(), Some("surface-main"));
    let dialogue = receiver.recv().await.expect("attach dialogue broadcast");
    assert_eq!(dialogue.kind, "dialogue.say");
    assert_eq!(dialogue.payload["text"], json!("ここにいます。"));

    let records = home
        .event_log()
        .read(EventLogQuery::default())?
        .records
        .into_iter()
        .map(|record| record.kind)
        .collect::<Vec<_>>();
    assert!(records.contains(&"surface.attach".to_string()));
    assert!(records.contains(&"daihon.dispatch.result".to_string()));
    assert!(records.contains(&"dialogue.say".to_string()));
    Ok(())
}

#[tokio::test]
async fn choice_event_resolves_pending_daihon_choice() -> Result<()> {
    let home =
        ResidentHome::new("resident-default", choice_world(30), EventLog::in_memory()?).await?;
    let mut receiver = home.subscribe_commands();
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.id = "evt_choice_start".to_string();
    event.payload.insert("text".to_string(), json!("start"));
    let ingest_home = home.clone();
    let ingest = tokio::spawn(async move { ingest_home.ingest_event(event).await });

    let prompt_command = next_command_of_kind(&mut receiver, "dialogue.say").await;
    assert_eq!(prompt_command.payload["text"], json!("見る？"));

    let choices_command = next_command_of_kind(&mut receiver, "dialogue.choices").await;
    let choice_id = choices_command.payload["choiceId"]
        .as_str()
        .expect("choice id")
        .to_string();
    assert_eq!(
        choices_command.payload["choices"],
        json!(["見る", "あとで"])
    );

    let mut choice_event = RuntimeEvent::new("conversation.choice", "surface", "resident-default");
    choice_event
        .payload
        .insert("choiceId".to_string(), json!(choice_id));
    choice_event
        .payload
        .insert("choice".to_string(), json!("見る"));
    choice_event.payload.insert("index".to_string(), json!(0));
    assert!(home.ingest_event(choice_event).await?.is_empty());

    let commands = ingest.await.expect("choice dispatch task")?;
    assert!(commands.iter().any(
        |command| command.kind == "dialogue.say" && command.payload["text"] == json!("見る枝")
    ));
    Ok(())
}

#[tokio::test]
async fn choice_timeout_returns_unknown_and_clears_choices() -> Result<()> {
    let home =
        ResidentHome::new("resident-default", choice_world(5), EventLog::in_memory()?).await?;
    let mut receiver = home.subscribe_commands();
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.id = "evt_choice_timeout".to_string();
    event.payload.insert("text".to_string(), json!("start"));
    let commands = home.ingest_event(event).await?;

    let choices_command = next_command_of_kind(&mut receiver, "dialogue.choices").await;
    let clear_command = next_command_of_kind(&mut receiver, "dialogue.choices.clear").await;
    assert_eq!(
        clear_command.payload["choiceId"],
        choices_command.payload["choiceId"]
    );
    assert_eq!(clear_command.payload["reason"], json!("timeout"));
    assert!(commands.iter().any(
        |command| command.kind == "dialogue.say" && command.payload["text"] == json!("不明枝")
    ));
    Ok(())
}

#[tokio::test]
async fn mismatched_choice_id_is_recorded_but_does_not_resolve_pending_choice() -> Result<()> {
    let home =
        ResidentHome::new("resident-default", choice_world(30), EventLog::in_memory()?).await?;
    let mut receiver = home.subscribe_commands();
    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.id = "evt_choice_mismatch_start".to_string();
    event.payload.insert("text".to_string(), json!("start"));
    let ingest_home = home.clone();
    let ingest = tokio::spawn(async move { ingest_home.ingest_event(event).await });

    let choices_command = next_command_of_kind(&mut receiver, "dialogue.choices").await;
    let choice_id = choices_command.payload["choiceId"]
        .as_str()
        .expect("choice id")
        .to_string();

    let mut wrong_choice = RuntimeEvent::new("conversation.choice", "surface", "resident-default");
    wrong_choice
        .payload
        .insert("choiceId".to_string(), json!("choice_wrong"));
    wrong_choice
        .payload
        .insert("choice".to_string(), json!("見る"));
    wrong_choice.payload.insert("index".to_string(), json!(0));
    assert!(home.ingest_event(wrong_choice).await?.is_empty());
    assert!(!ingest.is_finished());

    let mut right_choice = RuntimeEvent::new("conversation.choice", "surface", "resident-default");
    right_choice
        .payload
        .insert("choiceId".to_string(), json!(choice_id));
    right_choice
        .payload
        .insert("choice".to_string(), json!("見る"));
    right_choice.payload.insert("index".to_string(), json!(0));
    home.ingest_event(right_choice).await?;

    let commands = ingest.await.expect("choice dispatch task")?;
    assert!(commands.iter().any(
        |command| command.kind == "dialogue.say" && command.payload["text"] == json!("見る枝")
    ));
    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert!(records.iter().any(|record| {
        record.kind == "conversation.choice" && record.payload["choiceId"] == json!("choice_wrong")
    }));
    Ok(())
}

#[tokio::test]
async fn conversation_text_is_queued_while_choice_is_pending() -> Result<()> {
    let home = ResidentHome::new(
        "resident-default",
        choice_queue_world(),
        EventLog::in_memory()?,
    )
    .await?;
    let mut receiver = home.subscribe_commands();
    let mut start_event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    start_event.id = "evt_choice_queue_start".to_string();
    start_event
        .payload
        .insert("text".to_string(), json!("start"));
    let ingest_home = home.clone();
    let ingest = tokio::spawn(async move { ingest_home.ingest_event(start_event).await });

    let choices_command = next_command_of_kind(&mut receiver, "dialogue.choices").await;
    let choice_id = choices_command.payload["choiceId"]
        .as_str()
        .expect("choice id")
        .to_string();

    let mut queued_event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    queued_event.id = "evt_choice_queue_text".to_string();
    queued_event
        .payload
        .insert("text".to_string(), json!("queued"));
    assert!(home.ingest_event(queued_event).await?.is_empty());

    let mut choice_event = RuntimeEvent::new("conversation.choice", "surface", "resident-default");
    choice_event
        .payload
        .insert("choiceId".to_string(), json!(choice_id));
    choice_event
        .payload
        .insert("choice".to_string(), json!("見る"));
    choice_event.payload.insert("index".to_string(), json!(0));
    home.ingest_event(choice_event).await?;

    let commands = ingest.await.expect("choice dispatch task")?;
    assert!(commands.iter().any(
        |command| command.kind == "dialogue.say" && command.payload["text"] == json!("見る枝")
    ));
    assert!(commands.iter().any(|command| command.kind == "dialogue.say"
        && command.payload["text"] == json!("queued handled")));
    Ok(())
}

#[tokio::test]
async fn extension_can_rewrite_dialogue_before_emit_and_tts() -> Result<()> {
    let provider = SpeechSynthesisProvider::new(JsonMap::from([
        (
            "audioPath".to_string(),
            json!("/tmp/yuukei-voicevox/rewrite.wav"),
        ),
        ("durationMs".to_string(), json!(500)),
        ("format".to_string(), json!("wav")),
    ]));
    let mut capabilities = CapabilityRouter::new();
    capabilities.register(provider)?;
    let home = ResidentHome::with_parts(
        "resident-default",
        world_pack(),
        EventLog::in_memory()?,
        Arc::new(YuukeiDaihonAdapter::default()),
        capabilities,
    )
    .await?;
    home.register_extension(DialogueSuffixExtension::new("nya-suffix", "にゃ"))
        .await?;
    let mut receiver = home.subscribe_commands();
    home.attach_surface(SurfaceSession {
        surface_id: "surface-main".to_string(),
        device_id: "device-local".to_string(),
        kind: SurfaceKind::Desktop,
        active: true,
        capabilities: vec!["dialogue.say".to_string()],
        presentation: SurfacePresentation {
            renderer: Some(SurfaceRenderer::Html),
            transparent: Some(false),
            accepts_input: Some(true),
        },
    })
    .await?;

    let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    event.id = "evt_text".to_string();
    event.device_id = Some("device-local".to_string());
    event.surface_id = Some("surface-main".to_string());
    event
        .payload
        .insert("text".to_string(), json!("こんにちは"));

    let commands = home.ingest_event(event).await?;
    let dialogue = commands
        .iter()
        .find(|command| command.kind == "dialogue.say")
        .expect("dialogue command");
    assert_eq!(dialogue.payload["text"], "聞こえています。こんにちはにゃ");
    assert_eq!(
        home.snapshot()?.actors["yuukei"].bubble.as_deref(),
        Some("聞こえています。こんにちはにゃ")
    );

    // 合成は非同期に走るので、audio.playの配信を待ってから記録を確認する。
    let _ = next_command_of_kind(&mut receiver, "audio.play").await;

    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert!(records
        .iter()
        .any(|record| record.kind == "extension.hook.result"));
    let speech_request = records
        .iter()
        .find(|record| {
            record.kind == "capability.invocation.request"
                && record.payload["capability"] == SPEECH_SYNTHESIS_CAPABILITY
        })
        .expect("speech request");
    assert_eq!(
        speech_request.payload["input"]["text"],
        "聞こえています。こんにちはにゃ"
    );
    Ok(())
}

#[tokio::test]
async fn extension_event_emission_requires_declared_namespace() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    home.register_extension(
        EventEmitterExtension::new("activity")
            .subscribed_to(["os.test"])
            .emits(["ext.activity.allowed"])
            .proposes("conversation.text"),
    )
    .await?;

    let event = RuntimeEvent::new("os.test", "device", "resident-default");
    home.ingest_event(event).await?;

    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert!(records
        .iter()
        .any(|record| record.kind == "extension.event.rejected"
            && record.payload["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("ext.activity."))));
    assert_eq!(
        records
            .iter()
            .filter(|record| record.kind == "ext.activity.allowed")
            .count(),
        0
    );
    Ok(())
}

#[tokio::test]
async fn extension_event_normalization_overwrites_spoofed_fields() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    let mut proposed = RuntimeEvent::new("ext.activity.spoof", "device", "other-resident");
    proposed.id = "evt_spoofed".to_string();
    proposed.timestamp = "2000-01-01T00:00:00.000Z".to_string();
    proposed.device_id = Some("device-spoofed".to_string());
    proposed.surface_id = Some("surface-spoofed".to_string());
    proposed.actor_id = Some("actor-spoofed".to_string());
    proposed.causality = Some(Causality {
        source_event_id: Some("evt_fake_source".to_string()),
        source_command_id: Some("cmd_fake_source".to_string()),
        trace_id: Some("trace-spoofed".to_string()),
    });
    proposed.payload.insert("ok".to_string(), json!(true));
    proposed.payload.insert(
        "yuukeiExtension".to_string(),
        json!({ "extensionId": "evil", "hopCount": 99 }),
    );
    home.register_extension(
        EventEmitterExtension::new("activity")
            .subscribed_to(["os.test"])
            .emits(["ext.activity.*"])
            .proposes_event(proposed),
    )
    .await?;

    let mut source = RuntimeEvent::new("os.test", "device", "resident-default");
    source.id = "evt_source".to_string();
    source.device_id = Some("device-real".to_string());
    source.surface_id = Some("surface-real".to_string());
    source.actor_id = Some("yuukei".to_string());
    source.causality = Some(Causality {
        source_event_id: None,
        source_command_id: None,
        trace_id: Some("trace-real".to_string()),
    });
    home.ingest_event(source).await?;

    let records = home.event_log().read(EventLogQuery::default())?.records;
    let emitted = records
        .iter()
        .find(|record| record.kind == "ext.activity.spoof")
        .expect("normalized extension event");
    assert_ne!(emitted.id, "evt_spoofed");
    assert_eq!(emitted.source, "extension");
    assert_eq!(emitted.resident_id, "resident-default");
    assert_eq!(emitted.device_id.as_deref(), Some("device-real"));
    assert_eq!(emitted.surface_id.as_deref(), Some("surface-real"));
    assert_eq!(emitted.actor_id.as_deref(), Some("yuukei"));
    assert_eq!(
        emitted
            .causality
            .as_ref()
            .and_then(|causality| causality.source_event_id.as_deref()),
        Some("evt_source")
    );
    assert_eq!(
        emitted
            .causality
            .as_ref()
            .and_then(|causality| causality.source_command_id.as_deref()),
        None
    );
    assert_eq!(
        emitted
            .causality
            .as_ref()
            .and_then(|causality| causality.trace_id.as_deref()),
        Some("trace-real")
    );
    assert_eq!(emitted.payload["ok"], json!(true));
    assert_eq!(
        emitted.payload["yuukeiExtension"],
        json!({ "extensionId": "activity", "hopCount": 1 })
    );
    Ok(())
}

#[tokio::test]
async fn process_extension_event_output_is_normalized_by_home() -> Result<()> {
    let dir = tempdir().map_err(ExtensionError::from)?;
    fs::write(
        dir.path().join("emit.js"),
        r#"
const fs = require("node:fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
process.stdout.write(JSON.stringify({
  proposedEvents: [{
    id: "evt_process_spoof",
    type: "ext.process.spoof",
    timestamp: "2000-01-01T00:00:00.000Z",
    source: "device",
    residentId: "other-resident",
    deviceId: "device-spoofed",
    surfaceId: "surface-spoofed",
    actorId: "actor-spoofed",
    causality: { sourceEventId: "evt_fake", sourceCommandId: "cmd_fake", traceId: "trace-fake" },
    payload: { yuukeiExtension: { extensionId: "evil", hopCount: 99 }, fromProcess: true }
  }],
  metadata: { invocationId: input.id }
}));
"#,
    )
    .map_err(ExtensionError::from)?;
    let manifest = ProcessExtensionManifest {
        schema_version: 1,
        id: "process".to_string(),
        display_name: "Process".to_string(),
        runtime: None,
        permissions: ExtensionPermissions::default(),
        hooks: Vec::new(),
        event_subscriptions: vec![ExtensionEventSubscription {
            event_types: vec!["os.test".to_string()],
        }],
        emitted_events: vec!["ext.process.*".to_string()],
        capabilities: Vec::new(),
        signal_aliases: Vec::new(),
        settings: None,
        process: ProcessCommandSpec {
            command: "node".to_string(),
            args: vec!["emit.js".to_string()],
            cwd: None,
            timeout_ms: Some(5_000),
        },
    };
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    home.register_extension(ProcessHookExtension::from_installed_manifest(
        manifest,
        dir.path(),
        true,
    ))
    .await?;

    let mut source = RuntimeEvent::new("os.test", "device", "resident-default");
    source.id = "evt_process_source".to_string();
    source.device_id = Some("device-real".to_string());
    home.ingest_event(source).await?;

    let records = home.event_log().read(EventLogQuery::default())?.records;
    let emitted = records
        .iter()
        .find(|record| record.kind == "ext.process.spoof")
        .expect("process-emitted event");
    assert_eq!(emitted.source, "extension");
    assert_eq!(emitted.resident_id, "resident-default");
    assert_eq!(emitted.device_id.as_deref(), Some("device-real"));
    assert_eq!(emitted.surface_id, None);
    assert_eq!(emitted.actor_id, None);
    assert_eq!(
        emitted.payload["yuukeiExtension"],
        json!({ "extensionId": "process", "hopCount": 1 })
    );
    assert_eq!(
        emitted
            .causality
            .as_ref()
            .and_then(|causality| causality.source_event_id.as_deref()),
        Some("evt_process_source")
    );
    Ok(())
}

#[tokio::test]
async fn extension_event_subscriptions_filter_by_event_type() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    let extension = EventEmitterExtension::new("activity")
        .subscribed_to(["presence.*"])
        .emits(["ext.activity.active-period.start"])
        .proposes("ext.activity.active-period.start");
    let calls = extension.calls();
    home.register_extension(extension).await?;

    home.ingest_event(RuntimeEvent::new("os.test", "device", "resident-default"))
        .await?;
    home.ingest_event(RuntimeEvent::new(
        "presence.life_tick",
        "device",
        "resident-default",
    ))
    .await?;

    assert_eq!(
        calls.lock().expect("calls lock").clone(),
        vec!["presence.life_tick".to_string()]
    );
    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert_eq!(
        records
            .iter()
            .filter(|record| record.kind == "ext.activity.active-period.start")
            .count(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn extension_events_do_not_self_subscribe_and_stop_at_hop_limit() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    let looper = EventEmitterExtension::new("looper")
        .subscribed_to(["*"])
        .with_broad_event_subscription()
        .emits(["ext.looper.tick"])
        .proposes("ext.looper.tick");
    home.register_extension(looper).await?;
    home.ingest_event(RuntimeEvent::new("os.test", "device", "resident-default"))
        .await?;
    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert_eq!(
        records
            .iter()
            .filter(|record| record.kind == "ext.looper.tick")
            .count(),
        1
    );

    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    home.register_extension(
        EventEmitterExtension::new("a")
            .subscribed_to(["os.test", "ext.b.*"])
            .emits(["ext.a.*"])
            .proposes("ext.a.ping"),
    )
    .await?;
    home.register_extension(
        EventEmitterExtension::new("b")
            .subscribed_to(["ext.a.*"])
            .emits(["ext.b.*"])
            .proposes("ext.b.ping"),
    )
    .await?;
    home.ingest_event(RuntimeEvent::new("os.test", "device", "resident-default"))
        .await?;

    let records = home.event_log().read(EventLogQuery::default())?.records;
    assert_eq!(
        records
            .iter()
            .filter(|record| record.kind == "ext.a.ping" || record.kind == "ext.b.ping")
            .count(),
        MAX_EXTENSION_EVENT_HOPS as usize
    );
    assert!(records
        .iter()
        .any(|record| record.kind == "extension.event.rejected"
            && record.payload["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("hop count exceeded"))));
    Ok(())
}

#[tokio::test]
async fn extension_signal_aliases_resolve_only_when_extension_is_enabled() -> Result<()> {
    let mut world = world_pack();
    world.signals.allow = vec!["活動時間_開始".to_string()];
    world.daihon.loaded_scripts[0].source = r#"
## desktop reactions
### activity
合図: ＠活動時間_開始
話者: yuukei
「活動開始です。」
"#
    .to_string();

    let home_without_extension =
        ResidentHome::new("resident-default", world.clone(), EventLog::in_memory()?).await?;
    let commands = home_without_extension
        .ingest_event(RuntimeEvent::new(
            "ext.activity.active-period.start",
            "device",
            "resident-default",
        ))
        .await?;
    assert!(commands.is_empty());

    let home = ResidentHome::new("resident-default", world, EventLog::in_memory()?).await?;
    home.register_extension(
        EventEmitterExtension::new("activity")
            .emits(["ext.activity.active-period.start"])
            .with_signal_alias("活動時間_開始", "ext.activity.active-period.start"),
    )
    .await?;
    let commands = home
        .ingest_event(RuntimeEvent::new(
            "ext.activity.active-period.start",
            "device",
            "resident-default",
        ))
        .await?;
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].payload["text"], "活動開始です。");
    Ok(())
}

#[tokio::test]
async fn extension_event_log_read_grant_is_clamped_to_manifest_permission() -> Result<()> {
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    home.register_extension(
        EventEmitterExtension::new("memory-extension")
            .subscribed_to(["conversation.*"])
            .with_event_log_read(ExtensionEventLogReadPermission {
                event_types: vec!["conversation.*".to_string()],
                privacy_categories: Vec::new(),
                allow_payloads: true,
                allow_references: false,
                max_records: 1,
                purpose: "rebuild extension state".to_string(),
            }),
    )
    .await?;

    let mut first = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    first.timestamp = "2026-01-01T00:00:00.000Z".to_string();
    first
        .payload
        .insert("text".to_string(), json!("こんにちは"));
    first.payload.insert(
        "reference".to_string(),
        json!({ "uri": "file:///secret.txt", "permissionRef": "secret" }),
    );
    home.event_log().append(NewEventLogRecord::from(first))?;

    let mut second = RuntimeEvent::new("conversation.text", "surface", "resident-default");
    second.timestamp = "2026-01-02T00:00:00.000Z".to_string();
    second.payload.insert("text".to_string(), json!("二つ目"));
    home.event_log().append(NewEventLogRecord::from(second))?;

    let mut private = RuntimeEvent::new("device.secret", "device", "resident-default");
    private.timestamp = "2026-01-01T00:30:00.000Z".to_string();
    private
        .payload
        .insert("secret".to_string(), json!("hidden"));
    let mut record = NewEventLogRecord::from(private);
    record.privacy = Some(Privacy {
        category: "device".to_string(),
        retention: RetentionPolicy::Short,
        extension_readable: false,
    });
    home.event_log().append(record)?;

    let page = home.read_event_log_for_extension(EventLogReadGrant {
        extension_id: "memory-extension".to_string(),
        resident_id: "resident-default".to_string(),
        event_types: Vec::new(),
        privacy_categories: Vec::new(),
        cursor_after_sequence: Some(0),
        until_timestamp: Some("2026-01-01T12:00:00.000Z".to_string()),
        max_records: 5,
        allow_payloads: true,
        allow_references: true,
        expires_at: future_timestamp(),
        purpose: "rebuild extension state".to_string(),
    })?;

    assert_eq!(page.records.len(), 1);
    assert!(page
        .records
        .iter()
        .all(|record| record.kind.starts_with("conversation.")));
    assert_eq!(page.records[0].payload["text"], json!("こんにちは"));
    assert_eq!(page.records[0].payload["reference"], Value::Null);
    Ok(())
}

#[tokio::test]
async fn extension_event_log_read_grant_rejects_unregistered_expired_and_out_of_scope() -> Result<()>
{
    let home = ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
    let base_grant = EventLogReadGrant {
        extension_id: "memory-extension".to_string(),
        resident_id: "resident-default".to_string(),
        event_types: vec!["conversation.*".to_string()],
        privacy_categories: Vec::new(),
        cursor_after_sequence: Some(0),
        until_timestamp: None,
        max_records: 5,
        allow_payloads: true,
        allow_references: false,
        expires_at: future_timestamp(),
        purpose: "rebuild extension state".to_string(),
    };

    let unregistered = home
        .read_event_log_for_extension(base_grant.clone())
        .unwrap_err();
    assert!(matches!(
        unregistered,
        ResidentHomeError::EventLogReadDenied(_)
    ));

    home.register_extension(
        EventEmitterExtension::new("memory-extension")
            .subscribed_to(["conversation.*"])
            .with_event_log_read(ExtensionEventLogReadPermission {
                event_types: vec!["conversation.*".to_string()],
                privacy_categories: Vec::new(),
                allow_payloads: true,
                allow_references: false,
                max_records: 5,
                purpose: "rebuild extension state".to_string(),
            }),
    )
    .await?;

    let mut expired = base_grant.clone();
    expired.expires_at = "2000-01-01T00:00:00.000Z".to_string();
    assert!(matches!(
        home.read_event_log_for_extension(expired).unwrap_err(),
        ResidentHomeError::EventLogReadDenied(_)
    ));

    let mut out_of_scope = base_grant;
    out_of_scope.event_types = vec!["device.*".to_string()];
    assert!(matches!(
        home.read_event_log_for_extension(out_of_scope).unwrap_err(),
        ResidentHomeError::EventLogReadDenied(_)
    ));
    Ok(())
}

#[tokio::test]
async fn rejects_world_pack_with_missing_required_capability() -> Result<()> {
    let mut world = world_pack();
    world.capabilities.required = vec!["dialogue.generate".to_string()];
    let error = match ResidentHome::new("resident-default", world, EventLog::in_memory()?).await {
        Ok(_) => panic!("missing required capability should reject the world pack"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        ResidentHomeError::MissingRequiredCapabilities(_)
    ));
    Ok(())
}
