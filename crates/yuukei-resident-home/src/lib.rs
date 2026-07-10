use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::{
    sync::{broadcast, oneshot},
    time::timeout,
};
use yuukei_capability::{
    CapabilityError, CapabilityProvider, CapabilityResult, CapabilityRouter, EventLogReadGrant,
    DIALOGUE_EXTRACT_CAPABILITY, DIALOGUE_GENERATE_CAPABILITY, DIALOGUE_INTERPRET_CAPABILITY,
    MEMORY_FORGET_CAPABILITY, MEMORY_INDEX_CAPABILITY, MEMORY_LIST_CAPABILITY,
    MEMORY_RETRIEVE_CAPABILITY, MEMORY_UPDATE_CAPABILITY, MOOD_EVALUATE_CAPABILITY,
    SPEECH_SYNTHESIS_CAPABILITY,
};
use yuukei_event_log::{
    DeleteSummary, EventLog, EventLogAdminQuery, EventLogDeleteSelector, EventLogError,
    EventLogPage, EventLogPrivacyFilter, EventLogQuery, TrimSummary,
};
use yuukei_extension::{
    event_type_matches, ExtensionCommandContext, ExtensionError, ExtensionEventContext,
    ExtensionEventReport, ExtensionHookReport, ExtensionRegistry, ProcessFailureKind,
    ProcessFailureReport, YuukeiExtension,
};
use yuukei_protocol::{
    new_id, ActorSnapshot, CapabilityInvocation, Causality, CommandTarget, DialogueExtractInput,
    DialogueExtractOutput, DialogueGenerateConstraints, DialogueGenerateEvent,
    DialogueGenerateInput, DialogueGenerateOutput, DialogueGeneratePersona,
    DialogueGenerateRecentContext, DialogueInterpretInput, DialogueInterpretOutput,
    DialogueInterpretTextInput, EventLogRecord, ExtensionHookPoint, JsonMap, MemoryEntryKind,
    MemoryForgetEntry, MemoryForgetInput, MemoryForgetOutput, MemoryIndexEvent, MemoryIndexInput,
    MemoryIndexOutput, MemoryListInput, MemoryListOutput, MemoryRetrieveInput,
    MemoryRetrieveLimits, MemoryRetrieveOutput, MemoryRetrieveQuery, MemoryUpdateInput,
    MemoryUpdateOutput, MoodEvaluateInput, MoodEvaluateOutput, NewEventLogRecord, ResidentSnapshot,
    RuntimeCommand, RuntimeEvent, SignalAliasTable, SurfaceSession,
};
use yuukei_world::{
    DaihonAdapter, DaihonChoiceRequest, DaihonDiagnosticEntry, DaihonDiagnosticReport,
    DaihonExtractRequest, DaihonGenerateRequest, DaihonGenerateResponse, DaihonInterpretHandler,
    DaihonInterpretRequest, WorldError, WorldPack, YuukeiDaihonAdapter,
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidentEventLogPage {
    pub records: Vec<EventLogRecord>,
    pub next_cursor: Option<i64>,
    pub total: usize,
}

#[derive(Clone, Debug, Default)]
pub struct ResidentEventLogReadOptions {
    pub kind_prefix: Option<String>,
    pub privacy_category: EventLogPrivacyFilter,
    pub before_sequence: Option<i64>,
    pub limit: Option<usize>,
}

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
const DEFAULT_LLM_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_RECENT_CONTEXT_COUNT: usize = 20;
const MEMORY_RETRIEVE_TIMEOUT: Duration = Duration::from_secs(10);
const SPEECH_SYNTHESIS_TIMEOUT: Duration = Duration::from_secs(10);
const MOOD_EVALUATE_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_MEMORY_INDEX_DAYS_PER_TRIGGER: usize = 7;
const MEMORY_RETRIEVE_FACT_LIMIT: usize = 10;
const MEMORY_RETRIEVE_EPISODE_LIMIT: usize = 5;
const MAX_QUEUED_CONVERSATION_EVENTS_DURING_INTERPRET: usize = 16;
const UNKNOWN_INTERPRETATION: &str = "不明";
const DEFAULT_MOOD_INTERVAL_MINUTES: u64 = 10;
const DEFAULT_LOW_TALK_DESIRE_THRESHOLD: u8 = 30;
const DEFAULT_HIGH_TALK_DESIRE_THRESHOLD: u8 = 80;
const MOOD_STATE_MAX_AGE: chrono::Duration = chrono::Duration::minutes(60);
const MOOD_CHANGED_EVENT: &str = "ext.yuukei-intelligence.mood.changed";
const TALK_IMPULSE_EVENT: &str = "presence.talk_impulse";

impl ResidentHomeError {
    pub fn daihon_report(&self) -> Option<&DaihonDiagnosticReport> {
        match self {
            Self::World(error) => error.daihon_report(),
            Self::EventLog(_)
            | Self::Capability(_)
            | Self::Extension(_)
            | Self::EventLogReadDenied(_)
            | Self::MissingRequiredCapabilities(_)
            | Self::PoisonedLock
            | Self::Serialization(_) => None,
        }
    }
}

#[derive(Clone)]
pub struct ResidentHome {
    event_log: EventLog,
    world_pack: Arc<WorldPack>,
    daihon: Arc<dyn DaihonAdapter>,
    capabilities: Arc<Mutex<CapabilityRouter>>,
    extensions: Arc<Mutex<ExtensionRegistry>>,
    runtime_settings: ResidentRuntimeSettings,
    state: Arc<Mutex<HomeState>>,
    command_tx: broadcast::Sender<RuntimeCommand>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResidentRuntimeSettings {
    pub llm_timeout: Duration,
    pub recent_context_count: usize,
    pub talk_desire_low: u8,
    pub talk_desire_high: u8,
    pub mood_state_path: Option<PathBuf>,
}

impl Default for ResidentRuntimeSettings {
    fn default() -> Self {
        Self {
            llm_timeout: DEFAULT_LLM_TIMEOUT,
            recent_context_count: DEFAULT_RECENT_CONTEXT_COUNT,
            talk_desire_low: DEFAULT_LOW_TALK_DESIRE_THRESHOLD,
            talk_desire_high: DEFAULT_HIGH_TALK_DESIRE_THRESHOLD,
            mood_state_path: None,
        }
    }
}

#[derive(Debug)]
struct HomeState {
    resident_id: String,
    active_surface_id: Option<String>,
    actors: BTreeMap<String, ActorSnapshot>,
    surfaces: BTreeMap<String, SurfaceSession>,
    recent_event_cursor: i64,
    daihon_diagnostics: Vec<DaihonDiagnosticEntry>,
    llm_delegation: LlmDelegationCounters,
    interpretation: InterpretationState,
    mood: MoodState,
}

#[derive(Clone, Debug, Default)]
struct LlmDelegationCounters {
    cooldowns: BTreeMap<String, DateTime<Utc>>,
    daily_budget: Option<DailyBudgetCounter>,
}

#[derive(Clone, Debug)]
struct DailyBudgetCounter {
    date: NaiveDate,
    used: u32,
}

#[derive(Debug, Default)]
struct InterpretationState {
    in_flight: bool,
    queued_events: VecDeque<(RuntimeEvent, EventLogRecord)>,
    pending_choice: Option<PendingChoice>,
}

#[derive(Debug)]
struct PendingChoice {
    choice_id: String,
    sender: oneshot::Sender<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MoodState {
    last_evaluated_at: Option<DateTime<Utc>>,
    current: Option<MoodSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct MoodSnapshot {
    mood: String,
    talk_desire: u8,
    topic: String,
}

#[derive(Default)]
struct DispatchOutcome {
    commands: Vec<RuntimeCommand>,
    events: Vec<RuntimeEvent>,
}

enum TalkImpulseModeration {
    Dispatch(RuntimeEvent),
    Skip { source_event: RuntimeEvent },
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

    pub async fn with_parts_and_runtime_settings(
        resident_id: impl Into<String>,
        world_pack: WorldPack,
        event_log: EventLog,
        daihon: Arc<dyn DaihonAdapter>,
        capabilities: CapabilityRouter,
        runtime_settings: ResidentRuntimeSettings,
    ) -> Result<Self> {
        Self::with_parts_and_extensions_and_runtime_settings(
            resident_id,
            world_pack,
            event_log,
            daihon,
            capabilities,
            ExtensionRegistry::new(),
            runtime_settings,
        )
        .await
    }

    pub async fn with_parts_and_extensions(
        resident_id: impl Into<String>,
        world_pack: WorldPack,
        event_log: EventLog,
        daihon: Arc<dyn DaihonAdapter>,
        capabilities: CapabilityRouter,
        extensions: ExtensionRegistry,
    ) -> Result<Self> {
        Self::with_parts_and_extensions_and_runtime_settings(
            resident_id,
            world_pack,
            event_log,
            daihon,
            capabilities,
            extensions,
            ResidentRuntimeSettings::default(),
        )
        .await
    }

    pub async fn with_parts_and_extensions_and_runtime_settings(
        resident_id: impl Into<String>,
        world_pack: WorldPack,
        event_log: EventLog,
        daihon: Arc<dyn DaihonAdapter>,
        capabilities: CapabilityRouter,
        extensions: ExtensionRegistry,
        runtime_settings: ResidentRuntimeSettings,
    ) -> Result<Self> {
        world_pack.validate()?;
        daihon
            .load_world_with_signal_aliases(&world_pack, &SignalAliasTable::default())
            .await?;
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
        let mood = load_mood_state(runtime_settings.mood_state_path.as_ref());
        let (command_tx, _) = broadcast::channel(128);
        Ok(Self {
            event_log,
            world_pack: Arc::new(world_pack),
            daihon,
            capabilities: Arc::new(Mutex::new(capabilities)),
            extensions: Arc::new(Mutex::new(extensions)),
            runtime_settings,
            state: Arc::new(Mutex::new(HomeState {
                resident_id,
                active_surface_id: None,
                actors,
                surfaces: BTreeMap::new(),
                recent_event_cursor: 0,
                daihon_diagnostics: Vec::new(),
                llm_delegation: LlmDelegationCounters::default(),
                interpretation: InterpretationState::default(),
                mood,
            })),
            command_tx,
        })
    }

    pub fn event_log(&self) -> EventLog {
        self.event_log.clone()
    }

    pub fn trim_event_log_to_record_limit(
        &self,
        max_records: usize,
        fraction_divisor: usize,
    ) -> Result<TrimSummary> {
        let summary = self
            .event_log
            .trim_to_record_limit(max_records, fraction_divisor)?;
        if summary.deleted == 0 {
            return Ok(summary);
        }
        let record = NewEventLogRecord {
            id: new_id("evt"),
            kind: "event_log.trimmed".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id: self.resident_id()?,
            source: "resident-home".to_string(),
            device_id: None,
            surface_id: None,
            actor_id: None,
            payload: JsonMap::from([
                ("deleted".to_string(), json!(summary.deleted)),
                (
                    "oldestTimestamp".to_string(),
                    summary
                        .oldest_timestamp
                        .as_ref()
                        .map(|value| Value::String(value.clone()))
                        .unwrap_or(Value::Null),
                ),
                (
                    "newestTimestamp".to_string(),
                    summary
                        .newest_timestamp
                        .as_ref()
                        .map(|value| Value::String(value.clone()))
                        .unwrap_or(Value::Null),
                ),
            ]),
            causality: None,
            privacy: None,
        };
        let appended = self.event_log.append(record)?;
        self.set_cursor(appended.sequence)?;
        Ok(summary)
    }

    pub fn subscribe_commands(&self) -> broadcast::Receiver<RuntimeCommand> {
        self.command_tx.subscribe()
    }

    pub fn daihon_diagnostics(&self) -> Result<Vec<DaihonDiagnosticEntry>> {
        let state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        Ok(state.daihon_diagnostics.clone())
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
            privacy: None,
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

    pub fn read_event_log_page(
        &self,
        options: ResidentEventLogReadOptions,
    ) -> Result<ResidentEventLogPage> {
        let query = EventLogAdminQuery {
            kind_prefix: options
                .kind_prefix
                .filter(|prefix| !prefix.trim().is_empty()),
            privacy_category: options.privacy_category,
            before_sequence: options.before_sequence,
            limit: options.limit,
        };
        let total = self
            .event_log
            .read_newest(EventLogAdminQuery {
                limit: None,
                ..query.clone()
            })?
            .records
            .len();
        let page = self.event_log.read_newest(query)?;
        Ok(ResidentEventLogPage {
            records: page.records,
            next_cursor: page.next_cursor,
            total,
        })
    }

    pub fn count_event_log_delete_before(&self, timestamp: impl Into<String>) -> Result<usize> {
        self.event_log
            .count_delete(EventLogDeleteSelector::BeforeTimestamp(timestamp.into()))
            .map_err(Into::into)
    }

    pub fn count_event_log_delete_by_kind_prefix(
        &self,
        prefix: impl Into<String>,
    ) -> Result<usize> {
        self.event_log
            .count_delete(EventLogDeleteSelector::KindPrefix(prefix.into()))
            .map_err(Into::into)
    }

    pub fn count_event_log_delete_all(&self) -> Result<usize> {
        self.event_log
            .count_delete(EventLogDeleteSelector::All)
            .map_err(Into::into)
    }

    pub fn delete_event_log_before(&self, timestamp: impl Into<String>) -> Result<DeleteSummary> {
        let timestamp = timestamp.into();
        self.delete_event_log_with_audit(
            EventLogDeleteSelector::BeforeTimestamp(timestamp.clone()),
            json!({ "condition": "before", "timestamp": timestamp }),
        )
    }

    pub fn delete_event_log_by_kind_prefix(
        &self,
        prefix: impl Into<String>,
    ) -> Result<DeleteSummary> {
        let prefix = prefix.into();
        self.delete_event_log_with_audit(
            EventLogDeleteSelector::KindPrefix(prefix.clone()),
            json!({ "condition": "kindPrefix", "kindPrefix": prefix }),
        )
    }

    pub fn delete_event_log_all(&self) -> Result<DeleteSummary> {
        self.delete_event_log_with_audit(EventLogDeleteSelector::All, json!({ "condition": "all" }))
    }

    fn delete_event_log_with_audit(
        &self,
        selector: EventLogDeleteSelector,
        mut payload: Value,
    ) -> Result<DeleteSummary> {
        let resident_id = self.resident_id()?;
        let deleted = self.event_log.count_delete(selector.clone())?;
        if let Value::Object(map) = &mut payload {
            map.insert("deleted".to_string(), Value::Number(deleted.into()));
        }
        let audit = NewEventLogRecord {
            id: new_id("evt"),
            kind: "event_log.deleted".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id,
            source: "resident-home".to_string(),
            device_id: None,
            surface_id: None,
            actor_id: None,
            payload: json_map_from_value(payload),
            causality: None,
            privacy: None,
        };
        let summary = self.event_log.delete_with_audit(selector, audit)?;
        let page = self.event_log.read_newest(EventLogAdminQuery {
            limit: Some(1),
            ..Default::default()
        })?;
        if let Some(record) = page.records.first() {
            self.set_cursor(record.sequence)?;
        }
        Ok(summary)
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

    fn resolve_pending_choice_event(&self, event: &RuntimeEvent) -> Result<bool> {
        if event.kind != "conversation.choice" {
            return Ok(false);
        }
        let Some(choice_id) = event.payload.get("choiceId").and_then(Value::as_str) else {
            return Ok(true);
        };
        let Some(choice) = event.payload.get("choice").and_then(Value::as_str) else {
            return Ok(true);
        };
        let sender = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?;
            let matches_pending = state
                .interpretation
                .pending_choice
                .as_ref()
                .is_some_and(|pending| pending.choice_id == choice_id);
            if matches_pending {
                state
                    .interpretation
                    .pending_choice
                    .take()
                    .map(|pending| pending.sender)
            } else {
                None
            }
        };
        if let Some(sender) = sender {
            let _ = sender.send(choice.to_string());
        }
        Ok(true)
    }

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

    fn defer_event_while_interpreting(
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

    async fn drain_interpretation_queue(&self) -> Result<Vec<RuntimeCommand>> {
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

    fn record_interpretation_queue_drop(
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

    async fn dispatch_recorded_event(
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

    fn enrich_event_for_daihon_dispatch(&self, mut event: RuntimeEvent) -> Result<RuntimeEvent> {
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

    fn recent_download_for_folder_event(&self, event: &RuntimeEvent) -> Result<(String, String)> {
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

    async fn maybe_evaluate_mood(
        &self,
        source_event: &RuntimeEvent,
    ) -> Result<Option<RuntimeEvent>> {
        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        if !router.has_healthy_provider(MOOD_EVALUATE_CAPABILITY) {
            return Ok(None);
        }
        let interval_minutes =
            mood_interval_minutes(&router).unwrap_or(DEFAULT_MOOD_INTERVAL_MINUTES);
        if interval_minutes == 0 {
            return Ok(None);
        }
        let now = event_timestamp_or_now(source_event);
        {
            let state = self
                .state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?;
            if state.mood.last_evaluated_at.is_some_and(|last| {
                now.signed_duration_since(last).num_minutes() < interval_minutes as i64
            }) {
                return Ok(None);
            }
        }

        let input = self.mood_evaluate_input(source_event, now)?;
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: MOOD_EVALUATE_CAPABILITY.to_string(),
            method: "evaluate".to_string(),
            resident_id: source_event.resident_id.clone(),
            actor_id: Some(self.world_pack.default_actor_id.clone()),
            input: json_map_omitting_null_values(serde_json::to_value(input)?),
            context: None,
        };
        self.record_capability_request(&invocation, source_event, None)?;
        let Some(result) = self
            .invoke_capability_with_timeout(
                router,
                invocation.clone(),
                MOOD_EVALUATE_TIMEOUT,
                source_event,
            )
            .await?
        else {
            return Ok(None);
        };
        self.record_capability_result(CapabilityResultRecord {
            invocation_id: result.invocation_id,
            extension_id: result.extension_id.clone(),
            capability: result.capability,
            output: result.output.clone(),
            metadata: result.metadata,
            source_event,
            source_command_id: None,
            actor_id: invocation.actor_id.as_deref(),
        })?;

        let output_value = Value::Object(result.output.into_iter().collect());
        let Ok(output) = serde_json::from_value::<MoodEvaluateOutput>(output_value) else {
            return Ok(None);
        };
        let snapshot = MoodSnapshot {
            mood: normalize_mood_word(&output.mood).to_string(),
            talk_desire: output.talk_desire.min(100),
            topic: output.topic.trim().to_string(),
        };
        let mood_to_save = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?;
            state.mood.current = Some(snapshot.clone());
            state.mood.last_evaluated_at = Some(now);
            state.mood.clone()
        };
        save_mood_state(
            self.runtime_settings.mood_state_path.as_ref(),
            &mood_to_save,
        );
        if !self.extension_can_emit_mood_changed(&result.extension_id)? {
            return Ok(None);
        }
        Ok(Some(self.mood_changed_event(
            source_event,
            &result.extension_id,
            &snapshot,
        )))
    }

    fn mood_evaluate_input(
        &self,
        source_event: &RuntimeEvent,
        now: DateTime<Utc>,
    ) -> Result<MoodEvaluateInput> {
        let actor = self
            .world_pack
            .actors
            .iter()
            .find(|actor| actor.id == self.world_pack.default_actor_id)
            .ok_or_else(|| {
                ResidentHomeError::MissingRequiredCapabilities(format!(
                    "actor is not declared: {}",
                    self.world_pack.default_actor_id
                ))
            })?;
        Ok(MoodEvaluateInput {
            resident_id: source_event.resident_id.clone(),
            world_pack_id: self.world_pack.id.clone(),
            current_time: source_event.timestamp.clone(),
            time_period: source_event
                .payload
                .get("timePeriod")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            seconds_since_last_user_activity: self
                .seconds_since_last_user_activity(&source_event.resident_id, now)?,
            persona: DialogueGeneratePersona {
                actor_id: actor.id.clone(),
                display_name: actor.display_name.clone(),
                profile: actor.profile.clone(),
            },
            recent_context: self.recent_dialogue_context(&source_event.resident_id)?,
        })
    }

    fn seconds_since_last_user_activity(
        &self,
        resident_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<u64>> {
        let records = self
            .event_log
            .read(EventLogQuery {
                resident_id: Some(resident_id.to_string()),
                kind: None,
                after_sequence: None,
                limit: None,
                extension_readable_only: false,
            })?
            .records;
        for record in records.into_iter().rev() {
            if !(record.kind.starts_with("conversation.")
                || record.kind.starts_with("avatar.gesture."))
            {
                continue;
            }
            let Some(timestamp) = event_record_timestamp(&record.timestamp) else {
                continue;
            };
            return Ok(now
                .signed_duration_since(timestamp)
                .to_std()
                .ok()
                .map(|duration| duration.as_secs()));
        }
        Ok(None)
    }

    fn extension_can_emit_mood_changed(&self, extension_id: &str) -> Result<bool> {
        Ok(self
            .extensions
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .can_emit_event(extension_id, MOOD_CHANGED_EVENT))
    }

    fn mood_changed_event(
        &self,
        source_event: &RuntimeEvent,
        extension_id: &str,
        mood: &MoodSnapshot,
    ) -> RuntimeEvent {
        let mut event = RuntimeEvent::new(
            MOOD_CHANGED_EVENT,
            "extension",
            source_event.resident_id.clone(),
        );
        event.device_id = source_event.device_id.clone();
        event.surface_id = source_event.surface_id.clone();
        event.actor_id = Some(self.world_pack.default_actor_id.clone());
        event.causality = Some(Causality {
            source_event_id: Some(source_event.id.clone()),
            source_command_id: None,
            trace_id: source_event
                .causality
                .as_ref()
                .and_then(|causality| causality.trace_id.clone()),
        });
        event.payload = JsonMap::from([
            ("mood".to_string(), json!(mood.mood)),
            ("talkDesire".to_string(), json!(mood.talk_desire)),
            ("topic".to_string(), json!(mood.topic)),
            (
                "yuukeiExtension".to_string(),
                json!({ "extensionId": extension_id, "hopCount": 0 }),
            ),
        ]);
        event
    }

    fn apply_mood_changed_event(
        &self,
        event: &RuntimeEvent,
        record: &EventLogRecord,
    ) -> Result<Option<RuntimeEvent>> {
        let snapshot = mood_snapshot_from_payload(&event.payload);
        let mood_to_save = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?;
            state.mood.current = Some(snapshot.clone());
            state.mood.last_evaluated_at = event_record_timestamp(&record.timestamp);
            state.mood.clone()
        };
        save_mood_state(
            self.runtime_settings.mood_state_path.as_ref(),
            &mood_to_save,
        );
        if snapshot.talk_desire < self.runtime_settings.talk_desire_high {
            return Ok(None);
        }
        let mut talk = RuntimeEvent::new(
            TALK_IMPULSE_EVENT,
            "resident-home",
            event.resident_id.clone(),
        );
        talk.device_id = event.device_id.clone();
        talk.surface_id = event.surface_id.clone();
        talk.actor_id = event.actor_id.clone();
        talk.payload = current_talk_impulse_payload(&snapshot, Some("mood.changed"));
        talk.causality = Some(Causality {
            source_event_id: Some(record.id.clone()),
            source_command_id: None,
            trace_id: event
                .causality
                .as_ref()
                .and_then(|causality| causality.trace_id.clone()),
        });
        Ok(Some(talk))
    }

    fn moderate_talk_impulse_event(
        &self,
        mut event: RuntimeEvent,
    ) -> Result<TalkImpulseModeration> {
        let mood = {
            self.state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?
                .mood
                .current
                .clone()
        };
        let mood = mood.unwrap_or_else(default_mood_snapshot);
        event.payload.insert("気分".to_string(), json!(mood.mood));
        event.payload.insert("話題".to_string(), json!(mood.topic));
        event.payload.insert("mood".to_string(), json!(mood.mood));
        event.payload.insert("topic".to_string(), json!(mood.topic));
        event
            .payload
            .insert("talkDesire".to_string(), json!(mood.talk_desire));
        if mood.talk_desire < self.runtime_settings.talk_desire_low {
            return Ok(TalkImpulseModeration::Skip {
                source_event: event,
            });
        }
        Ok(TalkImpulseModeration::Dispatch(event))
    }

    fn record_talk_impulse_skip(&self, event: &RuntimeEvent) -> Result<()> {
        let record = NewEventLogRecord {
            id: new_id("evt"),
            kind: "presence.talk_impulse.skipped".to_string(),
            timestamp: yuukei_protocol::now_timestamp(),
            resident_id: event.resident_id.clone(),
            source: "resident-home".to_string(),
            device_id: event.device_id.clone(),
            surface_id: event.surface_id.clone(),
            actor_id: event.actor_id.clone(),
            payload: JsonMap::from([
                ("reason".to_string(), json!("low-talk-desire")),
                (
                    "mood".to_string(),
                    event.payload.get("mood").cloned().unwrap_or(Value::Null),
                ),
                (
                    "talkDesire".to_string(),
                    event
                        .payload
                        .get("talkDesire")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                (
                    "topic".to_string(),
                    event.payload.get("topic").cloned().unwrap_or(Value::Null),
                ),
            ]),
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
        let appended = self.event_log.append(record)?;
        self.set_cursor(appended.sequence)?;
        Ok(())
    }

    async fn emit_command_for_event(
        &self,
        command: RuntimeCommand,
        source_event: &RuntimeEvent,
    ) -> Result<RuntimeCommand> {
        let command = self
            .apply_extensions_before_command_emit(command, source_event)
            .await?;
        let appended_command = self
            .event_log
            .append(NewEventLogRecord::from(command.clone()))?;
        self.set_cursor(appended_command.sequence)?;
        self.apply_command_to_snapshot(&command)?;
        let _ = self.command_tx.send(command.clone());
        self.spawn_speech_synthesis_if_needed(command.clone(), source_event.clone())?;
        Ok(command)
    }

    fn emit_internal_command_without_extensions(
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

    async fn maybe_generate_dialogue_fallback(
        &self,
        event: &RuntimeEvent,
        aliases: &SignalAliasTable,
    ) -> Result<Vec<RuntimeCommand>> {
        let Some(delegation) = self
            .world_pack
            .llm_delegation_for_signal_with_aliases(&event.kind, aliases)
        else {
            return Ok(Vec::new());
        };
        let canonical_signal = aliases.canonicalize(&delegation.signal);
        if !self.try_start_llm_delegation(&canonical_signal, delegation.cooldown_seconds)? {
            return Ok(Vec::new());
        }

        let memories = self.retrieve_memories_for_dialogue_generate(event).await?;
        let input =
            self.dialogue_generate_input(event, &self.world_pack.default_actor_id, None, memories)?;
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: DIALOGUE_GENERATE_CAPABILITY.to_string(),
            method: "generate".to_string(),
            resident_id: event.resident_id.clone(),
            actor_id: Some(self.world_pack.default_actor_id.clone()),
            input: json_map_omitting_null_values(serde_json::to_value(input)?),
            context: None,
        };
        self.record_capability_request(&invocation, event, None)?;

        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        let Some(result) = self
            .invoke_capability_with_timeout(
                router,
                invocation.clone(),
                self.runtime_settings.llm_timeout,
                event,
            )
            .await?
        else {
            return Ok(Vec::new());
        };

        let output_value = Value::Object(result.output.clone().into_iter().collect());
        let Ok(output) = serde_json::from_value::<DialogueGenerateOutput>(output_value) else {
            return Ok(Vec::new());
        };
        self.record_capability_result(CapabilityResultRecord {
            invocation_id: result.invocation_id,
            extension_id: result.extension_id,
            capability: result.capability,
            output: result.output,
            metadata: result.metadata,
            source_event: event,
            source_command_id: None,
            actor_id: invocation.actor_id.as_deref(),
        })?;
        self.commands_from_dialogue_generate_output(output, event)
    }

    fn try_start_llm_delegation(
        &self,
        signal: &str,
        cooldown_seconds: Option<u64>,
    ) -> Result<bool> {
        let now = Utc::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        if let Some(limit) = self.world_pack.llm_delegation.daily_budget {
            let today = now.date_naive();
            let counter = state
                .llm_delegation
                .daily_budget
                .get_or_insert(DailyBudgetCounter {
                    date: today,
                    used: 0,
                });
            if counter.date != today {
                counter.date = today;
                counter.used = 0;
            }
            if counter.used >= limit {
                return Ok(false);
            }
        }
        if let Some(cooldown_seconds) = cooldown_seconds {
            if let Some(last_called_at) = state.llm_delegation.cooldowns.get(signal) {
                if now.signed_duration_since(*last_called_at).num_seconds()
                    < cooldown_seconds as i64
                {
                    return Ok(false);
                }
            }
        }
        state
            .llm_delegation
            .cooldowns
            .insert(signal.to_string(), now);
        Ok(true)
    }

    fn record_llm_speech_budget_use(&self) -> Result<()> {
        if self.world_pack.llm_delegation.daily_budget.is_none() {
            return Ok(());
        }
        let today = Utc::now().date_naive();
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        let counter = state
            .llm_delegation
            .daily_budget
            .get_or_insert(DailyBudgetCounter {
                date: today,
                used: 0,
            });
        if counter.date != today {
            counter.date = today;
            counter.used = 0;
        }
        counter.used = counter.used.saturating_add(1);
        Ok(())
    }

    fn dialogue_generate_input(
        &self,
        event: &RuntimeEvent,
        actor_id: &str,
        instruction: Option<String>,
        memories: Option<Vec<String>>,
    ) -> Result<DialogueGenerateInput> {
        let actor = self
            .world_pack
            .actors
            .iter()
            .find(|actor| actor.id == actor_id)
            .ok_or_else(|| {
                ResidentHomeError::MissingRequiredCapabilities(format!(
                    "actor is not declared: {actor_id}"
                ))
            })?;
        Ok(DialogueGenerateInput {
            event: DialogueGenerateEvent {
                kind: event.kind.clone(),
                payload: event.payload.clone(),
            },
            instruction,
            memories,
            persona: DialogueGeneratePersona {
                actor_id: actor.id.clone(),
                display_name: actor.display_name.clone(),
                profile: actor.profile.clone(),
            },
            recent_context: self.recent_dialogue_context(&event.resident_id)?,
            constraints: DialogueGenerateConstraints { max_length: 120 },
        })
    }

    fn recent_dialogue_context(
        &self,
        resident_id: &str,
    ) -> Result<Vec<DialogueGenerateRecentContext>> {
        let limit = self.runtime_settings.recent_context_count;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut records = self
            .event_log
            .read(EventLogQuery {
                resident_id: Some(resident_id.to_string()),
                kind: None,
                after_sequence: None,
                limit: None,
                extension_readable_only: false,
            })?
            .records;
        if records.len() > limit {
            records = records.split_off(records.len() - limit);
        }
        Ok(records
            .into_iter()
            .map(|record| DialogueGenerateRecentContext {
                kind: record.kind,
                timestamp: record.timestamp,
                payload: major_payload(record.payload),
            })
            .collect())
    }

    fn commands_from_dialogue_generate_output(
        &self,
        output: DialogueGenerateOutput,
        source_event: &RuntimeEvent,
    ) -> Result<Vec<RuntimeCommand>> {
        if !output.speak {
            return Ok(Vec::new());
        }
        let Some(text) = output.text.filter(|text| !text.trim().is_empty()) else {
            return Ok(Vec::new());
        };
        self.record_llm_speech_budget_use()?;

        let actor_id = self.world_pack.default_actor_id.clone();
        let mut commands = Vec::new();
        if let Some(expression) = output.expression.filter(|value| !value.trim().is_empty()) {
            let mut command =
                generated_command("avatar.expression", source_event, actor_id.clone());
            command.payload = JsonMap::from([
                ("expression".to_string(), Value::String(expression)),
                ("speakerId".to_string(), Value::String(actor_id.clone())),
                (
                    "sourceCapability".to_string(),
                    Value::String(DIALOGUE_GENERATE_CAPABILITY.to_string()),
                ),
            ]);
            commands.push(command);
        }
        if let Some(motion) = output.motion.filter(|value| !value.trim().is_empty()) {
            let mut command = generated_command("avatar.motion", source_event, actor_id.clone());
            command.payload = JsonMap::from([
                ("motion".to_string(), Value::String(motion)),
                ("speakerId".to_string(), Value::String(actor_id.clone())),
                (
                    "sourceCapability".to_string(),
                    Value::String(DIALOGUE_GENERATE_CAPABILITY.to_string()),
                ),
            ]);
            commands.push(command);
        }
        let mut command = generated_command("dialogue.say", source_event, actor_id.clone());
        command.payload = JsonMap::from([
            ("text".to_string(), Value::String(text)),
            ("speakerId".to_string(), Value::String(actor_id)),
            ("emotion".to_string(), Value::String("neutral".to_string())),
            (
                "sourceCapability".to_string(),
                Value::String(DIALOGUE_GENERATE_CAPABILITY.to_string()),
            ),
        ]);
        commands.push(command);
        Ok(commands)
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

    fn record_daihon_report(&self, report: &DaihonDiagnosticReport) -> Result<()> {
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

    async fn handle_process_failure_report(
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

    async fn handle_capability_error(
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

    async fn invoke_capability_with_timeout(
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

    fn record_extension_process_failure(
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

    async fn maybe_index_memory_for_trigger(&self, trigger_event: &RuntimeEvent) -> Result<()> {
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

    async fn retrieve_memories_for_dialogue_generate(
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

    async fn generate_dialogue_for_daihon(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonGenerateRequest,
    ) -> Result<Option<DaihonGenerateResponse>> {
        self.set_interpretation_in_flight(true)?;
        let result = self
            .invoke_dialogue_generate_for_daihon(source_event, request)
            .await
            .unwrap_or(None);
        self.set_interpretation_in_flight(false)?;
        Ok(result)
    }

    async fn invoke_dialogue_generate_for_daihon(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonGenerateRequest,
    ) -> Result<Option<DaihonGenerateResponse>> {
        let actor_id = request
            .speaker_id
            .as_deref()
            .unwrap_or(&self.world_pack.default_actor_id)
            .to_string();
        let input = self.dialogue_generate_input(
            source_event,
            &actor_id,
            Some(request.instruction.clone()),
            self.retrieve_memories_for_dialogue_generate(source_event)
                .await?,
        )?;
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: DIALOGUE_GENERATE_CAPABILITY.to_string(),
            method: "generate".to_string(),
            resident_id: source_event.resident_id.clone(),
            actor_id: Some(actor_id.clone()),
            input: json_map_from_value(serde_json::to_value(input)?),
            context: None,
        };
        self.record_capability_request(&invocation, source_event, None)?;

        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        let Some(result) = self
            .invoke_capability_with_timeout(
                router,
                invocation.clone(),
                self.runtime_settings.llm_timeout,
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
            actor_id: invocation.actor_id.as_deref(),
        })?;
        let output_value = Value::Object(result.output.into_iter().collect());
        let Ok(output) = serde_json::from_value::<DialogueGenerateOutput>(output_value) else {
            return Ok(None);
        };
        if !output.speak {
            return Ok(None);
        }
        let Some(text) = output.text.filter(|text| !text.trim().is_empty()) else {
            return Ok(None);
        };
        Ok(Some(DaihonGenerateResponse {
            text,
            expression: output.expression,
            motion: output.motion,
        }))
    }

    async fn interpret_dialogue(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonInterpretRequest,
    ) -> Result<String> {
        self.set_interpretation_in_flight(true)?;
        let result = self
            .invoke_dialogue_interpret(source_event, request)
            .await
            .unwrap_or_else(|_| UNKNOWN_INTERPRETATION.to_string());
        self.set_interpretation_in_flight(false)?;
        Ok(result)
    }

    async fn invoke_dialogue_interpret(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonInterpretRequest,
    ) -> Result<String> {
        let input = DialogueInterpretInput {
            question: request.question,
            choices: request.choices.clone(),
            input: DialogueInterpretTextInput {
                text: request.input_text,
            },
        };
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: DIALOGUE_INTERPRET_CAPABILITY.to_string(),
            method: "interpret".to_string(),
            resident_id: source_event.resident_id.clone(),
            actor_id: None,
            input: json_map_from_value(serde_json::to_value(input)?),
            context: None,
        };
        self.record_capability_request(&invocation, source_event, None)?;

        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        let Some(result) = self
            .invoke_capability_with_timeout(
                router,
                invocation.clone(),
                self.runtime_settings.llm_timeout,
                source_event,
            )
            .await?
        else {
            return Ok(UNKNOWN_INTERPRETATION.to_string());
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
        let Ok(output) = serde_json::from_value::<DialogueInterpretOutput>(output_value) else {
            return Ok(UNKNOWN_INTERPRETATION.to_string());
        };
        let choice = output.choice.trim();
        if choice == UNKNOWN_INTERPRETATION
            || request.choices.iter().any(|candidate| candidate == choice)
        {
            Ok(choice.to_string())
        } else {
            Ok(UNKNOWN_INTERPRETATION.to_string())
        }
    }

    async fn extract_dialogue(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonExtractRequest,
    ) -> Result<String> {
        self.set_interpretation_in_flight(true)?;
        let result = self
            .invoke_dialogue_extract(source_event, request)
            .await
            .unwrap_or_else(|_| UNKNOWN_INTERPRETATION.to_string());
        self.set_interpretation_in_flight(false)?;
        Ok(result)
    }

    async fn invoke_dialogue_extract(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonExtractRequest,
    ) -> Result<String> {
        let input = DialogueExtractInput {
            instruction: request.instruction,
            input: DialogueInterpretTextInput {
                text: request.input_text,
            },
        };
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: DIALOGUE_EXTRACT_CAPABILITY.to_string(),
            method: "extract".to_string(),
            resident_id: source_event.resident_id.clone(),
            actor_id: None,
            input: json_map_from_value(serde_json::to_value(input)?),
            context: None,
        };
        self.record_capability_request(&invocation, source_event, None)?;

        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        let Some(result) = self
            .invoke_capability_with_timeout(
                router,
                invocation.clone(),
                self.runtime_settings.llm_timeout,
                source_event,
            )
            .await?
        else {
            return Ok(UNKNOWN_INTERPRETATION.to_string());
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
        let Ok(output) = serde_json::from_value::<DialogueExtractOutput>(output_value) else {
            return Ok(UNKNOWN_INTERPRETATION.to_string());
        };
        let value = output.value.trim();
        if output.found && !value.is_empty() && value.chars().count() <= 100 {
            Ok(value.to_string())
        } else {
            Ok(UNKNOWN_INTERPRETATION.to_string())
        }
    }

    async fn choose_for_daihon(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonChoiceRequest,
    ) -> Result<String> {
        let result = self
            .invoke_choice_for_daihon(source_event, request)
            .await
            .unwrap_or_else(|_| UNKNOWN_INTERPRETATION.to_string());
        self.set_interpretation_in_flight(false)?;
        Ok(result)
    }

    async fn invoke_choice_for_daihon(
        &self,
        source_event: &RuntimeEvent,
        request: DaihonChoiceRequest,
    ) -> Result<String> {
        let choice_id = new_id("choice");
        let timeout_seconds = request.timeout_seconds;
        let mut command = RuntimeCommand::new(
            "dialogue.choices",
            "daihon",
            source_event.resident_id.clone(),
        );
        command.causality = Some(Causality {
            source_event_id: Some(source_event.id.clone()),
            source_command_id: None,
            trace_id: source_event
                .causality
                .as_ref()
                .and_then(|causality| causality.trace_id.clone()),
        });
        command.target = Some(CommandTarget {
            device_id: source_event.device_id.clone(),
            surface_id: source_event.surface_id.clone(),
            actor_id: Some(self.world_pack.default_actor_id.clone()),
        });
        command.payload = JsonMap::from([
            ("choiceId".to_string(), json!(choice_id)),
            ("choices".to_string(), json!(request.choices)),
            ("timeoutSeconds".to_string(), json!(timeout_seconds)),
        ]);
        self.emit_command_for_event(command, source_event).await?;

        let (sender, receiver) = oneshot::channel();
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| ResidentHomeError::PoisonedLock)?;
            state.interpretation.pending_choice = Some(PendingChoice {
                choice_id: choice_id.clone(),
                sender,
            });
        }
        self.set_interpretation_in_flight(true)?;

        match timeout(Duration::from_secs(timeout_seconds), receiver).await {
            Ok(Ok(choice)) => Ok(choice),
            Ok(Err(_)) | Err(_) => {
                self.clear_pending_choice(&choice_id)?;
                self.emit_choice_clear(source_event, &choice_id, "timeout")
                    .await?;
                Ok(UNKNOWN_INTERPRETATION.to_string())
            }
        }
    }

    fn clear_pending_choice(&self, choice_id: &str) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        if state
            .interpretation
            .pending_choice
            .as_ref()
            .is_some_and(|pending| pending.choice_id == choice_id)
        {
            state.interpretation.pending_choice = None;
        }
        Ok(())
    }

    async fn emit_choice_clear(
        &self,
        source_event: &RuntimeEvent,
        choice_id: &str,
        reason: &str,
    ) -> Result<()> {
        let mut command = RuntimeCommand::new(
            "dialogue.choices.clear",
            "daihon",
            source_event.resident_id.clone(),
        );
        command.causality = Some(Causality {
            source_event_id: Some(source_event.id.clone()),
            source_command_id: None,
            trace_id: source_event
                .causality
                .as_ref()
                .and_then(|causality| causality.trace_id.clone()),
        });
        command.target = Some(CommandTarget {
            device_id: source_event.device_id.clone(),
            surface_id: source_event.surface_id.clone(),
            actor_id: Some(self.world_pack.default_actor_id.clone()),
        });
        command.payload = JsonMap::from([
            ("choiceId".to_string(), json!(choice_id)),
            ("reason".to_string(), json!(reason)),
        ]);
        self.emit_command_for_event(command, source_event).await?;
        Ok(())
    }

    fn set_interpretation_in_flight(&self, in_flight: bool) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        state.interpretation.in_flight = in_flight;
        Ok(())
    }

    fn spawn_speech_synthesis_if_needed(
        &self,
        command: RuntimeCommand,
        source_event: RuntimeEvent,
    ) -> Result<()> {
        if command.kind != "dialogue.say" {
            return Ok(());
        }
        let Some(text) = command.payload.get("text").and_then(Value::as_str) else {
            return Ok(());
        };
        if text.trim().is_empty() {
            return Ok(());
        }

        let has_provider = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .has_healthy_provider(SPEECH_SYNTHESIS_CAPABILITY);
        if !has_provider {
            return Ok(());
        }

        let home = Arc::new(self.clone());
        tokio::spawn(async move {
            let _ = home
                .synthesize_speech_for_dialogue(command, source_event)
                .await;
        });
        Ok(())
    }

    async fn synthesize_speech_for_dialogue(
        &self,
        command: RuntimeCommand,
        source_event: RuntimeEvent,
    ) -> Result<()> {
        let text = command
            .payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let invocation = CapabilityInvocation {
            id: new_id("cap"),
            capability: SPEECH_SYNTHESIS_CAPABILITY.to_string(),
            method: "synthesize".to_string(),
            resident_id: command.resident_id.clone(),
            actor_id: command
                .target
                .as_ref()
                .and_then(|target| target.actor_id.clone()),
            input: JsonMap::from([
                ("text".to_string(), Value::String(text)),
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

        self.record_capability_request(&invocation, &source_event, Some(&command.id))?;

        let router = self
            .capabilities
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?
            .clone();
        let Some(result) = self
            .invoke_capability_with_timeout(
                router,
                invocation,
                SPEECH_SYNTHESIS_TIMEOUT,
                &source_event,
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
            source_event: &source_event,
            source_command_id: Some(&command.id),
            actor_id: command
                .target
                .as_ref()
                .and_then(|target| target.actor_id.as_deref()),
        })?;
        let Some(audio_path) = result.output.get("audioPath").and_then(Value::as_str) else {
            return Ok(());
        };
        if audio_path.trim().is_empty() {
            return Ok(());
        }
        let mut audio_command =
            RuntimeCommand::new("audio.play", "capability", command.resident_id.clone());
        audio_command.target = command.target.clone();
        audio_command.causality = Some(Causality {
            source_event_id: command
                .causality
                .as_ref()
                .and_then(|causality| causality.source_event_id.clone())
                .or_else(|| Some(source_event.id.clone())),
            source_command_id: Some(command.id.clone()),
            trace_id: command
                .causality
                .as_ref()
                .and_then(|causality| causality.trace_id.clone()),
        });
        audio_command.payload = JsonMap::from([
            (
                "audioPath".to_string(),
                Value::String(audio_path.to_string()),
            ),
            (
                "durationMs".to_string(),
                result
                    .output
                    .get("durationMs")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
        ]);
        self.emit_command_for_event(audio_command, &source_event)
            .await?;
        Ok(())
    }

    fn record_capability_request(
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

    fn record_capability_result(&self, record: CapabilityResultRecord<'_>) -> Result<()> {
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

struct ResidentHomeInterpretHandler {
    home: ResidentHome,
    source_event: RuntimeEvent,
}

#[async_trait]
impl DaihonInterpretHandler for ResidentHomeInterpretHandler {
    async fn interpret(&mut self, request: DaihonInterpretRequest) -> String {
        self.home
            .interpret_dialogue(&self.source_event, request)
            .await
            .unwrap_or_else(|_| UNKNOWN_INTERPRETATION.to_string())
    }

    async fn flush_commands_before_choice(&mut self, commands: Vec<RuntimeCommand>) -> bool {
        for command in commands {
            if self
                .home
                .emit_command_for_event(command, &self.source_event)
                .await
                .is_err()
            {
                return false;
            }
        }
        true
    }

    async fn generate(&mut self, request: DaihonGenerateRequest) -> Option<DaihonGenerateResponse> {
        self.home
            .generate_dialogue_for_daihon(&self.source_event, request)
            .await
            .unwrap_or(None)
    }

    async fn extract(&mut self, request: DaihonExtractRequest) -> String {
        self.home
            .extract_dialogue(&self.source_event, request)
            .await
            .unwrap_or_else(|_| UNKNOWN_INTERPRETATION.to_string())
    }

    async fn choose(&mut self, request: DaihonChoiceRequest) -> String {
        self.home
            .choose_for_daihon(&self.source_event, request)
            .await
            .unwrap_or_else(|_| UNKNOWN_INTERPRETATION.to_string())
    }
}

fn json_map_from_value(value: Value) -> JsonMap {
    match value {
        Value::Object(map) => map.into_iter().collect(),
        other => JsonMap::from([("value".to_string(), other)]),
    }
}

fn json_map_omitting_null_values(value: Value) -> JsonMap {
    match value {
        Value::Object(map) => map
            .into_iter()
            .filter(|(_, value)| !value.is_null())
            .collect(),
        other => JsonMap::from([("value".to_string(), other)]),
    }
}

fn generated_command(
    kind: impl Into<String>,
    source_event: &RuntimeEvent,
    actor_id: String,
) -> RuntimeCommand {
    let mut command = RuntimeCommand::new(
        kind,
        "capability.dialogue.generate",
        source_event.resident_id.clone(),
    );
    command.causality = Some(Causality {
        source_event_id: Some(source_event.id.clone()),
        source_command_id: None,
        trace_id: source_event
            .causality
            .as_ref()
            .and_then(|causality| causality.trace_id.clone()),
    });
    command.target = Some(CommandTarget {
        device_id: source_event.device_id.clone(),
        surface_id: source_event.surface_id.clone(),
        actor_id: Some(actor_id),
    });
    command
}

struct CapabilityResultRecord<'a> {
    invocation_id: String,
    extension_id: String,
    capability: String,
    output: JsonMap,
    metadata: JsonMap,
    source_event: &'a RuntimeEvent,
    source_command_id: Option<&'a str>,
    actor_id: Option<&'a str>,
}

fn major_payload(payload: JsonMap) -> JsonMap {
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

fn event_record_date(timestamp: &str) -> Option<NaiveDate> {
    event_record_timestamp(timestamp).map(|timestamp| timestamp.date_naive())
}

fn event_record_timestamp(timestamp: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn event_timestamp_or_now(event: &RuntimeEvent) -> DateTime<Utc> {
    event_record_timestamp(&event.timestamp).unwrap_or_else(Utc::now)
}

fn mood_interval_minutes(router: &CapabilityRouter) -> Option<u64> {
    router
        .runtime_settings_for(MOOD_EVALUATE_CAPABILITY)?
        .get("mood.intervalMinutes")
        .and_then(Value::as_u64)
}

fn load_mood_state(path: Option<&PathBuf>) -> MoodState {
    let Some(path) = path else {
        return MoodState::default();
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return MoodState::default();
    };
    let Ok(state) = serde_json::from_str::<MoodState>(&raw) else {
        return MoodState::default();
    };
    let Some(last) = state.last_evaluated_at else {
        return MoodState::default();
    };
    if Utc::now().signed_duration_since(last) > MOOD_STATE_MAX_AGE {
        return MoodState::default();
    }
    state
}

fn save_mood_state(path: Option<&PathBuf>, state: &MoodState) {
    let Some(path) = path else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(state) {
        let _ = std::fs::write(path, bytes);
    }
}

fn normalize_mood_word(value: &str) -> &str {
    match value.trim() {
        "うれしい" => "うれしい",
        "たいくつ" => "たいくつ",
        "さみしい" => "さみしい",
        "心配" => "心配",
        "ねむい" => "ねむい",
        "ふつう" => "ふつう",
        _ => "ふつう",
    }
}

fn mood_snapshot_from_payload(payload: &JsonMap) -> MoodSnapshot {
    let mood = payload
        .get("mood")
        .and_then(Value::as_str)
        .map(normalize_mood_word)
        .unwrap_or("ふつう")
        .to_string();
    let talk_desire = payload
        .get("talkDesire")
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value.min(100)).ok())
        .unwrap_or(50);
    let topic = payload
        .get("topic")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    MoodSnapshot {
        mood,
        talk_desire,
        topic,
    }
}

fn default_mood_snapshot() -> MoodSnapshot {
    MoodSnapshot {
        mood: "ふつう".to_string(),
        talk_desire: 50,
        topic: String::new(),
    }
}

fn current_talk_impulse_payload(mood: &MoodSnapshot, trigger: Option<&str>) -> JsonMap {
    let mut payload = JsonMap::from([
        ("気分".to_string(), json!(mood.mood)),
        ("話題".to_string(), json!(mood.topic)),
        ("mood".to_string(), json!(mood.mood)),
        ("topic".to_string(), json!(mood.topic)),
        ("talkDesire".to_string(), json!(mood.talk_desire)),
    ]);
    if let Some(trigger) = trigger {
        payload.insert("trigger".to_string(), json!(trigger));
    }
    payload
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
    use yuukei_capability::{CapabilityError, CapabilityResult, ProviderRegistration};
    use yuukei_event_log::EventLogQuery;
    use yuukei_extension::{
        DialogueSuffixExtension, ProcessCommandSpec, ProcessExtensionManifest,
        ProcessHookExtension, YuukeiExtension,
    };
    use yuukei_protocol::{
        ExecutionLocation, ExtensionEventInvocation, ExtensionEventLogReadPermission,
        ExtensionEventResult, ExtensionEventSubscription, ExtensionHookAction,
        ExtensionHookInvocation, ExtensionHookPoint, ExtensionHookResult,
        ExtensionHookSubscription, ExtensionPermissions, ExtensionRuntimeKind,
        ExtensionSignalAlias, ExtensionSummary, Privacy, RetentionPolicy, SurfaceKind,
        SurfacePresentation, SurfaceRenderer,
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
        let broadcast_dialogue = next_command_of_kind(&mut receiver, "dialogue.say").await;
        assert_eq!(broadcast_dialogue.id, dialogue.id);
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
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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

        let mut conversation =
            RuntimeEvent::new("conversation.text", "surface", "resident-default");
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
    async fn dialogue_generate_statement_sends_instruction_and_emits_generated_commands(
    ) -> Result<()> {
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
            text == "ん、聞いてます。……いまは、うまく言葉が出ないんですけど。"
                || text == "……(こくり)"
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
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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
            let command =
                tokio::time::timeout(std::time::Duration::from_millis(200), receiver.recv())
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
        let home =
            ResidentHome::new("resident-default", world_pack(), EventLog::in_memory()?).await?;
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

        let mut choice_event =
            RuntimeEvent::new("conversation.choice", "surface", "resident-default");
        choice_event
            .payload
            .insert("choiceId".to_string(), json!(choice_id));
        choice_event
            .payload
            .insert("choice".to_string(), json!("見る"));
        choice_event.payload.insert("index".to_string(), json!(0));
        assert!(home.ingest_event(choice_event).await?.is_empty());

        let commands = ingest.await.expect("choice dispatch task")?;
        assert!(commands
            .iter()
            .any(|command| command.kind == "dialogue.say"
                && command.payload["text"] == json!("見る枝")));
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
        assert!(commands
            .iter()
            .any(|command| command.kind == "dialogue.say"
                && command.payload["text"] == json!("不明枝")));
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

        let mut wrong_choice =
            RuntimeEvent::new("conversation.choice", "surface", "resident-default");
        wrong_choice
            .payload
            .insert("choiceId".to_string(), json!("choice_wrong"));
        wrong_choice
            .payload
            .insert("choice".to_string(), json!("見る"));
        wrong_choice.payload.insert("index".to_string(), json!(0));
        assert!(home.ingest_event(wrong_choice).await?.is_empty());
        assert!(!ingest.is_finished());

        let mut right_choice =
            RuntimeEvent::new("conversation.choice", "surface", "resident-default");
        right_choice
            .payload
            .insert("choiceId".to_string(), json!(choice_id));
        right_choice
            .payload
            .insert("choice".to_string(), json!("見る"));
        right_choice.payload.insert("index".to_string(), json!(0));
        home.ingest_event(right_choice).await?;

        let commands = ingest.await.expect("choice dispatch task")?;
        assert!(commands
            .iter()
            .any(|command| command.kind == "dialogue.say"
                && command.payload["text"] == json!("見る枝")));
        let records = home.event_log().read(EventLogQuery::default())?.records;
        assert!(records.iter().any(|record| {
            record.kind == "conversation.choice"
                && record.payload["choiceId"] == json!("choice_wrong")
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

        let mut queued_event =
            RuntimeEvent::new("conversation.text", "surface", "resident-default");
        queued_event.id = "evt_choice_queue_text".to_string();
        queued_event
            .payload
            .insert("text".to_string(), json!("queued"));
        assert!(home.ingest_event(queued_event).await?.is_empty());

        let mut choice_event =
            RuntimeEvent::new("conversation.choice", "surface", "resident-default");
        choice_event
            .payload
            .insert("choiceId".to_string(), json!(choice_id));
        choice_event
            .payload
            .insert("choice".to_string(), json!("見る"));
        choice_event.payload.insert("index".to_string(), json!(0));
        home.ingest_event(choice_event).await?;

        let commands = ingest.await.expect("choice dispatch task")?;
        assert!(commands
            .iter()
            .any(|command| command.kind == "dialogue.say"
                && command.payload["text"] == json!("見る枝")));
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
            settings: None,
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
