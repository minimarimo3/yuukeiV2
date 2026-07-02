use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::broadcast;
use yuukei_capability::{
    CapabilityError, CapabilityProvider, CapabilityRouter, EventLogReadGrant,
    StubSpeechSynthesisProvider, DEFAULT_SPEECH_SYNTHESIS_EXTENSION_ID,
};
use yuukei_event_log::{EventLog, EventLogError, EventLogPage, EventLogQuery};
use yuukei_extension::{
    event_type_matches, ExtensionCommandContext, ExtensionError, ExtensionEventContext,
    ExtensionEventReport, ExtensionHookReport, ExtensionRegistry, YuukeiExtension,
};
use yuukei_protocol::{
    new_id, ActorSnapshot, CapabilityInvocation, Causality, EventLogRecord, ExtensionHookPoint,
    JsonMap, NewEventLogRecord, ResidentSnapshot, RuntimeCommand, RuntimeEvent, SignalAliasTable,
    SurfaceSession,
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
    #[error("event log read denied: {0}")]
    EventLogReadDenied(String),
    #[error("world pack requires unavailable capabilities: {0}")]
    MissingRequiredCapabilities(String),
    #[error("state lock is poisoned")]
    PoisonedLock,
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ResidentHomeError>;

pub const MAX_EXTENSION_EVENT_HOPS: u32 = 4;

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
        daihon
            .load_world_with_signal_aliases(&world_pack, &SignalAliasTable::default())
            .await?;
        if !capabilities
            .summaries()
            .contains_key(DEFAULT_SPEECH_SYNTHESIS_EXTENSION_ID)
        {
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
        self.process_appended_runtime_event(event, appended).await?;
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

    pub fn set_default_capability_extension(
        &self,
        capability: impl Into<String>,
        extension_id: impl Into<String>,
    ) -> Result<()> {
        self.capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .set_default_extension(capability, extension_id);
        Ok(())
    }

    pub async fn register_extension<E>(&self, extension: E) -> Result<()>
    where
        E: YuukeiExtension + 'static,
    {
        self.extensions
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .register(extension)?;
        self.reload_daihon_signal_aliases().await?;
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

    pub fn read_event_log_for_extension(&self, grant: EventLogReadGrant) -> Result<EventLogPage> {
        let validated = self.validate_event_log_read_grant(&grant)?;
        let page = self.event_log.read(EventLogQuery {
            resident_id: Some(grant.resident_id.clone()),
            kind: None,
            after_sequence: grant.cursor_after_sequence,
            limit: None,
            extension_readable_only: true,
        })?;

        let mut records = Vec::new();
        for mut record in page.records {
            if !event_type_matches(&validated.event_types, &record.kind) {
                continue;
            }
            if let Some(until_timestamp) = validated.until_timestamp.as_ref() {
                let record_timestamp = parse_rfc3339_utc(&record.timestamp)?;
                if &record_timestamp > until_timestamp {
                    continue;
                }
            }
            if !validated.privacy_categories.is_empty()
                && !record
                    .privacy
                    .as_ref()
                    .is_some_and(|privacy| validated.privacy_categories.contains(&privacy.category))
            {
                continue;
            }
            if !validated.allow_payloads {
                record.payload.clear();
            } else if !validated.allow_references {
                strip_references_from_payload(&mut record.payload);
            }
            records.push(record);
            if records.len() >= validated.max_records {
                break;
            }
        }
        let next_cursor = records.last().map(|record| record.sequence);
        Ok(EventLogPage {
            records,
            next_cursor,
        })
    }

    fn validate_event_log_read_grant(
        &self,
        grant: &EventLogReadGrant,
    ) -> Result<ValidatedEventLogReadGrant> {
        if grant.resident_id != self.resident_id()? {
            return Err(ResidentHomeError::EventLogReadDenied(format!(
                "grant resident {} does not match this Resident Home",
                grant.resident_id
            )));
        }

        let permission = self
            .extensions
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .event_log_read_permission(&grant.extension_id)
            .ok_or_else(|| {
                ResidentHomeError::EventLogReadDenied(format!(
                    "extension is not registered, enabled, or allowed to read the event log: {}",
                    grant.extension_id
                ))
            })?;

        let expires_at = parse_rfc3339_utc(&grant.expires_at)?;
        if expires_at <= Utc::now() {
            return Err(ResidentHomeError::EventLogReadDenied(format!(
                "grant expired at {}",
                grant.expires_at
            )));
        }

        if permission.event_types.is_empty() {
            return Err(ResidentHomeError::EventLogReadDenied(format!(
                "extension manifest does not allow event log event types: {}",
                grant.extension_id
            )));
        }
        let event_types = if grant.event_types.is_empty() {
            permission.event_types.clone()
        } else {
            for requested in &grant.event_types {
                if !event_type_matches(&permission.event_types, requested) {
                    return Err(ResidentHomeError::EventLogReadDenied(format!(
                        "requested event type is outside manifest permission: {requested}"
                    )));
                }
            }
            grant.event_types.clone()
        };

        for requested_category in &grant.privacy_categories {
            if !permission
                .privacy_categories
                .iter()
                .any(|allowed| allowed == requested_category)
            {
                return Err(ResidentHomeError::EventLogReadDenied(format!(
                    "requested privacy category is outside manifest permission: {requested_category}"
                )));
            }
        }

        let until_timestamp = grant
            .until_timestamp
            .as_deref()
            .map(parse_rfc3339_utc)
            .transpose()?;

        Ok(ValidatedEventLogReadGrant {
            event_types,
            privacy_categories: grant.privacy_categories.clone(),
            until_timestamp,
            max_records: grant.max_records.min(permission.max_records),
            allow_payloads: grant.allow_payloads && permission.allow_payloads,
            allow_references: grant.allow_references && permission.allow_references,
        })
    }

    pub async fn ingest_event(&self, event: RuntimeEvent) -> Result<Vec<RuntimeCommand>> {
        let appended_event = self
            .event_log
            .append(NewEventLogRecord::from(event.clone()))?;
        self.set_cursor(appended_event.sequence)?;
        self.process_appended_runtime_event(event, appended_event)
            .await
    }

    async fn process_appended_runtime_event(
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
            emitted_commands.extend(self.dispatch_recorded_event(event).await?);
        }

        Ok(emitted_commands)
    }

    async fn notify_extensions_event_appended(
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

    fn normalize_extension_event(
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

    fn record_extension_event_rejection(
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

    async fn dispatch_recorded_event(&self, event: RuntimeEvent) -> Result<Vec<RuntimeCommand>> {
        let aliases = self.extension_signal_alias_table()?;
        if !self
            .world_pack
            .allows_signal_with_aliases(&event.kind, &aliases)
        {
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

    async fn reload_daihon_signal_aliases(&self) -> Result<()> {
        let aliases = self.extension_signal_alias_table()?;
        self.daihon
            .load_world_with_signal_aliases(&self.world_pack, &aliases)
            .await?;
        Ok(())
    }

    fn extension_signal_alias_table(&self) -> Result<SignalAliasTable> {
        let aliases = self
            .extensions
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .signal_aliases();
        Ok(SignalAliasTable::with_standard_and_donated(aliases))
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
            (
                "extensionId".to_string(),
                Value::String(result.extension_id),
            ),
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

#[derive(Clone, Debug)]
struct ValidatedEventLogReadGrant {
    event_types: Vec<String>,
    privacy_categories: Vec<String>,
    until_timestamp: Option<DateTime<Utc>>,
    max_records: usize,
    allow_payloads: bool,
    allow_references: bool,
}

fn parse_rfc3339_utc(timestamp: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(timestamp)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| {
            ResidentHomeError::EventLogReadDenied(format!("invalid timestamp {timestamp}: {error}"))
        })
}

fn strip_references_from_payload(payload: &mut JsonMap) {
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use serde_json::{json, Value};
    use tempfile::tempdir;
    use yuukei_event_log::EventLogQuery;
    use yuukei_extension::{
        DialogueSuffixExtension, ProcessCommandSpec, ProcessExtensionManifest,
        ProcessHookExtension, YuukeiExtension,
    };
    use yuukei_protocol::{
        ExecutionLocation, ExtensionEventInvocation, ExtensionEventLogReadPermission,
        ExtensionEventResult, ExtensionEventSubscription, ExtensionHookAction,
        ExtensionHookInvocation, ExtensionHookResult, ExtensionPermissions, ExtensionRuntimeKind,
        ExtensionSignalAlias, ExtensionSummary, Privacy, RetentionPolicy, SurfaceKind,
        SurfacePresentation, SurfaceRenderer,
    };
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
            format!("yuukei-default-tts://{}", commands[1].id)
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
        home.register_extension(DialogueSuffixExtension::new("nya-suffix", "にゃ"))
            .await?;
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
    async fn extension_event_emission_requires_declared_namespace() -> Result<()> {
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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
            process: ProcessCommandSpec {
                command: "node".to_string(),
                args: vec!["emit.js".to_string()],
                cwd: None,
                timeout_ms: Some(5_000),
            },
        };
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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

        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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
    async fn extension_event_log_read_grant_rejects_unregistered_expired_and_out_of_scope(
    ) -> Result<()> {
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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
