use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use serde_json::Value;
use thiserror::Error;
use tokio::sync::broadcast;
use yuukei_capability::{
    CapabilityError, CapabilityProvider, CapabilityRouter, StubSpeechSynthesisProvider,
};
use yuukei_event_log::{EventLog, EventLogError};
use yuukei_extension::{
    ExtensionCommandContext, ExtensionError, ExtensionHookReport, ExtensionRegistry,
    YuukeiExtension,
};
use yuukei_protocol::{
    new_id, ActorSnapshot, CapabilityInvocation, Causality, ExtensionHookPoint, JsonMap,
    NewEventLogRecord, ResidentSnapshot, RuntimeCommand, RuntimeEvent, SurfaceSession,
};
use yuukei_world::{DaihonAdapter, WorldError, WorldPack, YuukeiDaihonAdapter};

#[derive(Debug, Error)]
pub enum ResidentHomeError {
    #[error("event log error: {0}")]
    EventLog(#[from] EventLogError),
    #[error("world error: {0}")]
    World(#[from] WorldError),
    #[error("capability error: {0}")]
    Capability(#[from] CapabilityError),
    #[error("extension error: {0}")]
    Extension(#[from] ExtensionError),
    #[error("world pack requires unavailable capabilities: {0}")]
    MissingRequiredCapabilities(String),
    #[error("state lock is poisoned")]
    PoisonedLock,
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ResidentHomeError>;

#[derive(Clone)]
pub struct ResidentHome {
    event_log: EventLog,
    world_pack: Arc<WorldPack>,
    daihon: Arc<dyn DaihonAdapter>,
    capabilities: Arc<Mutex<CapabilityRouter>>,
    extensions: Arc<Mutex<ExtensionRegistry>>,
    state: Arc<Mutex<HomeState>>,
    command_tx: broadcast::Sender<RuntimeCommand>,
}

#[derive(Clone, Debug)]
struct HomeState {
    resident_id: String,
    active_surface_id: Option<String>,
    actors: BTreeMap<String, ActorSnapshot>,
    surfaces: BTreeMap<String, SurfaceSession>,
    recent_event_cursor: i64,
}

impl ResidentHome {
    pub async fn new(
        resident_id: impl Into<String>,
        world_pack: WorldPack,
        event_log: EventLog,
    ) -> Result<Self> {
        Self::with_parts(
            resident_id,
            world_pack,
            event_log,
            Arc::new(YuukeiDaihonAdapter::default()),
            CapabilityRouter::new(),
        )
        .await
    }

    pub async fn with_parts(
        resident_id: impl Into<String>,
        world_pack: WorldPack,
        event_log: EventLog,
        daihon: Arc<dyn DaihonAdapter>,
        capabilities: CapabilityRouter,
    ) -> Result<Self> {
        Self::with_parts_and_extensions(
            resident_id,
            world_pack,
            event_log,
            daihon,
            capabilities,
            ExtensionRegistry::new(),
        )
        .await
    }

    pub async fn with_parts_and_extensions(
        resident_id: impl Into<String>,
        world_pack: WorldPack,
        event_log: EventLog,
        daihon: Arc<dyn DaihonAdapter>,
        mut capabilities: CapabilityRouter,
        extensions: ExtensionRegistry,
    ) -> Result<Self> {
        world_pack.validate()?;
        daihon.load_world(&world_pack).await?;
        if capabilities.summaries().values().all(|summary| {
            !summary
                .capabilities
                .iter()
                .any(|cap| cap == "speech.synthesis")
        }) {
            capabilities.register(StubSpeechSynthesisProvider)?;
        }
        let missing_capabilities = world_pack
            .capabilities
            .required
            .iter()
            .filter(|capability| !capabilities.has_healthy_provider(capability))
            .cloned()
            .collect::<Vec<_>>();
        if !missing_capabilities.is_empty() {
            return Err(ResidentHomeError::MissingRequiredCapabilities(
                missing_capabilities.join(", "),
            ));
        }

        let resident_id = resident_id.into();
        let actors = world_pack
            .actors
            .iter()
            .map(|actor| {
                (
                    actor.id.clone(),
                    ActorSnapshot {
                        display_name: actor.display_name.clone(),
                        expression: "neutral".to_string(),
                        motion: "idle".to_string(),
                        location: "desktop".to_string(),
                        speaking: Some(false),
                        bubble: None,
                    },
                )
            })
            .collect();
        let (command_tx, _) = broadcast::channel(128);
        Ok(Self {
            event_log,
            world_pack: Arc::new(world_pack),
            daihon,
            capabilities: Arc::new(Mutex::new(capabilities)),
            extensions: Arc::new(Mutex::new(extensions)),
            state: Arc::new(Mutex::new(HomeState {
                resident_id,
                active_surface_id: None,
                actors,
                surfaces: BTreeMap::new(),
                recent_event_cursor: 0,
            })),
            command_tx,
        })
    }

    pub fn event_log(&self) -> EventLog {
        self.event_log.clone()
    }

    pub fn subscribe_commands(&self) -> broadcast::Receiver<RuntimeCommand> {
        self.command_tx.subscribe()
    }

    pub fn snapshot(&self) -> Result<ResidentSnapshot> {
        let state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        let capabilities = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .summaries();
        let extensions = self
            .extensions
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .summaries();
        Ok(ResidentSnapshot {
            resident_id: state.resident_id.clone(),
            world_pack_id: self.world_pack.id.clone(),
            active_surface_id: state.active_surface_id.clone(),
            actors: state.actors.clone(),
            surfaces: state.surfaces.clone(),
            capabilities,
            extensions,
            recent_event_cursor: state.recent_event_cursor.to_string(),
        })
    }

    pub async fn attach_surface(&self, session: SurfaceSession) -> Result<ResidentSnapshot> {
        let payload = serde_json::to_value(&session)?;
        let event = RuntimeEvent {
            id: new_id("evt"),
            kind: "surface.attach".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            source: "device".to_string(),
            resident_id: self.resident_id()?,
            payload: json_map_from_value(payload),
            causality: None,
            device_id: Some(session.device_id.clone()),
            surface_id: Some(session.surface_id.clone()),
            actor_id: None,
        };
        let appended = self
            .event_log
            .append(NewEventLogRecord::from(event.clone()))?;
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?;
            state.recent_event_cursor = appended.sequence;
            state.active_surface_id = Some(session.surface_id.clone());
            state.surfaces.insert(session.surface_id.clone(), session);
        }
        self.dispatch_recorded_event(event).await?;
        self.snapshot()
    }

    pub fn register_provider<P>(&self, provider: P) -> Result<()>
    where
        P: CapabilityProvider + 'static,
    {
        self.capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .register(provider)?;
        Ok(())
    }

    pub fn register_extension<E>(&self, extension: E) -> Result<()>
    where
        E: YuukeiExtension + 'static,
    {
        self.extensions
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .register(extension)?;
        Ok(())
    }

    pub fn set_extension_hook_order(
        &self,
        hook_point: ExtensionHookPoint,
        extension_ids: Vec<String>,
    ) -> Result<()> {
        self.extensions
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .set_hook_order(hook_point, extension_ids);
        Ok(())
    }

    pub async fn ingest_event(&self, event: RuntimeEvent) -> Result<Vec<RuntimeCommand>> {
        let appended_event = self
            .event_log
            .append(NewEventLogRecord::from(event.clone()))?;
        self.set_cursor(appended_event.sequence)?;
        self.dispatch_recorded_event(event).await
    }

    async fn dispatch_recorded_event(&self, event: RuntimeEvent) -> Result<Vec<RuntimeCommand>> {
        if !self.world_pack.allows_signal(&event.kind) {
            return Ok(Vec::new());
        }

        let result = self.daihon.dispatch(&event, &self.world_pack).await?;
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
        let mut emitted_commands = Vec::with_capacity(result.commands.len());
        for command in result.commands {
            let mut command = self
                .apply_extensions_before_command_emit(command, &event)
                .await?;
            self.maybe_enrich_speech(&mut command, &event).await?;
            let appended_command = self
                .event_log
                .append(NewEventLogRecord::from(command.clone()))?;
            self.set_cursor(appended_command.sequence)?;
            self.apply_command_to_snapshot(&command)?;
            let _ = self.command_tx.send(command.clone());
            emitted_commands.push(command);
        }
        Ok(emitted_commands)
    }

    fn resident_id(&self) -> Result<String> {
        let state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        Ok(state.resident_id.clone())
    }

    fn set_cursor(&self, sequence: i64) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        state.recent_event_cursor = sequence;
        Ok(())
    }

    fn apply_command_to_snapshot(&self, command: &RuntimeCommand) -> Result<()> {
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
            _ => {}
        }
        Ok(())
    }

    async fn apply_extensions_before_command_emit(
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
        }
        Ok(result.command)
    }

    fn record_extension_hook_result(
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

    async fn maybe_enrich_speech(
        &self,
        command: &mut RuntimeCommand,
        source_event: &RuntimeEvent,
    ) -> Result<()> {
        if command.kind != "dialogue.say" {
            return Ok(());
        }
        let Some(text) = command.payload.get("text").and_then(Value::as_str) else {
            return Ok(());
        };

        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: "speech.synthesis".to_string(),
            method: "synthesize".to_string(),
            resident_id: command.resident_id.clone(),
            actor_id: command
                .target
                .as_ref()
                .and_then(|target| target.actor_id.clone()),
            input: JsonMap::from([
                ("text".to_string(), Value::String(text.to_string())),
                (
                    "speakerId".to_string(),
                    command
                        .payload
                        .get("speakerId")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                (
                    "emotion".to_string(),
                    command
                        .payload
                        .get("emotion")
                        .cloned()
                        .unwrap_or_else(|| Value::String("neutral".to_string())),
                ),
                (
                    "displayCommandId".to_string(),
                    Value::String(command.id.clone()),
                ),
            ]),
            context: None,
        };

        let request_payload = serde_json::to_value(&invocation)?;
        let request = NewEventLogRecord {
            id: new_id("evt"),
            kind: "capability.invocation.request".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id: command.resident_id.clone(),
            source: "resident-home".to_string(),
            device_id: source_event.device_id.clone(),
            surface_id: source_event.surface_id.clone(),
            actor_id: invocation.actor_id.clone(),
            payload: json_map_from_value(request_payload),
            causality: Some(Causality {
                source_event_id: Some(source_event.id.clone()),
                source_command_id: Some(command.id.clone()),
                trace_id: source_event
                    .causality
                    .as_ref()
                    .and_then(|causality| causality.trace_id.clone()),
            }),
            privacy: None,
        };
        let appended_request = self.event_log.append(request)?;
        self.set_cursor(appended_request.sequence)?;

        let result = {
            let router = self
                .capabilities
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?
                .clone();
            router.invoke(invocation).await?
        };

        if let Some(speech_ref) = result.output.get("speechRef").cloned() {
            command.payload.insert("speechRef".to_string(), speech_ref);
        }

        let result_payload = JsonMap::from([
            (
                "invocationId".to_string(),
                Value::String(result.invocation_id),
            ),
            ("providerId".to_string(), Value::String(result.provider_id)),
            ("capability".to_string(), Value::String(result.capability)),
            (
                "output".to_string(),
                Value::Object(result.output.into_iter().collect()),
            ),
            (
                "metadata".to_string(),
                Value::Object(result.metadata.into_iter().collect()),
            ),
        ]);
        let result_record = NewEventLogRecord {
            id: new_id("evt"),
            kind: "capability.invocation.result".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id: command.resident_id.clone(),
            source: "capability".to_string(),
            device_id: source_event.device_id.clone(),
            surface_id: source_event.surface_id.clone(),
            actor_id: command
                .target
                .as_ref()
                .and_then(|target| target.actor_id.clone()),
            payload: result_payload,
            causality: Some(Causality {
                source_event_id: Some(source_event.id.clone()),
                source_command_id: Some(command.id.clone()),
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

fn json_map_from_value(value: Value) -> JsonMap {
    match value {
        Value::Object(map) => map.into_iter().collect(),
        other => JsonMap::from([("value".to_string(), other)]),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use yuukei_event_log::EventLogQuery;
    use yuukei_extension::DialogueSuffixExtension;
    use yuukei_protocol::{SurfaceKind, SurfacePresentation, SurfaceRenderer};
    use yuukei_world::{
        ActorDefinition, CapabilityDeclarations, DaihonConfig, DaihonScriptSource, SignalAllowlist,
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
                profile: JsonMap::new(),
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

    #[tokio::test]
    async fn headless_text_event_is_logged_and_broadcasts_dialogue() -> Result<()> {
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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
        assert_eq!(
            commands[1].payload["speechRef"],
            format!("stub-speech://{}", commands[1].id)
        );
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
        assert!(records.contains(&"capability.invocation.request".to_string()));
        assert!(records.contains(&"capability.invocation.result".to_string()));

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
    async fn disallowed_signal_is_logged_but_not_dispatched() -> Result<()> {
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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
    async fn extension_can_rewrite_dialogue_before_emit_and_tts() -> Result<()> {
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
        home.register_extension(DialogueSuffixExtension::new("nya-suffix", "にゃ"))?;
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

        let records = home.event_log().read(EventLogQuery::default())?.records;
        assert!(records
            .iter()
            .any(|record| record.kind == "extension.hook.result"));
        let speech_request = records
            .iter()
            .find(|record| record.kind == "capability.invocation.request")
            .expect("speech request");
        assert_eq!(
            speech_request.payload["input"]["text"],
            "聞こえています。こんにちはにゃ"
        );
        Ok(())
    }

    #[tokio::test]
    async fn rejects_world_pack_with_missing_required_capability() -> Result<()> {
        let mut world = world_pack();
        world.capabilities.required = vec!["dialogue.generate".to_string()];
        let error = match ResidentHome::new("resident-default", world, EventLog::in_memory()?).await
        {
            Ok(_) => panic!("missing required capability should reject the world pack"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            ResidentHomeError::MissingRequiredCapabilities(_)
        ));
        Ok(())
    }
}
