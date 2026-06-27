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
use yuukei_protocol::{
    new_id, ActorSnapshot, CapabilityInvocation, Causality, JsonMap, NewEventLogRecord,
    ResidentSnapshot, RuntimeCommand, RuntimeEvent, SurfaceSession,
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
        mut capabilities: CapabilityRouter,
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
        Ok(ResidentSnapshot {
            resident_id: state.resident_id.clone(),
            world_pack_id: self.world_pack.id.clone(),
            active_surface_id: state.active_surface_id.clone(),
            actors: state.actors.clone(),
            surfaces: state.surfaces.clone(),
            capabilities,
            recent_event_cursor: state.recent_event_cursor.to_string(),
        })
    }

    pub fn attach_surface(&self, session: SurfaceSession) -> Result<ResidentSnapshot> {
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
        let appended = self.event_log.append(NewEventLogRecord::from(event))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        state.recent_event_cursor = appended.sequence;
        state.active_surface_id = Some(session.surface_id.clone());
        state.surfaces.insert(session.surface_id.clone(), session);
        drop(state);
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

    pub async fn ingest_event(&self, event: RuntimeEvent) -> Result<Vec<RuntimeCommand>> {
        let appended_event = self
            .event_log
            .append(NewEventLogRecord::from(event.clone()))?;
        self.set_cursor(appended_event.sequence)?;

        if !self.world_pack.allows_signal(&event.kind) {
            return Ok(Vec::new());
        }

        let mut result = self.daihon.dispatch(&event, &self.world_pack).await?;
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
        for command in &mut result.commands {
            self.maybe_enrich_speech(command, &event).await?;
            let appended_command = self
                .event_log
                .append(NewEventLogRecord::from(command.clone()))?;
            self.set_cursor(appended_command.sequence)?;
            self.apply_command_to_snapshot(command)?;
            let _ = self.command_tx.send(command.clone());
        }
        Ok(result.commands)
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
        })?;

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
}
