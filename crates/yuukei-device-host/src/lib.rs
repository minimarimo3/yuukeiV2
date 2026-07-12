use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{DateTime, Local, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use thiserror::Error;
use tokio::task::JoinHandle;
use yuukei_capability::CapabilityRouter;
use yuukei_event_log::{
    DeleteSummary, EventLog, EventLogPrivacyFilter, EventLogQuery,
    DEFAULT_EVENT_LOG_TRIM_FRACTION_DIVISOR, DEFAULT_MAX_EVENT_LOG_RECORDS,
};
use yuukei_extension::{ProcessHookExtension, ProcessRuntimeSupervisor};
use yuukei_protocol::{
    new_id, now_timestamp, Causality, EventLogRecord, ExtensionHookPoint, JsonMap, MemoryEntryKind,
    MemoryForgetEntry, MemoryForgetOutput, MemoryListOutput, MemoryUpdateOutput, Privacy,
    ResidentSnapshot, RetentionPolicy, RuntimeCommand, RuntimeEvent, SurfaceKind,
    SurfacePresentation, SurfaceRenderer, SurfaceSession,
};
pub use yuukei_resident_home::ResidentEventLogPage;
use yuukei_resident_home::{
    ResidentEventLogReadOptions, ResidentHome, ResidentHomeError, ResidentRuntimeSettings,
};
use yuukei_world::{
    ActorHitZoneShape, ActorHitZoneSource, ActorRendererKind, DaihonDiagnosticEntry,
    SceneHistoryEntry, WorldError, WorldPack, YuukeiDaihonAdapter,
};

mod extension_settings;
mod world_pack_registry;

use extension_settings::{ExtensionRuntimeEntry, ExtensionSettingsRegistry};
pub use extension_settings::{
    ExtensionSettingsChangeResult, ExtensionSettingsState, InstalledExtension, TRUSTED_CODE_NOTICE,
};
pub use world_pack_registry::{
    LocalRuntimeEnvironment, WorldPackInstall, WorldPackSelectionState, WorldPackSource,
    WorldPackSwitchResult, DEFAULT_WORLD_PACK_INSTALL_ID,
};

pub const DEFAULT_RESIDENT_ID: &str = "resident-default";
pub const DEFAULT_DEVICE_ID: &str = "device-local";
pub const TAURI_SURFACE_ID: &str = "surface-tauri";
pub const CLI_SURFACE_ID: &str = "surface-cli";
pub const PRESENCE_LIFE_TICK_INTERVAL: Duration = Duration::from_secs(5 * 60);
pub const DEFAULT_TALK_INTERVAL_MINUTES: u64 = 5;
pub const DEFAULT_ACTOR_SCALE_PERCENT: u16 = 100;
pub const MIN_ACTOR_SCALE_PERCENT: u16 = 50;
pub const MAX_ACTOR_SCALE_PERCENT: u16 = 200;
pub const DEFAULT_LLM_TIMEOUT_MS: u64 = 30_000;
pub const MIN_LLM_TIMEOUT_MS: u64 = 1_000;
pub const MAX_LLM_TIMEOUT_MS: u64 = 300_000;
pub const DEFAULT_RECENT_CONTEXT_COUNT: usize = 20;
pub const MAX_RECENT_CONTEXT_COUNT: usize = 100;
pub const DEFAULT_TALK_DESIRE_LOW: u8 = 30;
pub const DEFAULT_TALK_DESIRE_HIGH: u8 = 80;
const PRESENCE_LOOP_POLL_INTERVAL: Duration = Duration::from_secs(1);
const PRESENCE_IDLE_THRESHOLD: Duration = Duration::from_secs(5 * 60);
const TALK_IMPULSE_RECENT_ACTIVITY_SUPPRESSION: Duration = Duration::from_secs(60);
const EVENT_LOG_TRIM_CHECK_INTERVAL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Error)]
pub enum DeviceHostError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("event log error: {0}")]
    EventLog(#[from] yuukei_event_log::EventLogError),
    #[error("resident home error: {0}")]
    ResidentHome(#[from] ResidentHomeError),
    #[error("world error: {0}")]
    World(#[from] WorldError),
    #[error("app log error: {0}")]
    AppLog(#[from] AppLogError),
    #[error("extension settings error: {0}")]
    ExtensionSettings(String),
    #[error("app settings error: {0}")]
    AppSettings(String),
    #[error("stage settings error: {0}")]
    StageSettings(String),
    #[error("runtime settings error: {0}")]
    RuntimeSettings(String),
    #[error("observation settings error: {0}")]
    ObservationSettings(String),
    #[error("onboarding settings error: {0}")]
    OnboardingSettings(String),
    #[error("world pack import error: {0}")]
    WorldPackImport(String),
    #[error("presence state lock is poisoned")]
    PresenceState,
    #[error("Daihon diagnostic state lock is poisoned")]
    DaihonDiagnosticState,
}

pub type Result<T> = std::result::Result<T, DeviceHostError>;

impl DeviceHostError {
    pub fn daihon_report(&self) -> Option<&yuukei_world::DaihonDiagnosticReport> {
        match self {
            Self::ResidentHome(error) => error.daihon_report(),
            Self::World(error) => error.daihon_report(),
            Self::Io(_)
            | Self::Json(_)
            | Self::EventLog(_)
            | Self::AppLog(_)
            | Self::ExtensionSettings(_)
            | Self::AppSettings(_)
            | Self::StageSettings(_)
            | Self::RuntimeSettings(_)
            | Self::ObservationSettings(_)
            | Self::OnboardingSettings(_)
            | Self::WorldPackImport(_)
            | Self::PresenceState
            | Self::DaihonDiagnosticState => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePaths {
    pub workspace_root: PathBuf,
    pub data_dir: PathBuf,
    pub world_root: PathBuf,
    pub extension_root: PathBuf,
    pub event_log_path: PathBuf,
    pub scene_history_path: PathBuf,
    pub variables_path: PathBuf,
    pub mood_state_path: PathBuf,
    pub app_log_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalRuntimeConfig {
    pub install_id: String,
    pub resident_id: String,
    pub device_id: String,
    pub workspace_root: PathBuf,
    pub data_dir: PathBuf,
    pub world_root: PathBuf,
    pub extension_root: PathBuf,
    pub event_log_path: PathBuf,
    pub scene_history_path: PathBuf,
    pub variables_path: PathBuf,
    pub mood_state_path: PathBuf,
    pub app_log_path: PathBuf,
}

impl LocalRuntimeConfig {
    pub fn default_local() -> Self {
        let env = LocalRuntimeEnvironment::default_local();
        let workspace_root = env.workspace_root;
        let data_dir = env.data_dir;
        let world_root = env.default_world_root;
        let extension_root = data_dir.join("extensions");
        Self {
            install_id: DEFAULT_WORLD_PACK_INSTALL_ID.to_string(),
            resident_id: DEFAULT_RESIDENT_ID.to_string(),
            device_id: DEFAULT_DEVICE_ID.to_string(),
            event_log_path: data_dir
                .join("residents")
                .join(DEFAULT_WORLD_PACK_INSTALL_ID)
                .join("events.sqlite3"),
            scene_history_path: data_dir
                .join("residents")
                .join(DEFAULT_WORLD_PACK_INSTALL_ID)
                .join("scene-history.json"),
            variables_path: data_dir
                .join("residents")
                .join(DEFAULT_WORLD_PACK_INSTALL_ID)
                .join("variables.json"),
            mood_state_path: data_dir
                .join("residents")
                .join(DEFAULT_WORLD_PACK_INSTALL_ID)
                .join("mood.json"),
            app_log_path: data_dir.join("app-activity.jsonl"),
            workspace_root,
            data_dir,
            world_root,
            extension_root,
        }
    }

    pub fn paths(&self) -> RuntimePaths {
        RuntimePaths {
            workspace_root: self.workspace_root.clone(),
            data_dir: self.data_dir.clone(),
            world_root: self.world_root.clone(),
            extension_root: self.extension_root.clone(),
            event_log_path: self.event_log_path.clone(),
            scene_history_path: self.scene_history_path.clone(),
            variables_path: self.variables_path.clone(),
            mood_state_path: self.mood_state_path.clone(),
            app_log_path: self.app_log_path.clone(),
        }
    }

    fn extension_settings_registry(&self) -> Result<ExtensionSettingsRegistry> {
        ExtensionSettingsRegistry::open(&self.data_dir, &self.extension_root)
    }
}

mod app_logger;
mod bootstrap;
mod capability_usage;
mod desktop_observation;
mod event_builders;
mod life_rhythm;
mod paths;
mod runtime;
mod runtime_settings_api;
mod settings;
mod surface_assets;
#[cfg(test)]
mod tests;
mod world_pack_import;

pub use app_logger::*;
pub use capability_usage::*;
pub use desktop_observation::*;
pub use event_builders::*;
pub use life_rhythm::*;
pub use runtime::*;
pub use settings::*;
pub use surface_assets::*;
pub use world_pack_import::*;

#[allow(unused_imports)]
pub(crate) use bootstrap::*;
#[allow(unused_imports)]
pub(crate) use capability_usage::*;
#[allow(unused_imports)]
pub(crate) use desktop_observation::*;
#[allow(unused_imports)]
pub(crate) use life_rhythm::*;
#[allow(unused_imports)]
pub(crate) use paths::*;
#[allow(unused_imports)]
pub(crate) use runtime::*;
#[allow(unused_imports)]
pub(crate) use settings::*;
#[allow(unused_imports)]
pub(crate) use surface_assets::*;
#[allow(unused_imports)]
pub(crate) use world_pack_import::*;
