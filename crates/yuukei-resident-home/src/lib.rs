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

#[derive(Default)]
struct DispatchOutcome {
    commands: Vec<RuntimeCommand>,
    events: Vec<RuntimeEvent>,
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

    fn resident_id(&self) -> Result<String> {
        let state = self
            .state
            .lock()
            .map_err(|_| ResidentHomeError::PoisonedLock)?;
        Ok(state.resident_id.clone())
    }
}

mod choice;
mod dialogue;
mod event_flow;
mod event_log_admin;
mod extensions;
mod memory;
mod metrics;
mod mood;
mod speech;
#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use choice::*;
#[allow(unused_imports)]
pub(crate) use dialogue::*;
#[allow(unused_imports)]
pub(crate) use event_flow::*;
#[allow(unused_imports)]
pub(crate) use event_log_admin::*;
#[allow(unused_imports)]
pub(crate) use extensions::*;
#[allow(unused_imports)]
pub(crate) use memory::*;
#[allow(unused_imports)]
pub(crate) use metrics::*;
#[allow(unused_imports)]
pub(crate) use mood::*;
#[allow(unused_imports)]
pub(crate) use speech::*;
