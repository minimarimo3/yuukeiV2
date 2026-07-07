use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
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
use yuukei_event_log::{EventLog, EventLogQuery};
use yuukei_extension::ProcessHookExtension;
use yuukei_protocol::{
    new_id, now_timestamp, EventLogRecord, ExtensionHookPoint, JsonMap, MemoryEntryKind,
    MemoryForgetEntry, MemoryForgetOutput, MemoryListOutput, MemoryUpdateOutput, ResidentSnapshot,
    RuntimeCommand, RuntimeEvent, SurfaceKind, SurfacePresentation, SurfaceRenderer,
    SurfaceSession,
};
use yuukei_resident_home::{ResidentHome, ResidentHomeError};
use yuukei_world::{
    ActorHitZoneShape, ActorHitZoneSource, ActorRendererKind, DaihonDiagnosticEntry, WorldError,
    WorldPack, YuukeiDaihonAdapter,
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
const PRESENCE_LOOP_POLL_INTERVAL: Duration = Duration::from_secs(1);
const PRESENCE_IDLE_THRESHOLD: Duration = Duration::from_secs(5 * 60);
const TALK_IMPULSE_RECENT_ACTIVITY_SUPPRESSION: Duration = Duration::from_secs(60);

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
            app_log_path: self.app_log_path.clone(),
        }
    }

    fn extension_settings_registry(&self) -> Result<ExtensionSettingsRegistry> {
        ExtensionSettingsRegistry::open(&self.data_dir, &self.extension_root)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsState {
    pub talk_interval_minutes: u64,
    pub settings_path: PathBuf,
}

#[derive(Clone, Debug)]
struct AppSettingsRegistry {
    settings_path: PathBuf,
    stored: StoredAppSettings,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredAppSettings {
    schema_version: u32,
    talk_interval_minutes: u64,
}

impl Default for StoredAppSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            talk_interval_minutes: DEFAULT_TALK_INTERVAL_MINUTES,
        }
    }
}

impl AppSettingsRegistry {
    fn open(data_dir: &Path) -> Result<Self> {
        let settings_path = data_dir.join("settings").join("app.json");
        let exists = settings_path.exists();
        let stored = if exists {
            let raw = fs::read_to_string(&settings_path)?;
            let stored: StoredAppSettings = serde_json::from_str(&raw)?;
            if stored.schema_version != 1 {
                return Err(DeviceHostError::AppSettings(format!(
                    "unsupported app settings schemaVersion: {}",
                    stored.schema_version
                )));
            }
            stored
        } else {
            StoredAppSettings::default()
        };
        let registry = Self {
            settings_path,
            stored,
        };
        if !exists {
            registry.save()?;
        }
        Ok(registry)
    }

    fn state(&self) -> AppSettingsState {
        AppSettingsState {
            talk_interval_minutes: self.stored.talk_interval_minutes,
            settings_path: self.settings_path.clone(),
        }
    }

    fn set_talk_interval_minutes(&mut self, minutes: u64) -> Result<AppSettingsState> {
        self.stored.talk_interval_minutes = minutes;
        self.save()?;
        Ok(self.state())
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.settings_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            &self.settings_path,
            serde_json::to_vec_pretty(&self.stored)?,
        )?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorSurfaceAssetCatalog {
    pub world_pack_id: String,
    pub actors: Vec<ActorSurfaceAsset>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorSurfaceAsset {
    pub actor_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer: Option<ActorSurfaceRendererAsset>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorSurfaceRendererAsset {
    pub kind: ActorSurfaceRendererKind,
    pub model: String,
    #[serde(default)]
    pub motions: BTreeMap<String, String>,
    #[serde(default)]
    pub hit_zones: Vec<ActorSurfaceHitZoneDefinition>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorSurfaceRendererKind {
    Vrm,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorSurfaceHitZoneDefinition {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub source: ActorSurfaceHitZoneSource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bones: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ActorSurfaceHitZoneShape>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorSurfaceHitZoneSource {
    HumanoidBone,
    NodeName,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorSurfaceHitZoneShape {
    Auto,
    Mesh,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityUsageState {
    pub extensions: Vec<ExtensionCapabilityUsage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionCapabilityUsage {
    pub extension_id: String,
    pub capabilities: Vec<CapabilityUsageByCapability>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityUsageByCapability {
    pub capability: String,
    pub models: Vec<ModelCapabilityUsage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilityUsage {
    pub provider: String,
    pub model: String,
    pub all_time: TokenUsageTotals,
    pub last_7_days: TokenUsageTotals,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageTotals {
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarGesturePoke {
    pub actor_id: String,
    pub hit_zone_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_zone_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_surface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_bone: Option<String>,
    pub input: AvatarGestureInput,
    pub screen: AvatarGestureScreen,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarGestureInput {
    pub kind: String,
    pub button: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarGestureScreen {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone)]
pub struct LocalYuukeiRuntime {
    home: Arc<ResidentHome>,
    logger: AppLogger,
    install_id: String,
    resident_id: String,
    device_id: String,
    paths: RuntimePaths,
    world_pack_status: WorldPackSelectionState,
    actor_surface_assets: ActorSurfaceAssetCatalog,
    presence_state: Arc<Mutex<PresenceState>>,
    session_daihon_diagnostics: Arc<Mutex<Vec<DaihonDiagnosticEntry>>>,
}

#[derive(Clone, Debug, Default)]
struct PresenceState {
    startup_emitted: bool,
    last_time_period: Option<String>,
    next_life_tick_at: Option<DateTime<Utc>>,
    last_user_activity_at: Option<DateTime<Utc>>,
    talk_interval_minutes: Option<u64>,
    next_talk_impulse_at: Option<DateTime<Utc>>,
    talk_rng_state: u64,
    idle_active: bool,
    last_idle_elapsed_seconds: Option<f64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalTimePeriod {
    Morning,
    Day,
    Evening,
    LateNight,
}

impl LocalTimePeriod {
    pub fn as_daihon_value(self) -> &'static str {
        match self {
            Self::Morning => "朝",
            Self::Day => "昼",
            Self::Evening => "夜",
            Self::LateNight => "深夜",
        }
    }
}

fn capability_usage_from_records(
    records: &[EventLogRecord],
    now: DateTime<Utc>,
) -> CapabilityUsageState {
    let cutoff = now - chrono::Duration::days(7);
    let mut usage_by_key: BTreeMap<
        (String, String, String, String),
        (TokenUsageTotals, TokenUsageTotals),
    > = BTreeMap::new();

    for record in records {
        if record.kind != "capability.invocation.result" {
            continue;
        }
        let Some(usage) = record
            .payload
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("usage"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        let Some(extension_id) = record.payload.get("extensionId").and_then(Value::as_str) else {
            continue;
        };
        let Some(capability) = record.payload.get("capability").and_then(Value::as_str) else {
            continue;
        };
        let Some(provider) = usage.get("provider").and_then(Value::as_str) else {
            continue;
        };
        let Some(model) = usage.get("model").and_then(Value::as_str) else {
            continue;
        };
        let Some(input_tokens) = usage.get("inputTokens").and_then(Value::as_u64) else {
            continue;
        };
        let Some(output_tokens) = usage.get("outputTokens").and_then(Value::as_u64) else {
            continue;
        };

        let timestamp = DateTime::parse_from_rfc3339(&record.timestamp)
            .map(|timestamp| timestamp.with_timezone(&Utc))
            .ok();
        let entry = usage_by_key
            .entry((
                extension_id.to_string(),
                capability.to_string(),
                provider.to_string(),
                model.to_string(),
            ))
            .or_default();
        add_usage(&mut entry.0, input_tokens, output_tokens);
        if timestamp.is_some_and(|timestamp| timestamp >= cutoff) {
            add_usage(&mut entry.1, input_tokens, output_tokens);
        }
    }

    let mut extension_map: BTreeMap<String, BTreeMap<String, Vec<ModelCapabilityUsage>>> =
        BTreeMap::new();
    for ((extension_id, capability, provider, model), (all_time, last_7_days)) in usage_by_key {
        extension_map
            .entry(extension_id)
            .or_default()
            .entry(capability)
            .or_default()
            .push(ModelCapabilityUsage {
                provider,
                model,
                all_time,
                last_7_days,
            });
    }

    CapabilityUsageState {
        extensions: extension_map
            .into_iter()
            .map(|(extension_id, capabilities)| ExtensionCapabilityUsage {
                extension_id,
                capabilities: capabilities
                    .into_iter()
                    .map(|(capability, models)| CapabilityUsageByCapability { capability, models })
                    .collect(),
            })
            .collect(),
    }
}

fn add_usage(totals: &mut TokenUsageTotals, input_tokens: u64, output_tokens: u64) {
    totals.requests += 1;
    totals.input_tokens += input_tokens;
    totals.output_tokens += output_tokens;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TalkImpulseDecision {
    Disabled,
    Waiting,
    SkippedRecentActivity,
    Emit,
}

#[derive(Clone, Debug, PartialEq)]
enum IdlePresenceDecision {
    Start {
        threshold_seconds: u64,
    },
    End {
        idle_seconds: u64,
        idle_minutes: u64,
    },
}

fn evaluate_life_tick(now: DateTime<Utc>, state: &mut PresenceState) -> bool {
    let next = state
        .next_life_tick_at
        .get_or_insert_with(|| add_duration(now, PRESENCE_LIFE_TICK_INTERVAL).unwrap_or(now));
    if now < *next {
        return false;
    }
    state.next_life_tick_at = Some(add_duration(now, PRESENCE_LIFE_TICK_INTERVAL).unwrap_or(now));
    true
}

fn evaluate_talk_impulse(
    now: DateTime<Utc>,
    interval_minutes: u64,
    random_permyriad: u16,
    state: &mut PresenceState,
) -> TalkImpulseDecision {
    if interval_minutes == 0 {
        state.talk_interval_minutes = Some(0);
        state.next_talk_impulse_at = None;
        return TalkImpulseDecision::Disabled;
    }

    if state.talk_interval_minutes != Some(interval_minutes) || state.next_talk_impulse_at.is_none()
    {
        state.talk_interval_minutes = Some(interval_minutes);
        state.next_talk_impulse_at = Some(schedule_next_talk_impulse(
            now,
            interval_minutes,
            random_permyriad,
        ));
        return TalkImpulseDecision::Waiting;
    }

    let Some(next_due) = state.next_talk_impulse_at else {
        return TalkImpulseDecision::Waiting;
    };
    if now < next_due {
        return TalkImpulseDecision::Waiting;
    }

    state.next_talk_impulse_at = Some(schedule_next_talk_impulse(
        now,
        interval_minutes,
        random_permyriad,
    ));
    if recently_active(now, state.last_user_activity_at) {
        return TalkImpulseDecision::SkippedRecentActivity;
    }
    TalkImpulseDecision::Emit
}

fn evaluate_idle_presence(
    idle_seconds_since_last_input: Option<f64>,
    state: &mut PresenceState,
) -> Option<IdlePresenceDecision> {
    let idle_seconds = idle_seconds_since_last_input?;
    if !idle_seconds.is_finite() || idle_seconds < 0.0 {
        return None;
    }

    let threshold_seconds = PRESENCE_IDLE_THRESHOLD.as_secs();
    if idle_seconds >= threshold_seconds as f64 {
        state.last_idle_elapsed_seconds = Some(
            state
                .last_idle_elapsed_seconds
                .map_or(idle_seconds, |previous| previous.max(idle_seconds)),
        );
        if state.idle_active {
            return None;
        }
        state.idle_active = true;
        return Some(IdlePresenceDecision::Start { threshold_seconds });
    }

    if !state.idle_active {
        state.last_idle_elapsed_seconds = None;
        return None;
    }

    state.idle_active = false;
    let idle_seconds = state
        .last_idle_elapsed_seconds
        .take()
        .unwrap_or(threshold_seconds as f64)
        .floor()
        .max(0.0) as u64;
    Some(IdlePresenceDecision::End {
        idle_seconds,
        idle_minutes: idle_seconds / 60,
    })
}

fn recently_active(now: DateTime<Utc>, last_user_activity_at: Option<DateTime<Utc>>) -> bool {
    let Some(last) = last_user_activity_at else {
        return false;
    };
    match now.signed_duration_since(last).to_std() {
        Ok(elapsed) => elapsed < TALK_IMPULSE_RECENT_ACTIVITY_SUPPRESSION,
        Err(_) => true,
    }
}

fn schedule_next_talk_impulse(
    now: DateTime<Utc>,
    interval_minutes: u64,
    random_permyriad: u16,
) -> DateTime<Utc> {
    let duration = jittered_talk_interval(interval_minutes, random_permyriad);
    add_duration(now, duration).unwrap_or(now)
}

fn jittered_talk_interval(interval_minutes: u64, random_permyriad: u16) -> Duration {
    let base_secs = interval_minutes.saturating_mul(60).max(1);
    let min_secs = base_secs.saturating_mul(80) / 100;
    let spread_secs = (base_secs.saturating_mul(40) / 100).max(1);
    let random = u64::from(random_permyriad.min(9_999));
    Duration::from_secs(min_secs + (spread_secs * random / 9_999))
}

fn add_duration(at: DateTime<Utc>, duration: Duration) -> Option<DateTime<Utc>> {
    chrono::Duration::from_std(duration)
        .ok()
        .and_then(|duration| at.checked_add_signed(duration))
}

fn next_rng_permyriad(state: &mut u64, now: DateTime<Utc>) -> u16 {
    if *state == 0 {
        *state = (now.timestamp_nanos_opt().unwrap_or_default() as u64) ^ 0xA5A5_5A5A_D3C1_B2E0;
    }
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    (x % 10_000) as u16
}

impl LocalYuukeiRuntime {
    pub async fn open_default() -> Result<Self> {
        Self::open_selected().await
    }

    pub async fn open_selected() -> Result<Self> {
        Self::open_selected_in(LocalRuntimeEnvironment::default_local()).await
    }

    pub async fn open_selected_in(env: LocalRuntimeEnvironment) -> Result<Self> {
        let mut registry = world_pack_registry::WorldPackRegistry::open(env)?;
        let requested_install = registry.active_install()?;
        let status = registry.selection_state(&requested_install, false);
        match Self::open_with_status(registry.config_for_install(&requested_install), status).await
        {
            Ok(runtime) => Ok(runtime),
            Err(error) if requested_install.install_id != DEFAULT_WORLD_PACK_INSTALL_ID => {
                let daihon_diagnostics =
                    diagnostics_from_error_for_install(&error, &requested_install);
                registry.mark_load_error(&requested_install.install_id, error.to_string())?;
                let default_install = registry.default_install()?;
                let mut status = registry.selection_state(&default_install, true);
                status.daihon_diagnostics = daihon_diagnostics;
                Self::open_with_status(registry.config_for_install(&default_install), status).await
            }
            Err(error) => Err(error),
        }
    }

    pub async fn select_world_pack_directory(path: impl AsRef<Path>) -> Result<Self> {
        Self::select_world_pack_directory_in(LocalRuntimeEnvironment::default_local(), path).await
    }

    pub async fn select_world_pack_directory_in(
        env: LocalRuntimeEnvironment,
        path: impl AsRef<Path>,
    ) -> Result<Self> {
        let mut registry = world_pack_registry::WorldPackRegistry::open(env)?;
        let install = registry.install_from_directory(path)?;
        registry.stage_active_install(install.clone());
        let status = registry.selection_state(&install, false);
        let runtime = Self::open_with_status(registry.config_for_install(&install), status).await?;
        registry.save()?;
        Ok(runtime)
    }

    pub async fn reset_world_pack_to_default() -> Result<Self> {
        Self::reset_world_pack_to_default_in(LocalRuntimeEnvironment::default_local()).await
    }

    pub async fn reset_world_pack_to_default_in(env: LocalRuntimeEnvironment) -> Result<Self> {
        let mut registry = world_pack_registry::WorldPackRegistry::open(env)?;
        let install = registry.default_install()?;
        registry.stage_active_install(install.clone());
        let status = registry.selection_state(&install, false);
        let runtime = Self::open_with_status(registry.config_for_install(&install), status).await?;
        registry.save()?;
        Ok(runtime)
    }

    pub fn extension_settings_state() -> Result<ExtensionSettingsState> {
        Self::extension_settings_state_in(LocalRuntimeEnvironment::default_local())
    }

    pub fn extension_settings_state_in(
        env: LocalRuntimeEnvironment,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        Ok(config.extension_settings_registry()?.state())
    }

    pub fn app_settings_state() -> Result<AppSettingsState> {
        Self::app_settings_state_in(LocalRuntimeEnvironment::default_local())
    }

    pub fn app_settings_state_in(env: LocalRuntimeEnvironment) -> Result<AppSettingsState> {
        let registry = AppSettingsRegistry::open(&env.data_dir)?;
        Ok(registry.state())
    }

    pub fn set_app_talk_interval_minutes(minutes: u64) -> Result<AppSettingsState> {
        Self::set_app_talk_interval_minutes_in(LocalRuntimeEnvironment::default_local(), minutes)
    }

    pub fn set_app_talk_interval_minutes_in(
        env: LocalRuntimeEnvironment,
        minutes: u64,
    ) -> Result<AppSettingsState> {
        let mut registry = AppSettingsRegistry::open(&env.data_dir)?;
        registry.set_talk_interval_minutes(minutes)
    }

    pub fn install_extension_directory(path: impl AsRef<Path>) -> Result<ExtensionSettingsState> {
        Self::install_extension_directory_in(LocalRuntimeEnvironment::default_local(), path)
    }

    pub fn install_extension_directory_in(
        env: LocalRuntimeEnvironment,
        path: impl AsRef<Path>,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.install_from_directory(path)
    }

    pub fn uninstall_extension(extension_id: &str) -> Result<ExtensionSettingsState> {
        Self::uninstall_extension_in(LocalRuntimeEnvironment::default_local(), extension_id)
    }

    pub fn uninstall_extension_in(
        env: LocalRuntimeEnvironment,
        extension_id: &str,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.uninstall(extension_id)
    }

    pub fn set_extension_enabled(
        extension_id: &str,
        enabled: bool,
    ) -> Result<ExtensionSettingsState> {
        Self::set_extension_enabled_in(
            LocalRuntimeEnvironment::default_local(),
            extension_id,
            enabled,
        )
    }

    pub fn set_extension_enabled_in(
        env: LocalRuntimeEnvironment,
        extension_id: &str,
        enabled: bool,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.set_enabled(extension_id, enabled)
    }

    pub fn set_extension_hook_order(
        hook_point: ExtensionHookPoint,
        extension_ids: Vec<String>,
    ) -> Result<ExtensionSettingsState> {
        Self::set_extension_hook_order_in(
            LocalRuntimeEnvironment::default_local(),
            hook_point,
            extension_ids,
        )
    }

    pub fn set_extension_hook_order_in(
        env: LocalRuntimeEnvironment,
        hook_point: ExtensionHookPoint,
        extension_ids: Vec<String>,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.set_hook_order(hook_point, extension_ids)
    }

    pub fn set_capability_default(
        capability: &str,
        extension_id: &str,
    ) -> Result<ExtensionSettingsState> {
        Self::set_capability_default_in(
            LocalRuntimeEnvironment::default_local(),
            capability,
            extension_id,
        )
    }

    pub fn set_capability_default_in(
        env: LocalRuntimeEnvironment,
        capability: &str,
        extension_id: &str,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.set_capability_default(capability, extension_id)
    }

    pub fn set_extension_setting_values(
        extension_id: &str,
        values: Map<String, Value>,
    ) -> Result<ExtensionSettingsState> {
        Self::set_extension_setting_values_in(
            LocalRuntimeEnvironment::default_local(),
            extension_id,
            values,
        )
    }

    pub fn set_extension_setting_values_in(
        env: LocalRuntimeEnvironment,
        extension_id: &str,
        values: Map<String, Value>,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.set_extension_setting_values(extension_id, values)
    }

    pub fn set_extension_secret(
        extension_id: &str,
        key: &str,
        value: Option<String>,
    ) -> Result<ExtensionSettingsState> {
        Self::set_extension_secret_in(
            LocalRuntimeEnvironment::default_local(),
            extension_id,
            key,
            value,
        )
    }

    pub fn set_extension_secret_in(
        env: LocalRuntimeEnvironment,
        extension_id: &str,
        key: &str,
        value: Option<String>,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.set_extension_secret(extension_id, key, value)
    }

    pub fn capability_usage(&self) -> Result<CapabilityUsageState> {
        let records = self
            .home
            .event_log()
            .read(EventLogQuery::default())?
            .records;
        Ok(capability_usage_from_records(&records, Utc::now()))
    }

    pub async fn open(config: LocalRuntimeConfig) -> Result<Self> {
        let install = WorldPackInstall {
            install_id: config.install_id.clone(),
            resident_id: config.resident_id.clone(),
            world_pack_id: "custom".to_string(),
            display_name: "Custom World Pack".to_string(),
            canonical_root: config.world_root.clone(),
            source: WorldPackSource::ExternalDirectory,
            last_load_error: None,
        };
        let status = WorldPackSelectionState {
            configured_install_id: install.install_id.clone(),
            running_install_id: install.install_id.clone(),
            active_install: install.clone(),
            installs: vec![install],
            fallback_active: false,
            last_load_error: None,
            daihon_diagnostics: Vec::new(),
            settings_path: config.data_dir.join("settings").join("world-packs.json"),
        };
        Self::open_with_status(config, status).await
    }

    async fn open_with_status(
        config: LocalRuntimeConfig,
        mut world_pack_status: WorldPackSelectionState,
    ) -> Result<Self> {
        fs::create_dir_all(&config.data_dir)?;
        fs::create_dir_all(&config.extension_root)?;
        if let Some(parent) = config.event_log_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Some(parent) = config.scene_history_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let logger = AppLogger::open(&config.app_log_path)?;
        let paths = config.paths();
        let initial_session_daihon_diagnostics =
            std::mem::take(&mut world_pack_status.daihon_diagnostics);
        if !initial_session_daihon_diagnostics.is_empty() {
            let _ = record_daihon_diagnostics_to_app_log(
                &logger,
                "world-pack.fallback-load",
                &initial_session_daihon_diagnostics,
            );
        }
        logger.record("runtime.open.request", "device-host", paths_payload(&paths))?;

        let world = match WorldPack::load_from_dir(&config.world_root) {
            Ok(world) => world,
            Err(error) => {
                if let Some(report) = error.daihon_report() {
                    let diagnostics = diagnostics_for_install(
                        &world_pack_status.active_install,
                        report.diagnostics.clone(),
                    );
                    let _ = record_daihon_diagnostics_to_app_log(
                        &logger,
                        "world-pack.load",
                        &diagnostics,
                    );
                }
                let _ = logger.record(
                    "runtime.open.error",
                    "device-host",
                    error_payload("world-pack", &error),
                );
                return Err(error.into());
            }
        };
        let actor_surface_assets = actor_surface_asset_catalog(&world);
        let event_log = match EventLog::open(&config.event_log_path) {
            Ok(event_log) => event_log,
            Err(error) => {
                let _ = logger.record(
                    "runtime.open.error",
                    "device-host",
                    error_payload("event-log", &error),
                );
                return Err(error.into());
            }
        };
        let extension_settings = config.extension_settings_registry()?;
        let capabilities =
            build_extension_capability_router(&extension_settings, &logger, &config.data_dir)?;
        let scene_history_logger = {
            let logger = logger.clone();
            Arc::new(move |error: yuukei_world::SceneHistoryPersistenceError| {
                let _ = logger.record(
                    "scene-history.persistence-error",
                    "device-host",
                    JsonMap::from([
                        ("path".to_string(), json!(display_path(&error.path))),
                        ("message".to_string(), json!(error.message)),
                    ]),
                );
            })
        };
        let variable_logger = {
            let logger = logger.clone();
            Arc::new(move |error: yuukei_world::VariablePersistenceError| {
                let _ = logger.record(
                    "variables.persistence-error",
                    "device-host",
                    JsonMap::from([
                        ("path".to_string(), json!(display_path(&error.path))),
                        ("message".to_string(), json!(error.message)),
                    ]),
                );
            })
        };
        let home = match ResidentHome::with_parts(
            &config.resident_id,
            world,
            event_log,
            Arc::new(YuukeiDaihonAdapter::with_persistent_state_loggers(
                config.scene_history_path.clone(),
                scene_history_logger,
                config.variables_path.clone(),
                variable_logger,
            )),
            capabilities,
        )
        .await
        {
            Ok(home) => Arc::new(home),
            Err(error) => {
                if let Some(report) = error.daihon_report() {
                    let diagnostics = diagnostics_for_install(
                        &world_pack_status.active_install,
                        report.diagnostics.clone(),
                    );
                    let _ = record_daihon_diagnostics_to_app_log(
                        &logger,
                        "resident-home.load",
                        &diagnostics,
                    );
                }
                let _ = logger.record(
                    "runtime.open.error",
                    "device-host",
                    error_payload("resident-home", &error),
                );
                return Err(error.into());
            }
        };
        let loaded_extensions =
            load_trusted_extensions(&extension_settings, &home, &logger, &config.data_dir).await?;

        logger.record(
            "runtime.open.ready",
            "device-host",
            JsonMap::from([
                ("residentId".to_string(), json!(config.resident_id)),
                ("deviceId".to_string(), json!(config.device_id)),
                (
                    "worldRoot".to_string(),
                    json!(display_path(&config.world_root)),
                ),
                (
                    "eventLogPath".to_string(),
                    json!(display_path(&config.event_log_path)),
                ),
                (
                    "extensionRoot".to_string(),
                    json!(display_path(&config.extension_root)),
                ),
                ("loadedExtensions".to_string(), json!(loaded_extensions)),
                (
                    "appLogPath".to_string(),
                    json!(display_path(&config.app_log_path)),
                ),
            ]),
        )?;

        let runtime = Self {
            home,
            logger,
            install_id: config.install_id,
            resident_id: config.resident_id,
            device_id: config.device_id,
            paths,
            world_pack_status,
            actor_surface_assets,
            presence_state: Arc::new(Mutex::new(PresenceState::default())),
            session_daihon_diagnostics: Arc::new(Mutex::new(initial_session_daihon_diagnostics)),
        };
        runtime.record_world_pack_activated().await?;
        Ok(runtime)
    }

    pub fn home(&self) -> Arc<ResidentHome> {
        self.home.clone()
    }

    pub fn logger(&self) -> AppLogger {
        self.logger.clone()
    }

    pub fn paths(&self) -> &RuntimePaths {
        &self.paths
    }

    pub fn install_id(&self) -> &str {
        &self.install_id
    }

    pub fn resident_id(&self) -> &str {
        &self.resident_id
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn snapshot(&self) -> Result<ResidentSnapshot> {
        self.home.snapshot().map_err(Into::into)
    }

    pub fn world_pack_status(&self) -> WorldPackSelectionState {
        let mut status = self.world_pack_status.clone();
        status.daihon_diagnostics = self.session_daihon_diagnostics();
        status
    }

    pub fn actor_surface_assets(&self) -> ActorSurfaceAssetCatalog {
        self.actor_surface_assets.clone()
    }

    pub fn extension_settings(&self) -> Result<ExtensionSettingsState> {
        ExtensionSettingsRegistry::open(&self.paths.data_dir, &self.paths.extension_root)
            .map(|registry| registry.state())
    }

    pub fn app_settings(&self) -> Result<AppSettingsState> {
        AppSettingsRegistry::open(&self.paths.data_dir).map(|registry| registry.state())
    }

    pub async fn list_resident_memories(
        &self,
        episode_limit: Option<usize>,
        episode_offset: Option<usize>,
    ) -> Result<MemoryListOutput> {
        self.home
            .list_memories(episode_limit, episode_offset)
            .await
            .map_err(Into::into)
    }

    pub async fn update_resident_memory(
        &self,
        kind: MemoryEntryKind,
        id: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<MemoryUpdateOutput> {
        self.home
            .update_memory(kind, id, text)
            .await
            .map_err(Into::into)
    }

    pub async fn forget_resident_memories(
        &self,
        entries: Vec<MemoryForgetEntry>,
        all: bool,
    ) -> Result<MemoryForgetOutput> {
        self.home
            .forget_memories(entries, all)
            .await
            .map_err(Into::into)
    }

    pub fn record_session_daihon_diagnostics_from_error(
        &self,
        error: &DeviceHostError,
        pack_root: Option<&Path>,
    ) -> Result<usize> {
        let Some(report) = error.daihon_report() else {
            return Ok(0);
        };
        let diagnostics = match pack_root {
            Some(pack_root) => diagnostics_for_pack_root(pack_root, report.diagnostics.clone()),
            None => diagnostics_for_install(
                &self.world_pack_status.active_install,
                report.diagnostics.clone(),
            ),
        };
        let count = diagnostics.len();
        if count == 0 {
            return Ok(0);
        }
        {
            let mut session = self
                .session_daihon_diagnostics
                .lock()
                .map_err(|_| DeviceHostError::DaihonDiagnosticState)?;
            session.extend(diagnostics.clone());
        }
        record_daihon_diagnostics_to_app_log(&self.logger, "world-pack.selection", &diagnostics)?;
        Ok(count)
    }

    pub async fn attach_surface(&self, session: SurfaceSession) -> Result<ResidentSnapshot> {
        self.logger.record(
            "surface.attach.request",
            "device-host",
            JsonMap::from([
                ("surfaceId".to_string(), json!(session.surface_id)),
                ("deviceId".to_string(), json!(session.device_id)),
                ("kind".to_string(), json!(session.kind)),
                ("presentation".to_string(), json!(session.presentation)),
            ]),
        )?;
        match self.home.attach_surface(session.clone()).await {
            Ok(snapshot) => {
                self.logger.record(
                    "surface.attach.ready",
                    "resident-home",
                    JsonMap::from([
                        ("surfaceId".to_string(), json!(session.surface_id)),
                        (
                            "activeSurfaceId".to_string(),
                            json!(snapshot.active_surface_id),
                        ),
                        (
                            "recentEventCursor".to_string(),
                            json!(snapshot.recent_event_cursor),
                        ),
                    ]),
                )?;
                Ok(snapshot)
            }
            Err(error) => {
                self.record_runtime_daihon_diagnostics("surface.attach", &error);
                let _ = self.logger.record(
                    "surface.attach.error",
                    "resident-home",
                    error_payload("surface.attach", &error),
                );
                Err(error.into())
            }
        }
    }

    pub async fn send_conversation_text(
        &self,
        surface_id: &str,
        text: &str,
    ) -> Result<Vec<RuntimeCommand>> {
        self.mark_user_activity(Utc::now())?;
        let event = build_conversation_text_event(
            self.resident_id(),
            self.device_id(),
            surface_id,
            text.trim(),
        );
        self.logger.record(
            "surface.input.conversation_text",
            "surface",
            JsonMap::from([
                ("eventId".to_string(), json!(event.id)),
                ("surfaceId".to_string(), json!(surface_id)),
                ("textLength".to_string(), json!(text.chars().count())),
            ]),
        )?;

        match self.home.ingest_event(event.clone()).await {
            Ok(commands) => {
                self.logger.record(
                    "runtime.commands.emitted",
                    "resident-home",
                    JsonMap::from([
                        ("sourceEventId".to_string(), json!(event.id)),
                        (
                            "commandIds".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.id)
                                .collect::<Vec<_>>()),
                        ),
                        (
                            "commandTypes".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.kind)
                                .collect::<Vec<_>>()),
                        ),
                        ("count".to_string(), json!(commands.len())),
                    ]),
                )?;
                Ok(commands)
            }
            Err(error) => {
                self.record_runtime_daihon_diagnostics("conversation.text", &error);
                let _ = self.logger.record(
                    "runtime.commands.error",
                    "resident-home",
                    error_payload("conversation.text", &error),
                );
                Err(error.into())
            }
        }
    }

    pub async fn send_conversation_choice(
        &self,
        surface_id: &str,
        choice_id: &str,
        choice: &str,
        index: usize,
    ) -> Result<Vec<RuntimeCommand>> {
        self.mark_user_activity(Utc::now())?;
        let event = build_conversation_choice_event(
            self.resident_id(),
            self.device_id(),
            surface_id,
            choice_id,
            choice,
            index,
        );
        self.logger.record(
            "surface.input.conversation_choice",
            "surface",
            JsonMap::from([
                ("eventId".to_string(), json!(event.id)),
                ("surfaceId".to_string(), json!(surface_id)),
                ("choiceId".to_string(), json!(choice_id)),
                ("choice".to_string(), json!(choice)),
                ("index".to_string(), json!(index)),
            ]),
        )?;

        match self.home.ingest_event(event.clone()).await {
            Ok(commands) => {
                self.logger.record(
                    "runtime.commands.emitted",
                    "resident-home",
                    JsonMap::from([
                        ("sourceEventId".to_string(), json!(event.id)),
                        (
                            "commandIds".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.id)
                                .collect::<Vec<_>>()),
                        ),
                        (
                            "commandTypes".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.kind)
                                .collect::<Vec<_>>()),
                        ),
                        ("count".to_string(), json!(commands.len())),
                    ]),
                )?;
                Ok(commands)
            }
            Err(error) => {
                self.record_runtime_daihon_diagnostics("conversation.choice", &error);
                let _ = self.logger.record(
                    "runtime.commands.error",
                    "resident-home",
                    error_payload("conversation.choice", &error),
                );
                Err(error.into())
            }
        }
    }

    pub async fn send_avatar_gesture_poke(
        &self,
        surface_id: &str,
        gesture: AvatarGesturePoke,
    ) -> Result<Vec<RuntimeCommand>> {
        self.mark_user_activity(Utc::now())?;
        let event = build_avatar_gesture_poke_event(
            self.resident_id(),
            self.device_id(),
            surface_id,
            gesture,
        );
        self.logger.record(
            "surface.input.avatar_gesture",
            "surface",
            JsonMap::from([
                ("eventId".to_string(), json!(event.id)),
                ("surfaceId".to_string(), json!(surface_id)),
                ("actorId".to_string(), json!(event.actor_id.clone())),
                (
                    "hitZoneId".to_string(),
                    event
                        .payload
                        .get("hitZoneId")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
            ]),
        )?;

        match self.home.ingest_event(event.clone()).await {
            Ok(commands) => {
                self.logger.record(
                    "runtime.commands.emitted",
                    "resident-home",
                    JsonMap::from([
                        ("sourceEventId".to_string(), json!(event.id)),
                        (
                            "commandIds".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.id)
                                .collect::<Vec<_>>()),
                        ),
                        (
                            "commandTypes".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.kind)
                                .collect::<Vec<_>>()),
                        ),
                        ("count".to_string(), json!(commands.len())),
                    ]),
                )?;
                Ok(commands)
            }
            Err(error) => {
                self.record_runtime_daihon_diagnostics("avatar.gesture.poke", &error);
                let _ = self.logger.record(
                    "runtime.commands.error",
                    "resident-home",
                    error_payload("avatar.gesture.poke", &error),
                );
                Err(error.into())
            }
        }
    }

    pub async fn emit_app_startup(&self) -> Result<Vec<RuntimeCommand>> {
        let snapshot = current_presence_snapshot();
        {
            let mut state = self
                .presence_state
                .lock()
                .map_err(|_| DeviceHostError::PresenceState)?;
            if state.startup_emitted {
                return Ok(Vec::new());
            }
            state.startup_emitted = true;
            state.last_time_period = Some(snapshot.time_period.to_string());
        }
        let result = self
            .emit_runtime_event("app.startup", snapshot.into_payload())
            .await;
        if result.is_err() {
            if let Ok(mut state) = self.presence_state.lock() {
                state.startup_emitted = false;
                state.last_time_period = None;
            }
        }
        result
    }

    pub async fn emit_presence_tick(&self) -> Result<Vec<RuntimeCommand>> {
        let snapshot = current_presence_snapshot();
        let time_period_changed = {
            let mut state = self
                .presence_state
                .lock()
                .map_err(|_| DeviceHostError::PresenceState)?;
            let changed = state.last_time_period.as_deref() != Some(snapshot.time_period);
            if changed {
                state.last_time_period = Some(snapshot.time_period.to_string());
            }
            changed
        };

        let mut commands = Vec::new();
        if time_period_changed {
            commands.extend(
                self.emit_runtime_event("presence.time_period", snapshot.clone().into_payload())
                    .await?,
            );
        }
        commands.extend(
            self.emit_runtime_event("presence.life_tick", snapshot.into_payload())
                .await?,
        );
        Ok(commands)
    }

    pub async fn emit_talk_impulse(&self) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event("presence.talk_impulse", current_presence_payload())
            .await
    }

    pub async fn emit_device_sleep_before(&self) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event("device.sleep.before", current_presence_payload())
            .await
    }

    pub async fn emit_device_wake(&self) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event("device.wake", current_presence_payload())
            .await
    }

    pub fn spawn_presence_loop(&self) -> JoinHandle<()> {
        self.spawn_presence_loop_with_idle_sampler(|| None)
    }

    pub fn spawn_presence_loop_with_idle_sampler<F>(&self, idle_sampler: F) -> JoinHandle<()>
    where
        F: Fn() -> Option<f64> + Send + Sync + 'static,
    {
        let runtime = self.clone();
        let idle_sampler = Arc::new(idle_sampler);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(PRESENCE_LOOP_POLL_INTERVAL).await;
                let idle_seconds_since_last_input = idle_sampler();
                if let Err(error) = runtime
                    .run_presence_loop_step(Utc::now(), idle_seconds_since_last_input)
                    .await
                {
                    let _ = runtime.logger.record(
                        "presence.loop.error",
                        "device-host",
                        error_payload("presence", &error),
                    );
                }
            }
        })
    }

    async fn run_presence_loop_step(
        &self,
        now: DateTime<Utc>,
        idle_seconds_since_last_input: Option<f64>,
    ) -> Result<()> {
        let settings = self.app_settings()?;
        let random = {
            let mut state = self
                .presence_state
                .lock()
                .map_err(|_| DeviceHostError::PresenceState)?;
            next_rng_permyriad(&mut state.talk_rng_state, now)
        };

        let (emit_life_tick, talk_decision, idle_decision) = {
            let mut state = self
                .presence_state
                .lock()
                .map_err(|_| DeviceHostError::PresenceState)?;
            let emit_life_tick = evaluate_life_tick(now, &mut state);
            let talk_decision =
                evaluate_talk_impulse(now, settings.talk_interval_minutes, random, &mut state);
            let idle_decision = evaluate_idle_presence(idle_seconds_since_last_input, &mut state);
            (emit_life_tick, talk_decision, idle_decision)
        };

        match idle_decision {
            Some(IdlePresenceDecision::Start { threshold_seconds }) => {
                self.emit_runtime_event(
                    "presence.idle.start",
                    JsonMap::from([("thresholdSeconds".to_string(), json!(threshold_seconds))]),
                )
                .await?;
            }
            Some(IdlePresenceDecision::End {
                idle_seconds,
                idle_minutes,
            }) => {
                self.emit_runtime_event(
                    "presence.idle.end",
                    JsonMap::from([
                        ("idleMinutes".to_string(), json!(idle_minutes)),
                        ("idleSeconds".to_string(), json!(idle_seconds)),
                    ]),
                )
                .await?;
            }
            None => {}
        }
        if emit_life_tick {
            self.emit_presence_tick().await?;
        }
        match talk_decision {
            TalkImpulseDecision::Emit => {
                self.emit_talk_impulse().await?;
            }
            TalkImpulseDecision::SkippedRecentActivity => {
                self.logger.record(
                    "presence.talk_impulse.skipped",
                    "device-host",
                    JsonMap::from([("reason".to_string(), json!("recent-user-activity"))]),
                )?;
            }
            TalkImpulseDecision::Disabled | TalkImpulseDecision::Waiting => {}
        }
        Ok(())
    }

    fn mark_user_activity(&self, at: DateTime<Utc>) -> Result<()> {
        let mut state = self
            .presence_state
            .lock()
            .map_err(|_| DeviceHostError::PresenceState)?;
        state.last_user_activity_at = Some(at);
        Ok(())
    }

    async fn emit_runtime_event(
        &self,
        kind: &str,
        payload: JsonMap,
    ) -> Result<Vec<RuntimeCommand>> {
        let active_surface_id = self.home.snapshot()?.active_surface_id;
        let event = RuntimeEvent {
            id: new_id("evt"),
            kind: kind.to_string(),
            timestamp: now_timestamp(),
            source: "device".to_string(),
            resident_id: self.resident_id.clone(),
            payload,
            causality: None,
            device_id: Some(self.device_id.clone()),
            surface_id: active_surface_id,
            actor_id: None,
        };
        self.logger.record(
            "runtime.event.emit",
            "device-host",
            JsonMap::from([
                ("eventId".to_string(), json!(event.id)),
                ("eventType".to_string(), json!(event.kind)),
            ]),
        )?;
        match self.home.ingest_event(event.clone()).await {
            Ok(commands) => {
                self.logger.record(
                    "runtime.commands.emitted",
                    "resident-home",
                    JsonMap::from([
                        ("sourceEventId".to_string(), json!(event.id)),
                        (
                            "commandIds".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.id)
                                .collect::<Vec<_>>()),
                        ),
                        (
                            "commandTypes".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.kind)
                                .collect::<Vec<_>>()),
                        ),
                        ("count".to_string(), json!(commands.len())),
                    ]),
                )?;
                Ok(commands)
            }
            Err(error) => {
                self.record_runtime_daihon_diagnostics(kind, &error);
                let _ = self.logger.record(
                    "runtime.commands.error",
                    "resident-home",
                    error_payload(kind, &error),
                );
                Err(error.into())
            }
        }
    }

    fn session_daihon_diagnostics(&self) -> Vec<DaihonDiagnosticEntry> {
        let mut diagnostics = self
            .session_daihon_diagnostics
            .lock()
            .map(|diagnostics| diagnostics.clone())
            .unwrap_or_default();
        diagnostics.extend(
            self.home
                .daihon_diagnostics()
                .unwrap_or_default()
                .into_iter()
                .map(|diagnostic| {
                    enrich_diagnostic_for_install(
                        diagnostic,
                        &self.world_pack_status.active_install,
                        None,
                    )
                }),
        );
        diagnostics.sort_by(|left, right| {
            left.occurred_at
                .cmp(&right.occurred_at)
                .then_with(|| left.script_path.cmp(&right.script_path))
                .then_with(|| left.line.cmp(&right.line))
                .then_with(|| left.column.cmp(&right.column))
        });
        diagnostics
    }

    fn record_runtime_daihon_diagnostics(&self, context: &str, error: &ResidentHomeError) {
        let Some(report) = error.daihon_report() else {
            return;
        };
        let diagnostics = diagnostics_for_install(
            &self.world_pack_status.active_install,
            report.diagnostics.clone(),
        );
        let _ = record_daihon_diagnostics_to_app_log(&self.logger, context, &diagnostics);
    }

    pub fn export_event_log_jsonl(&self, path: impl AsRef<Path>) -> Result<usize> {
        let path = path.as_ref();
        let summary = self
            .home
            .event_log()
            .export_jsonl(EventLogQuery::default(), path)?;
        self.logger.record(
            "event_log.export",
            "device-host",
            JsonMap::from([
                ("path".to_string(), json!(display_path(path))),
                ("exported".to_string(), json!(summary.exported)),
            ]),
        )?;
        Ok(summary.exported)
    }

    async fn record_world_pack_activated(&self) -> Result<()> {
        let source = match &self.world_pack_status.active_install.source {
            WorldPackSource::BundledDefault => "bundledDefault",
            WorldPackSource::ExternalDirectory => "externalDirectory",
        };
        let event = RuntimeEvent {
            id: new_id("evt"),
            kind: "world_pack.activated".to_string(),
            timestamp: now_timestamp(),
            source: "device-host".to_string(),
            resident_id: self.resident_id.clone(),
            payload: JsonMap::from([
                ("installId".to_string(), json!(self.install_id.clone())),
                (
                    "worldPackId".to_string(),
                    json!(self.world_pack_status.active_install.world_pack_id.clone()),
                ),
                (
                    "displayName".to_string(),
                    json!(self.world_pack_status.active_install.display_name.clone()),
                ),
                ("source".to_string(), json!(source)),
                (
                    "configuredInstallId".to_string(),
                    json!(self.world_pack_status.configured_install_id.clone()),
                ),
                (
                    "fallbackActive".to_string(),
                    json!(self.world_pack_status.fallback_active),
                ),
            ]),
            causality: None,
            device_id: Some(self.device_id.clone()),
            surface_id: None,
            actor_id: None,
        };
        self.home.ingest_event(event).await?;
        self.logger.record(
            "world_pack.activated",
            "device-host",
            JsonMap::from([
                ("installId".to_string(), json!(self.install_id.clone())),
                (
                    "worldPackId".to_string(),
                    json!(self.world_pack_status.active_install.world_pack_id.clone()),
                ),
                (
                    "worldRoot".to_string(),
                    json!(display_path(&self.paths.world_root)),
                ),
                (
                    "fallbackActive".to_string(),
                    json!(self.world_pack_status.fallback_active),
                ),
            ]),
        )?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum AppLogError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("app log lock is poisoned")]
    PoisonedLock,
}

#[derive(Clone)]
pub struct AppLogger {
    path: PathBuf,
    file: Arc<Mutex<File>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppLogRecord {
    pub id: String,
    pub timestamp: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub source: String,
    pub payload: JsonMap,
}

impl AppLogger {
    pub fn open(path: impl AsRef<Path>) -> std::result::Result<Self, AppLogError> {
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            file: Arc::new(Mutex::new(file)),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record(
        &self,
        kind: impl Into<String>,
        source: impl Into<String>,
        payload: JsonMap,
    ) -> std::result::Result<AppLogRecord, AppLogError> {
        let record = AppLogRecord {
            id: new_id("app"),
            timestamp: now_timestamp(),
            kind: kind.into(),
            source: source.into(),
            payload,
        };
        let mut file = self.file.lock().map_err(|_| AppLogError::PoisonedLock)?;
        serde_json::to_writer(&mut *file, &record)?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(record)
    }
}

pub fn tauri_surface_session(device_id: &str) -> SurfaceSession {
    SurfaceSession {
        surface_id: TAURI_SURFACE_ID.to_string(),
        device_id: device_id.to_string(),
        kind: SurfaceKind::Desktop,
        active: true,
        capabilities: vec![
            "dialogue.say".to_string(),
            "avatar.expression".to_string(),
            "avatar.motion".to_string(),
            "avatar.gesture.poke".to_string(),
            "actor.place".to_string(),
            "screen.effect.start".to_string(),
            "screen.effect.stop".to_string(),
            "screen.dialogBurst.start".to_string(),
            "screen.dialogBurst.clear".to_string(),
        ],
        presentation: SurfacePresentation {
            renderer: Some(SurfaceRenderer::Vrm),
            transparent: Some(true),
            accepts_input: Some(true),
        },
    }
}

pub fn cli_surface_session(device_id: &str) -> SurfaceSession {
    SurfaceSession {
        surface_id: CLI_SURFACE_ID.to_string(),
        device_id: device_id.to_string(),
        kind: SurfaceKind::Cli,
        active: true,
        capabilities: vec![
            "dialogue.say".to_string(),
            "conversation.text".to_string(),
            "wizard.select".to_string(),
        ],
        presentation: SurfacePresentation {
            renderer: Some(SurfaceRenderer::Terminal),
            transparent: Some(false),
            accepts_input: Some(true),
        },
    }
}

pub fn build_conversation_text_event(
    resident_id: &str,
    device_id: &str,
    surface_id: &str,
    text: &str,
) -> RuntimeEvent {
    RuntimeEvent {
        id: new_id("evt"),
        kind: "conversation.text".to_string(),
        timestamp: now_timestamp(),
        source: "surface".to_string(),
        resident_id: resident_id.to_string(),
        payload: JsonMap::from([("text".to_string(), Value::String(text.to_string()))]),
        causality: None,
        device_id: Some(device_id.to_string()),
        surface_id: Some(surface_id.to_string()),
        actor_id: None,
    }
}

pub fn build_conversation_choice_event(
    resident_id: &str,
    device_id: &str,
    surface_id: &str,
    choice_id: &str,
    choice: &str,
    index: usize,
) -> RuntimeEvent {
    RuntimeEvent {
        id: new_id("evt"),
        kind: "conversation.choice".to_string(),
        timestamp: now_timestamp(),
        source: "surface".to_string(),
        resident_id: resident_id.to_string(),
        payload: JsonMap::from([
            ("choiceId".to_string(), json!(choice_id)),
            ("choice".to_string(), json!(choice)),
            ("index".to_string(), json!(index)),
        ]),
        causality: None,
        device_id: Some(device_id.to_string()),
        surface_id: Some(surface_id.to_string()),
        actor_id: None,
    }
}

pub fn build_avatar_gesture_poke_event(
    resident_id: &str,
    device_id: &str,
    surface_id: &str,
    gesture: AvatarGesturePoke,
) -> RuntimeEvent {
    let AvatarGesturePoke {
        actor_id,
        hit_zone_id,
        hit_zone_label,
        hit_surface,
        hit_bone,
        input,
        screen,
    } = gesture;
    let mut payload = JsonMap::from([
        ("actorId".to_string(), Value::String(actor_id.clone())),
        ("hitZoneId".to_string(), Value::String(hit_zone_id)),
        (
            "input".to_string(),
            json!({
                "kind": input.kind,
                "button": input.button,
            }),
        ),
        (
            "screen".to_string(),
            json!({
                "x": screen.x,
                "y": screen.y,
            }),
        ),
    ]);
    if let Some(label) = hit_zone_label {
        payload.insert("hitZoneLabel".to_string(), Value::String(label));
    }
    if let Some(surface) = hit_surface {
        payload.insert("hitSurface".to_string(), Value::String(surface));
    }
    if let Some(bone) = hit_bone {
        payload.insert("hitBone".to_string(), Value::String(bone));
    }

    RuntimeEvent {
        id: new_id("evt"),
        kind: "avatar.gesture.poke".to_string(),
        timestamp: now_timestamp(),
        source: "surface".to_string(),
        resident_id: resident_id.to_string(),
        payload,
        causality: None,
        device_id: Some(device_id.to_string()),
        surface_id: Some(surface_id.to_string()),
        actor_id: Some(actor_id),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PresenceSnapshot {
    local_hour: u32,
    local_minute: u32,
    time_period: &'static str,
}

impl PresenceSnapshot {
    fn into_payload(self) -> JsonMap {
        JsonMap::from([
            ("localHour".to_string(), json!(self.local_hour)),
            ("localMinute".to_string(), json!(self.local_minute)),
            ("timePeriod".to_string(), json!(self.time_period)),
        ])
    }
}

fn current_presence_payload() -> JsonMap {
    current_presence_snapshot().into_payload()
}

fn current_presence_snapshot() -> PresenceSnapshot {
    presence_snapshot_at(Local::now())
}

fn presence_snapshot_at(now: DateTime<Local>) -> PresenceSnapshot {
    let local_hour = now.hour();
    PresenceSnapshot {
        local_hour,
        local_minute: now.minute(),
        time_period: time_period_for_hour(local_hour).as_daihon_value(),
    }
}

pub fn time_period_for_hour(hour: u32) -> LocalTimePeriod {
    match hour {
        5..=9 => LocalTimePeriod::Morning,
        10..=16 => LocalTimePeriod::Day,
        17..=21 => LocalTimePeriod::Evening,
        _ => LocalTimePeriod::LateNight,
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("yuukei-device-host is nested under crates")
        .to_path_buf()
}

fn default_data_dir() -> PathBuf {
    std::env::var_os("YUUKEI_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("yuukei-v2"))
}

fn extension_config_for_env(env: LocalRuntimeEnvironment) -> LocalRuntimeConfig {
    let LocalRuntimeEnvironment {
        workspace_root,
        default_world_root,
        data_dir,
        device_id,
    } = env;
    let world_root = default_world_root;
    let extension_root = data_dir.join("extensions");
    let event_log_path = data_dir
        .join("residents")
        .join(DEFAULT_WORLD_PACK_INSTALL_ID)
        .join("events.sqlite3");
    let scene_history_path = data_dir
        .join("residents")
        .join(DEFAULT_WORLD_PACK_INSTALL_ID)
        .join("scene-history.json");
    let variables_path = data_dir
        .join("residents")
        .join(DEFAULT_WORLD_PACK_INSTALL_ID)
        .join("variables.json");
    let app_log_path = data_dir.join("app-activity.jsonl");
    LocalRuntimeConfig {
        install_id: DEFAULT_WORLD_PACK_INSTALL_ID.to_string(),
        resident_id: DEFAULT_RESIDENT_ID.to_string(),
        device_id,
        workspace_root,
        world_root,
        extension_root,
        event_log_path,
        scene_history_path,
        variables_path,
        app_log_path,
        data_dir,
    }
}

fn extension_data_dir(data_dir: &Path, extension_id: &str) -> PathBuf {
    data_dir.join("extension-data").join(extension_id)
}

fn actor_surface_asset_catalog(world: &WorldPack) -> ActorSurfaceAssetCatalog {
    ActorSurfaceAssetCatalog {
        world_pack_id: world.id.clone(),
        actors: world
            .actors
            .iter()
            .map(|actor| ActorSurfaceAsset {
                actor_id: actor.id.clone(),
                display_name: actor.display_name.clone(),
                renderer: actor
                    .renderer
                    .as_ref()
                    .map(|renderer| ActorSurfaceRendererAsset {
                        kind: match renderer.kind {
                            ActorRendererKind::Vrm => ActorSurfaceRendererKind::Vrm,
                        },
                        model: renderer.model.clone(),
                        motions: renderer.motions.clone(),
                        hit_zones: renderer
                            .hit_zones
                            .iter()
                            .map(|hit_zone| ActorSurfaceHitZoneDefinition {
                                id: hit_zone.id.clone(),
                                label: hit_zone.label.clone(),
                                source: match hit_zone.source {
                                    ActorHitZoneSource::HumanoidBone => {
                                        ActorSurfaceHitZoneSource::HumanoidBone
                                    }
                                    ActorHitZoneSource::NodeName => {
                                        ActorSurfaceHitZoneSource::NodeName
                                    }
                                },
                                bones: hit_zone.bones.clone(),
                                nodes: hit_zone.nodes.clone(),
                                shape: hit_zone.shape.map(|shape| match shape {
                                    ActorHitZoneShape::Auto => ActorSurfaceHitZoneShape::Auto,
                                    ActorHitZoneShape::Mesh => ActorSurfaceHitZoneShape::Mesh,
                                }),
                                events: hit_zone.events.clone(),
                                priority: hit_zone.priority,
                            })
                            .collect(),
                    }),
            })
            .collect(),
    }
}

fn paths_payload(paths: &RuntimePaths) -> JsonMap {
    JsonMap::from([
        (
            "workspaceRoot".to_string(),
            json!(display_path(&paths.workspace_root)),
        ),
        ("dataDir".to_string(), json!(display_path(&paths.data_dir))),
        (
            "worldRoot".to_string(),
            json!(display_path(&paths.world_root)),
        ),
        (
            "extensionRoot".to_string(),
            json!(display_path(&paths.extension_root)),
        ),
        (
            "eventLogPath".to_string(),
            json!(display_path(&paths.event_log_path)),
        ),
        (
            "sceneHistoryPath".to_string(),
            json!(display_path(&paths.scene_history_path)),
        ),
        (
            "variablesPath".to_string(),
            json!(display_path(&paths.variables_path)),
        ),
        (
            "appLogPath".to_string(),
            json!(display_path(&paths.app_log_path)),
        ),
    ])
}

fn error_payload(stage: &str, error: &dyn std::fmt::Display) -> JsonMap {
    JsonMap::from([
        ("stage".to_string(), json!(stage)),
        ("message".to_string(), json!(error.to_string())),
    ])
}

fn diagnostics_from_error_for_install(
    error: &DeviceHostError,
    install: &WorldPackInstall,
) -> Vec<DaihonDiagnosticEntry> {
    error
        .daihon_report()
        .map(|report| diagnostics_for_install(install, report.diagnostics.clone()))
        .unwrap_or_default()
}

fn diagnostics_for_install(
    install: &WorldPackInstall,
    diagnostics: Vec<DaihonDiagnosticEntry>,
) -> Vec<DaihonDiagnosticEntry> {
    let occurred_at = now_timestamp();
    diagnostics
        .into_iter()
        .map(|diagnostic| enrich_diagnostic_for_install(diagnostic, install, Some(&occurred_at)))
        .collect()
}

fn diagnostics_for_pack_root(
    pack_root: &Path,
    diagnostics: Vec<DaihonDiagnosticEntry>,
) -> Vec<DaihonDiagnosticEntry> {
    let occurred_at = now_timestamp();
    let pack_root = display_path(pack_root);
    diagnostics
        .into_iter()
        .map(|mut diagnostic| {
            if diagnostic.occurred_at.is_none() {
                diagnostic.occurred_at = Some(occurred_at.clone());
            }
            if diagnostic.pack_root.is_none() {
                diagnostic.pack_root = Some(pack_root.clone());
            }
            diagnostic
        })
        .collect()
}

fn enrich_diagnostic_for_install(
    mut diagnostic: DaihonDiagnosticEntry,
    install: &WorldPackInstall,
    occurred_at: Option<&str>,
) -> DaihonDiagnosticEntry {
    if diagnostic.occurred_at.is_none() {
        if let Some(occurred_at) = occurred_at {
            diagnostic.occurred_at = Some(occurred_at.to_string());
        }
    }
    if diagnostic.install_id.is_none() {
        diagnostic.install_id = Some(install.install_id.clone());
    }
    if diagnostic.world_pack_id.is_none() {
        diagnostic.world_pack_id = Some(install.world_pack_id.clone());
    }
    if diagnostic.pack_root.is_none() {
        diagnostic.pack_root = Some(display_path(&install.canonical_root));
    }
    diagnostic
}

fn record_daihon_diagnostics_to_app_log(
    logger: &AppLogger,
    context: &str,
    diagnostics: &[DaihonDiagnosticEntry],
) -> Result<()> {
    if diagnostics.is_empty() {
        return Ok(());
    }
    logger.record(
        "daihon.diagnostics",
        "device-host",
        JsonMap::from([
            ("context".to_string(), json!(context)),
            ("count".to_string(), json!(diagnostics.len())),
            (
                "diagnostics".to_string(),
                serde_json::to_value(diagnostics)?,
            ),
        ]),
    )?;
    Ok(())
}

async fn load_trusted_extensions(
    extension_settings: &ExtensionSettingsRegistry,
    home: &ResidentHome,
    logger: &AppLogger,
    data_dir: &Path,
) -> Result<usize> {
    let mut loaded = 0;
    let hook_order = extension_settings.hook_order(&ExtensionHookPoint::BeforeCommandEmit);
    for entry in extension_settings.runtime_entries() {
        match entry {
            ExtensionRuntimeEntry::Ready(install) => {
                let extension_id = install.extension_id.clone();
                let manifest_path = install.manifest_path.clone();
                let extension_data_dir = extension_data_dir(data_dir, &extension_id);
                fs::create_dir_all(&extension_data_dir)?;
                home.register_extension(extension_with_runtime_environment(
                    install.manifest,
                    install.install_dir,
                    install.enabled,
                    extension_data_dir,
                    install.settings_json,
                ))
                .await?;
                loaded += 1;
                logger.record(
                    "extension.load.ready",
                    "device-host",
                    JsonMap::from([
                        ("extensionId".to_string(), json!(extension_id)),
                        (
                            "manifestPath".to_string(),
                            json!(display_path(&manifest_path)),
                        ),
                        ("enabled".to_string(), json!(install.enabled)),
                    ]),
                )?;
            }
            ExtensionRuntimeEntry::Error(error) => {
                logger.record(
                    "extension.load.error",
                    "device-host",
                    JsonMap::from([
                        ("extensionId".to_string(), json!(error.extension_id)),
                        (
                            "manifestPath".to_string(),
                            json!(display_path(&error.manifest_path)),
                        ),
                        ("message".to_string(), json!(error.message)),
                    ]),
                )?;
            }
        }
    }
    home.set_extension_hook_order(ExtensionHookPoint::BeforeCommandEmit, hook_order)?;
    Ok(loaded)
}

fn build_extension_capability_router(
    extension_settings: &ExtensionSettingsRegistry,
    logger: &AppLogger,
    data_dir: &Path,
) -> Result<CapabilityRouter> {
    let mut router = CapabilityRouter::new();
    for entry in extension_settings.runtime_entries() {
        match entry {
            ExtensionRuntimeEntry::Ready(install) => {
                if install.manifest.capabilities.is_empty() {
                    continue;
                }
                let extension_data_dir = extension_data_dir(data_dir, &install.extension_id);
                fs::create_dir_all(&extension_data_dir)?;
                router
                    .register(extension_with_runtime_environment(
                        install.manifest,
                        install.install_dir,
                        install.enabled,
                        extension_data_dir,
                        install.settings_json,
                    ))
                    .map_err(|error| DeviceHostError::ExtensionSettings(error.to_string()))?;
            }
            ExtensionRuntimeEntry::Error(error) => {
                logger.record(
                    "extension.load.error",
                    "device-host",
                    JsonMap::from([
                        ("extensionId".to_string(), json!(error.extension_id)),
                        (
                            "manifestPath".to_string(),
                            json!(display_path(&error.manifest_path)),
                        ),
                        ("message".to_string(), json!(error.message)),
                    ]),
                )?;
            }
        }
    }
    for (capability, extension_id) in extension_settings.capability_defaults() {
        router.set_default_extension(capability, extension_id);
    }
    Ok(router)
}

fn extension_with_runtime_environment(
    manifest: yuukei_extension::ProcessExtensionManifest,
    install_dir: impl Into<PathBuf>,
    enabled: bool,
    data_dir: impl Into<PathBuf>,
    settings_json: Option<String>,
) -> ProcessHookExtension {
    let extension = ProcessHookExtension::from_installed_manifest(manifest, install_dir, enabled)
        .with_data_dir(data_dir);
    if let Some(settings_json) = settings_json {
        extension.with_settings_json(settings_json)
    } else {
        extension
    }
}

fn display_path(path: impl AsRef<Path>) -> String {
    path.as_ref().display().to_string()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;
    use tempfile::tempdir;
    use yuukei_protocol::NewEventLogRecord;

    use super::*;

    #[test]
    fn cli_session_declares_terminal_surface() {
        let session = cli_surface_session("device-test");
        assert_eq!(session.surface_id, CLI_SURFACE_ID);
        assert_eq!(session.device_id, "device-test");
        assert_eq!(session.kind, SurfaceKind::Cli);
        assert_eq!(
            session.presentation.renderer,
            Some(SurfaceRenderer::Terminal)
        );
        assert_eq!(session.presentation.accepts_input, Some(true));
    }

    #[test]
    fn tauri_session_declares_transparent_vrm_surface() {
        let session = tauri_surface_session("device-test");
        assert_eq!(session.surface_id, TAURI_SURFACE_ID);
        assert_eq!(session.device_id, "device-test");
        assert_eq!(session.kind, SurfaceKind::Desktop);
        assert_eq!(session.presentation.renderer, Some(SurfaceRenderer::Vrm));
        assert_eq!(session.presentation.transparent, Some(true));
        assert_eq!(session.presentation.accepts_input, Some(true));
        assert!(session
            .capabilities
            .iter()
            .any(|capability| capability == "avatar.gesture.poke"));
    }

    #[test]
    fn app_logger_writes_jsonl_records() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let path = dir.path().join("app-activity.jsonl");
        let logger = AppLogger::open(&path)?;

        logger.record(
            "test.event",
            "test",
            JsonMap::from([("ok".to_string(), json!(true))]),
        )?;

        let raw = fs::read_to_string(&path)?;
        assert!(raw.contains("\"type\":\"test.event\""));
        assert!(raw.contains("\"ok\":true"));
        assert_eq!(logger.path(), path.as_path());
        Ok(())
    }

    #[test]
    fn app_settings_persist_talk_interval_minutes(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let data = tempdir()?;
        let mut registry = AppSettingsRegistry::open(data.path())?;

        assert_eq!(
            registry.state().talk_interval_minutes,
            DEFAULT_TALK_INTERVAL_MINUTES
        );
        registry.set_talk_interval_minutes(12)?;

        let reopened = AppSettingsRegistry::open(data.path())?;
        assert_eq!(reopened.state().talk_interval_minutes, 12);
        assert!(data.path().join("settings").join("app.json").exists());
        Ok(())
    }

    #[test]
    fn talk_impulse_jitter_stays_within_twenty_percent() {
        assert_eq!(jittered_talk_interval(5, 0), Duration::from_secs(240));
        assert_eq!(jittered_talk_interval(5, 9_999), Duration::from_secs(360));
    }

    #[test]
    fn talk_impulse_evaluation_disables_schedules_skips_and_emits() {
        let now = DateTime::parse_from_rfc3339("2026-07-06T12:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let mut state = PresenceState::default();

        assert_eq!(
            evaluate_talk_impulse(now, 0, 0, &mut state),
            TalkImpulseDecision::Disabled
        );
        assert_eq!(state.next_talk_impulse_at, None);

        assert_eq!(
            evaluate_talk_impulse(now, 5, 0, &mut state),
            TalkImpulseDecision::Waiting
        );
        assert_eq!(
            state.next_talk_impulse_at,
            Some(now + chrono::Duration::seconds(240))
        );

        state.last_user_activity_at = Some(now + chrono::Duration::seconds(200));
        assert_eq!(
            evaluate_talk_impulse(now + chrono::Duration::seconds(240), 5, 9_999, &mut state),
            TalkImpulseDecision::SkippedRecentActivity
        );
        assert_eq!(
            state.next_talk_impulse_at,
            Some(now + chrono::Duration::seconds(240 + 360))
        );

        state.last_user_activity_at = Some(now);
        assert_eq!(
            evaluate_talk_impulse(now + chrono::Duration::seconds(600), 5, 0, &mut state),
            TalkImpulseDecision::Emit
        );
    }

    #[test]
    fn talk_impulse_setting_change_reschedules_without_emitting() {
        let now = DateTime::parse_from_rfc3339("2026-07-06T12:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let mut state = PresenceState {
            talk_interval_minutes: Some(5),
            next_talk_impulse_at: Some(now - chrono::Duration::seconds(1)),
            ..PresenceState::default()
        };

        assert_eq!(
            evaluate_talk_impulse(now, 10, 0, &mut state),
            TalkImpulseDecision::Waiting
        );
        assert_eq!(state.talk_interval_minutes, Some(10));
        assert_eq!(
            state.next_talk_impulse_at,
            Some(now + chrono::Duration::seconds(480))
        );
    }

    #[test]
    fn idle_presence_evaluation_emits_start_and_end_once() {
        let mut state = PresenceState::default();

        assert_eq!(evaluate_idle_presence(Some(299.0), &mut state), None);
        assert_eq!(
            evaluate_idle_presence(Some(300.0), &mut state),
            Some(IdlePresenceDecision::Start {
                threshold_seconds: 300
            })
        );
        assert_eq!(evaluate_idle_presence(Some(301.9), &mut state), None);
        assert_eq!(
            evaluate_idle_presence(Some(1.0), &mut state),
            Some(IdlePresenceDecision::End {
                idle_seconds: 301,
                idle_minutes: 5
            })
        );
        assert_eq!(evaluate_idle_presence(Some(0.5), &mut state), None);
    }

    #[test]
    fn idle_presence_evaluation_ignores_unavailable_input() {
        let mut state = PresenceState::default();

        assert_eq!(evaluate_idle_presence(None, &mut state), None);
        assert_eq!(evaluate_idle_presence(Some(f64::NAN), &mut state), None);
        assert_eq!(evaluate_idle_presence(Some(-1.0), &mut state), None);
        assert_eq!(
            evaluate_idle_presence(Some(300.0), &mut state),
            Some(IdlePresenceDecision::Start {
                threshold_seconds: 300
            })
        );
        assert_eq!(evaluate_idle_presence(None, &mut state), None);
        assert_eq!(evaluate_idle_presence(Some(300.5), &mut state), None);
    }

    #[test]
    fn idle_presence_evaluation_reemits_after_returning_active() {
        let mut state = PresenceState::default();

        assert_eq!(
            evaluate_idle_presence(Some(320.2), &mut state),
            Some(IdlePresenceDecision::Start {
                threshold_seconds: 300
            })
        );
        assert_eq!(
            evaluate_idle_presence(Some(10.0), &mut state),
            Some(IdlePresenceDecision::End {
                idle_seconds: 320,
                idle_minutes: 5
            })
        );
        assert_eq!(
            evaluate_idle_presence(Some(600.0), &mut state),
            Some(IdlePresenceDecision::Start {
                threshold_seconds: 300
            })
        );
    }

    #[test]
    fn conversation_event_uses_surface_boundary_fields() {
        let event =
            build_conversation_text_event("resident-test", "device-test", "surface-test", "hello");
        assert_eq!(event.kind, "conversation.text");
        assert_eq!(event.source, "surface");
        assert_eq!(event.resident_id, "resident-test");
        assert_eq!(event.device_id.as_deref(), Some("device-test"));
        assert_eq!(event.surface_id.as_deref(), Some("surface-test"));
        assert_eq!(event.payload["text"], json!("hello"));
    }

    #[test]
    fn conversation_choice_event_uses_surface_boundary_fields() {
        let event = build_conversation_choice_event(
            "resident-test",
            "device-test",
            "surface-test",
            "choice-1",
            "見る",
            0,
        );
        assert_eq!(event.kind, "conversation.choice");
        assert_eq!(event.source, "surface");
        assert_eq!(event.resident_id, "resident-test");
        assert_eq!(event.device_id.as_deref(), Some("device-test"));
        assert_eq!(event.surface_id.as_deref(), Some("surface-test"));
        assert_eq!(event.payload["choiceId"], json!("choice-1"));
        assert_eq!(event.payload["choice"], json!("見る"));
        assert_eq!(event.payload["index"], json!(0));
    }

    #[test]
    fn avatar_gesture_poke_event_uses_surface_boundary_fields() {
        let event = build_avatar_gesture_poke_event(
            "resident-test",
            "device-test",
            "surface-test",
            AvatarGesturePoke {
                actor_id: "yuukei".to_string(),
                hit_zone_id: "head".to_string(),
                hit_zone_label: Some("頭".to_string()),
                hit_surface: Some("face".to_string()),
                hit_bone: Some("head".to_string()),
                input: AvatarGestureInput {
                    kind: "pointer".to_string(),
                    button: "primary".to_string(),
                },
                screen: AvatarGestureScreen { x: 123.0, y: 456.0 },
            },
        );

        assert_eq!(event.kind, "avatar.gesture.poke");
        assert_eq!(event.source, "surface");
        assert_eq!(event.resident_id, "resident-test");
        assert_eq!(event.device_id.as_deref(), Some("device-test"));
        assert_eq!(event.surface_id.as_deref(), Some("surface-test"));
        assert_eq!(event.actor_id.as_deref(), Some("yuukei"));
        assert_eq!(event.payload["actorId"], json!("yuukei"));
        assert_eq!(event.payload["hitZoneId"], json!("head"));
        assert_eq!(event.payload["hitZoneLabel"], json!("頭"));
        assert_eq!(event.payload["hitSurface"], json!("face"));
        assert_eq!(event.payload["hitBone"], json!("head"));
        assert_eq!(event.payload["input"]["kind"], json!("pointer"));
        assert_eq!(event.payload["input"]["button"], json!("primary"));
        assert_eq!(event.payload["screen"]["x"], json!(123.0));
        assert_eq!(event.payload["screen"]["y"], json!(456.0));
    }

    #[test]
    fn time_period_uses_four_life_periods() {
        assert_eq!(time_period_for_hour(5).as_daihon_value(), "朝");
        assert_eq!(time_period_for_hour(9).as_daihon_value(), "朝");
        assert_eq!(time_period_for_hour(10).as_daihon_value(), "昼");
        assert_eq!(time_period_for_hour(16).as_daihon_value(), "昼");
        assert_eq!(time_period_for_hour(17).as_daihon_value(), "夜");
        assert_eq!(time_period_for_hour(21).as_daihon_value(), "夜");
        assert_eq!(time_period_for_hour(22).as_daihon_value(), "深夜");
        assert_eq!(time_period_for_hour(4).as_daihon_value(), "深夜");
    }

    #[tokio::test]
    async fn app_startup_event_dispatches_once_with_japanese_time_input() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_lifecycle_pack(&workspace.path().join("packs").join("default-yuukei"))?;
        let env = test_env(workspace.path(), data.path());

        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        runtime
            .attach_surface(cli_surface_session(runtime.device_id()))
            .await?;
        let commands = runtime.emit_app_startup().await?;
        let second = runtime.emit_app_startup().await?;

        assert_eq!(commands.len(), 1);
        assert!(commands[0]
            .payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .starts_with("起動："));
        assert_eq!(
            commands[0]
                .target
                .as_ref()
                .and_then(|target| target.surface_id.as_deref()),
            Some(CLI_SURFACE_ID)
        );
        assert!(second.is_empty());

        let records = runtime
            .home()
            .event_log()
            .read(EventLogQuery::default())?
            .records
            .into_iter()
            .map(|record| record.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            records
                .iter()
                .filter(|kind| kind.as_str() == "app.startup")
                .count(),
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn device_power_events_are_logged_and_dispatched() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_lifecycle_pack(&workspace.path().join("packs").join("default-yuukei"))?;
        let env = test_env(workspace.path(), data.path());

        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        runtime
            .attach_surface(cli_surface_session(runtime.device_id()))
            .await?;
        let sleep = runtime.emit_device_sleep_before().await?;
        let wake = runtime.emit_device_wake().await?;

        assert_eq!(sleep[0].payload["text"], "少し眠ります。");
        assert_eq!(wake[0].payload["text"], "おかえりなさい。");
        assert_eq!(
            wake[0]
                .target
                .as_ref()
                .and_then(|target| target.surface_id.as_deref()),
            Some(CLI_SURFACE_ID)
        );

        let records = runtime
            .home()
            .event_log()
            .read(EventLogQuery::default())?
            .records
            .into_iter()
            .map(|record| record.kind)
            .collect::<Vec<_>>();
        assert!(records.iter().any(|kind| kind == "device.sleep.before"));
        assert!(records.iter().any(|kind| kind == "device.wake"));
        Ok(())
    }

    #[tokio::test]
    async fn presence_tick_uses_life_tick_signal() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_presence_pack(&workspace.path().join("packs").join("default-yuukei"))?;
        let env = test_env(workspace.path(), data.path());

        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        runtime
            .attach_surface(cli_surface_session(runtime.device_id()))
            .await?;
        let commands = runtime.emit_presence_tick().await?;

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].payload["text"], "生活時計です。");

        let records = runtime
            .home()
            .event_log()
            .read(EventLogQuery::default())?
            .records
            .into_iter()
            .map(|record| record.kind)
            .collect::<Vec<_>>();
        assert!(records.iter().any(|kind| kind == "presence.life_tick"));
        assert!(!records.iter().any(|kind| kind == "presence.idle_tick"));
        Ok(())
    }

    #[tokio::test]
    async fn talk_impulse_uses_standard_alias_and_dispatches_dialogue() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_talk_impulse_pack(&workspace.path().join("packs").join("default-yuukei"))?;
        let env = test_env(workspace.path(), data.path());

        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        runtime
            .attach_surface(cli_surface_session(runtime.device_id()))
            .await?;
        let commands = runtime.emit_talk_impulse().await?;

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].payload["text"], "少ししゃべります。");

        let records = runtime
            .home()
            .event_log()
            .read(EventLogQuery::default())?
            .records
            .into_iter()
            .map(|record| record.kind)
            .collect::<Vec<_>>();
        assert!(records.iter().any(|kind| kind == "presence.talk_impulse"));
        Ok(())
    }

    #[tokio::test]
    async fn scene_history_persists_across_runtime_reopen() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_once_talk_impulse_pack(&workspace.path().join("packs").join("default-yuukei"))?;
        let env = test_env(workspace.path(), data.path());

        let first_runtime = LocalYuukeiRuntime::open_selected_in(env.clone()).await?;
        first_runtime
            .attach_surface(cli_surface_session(first_runtime.device_id()))
            .await?;
        let first_commands = first_runtime.emit_talk_impulse().await?;
        assert_eq!(first_commands.len(), 1);
        assert!(first_runtime.paths().scene_history_path.exists());
        drop(first_runtime);

        let second_runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        second_runtime
            .attach_surface(cli_surface_session(second_runtime.device_id()))
            .await?;
        let second_commands = second_runtime.emit_talk_impulse().await?;
        assert!(second_commands.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn selected_default_uses_per_pack_event_log() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_pack(
            &workspace.path().join("packs").join("default-yuukei"),
            "default-yuukei",
            "Default Yuukei",
            &[],
        )?;
        let env = test_env(workspace.path(), data.path());

        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        let snapshot = runtime.snapshot()?;

        assert_eq!(snapshot.world_pack_id, "default-yuukei");
        assert_eq!(runtime.install_id(), DEFAULT_WORLD_PACK_INSTALL_ID);
        assert!(runtime
            .paths()
            .event_log_path
            .ends_with("residents/default-yuukei/events.sqlite3"));
        assert!(runtime
            .paths()
            .scene_history_path
            .ends_with("residents/default-yuukei/scene-history.json"));
        assert!(runtime
            .paths()
            .variables_path
            .ends_with("residents/default-yuukei/variables.json"));
        assert!(runtime.paths().event_log_path.exists());
        assert!(runtime
            .world_pack_status()
            .settings_path
            .ends_with("settings/world-packs.json"));
        Ok(())
    }

    #[tokio::test]
    async fn actor_surface_assets_are_read_from_world_pack() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        let root = workspace.path().join("packs").join("default-yuukei");
        write_pack_with_renderer_assets(&root)?;
        let env = test_env(workspace.path(), data.path());

        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        let catalog = runtime.actor_surface_assets();

        assert_eq!(catalog.world_pack_id, "default-yuukei");
        assert_eq!(catalog.actors.len(), 2);
        assert_eq!(catalog.actors[0].actor_id, "yuukei");
        assert_eq!(
            catalog.actors[0]
                .renderer
                .as_ref()
                .map(|renderer| renderer.model.as_str()),
            Some("character/character_1.vrm")
        );
        assert_eq!(
            catalog.actors[0]
                .renderer
                .as_ref()
                .and_then(|renderer| renderer.hit_zones.first())
                .map(|hit_zone| hit_zone.id.as_str()),
            Some("head")
        );
        assert_eq!(
            catalog.actors[1]
                .renderer
                .as_ref()
                .and_then(|renderer| renderer.motions.get("walk"))
                .map(String::as_str),
            Some("motion/walk.vrma")
        );
        Ok(())
    }

    #[tokio::test]
    async fn avatar_gesture_poke_is_logged_and_dispatches_daihon() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_avatar_gesture_pack(&workspace.path().join("packs").join("default-yuukei"))?;
        let env = test_env(workspace.path(), data.path());

        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        runtime
            .attach_surface(tauri_surface_session(runtime.device_id()))
            .await?;
        let commands = runtime
            .send_avatar_gesture_poke(
                TAURI_SURFACE_ID,
                AvatarGesturePoke {
                    actor_id: "yuukei".to_string(),
                    hit_zone_id: "head".to_string(),
                    hit_zone_label: Some("頭".to_string()),
                    hit_surface: None,
                    hit_bone: None,
                    input: AvatarGestureInput {
                        kind: "pointer".to_string(),
                        button: "primary".to_string(),
                    },
                    screen: AvatarGestureScreen { x: 12.0, y: 34.0 },
                },
            )
            .await?;

        let dialogue = commands
            .iter()
            .find(|command| command.kind == "dialogue.say")
            .expect("poke dialogue command");
        assert_eq!(
            dialogue.payload["text"],
            "わ、頭は急に触らないでください……！"
        );

        let records = runtime
            .home()
            .event_log()
            .read(EventLogQuery::default())?
            .records;
        let poke = records
            .iter()
            .find(|record| record.kind == "avatar.gesture.poke")
            .expect("poke event log record");
        assert_eq!(poke.actor_id.as_deref(), Some("yuukei"));
        assert_eq!(poke.surface_id.as_deref(), Some(TAURI_SURFACE_ID));
        assert_eq!(poke.payload["hitZoneId"], json!("head"));
        assert_eq!(poke.payload["hitZoneLabel"], json!("頭"));
        assert!(records
            .iter()
            .any(|record| record.kind == "daihon.dispatch.result"));
        Ok(())
    }

    #[tokio::test]
    async fn external_world_pack_persists_and_reuses_install() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_pack(
            &workspace.path().join("packs").join("default-yuukei"),
            "default-yuukei",
            "Default Yuukei",
            &[],
        )?;
        let external_root = workspace.path().join("external-pack");
        write_pack(&external_root, "external-yuukei", "External Yuukei", &[])?;
        let env = test_env(workspace.path(), data.path());

        let runtime =
            LocalYuukeiRuntime::select_world_pack_directory_in(env.clone(), &external_root).await?;
        let install_id = runtime.install_id().to_string();
        assert_eq!(runtime.snapshot()?.world_pack_id, "external-yuukei");
        assert!(runtime
            .paths()
            .event_log_path
            .ends_with(format!("residents/{install_id}/events.sqlite3")));
        assert_ne!(install_id, DEFAULT_WORLD_PACK_INSTALL_ID);

        let reopened = LocalYuukeiRuntime::open_selected_in(env.clone()).await?;
        assert_eq!(reopened.install_id(), install_id);
        assert_eq!(reopened.resident_id(), runtime.resident_id());
        assert_eq!(reopened.snapshot()?.world_pack_id, "external-yuukei");

        let selected_again =
            LocalYuukeiRuntime::select_world_pack_directory_in(env, &external_root).await?;
        assert_eq!(selected_again.install_id(), install_id);
        Ok(())
    }

    #[tokio::test]
    async fn invalid_saved_external_pack_falls_back_to_default() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_pack(
            &workspace.path().join("packs").join("default-yuukei"),
            "default-yuukei",
            "Default Yuukei",
            &[],
        )?;
        let external_root = workspace.path().join("external-pack");
        write_pack(&external_root, "external-yuukei", "External Yuukei", &[])?;
        let env = test_env(workspace.path(), data.path());

        let selected =
            LocalYuukeiRuntime::select_world_pack_directory_in(env.clone(), &external_root).await?;
        let external_install_id = selected.install_id().to_string();
        fs::remove_file(external_root.join("pack.json"))?;

        let reopened = LocalYuukeiRuntime::open_selected_in(env).await?;
        let status = reopened.world_pack_status();
        assert_eq!(reopened.install_id(), DEFAULT_WORLD_PACK_INSTALL_ID);
        assert_eq!(reopened.snapshot()?.world_pack_id, "default-yuukei");
        assert!(status.fallback_active);
        assert_eq!(status.configured_install_id, external_install_id);
        assert!(status.last_load_error.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn invalid_saved_external_daihon_is_reported_in_session_status_and_app_log() -> Result<()>
    {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_pack(
            &workspace.path().join("packs").join("default-yuukei"),
            "default-yuukei",
            "Default Yuukei",
            &[],
        )?;
        let external_root = workspace.path().join("external-pack");
        write_pack(&external_root, "external-yuukei", "External Yuukei", &[])?;
        let env = test_env(workspace.path(), data.path());

        let selected =
            LocalYuukeiRuntime::select_world_pack_directory_in(env.clone(), &external_root).await?;
        let external_install_id = selected.install_id().to_string();
        fs::write(
            external_root
                .join("scripts")
                .join("desktop_reactions.daihon"),
            r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: だれ
「届きません。」
"#,
        )?;

        let reopened = LocalYuukeiRuntime::open_selected_in(env).await?;
        let status = reopened.world_pack_status();

        assert_eq!(reopened.install_id(), DEFAULT_WORLD_PACK_INSTALL_ID);
        assert!(status.fallback_active);
        assert_eq!(status.configured_install_id, external_install_id);
        assert_eq!(status.daihon_diagnostics.len(), 1);
        assert_eq!(
            status.daihon_diagnostics[0].install_id.as_deref(),
            Some(external_install_id.as_str())
        );
        let external_root = display_path(fs::canonicalize(&external_root)?);
        assert_eq!(
            status.daihon_diagnostics[0].pack_root.as_deref(),
            Some(external_root.as_str())
        );
        assert!(status.daihon_diagnostics[0]
            .message
            .contains("unknown Daihon speaker"));

        let raw_log = fs::read_to_string(data.path().join("app-activity.jsonl"))?;
        assert!(raw_log.contains("\"type\":\"daihon.diagnostics\""));
        assert!(raw_log.contains("\"context\":\"world-pack.fallback-load\""));
        Ok(())
    }

    #[tokio::test]
    async fn manual_selection_rejects_missing_required_capability_without_saving() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_pack(
            &workspace.path().join("packs").join("default-yuukei"),
            "default-yuukei",
            "Default Yuukei",
            &[],
        )?;
        let external_root = workspace.path().join("external-pack");
        write_pack(
            &external_root,
            "external-yuukei",
            "External Yuukei",
            &["dialogue.generate"],
        )?;
        let env = test_env(workspace.path(), data.path());

        let error =
            match LocalYuukeiRuntime::select_world_pack_directory_in(env.clone(), &external_root)
                .await
            {
                Ok(_) => panic!("missing required capability should reject the world pack"),
                Err(error) => error,
            };
        assert!(error.to_string().contains("dialogue.generate"));

        let reopened = LocalYuukeiRuntime::open_selected_in(env).await?;
        assert_eq!(reopened.install_id(), DEFAULT_WORLD_PACK_INSTALL_ID);
        assert!(!reopened.world_pack_status().fallback_active);
        Ok(())
    }

    #[tokio::test]
    async fn manual_selection_rejects_invalid_speaker_alias_without_saving() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_pack(
            &workspace.path().join("packs").join("default-yuukei"),
            "default-yuukei",
            "Default Yuukei",
            &[],
        )?;
        let external_root = workspace.path().join("external-pack");
        write_pack(&external_root, "external-yuukei", "External Yuukei", &[])?;
        let pack_path = external_root.join("pack.json");
        let mut manifest: Value = serde_json::from_str(&fs::read_to_string(&pack_path)?)?;
        manifest["actors"][0]["speakerAliases"] = json!(["ゆ", "ゆ"]);
        fs::write(&pack_path, serde_json::to_string_pretty(&manifest)?)?;
        let env = test_env(workspace.path(), data.path());

        let error =
            match LocalYuukeiRuntime::select_world_pack_directory_in(env.clone(), &external_root)
                .await
            {
                Ok(_) => panic!("invalid speaker alias should reject the world pack"),
                Err(error) => error,
            };
        assert!(error.to_string().contains("duplicate speaker alias"));

        let reopened = LocalYuukeiRuntime::open_selected_in(env).await?;
        assert_eq!(reopened.install_id(), DEFAULT_WORLD_PACK_INSTALL_ID);
        assert!(!reopened.world_pack_status().fallback_active);
        Ok(())
    }

    #[tokio::test]
    async fn extension_install_copies_folder_and_persists_settings() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_pack(
            &workspace.path().join("packs").join("default-yuukei"),
            "default-yuukei",
            "Default Yuukei",
            &[],
        )?;
        let source = workspace.path().join("downloads").join("nya-process");
        write_extension_source(&source, "nya-process", "Nya Process", "にゃ")?;
        let env = test_env(workspace.path(), data.path());

        let state = LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;

        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        let snapshot = runtime.snapshot()?;

        assert!(runtime.paths().extension_root.ends_with("extensions"));
        assert!(data
            .path()
            .join("extensions")
            .join("nya-process")
            .join("manifest.json")
            .exists());
        assert!(data
            .path()
            .join("settings")
            .join("extensions.json")
            .exists());
        assert_eq!(state.installed[0].extension_id, "nya-process");
        assert_eq!(
            state
                .hook_order
                .get(&ExtensionHookPoint::BeforeCommandEmit)
                .cloned()
                .unwrap_or_default(),
            vec!["nya-process".to_string()]
        );
        assert!(snapshot.extensions.contains_key("nya-process"));
        Ok(())
    }

    #[tokio::test]
    async fn disabled_extension_is_preserved_but_not_executed() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_pack(
            &workspace.path().join("packs").join("default-yuukei"),
            "default-yuukei",
            "Default Yuukei",
            &[],
        )?;
        let source = workspace.path().join("downloads").join("disabled-process");
        write_extension_source(&source, "disabled-process", "Disabled Process", "にゃ")?;
        let env = test_env(workspace.path(), data.path());

        LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
        let state =
            LocalYuukeiRuntime::set_extension_enabled_in(env.clone(), "disabled-process", false)?;
        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        let commands = runtime
            .send_conversation_text(CLI_SURFACE_ID, "こんにちは")
            .await?;

        assert!(!state.installed[0].enabled);
        assert_eq!(
            state
                .hook_order
                .get(&ExtensionHookPoint::BeforeCommandEmit)
                .cloned()
                .unwrap_or_default(),
            vec!["disabled-process".to_string()]
        );
        assert!(!runtime.snapshot()?.extensions["disabled-process"].enabled);
        assert!(!commands[0]
            .payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .ends_with("にゃ"));
        Ok(())
    }

    #[tokio::test]
    async fn extension_hook_order_is_user_owned_and_process_cwd_is_installed_dir() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_pack(
            &workspace.path().join("packs").join("default-yuukei"),
            "default-yuukei",
            "Default Yuukei",
            &[],
        )?;
        let nya_source = workspace.path().join("downloads").join("z-nya");
        let english_source = workspace.path().join("downloads").join("a-english");
        write_extension_source(&nya_source, "nya-suffix", "Nya Suffix", "にゃ")?;
        write_extension_source(&english_source, "english-marker", "English Marker", " EN")?;
        let env = test_env(workspace.path(), data.path());

        LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &nya_source)?;
        LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &english_source)?;
        LocalYuukeiRuntime::set_extension_hook_order_in(
            env.clone(),
            ExtensionHookPoint::BeforeCommandEmit,
            vec!["english-marker".to_string(), "nya-suffix".to_string()],
        )?;
        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        let commands = runtime
            .send_conversation_text(CLI_SURFACE_ID, "こんにちは")
            .await?;

        assert!(commands[0]
            .payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .ends_with(" ENにゃ"));
        Ok(())
    }

    #[tokio::test]
    async fn extension_capability_defaults_persist_and_apply_before_home_start() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_pack(
            &workspace.path().join("packs").join("default-yuukei"),
            "default-yuukei",
            "Default Yuukei",
            &["dialogue.generate"],
        )?;
        let source = workspace.path().join("downloads").join("user-tts");
        write_capability_extension_source(
            &source,
            "user-tts",
            "User TTS",
            &["speech.synthesis", "dialogue.generate"],
        )?;
        let env = test_env(workspace.path(), data.path());

        LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
        let runtime = LocalYuukeiRuntime::open_selected_in(env.clone()).await?;
        let mut receiver = runtime.home().subscribe_commands();
        runtime
            .send_conversation_text(CLI_SURFACE_ID, "こんにちは")
            .await?;
        let audio = next_runtime_command_of_kind(&mut receiver, "audio.play").await;
        assert_eq!(audio.payload["audioPath"], json!("/tmp/user-tts/cmd_1.wav"));
        let records = runtime
            .home()
            .event_log()
            .read(EventLogQuery::default())?
            .records;
        assert!(records.iter().any(|record| {
            record.kind == "capability.invocation.result"
                && record.payload["extensionId"] == json!("user-tts")
        }));

        let state = LocalYuukeiRuntime::set_capability_default_in(
            env.clone(),
            "speech.synthesis",
            "yuukei.default-tts",
        )?;
        assert_eq!(
            state.capability_defaults.get("speech.synthesis"),
            Some(&"yuukei.default-tts".to_string())
        );
        let reopened = LocalYuukeiRuntime::open_selected_in(env.clone()).await?;
        let mut receiver = reopened.home().subscribe_commands();
        reopened
            .send_conversation_text(CLI_SURFACE_ID, "もう一度")
            .await?;
        let audio = next_runtime_command_of_kind(&mut receiver, "audio.play").await;
        assert_eq!(audio.payload["audioPath"], json!("/tmp/user-tts/cmd_1.wav"));

        let state = LocalYuukeiRuntime::set_capability_default_in(
            env.clone(),
            "speech.synthesis",
            "user-tts",
        )?;
        assert_eq!(
            state.capability_defaults.get("speech.synthesis"),
            Some(&"user-tts".to_string())
        );
        let raw_settings =
            fs::read_to_string(data.path().join("settings").join("extensions.json"))?;
        assert!(raw_settings.contains("\"capabilityDefaults\""));
        assert!(raw_settings.contains("\"user-tts\""));

        let reopened = LocalYuukeiRuntime::open_selected_in(env).await?;
        let mut receiver = reopened.home().subscribe_commands();
        reopened
            .send_conversation_text(CLI_SURFACE_ID, "さらに")
            .await?;
        let audio = next_runtime_command_of_kind(&mut receiver, "audio.play").await;
        assert_eq!(audio.payload["audioPath"], json!("/tmp/user-tts/cmd_1.wav"));
        Ok(())
    }

    #[tokio::test]
    async fn process_extension_receives_persistent_extension_data_dir() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_llm_fallback_pack(&workspace.path().join("packs").join("default-yuukei"))?;
        let source = workspace.path().join("downloads").join("memory-env");
        write_extension_data_dir_probe_source(&source)?;
        let env = test_env(workspace.path(), data.path());

        LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        let commands = runtime
            .send_conversation_text(CLI_SURFACE_ID, "環境変数は？")
            .await?;
        assert!(commands
            .iter()
            .any(|command| command.payload["text"] == json!("data dir ok")));

        let expected_dir = data.path().join("extension-data").join("memory-env");
        assert!(expected_dir.is_dir());
        let records = runtime
            .home()
            .event_log()
            .read(EventLogQuery::default())?
            .records;
        let result = records
            .iter()
            .find(|record| {
                record.kind == "capability.invocation.result"
                    && record.payload["extensionId"] == json!("memory-env")
            })
            .expect("capability result");
        assert_eq!(result.payload["metadata"]["exists"], json!(true));
        assert_eq!(
            result.payload["metadata"]["dataDir"],
            json!(expected_dir.to_string_lossy().to_string())
        );
        Ok(())
    }

    #[tokio::test]
    async fn extension_setting_values_and_secrets_persist_without_exposing_secret() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_llm_fallback_pack(&workspace.path().join("packs").join("default-yuukei"))?;
        let source = workspace.path().join("downloads").join("settings-probe");
        write_settings_probe_extension_source(&source)?;
        let env = test_env(workspace.path(), data.path());

        LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
        let state = LocalYuukeiRuntime::set_extension_setting_values_in(
            env.clone(),
            "settings-probe",
            Map::from_iter([
                ("provider".to_string(), json!("gemini")),
                ("timeoutMs".to_string(), json!(45000)),
            ]),
        )?;
        assert_eq!(
            state.installed[0].setting_values["provider"],
            json!("gemini")
        );
        assert!(state.installed[0].secrets_set.is_empty());

        let state = LocalYuukeiRuntime::set_extension_secret_in(
            env.clone(),
            "settings-probe",
            "gemini.apiKey",
            Some("super-secret".to_string()),
        )?;
        assert_eq!(state.installed[0].secrets_set, vec!["gemini.apiKey"]);
        assert!(!serde_json::to_string(&state)?.contains("super-secret"));

        let raw_settings =
            fs::read_to_string(data.path().join("settings").join("extensions.json"))?;
        assert!(raw_settings.contains("\"extensionValues\""));
        assert!(raw_settings.contains("\"provider\""));
        assert!(!raw_settings.contains("super-secret"));

        let secrets_path = data.path().join("settings").join("extension-secrets.json");
        let raw_secrets = fs::read_to_string(&secrets_path)?;
        assert!(raw_secrets.contains("super-secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&secrets_path)?.permissions().mode() & 0o777,
                0o600
            );
        }

        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        let commands = runtime
            .send_conversation_text(CLI_SURFACE_ID, "設定は？")
            .await?;
        assert!(commands
            .iter()
            .any(|command| command.payload["text"] == json!("settings ok")));
        let records = runtime
            .home()
            .event_log()
            .read(EventLogQuery::default())?
            .records;
        assert!(!serde_json::to_string(&records)?.contains("super-secret"));
        Ok(())
    }

    #[test]
    fn extension_setting_values_reject_invalid_values() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        let source = workspace.path().join("downloads").join("settings-probe");
        write_settings_probe_extension_source(&source)?;
        let env = test_env(workspace.path(), data.path());

        LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
        let unknown = LocalYuukeiRuntime::set_extension_setting_values_in(
            env.clone(),
            "settings-probe",
            Map::from_iter([("missing".to_string(), json!("value"))]),
        )
        .unwrap_err();
        assert!(unknown.to_string().contains("unknown setting key"));

        let type_error = LocalYuukeiRuntime::set_extension_setting_values_in(
            env.clone(),
            "settings-probe",
            Map::from_iter([("timeoutMs".to_string(), json!("fast"))]),
        )
        .unwrap_err();
        assert!(type_error.to_string().contains("must be number"));

        let select_error = LocalYuukeiRuntime::set_extension_setting_values_in(
            env.clone(),
            "settings-probe",
            Map::from_iter([("provider".to_string(), json!("unknown-provider"))]),
        )
        .unwrap_err();
        assert!(select_error.to_string().contains("declared options"));

        let min_error = LocalYuukeiRuntime::set_extension_setting_values_in(
            env,
            "settings-probe",
            Map::from_iter([("timeoutMs".to_string(), json!(500))]),
        )
        .unwrap_err();
        assert!(min_error.to_string().contains("below minimum"));
        Ok(())
    }

    #[test]
    fn extension_manifest_rejects_invalid_settings_schema() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        let source = workspace.path().join("downloads").join("settings-probe");
        write_settings_probe_extension_source(&source)?;
        let manifest_path = source.join("manifest.json");
        let mut manifest: Value = serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
        manifest["settings"]["fields"][1]["key"] = json!("provider");
        fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
        let env = test_env(workspace.path(), data.path());
        let duplicate =
            LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source).unwrap_err();
        assert!(duplicate.to_string().contains("duplicate setting key"));

        write_settings_probe_extension_source(&source)?;
        let mut manifest: Value = serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
        manifest["settings"]["fields"][2]["default"] = json!("should-not-load");
        fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
        let secret_default =
            LocalYuukeiRuntime::install_extension_directory_in(env, &source).unwrap_err();
        assert!(secret_default
            .to_string()
            .contains("secret setting cannot declare default"));
        Ok(())
    }

    #[test]
    fn capability_usage_aggregates_usage_metadata_with_7_day_boundary() {
        let now = DateTime::parse_from_rfc3339("2026-07-06T12:00:00.000Z")
            .unwrap()
            .with_timezone(&Utc);
        let records = vec![
            usage_record(
                1,
                "2026-07-06T12:00:00.000Z",
                "yuukei-intelligence",
                "dialogue.generate",
                "openai-compatible",
                "local-model",
                10,
                4,
            ),
            usage_record(
                2,
                "2026-06-29T12:00:00.000Z",
                "yuukei-intelligence",
                "dialogue.generate",
                "openai-compatible",
                "local-model",
                20,
                6,
            ),
            usage_record(
                3,
                "2026-06-29T11:59:59.000Z",
                "yuukei-intelligence",
                "dialogue.generate",
                "openai-compatible",
                "local-model",
                30,
                8,
            ),
            usage_record(
                4,
                "2026-07-06T12:00:00.000Z",
                "yuukei-intelligence",
                "memory.index",
                "gemini",
                "gemini-2.5-flash",
                40,
                12,
            ),
        ];

        let usage = capability_usage_from_records(&records, now);
        let extension = usage
            .extensions
            .iter()
            .find(|extension| extension.extension_id == "yuukei-intelligence")
            .expect("extension usage");
        let dialogue = extension
            .capabilities
            .iter()
            .find(|capability| capability.capability == "dialogue.generate")
            .expect("dialogue usage");
        let local_model = &dialogue.models[0];

        assert_eq!(
            local_model.all_time,
            TokenUsageTotals {
                requests: 3,
                input_tokens: 60,
                output_tokens: 18
            }
        );
        assert_eq!(
            local_model.last_7_days,
            TokenUsageTotals {
                requests: 2,
                input_tokens: 30,
                output_tokens: 10
            }
        );
        assert!(extension
            .capabilities
            .iter()
            .any(|capability| capability.capability == "memory.index"));
    }

    #[tokio::test]
    async fn capability_usage_reads_capability_result_events() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_pack(
            &workspace.path().join("packs").join("default-yuukei"),
            "default-yuukei",
            "Default Yuukei",
            &[],
        )?;
        let env = test_env(workspace.path(), data.path());
        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        runtime.home().event_log().append(NewEventLogRecord {
            id: "evt_usage_runtime".to_string(),
            kind: "capability.invocation.result".to_string(),
            timestamp: Utc::now().to_rfc3339(),
            resident_id: DEFAULT_RESIDENT_ID.to_string(),
            source: "capability".to_string(),
            device_id: None,
            surface_id: None,
            actor_id: None,
            payload: JsonMap::from([
                ("invocationId".to_string(), json!("cap_1")),
                ("extensionId".to_string(), json!("usage-extension")),
                ("capability".to_string(), json!("dialogue.generate")),
                ("output".to_string(), json!({ "speak": true })),
                (
                    "metadata".to_string(),
                    json!({
                        "usage": {
                            "inputTokens": 12,
                            "outputTokens": 5,
                            "model": "usage-model",
                            "provider": "openai-compatible"
                        }
                    }),
                ),
            ]),
            causality: None,
            privacy: None,
        })?;

        let usage = runtime.capability_usage()?;
        let extension = usage
            .extensions
            .iter()
            .find(|extension| extension.extension_id == "usage-extension")
            .expect("extension usage");
        let model = &extension.capabilities[0].models[0];

        assert_eq!(extension.capabilities[0].capability, "dialogue.generate");
        assert_eq!(model.provider, "openai-compatible");
        assert_eq!(model.model, "usage-model");
        assert_eq!(
            model.all_time,
            TokenUsageTotals {
                requests: 1,
                input_tokens: 12,
                output_tokens: 5
            }
        );
        assert_eq!(model.last_7_days, model.all_time);
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_intelligence_process_generates_dialogue_through_device_host() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_llm_fallback_pack(&workspace.path().join("packs").join("default-yuukei"))?;
        let source = workspace
            .path()
            .join("downloads")
            .join("yuukei-intelligence");
        write_yuukei_intelligence_extension_source(&source)?;
        let env = test_env(workspace.path(), data.path());

        LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        let commands = runtime
            .send_conversation_text(CLI_SURFACE_ID, "何か言う？")
            .await?;

        let dialogue = commands
            .iter()
            .find(|command| command.kind == "dialogue.say")
            .expect("generated dialogue command");
        assert_eq!(dialogue.payload["text"], json!("スタブから返事します。"));
        assert_eq!(dialogue.source, "capability.dialogue.generate");
        let records = runtime
            .home()
            .event_log()
            .read(EventLogQuery::default())?
            .records;
        let result = records
            .iter()
            .find(|record| {
                record.kind == "capability.invocation.result"
                    && record.payload["extensionId"] == json!("yuukei-intelligence")
                    && record.payload["capability"] == json!("dialogue.generate")
            })
            .expect("dialogue.generate result");
        assert_eq!(
            result.payload["metadata"]["usage"],
            json!({
                "inputTokens": 13,
                "outputTokens": 5,
                "model": "stub-model",
                "provider": "openai-compatible"
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_intelligence_process_interprets_dialogue_through_device_host() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_interpret_pack(&workspace.path().join("packs").join("default-yuukei"))?;
        let source = workspace
            .path()
            .join("downloads")
            .join("yuukei-intelligence");
        write_yuukei_intelligence_extension_source(&source)?;
        let env = test_env(workspace.path(), data.path());

        LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        let startup = runtime.emit_app_startup().await?;
        assert!(startup.iter().any(|command| {
            command.kind == "dialogue.say"
                && command.payload["text"] == json!("今日はお出かけの日だよね！")
        }));

        let commands = runtime
            .send_conversation_text(CLI_SURFACE_ID, "あ〜うん。いやちょっと忙しくて...")
            .await?;
        let dialogue = commands
            .iter()
            .find(|command| command.kind == "dialogue.say")
            .expect("interpreted dialogue command");
        assert_eq!(
            dialogue.payload["text"],
            json!("そっか、残念だけど無理しないでね。")
        );
        let records = runtime
            .home()
            .event_log()
            .read(EventLogQuery::default())?
            .records;
        assert!(records.iter().any(|record| {
            record.kind == "capability.invocation.result"
                && record.payload["extensionId"] == json!("yuukei-intelligence")
                && record.payload["capability"] == json!("dialogue.interpret")
                && record.payload["output"]["choice"] == json!("いいえ")
        }));
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_intelligence_process_indexes_and_retrieves_memory_through_device_host(
    ) -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_llm_fallback_pack(&workspace.path().join("packs").join("default-yuukei"))?;
        let source = workspace
            .path()
            .join("downloads")
            .join("yuukei-intelligence");
        write_yuukei_intelligence_extension_source(&source)?;
        let env = test_env(workspace.path(), data.path());

        LocalYuukeiRuntime::install_extension_directory_in(env.clone(), &source)?;
        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        let mut yesterday = RuntimeEvent::new("conversation.text", "surface", DEFAULT_RESIDENT_ID);
        yesterday.timestamp = (Local::now() - chrono::Duration::days(1)).to_rfc3339();
        yesterday
            .payload
            .insert("text".to_string(), json!("唐揚げを食べた"));
        runtime
            .home()
            .event_log()
            .append(NewEventLogRecord::from(yesterday))?;

        runtime.emit_app_startup().await?;
        let commands = runtime
            .send_conversation_text(CLI_SURFACE_ID, "唐揚げのこと覚えてる？")
            .await?;
        let dialogue = commands
            .iter()
            .find(|command| command.kind == "dialogue.say")
            .expect("generated dialogue command");
        assert_eq!(
            dialogue.payload["text"],
            json!("覚えています。唐揚げが好きなんですよね。")
        );

        let records = runtime
            .home()
            .event_log()
            .read(EventLogQuery::default())?
            .records;
        assert!(records.iter().any(|record| {
            record.kind == "capability.invocation.result"
                && record.payload["capability"] == json!("memory.index")
                && record.payload["output"]["indexed"] == json!(true)
        }));
        assert!(records.iter().any(|record| {
            record.kind == "capability.invocation.result"
                && record.payload["capability"] == json!("memory.retrieve")
                && record.payload["output"]["memories"][0]["text"] == json!("唐揚げが好き。")
        }));
        Ok(())
    }

    #[test]
    fn process_extension_manifest_rejects_non_process_runtime() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        let source = workspace.path().join("downloads").join("bad-runtime");
        write_extension_source(&source, "bad-runtime", "Bad Runtime", "にゃ")?;
        let manifest_path = source.join("manifest.json");
        let mut manifest: Value = serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
        manifest["runtime"] = json!("wasm");
        fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;

        let env = test_env(workspace.path(), data.path());
        let error = LocalYuukeiRuntime::install_extension_directory_in(env, &source).unwrap_err();
        assert!(error.to_string().contains("runtime"));
        assert!(error.to_string().contains("process"));
        Ok(())
    }

    #[test]
    fn process_extension_manifest_rejects_cross_namespace_signal_alias() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        let source = workspace.path().join("downloads").join("bad-alias");
        write_extension_source(&source, "bad-alias", "Bad Alias", "にゃ")?;
        let manifest_path = source.join("manifest.json");
        let mut manifest: Value = serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
        manifest["emittedEvents"] = json!(["ext.bad-alias.allowed"]);
        manifest["signalAliases"] = json!([
            { "alias": "会話_偽装", "signal": "conversation.text" }
        ]);
        fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;

        let env = test_env(workspace.path(), data.path());
        let error = LocalYuukeiRuntime::install_extension_directory_in(env, &source).unwrap_err();
        assert!(error.to_string().contains("signal alias"));
        Ok(())
    }

    fn test_env(workspace_root: &Path, data_dir: &Path) -> LocalRuntimeEnvironment {
        LocalRuntimeEnvironment {
            workspace_root: workspace_root.to_path_buf(),
            default_world_root: workspace_root.join("packs").join("default-yuukei"),
            data_dir: data_dir.to_path_buf(),
            device_id: "device-test".to_string(),
        }
    }

    fn write_pack(
        root: &Path,
        id: &str,
        display_name: &str,
        required_capabilities: &[&str],
    ) -> Result<()> {
        fs::create_dir_all(root.join("scripts"))?;
        fs::write(
            root.join("scripts").join("desktop_reactions.daihon"),
            r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: yuukei
ユーザー発言=入力#ユーザー発言
「聞こえています。＜ユーザー発言＞」
"#,
        )?;
        fs::write(
            root.join("pack.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": id,
                "displayName": display_name,
                "defaultActorId": "yuukei",
                "actors": [
                    {
                        "id": "yuukei",
                        "displayName": display_name,
                        "profile": {}
                    }
                ],
                "signals": {
                    "allow": ["conversation.text", "surface.attach"]
                },
                "capabilities": {
                    "required": required_capabilities,
                    "optional": ["speech.synthesis"]
                },
                "daihon": {
                    "scripts": ["scripts/desktop_reactions.daihon"]
                },
                "initialVariables": {},
                "uiSpace": {}
            }))?,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn usage_record(
        sequence: i64,
        timestamp: &str,
        extension_id: &str,
        capability: &str,
        provider: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> EventLogRecord {
        EventLogRecord {
            sequence,
            id: format!("evt_usage_{sequence}"),
            kind: "capability.invocation.result".to_string(),
            timestamp: timestamp.to_string(),
            resident_id: DEFAULT_RESIDENT_ID.to_string(),
            source: "capability".to_string(),
            device_id: None,
            surface_id: None,
            actor_id: None,
            payload: JsonMap::from([
                ("extensionId".to_string(), json!(extension_id)),
                ("capability".to_string(), json!(capability)),
                (
                    "metadata".to_string(),
                    json!({
                        "usage": {
                            "inputTokens": input_tokens,
                            "outputTokens": output_tokens,
                            "model": model,
                            "provider": provider
                        }
                    }),
                ),
            ]),
            causality: None,
            privacy: None,
        }
    }

    fn write_llm_fallback_pack(root: &Path) -> Result<()> {
        fs::create_dir_all(root)?;
        fs::write(
            root.join("pack.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": "default-yuukei",
                "displayName": "Default Yuukei",
                "defaultActorId": "yuukei",
                "actors": [
                    {
                        "id": "yuukei",
                        "displayName": "Yuukei",
                        "profile": {
                            "role": "UI resident",
                            "speechStyle": "short and present"
                        }
                    }
                ],
                "signals": {
                    "allow": ["conversation.text", "surface.attach"]
                },
                "capabilities": {
                    "required": [],
                    "optional": ["speech.synthesis", "dialogue.generate"]
                },
                "llmDelegation": {
                    "signals": [{ "signal": "conversation.text" }]
                },
                "initialVariables": {},
                "uiSpace": {}
            }))?,
        )?;
        Ok(())
    }

    fn write_interpret_pack(root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("scripts"))?;
        fs::write(
            root.join("scripts").join("interpret_demo.daihon"),
            r#"
## interpret demo
### startup
合図: ＠アプリ_起動
話者: yuukei
話題=「お出かけ確認」
「今日はお出かけの日だよね！」
### reply
合図: ＠会話_入力
条件:（話題 = 「お出かけ確認」）
話者: yuukei
判定=＜解釈 (入力#ユーザー発言) 「ユーザーは今日のお出かけに行けますか？」 「はい/いいえ」＞
※（判定 = 「はい」）なら:
「やった、楽しみにしてるね。」
話題=「」
※あるいは（判定 = 「いいえ」）なら:
「そっか、残念だけど無理しないでね。」
話題=「」
※あるいは（判定 = 「不明」）なら:
「ん、今日は行けそう？それとも難しそう？」
※それ以外:
「ん、今日は行けそう？それとも難しそう？」
おわり
"#,
        )?;
        fs::write(
            root.join("pack.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": "default-yuukei",
                "displayName": "Default Yuukei",
                "defaultActorId": "yuukei",
                "actors": [
                    {
                        "id": "yuukei",
                        "displayName": "Yuukei",
                        "profile": {
                            "role": "UI resident",
                            "speechStyle": "short and present"
                        }
                    }
                ],
                "signals": {
                    "allow": ["conversation.text", "app.startup"]
                },
                "capabilities": {
                    "required": [],
                    "optional": ["dialogue.interpret", "dialogue.generate"]
                },
                "daihon": {
                    "scripts": ["scripts/interpret_demo.daihon"]
                },
                "initialVariables": {
                    "話題": ""
                },
                "uiSpace": {}
            }))?,
        )?;
        Ok(())
    }

    fn write_pack_with_renderer_assets(root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("scripts"))?;
        fs::create_dir_all(root.join("character"))?;
        fs::create_dir_all(root.join("motion"))?;
        fs::write(
            root.join("scripts").join("desktop_reactions.daihon"),
            r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: yuukei
「聞こえています。」
"#,
        )?;
        fs::write(root.join("character").join("character_1.vrm"), [])?;
        fs::write(root.join("character").join("character_2.vrm"), [])?;
        fs::write(root.join("motion").join("walk.vrma"), [])?;
        fs::write(
            root.join("pack.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": "default-yuukei",
                "displayName": "Default Yuukei",
                "defaultActorId": "yuukei",
                "actors": [
                    {
                        "id": "yuukei",
                        "displayName": "Yuukei",
                        "profile": {},
                        "renderer": {
                            "kind": "vrm",
                            "model": "character/character_1.vrm",
                            "motions": {
                                "walk": "motion/walk.vrma"
                            },
                            "hitZones": [
                                {
                                    "id": "head",
                                    "label": "頭",
                                    "source": "humanoidBone",
                                    "bones": ["head"],
                                    "shape": "auto",
                                    "events": ["avatar.gesture.poke", "avatar.gesture.pat"],
                                    "priority": 40
                                }
                            ]
                        }
                    },
                    {
                        "id": "partner",
                        "displayName": "Partner",
                        "profile": {},
                        "renderer": {
                            "kind": "vrm",
                            "model": "character/character_2.vrm",
                            "motions": {
                                "walk": "motion/walk.vrma"
                            }
                        }
                    }
                ],
                "signals": {
                    "allow": ["conversation.text", "surface.attach"]
                },
                "capabilities": {
                    "required": [],
                    "optional": []
                },
                "daihon": {
                    "scripts": ["scripts/desktop_reactions.daihon"]
                },
                "initialVariables": {},
                "uiSpace": {}
            }))?,
        )?;
        Ok(())
    }

    fn write_avatar_gesture_pack(root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("scripts"))?;
        fs::write(
            root.join("scripts").join("desktop_reactions.daihon"),
            r#"
## desktop reactions
### poke head
合図: ＠住人_つつく
条件:（入力#hitZoneId = 「head」）
話者: yuukei
＜表情 照れ＞
「わ、頭は急に触らないでください……！」
"#,
        )?;
        fs::write(
            root.join("pack.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": "default-yuukei",
                "displayName": "Default Yuukei",
                "defaultActorId": "yuukei",
                "actors": [
                    {
                        "id": "yuukei",
                        "displayName": "Yuukei",
                        "profile": {},
                        "renderer": {
                            "kind": "vrm",
                            "model": "character/character_1.vrm",
                            "hitZones": [
                                {
                                    "id": "head",
                                    "label": "頭",
                                    "source": "humanoidBone",
                                    "bones": ["head"],
                                    "shape": "auto",
                                    "events": ["avatar.gesture.poke", "avatar.gesture.pat"]
                                }
                            ]
                        }
                    }
                ],
                "signals": {
                    "allow": ["avatar.gesture.poke", "surface.attach"]
                },
                "capabilities": {
                    "required": [],
                    "optional": ["speech.synthesis"]
                },
                "daihon": {
                    "scripts": ["scripts/desktop_reactions.daihon"]
                },
                "initialVariables": {},
                "uiSpace": {}
            }))?,
        )?;
        fs::create_dir_all(root.join("character"))?;
        fs::write(root.join("character").join("character_1.vrm"), [])?;
        Ok(())
    }

    fn write_lifecycle_pack(root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("scripts"))?;
        fs::write(
            root.join("scripts").join("desktop_reactions.daihon"),
            r#"
## desktop reactions
### startup
合図: ＠アプリ_起動
話者: yuukei
「起動：＜入力#時間帯＞」

### before sleep
合図: ＠端末_スリープ前
話者: yuukei
「少し眠ります。」

### wake
合図: ＠端末_復帰
話者: yuukei
「おかえりなさい。」
"#,
        )?;
        fs::write(
            root.join("pack.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": "default-yuukei",
                "displayName": "Default Yuukei",
                "defaultActorId": "yuukei",
                "actors": [
                    {
                        "id": "yuukei",
                        "displayName": "Default Yuukei",
                        "profile": {}
                    }
                ],
                "signals": {
                    "allow": ["app.startup", "surface.attach", "device.sleep.before", "device.wake"]
                },
                "capabilities": {
                    "required": [],
                    "optional": ["speech.synthesis"]
                },
                "daihon": {
                    "scripts": ["scripts/desktop_reactions.daihon"]
                },
                "initialVariables": {},
                "uiSpace": {}
            }))?,
        )?;
        Ok(())
    }

    fn write_presence_pack(root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("scripts"))?;
        fs::write(
            root.join("scripts").join("desktop_reactions.daihon"),
            r#"
## desktop reactions
### life tick
合図: ＠生活_定期
話者: yuukei
「生活時計です。」
"#,
        )?;
        fs::write(
            root.join("pack.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": "default-yuukei",
                "displayName": "Default Yuukei",
                "defaultActorId": "yuukei",
                "actors": [
                    {
                        "id": "yuukei",
                        "displayName": "Default Yuukei",
                        "profile": {}
                    }
                ],
                "signals": {
                    "allow": ["presence.life_tick", "surface.attach"]
                },
                "capabilities": {
                    "required": [],
                    "optional": ["speech.synthesis"]
                },
                "daihon": {
                    "scripts": ["scripts/desktop_reactions.daihon"]
                },
                "initialVariables": {},
                "uiSpace": {}
            }))?,
        )?;
        Ok(())
    }

    fn write_talk_impulse_pack(root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("scripts"))?;
        fs::write(
            root.join("scripts").join("random_talk.daihon"),
            r#"
## 雑談
### talk
合図: ＠雑談_定期
話者: yuukei
「少ししゃべります。」
"#,
        )?;
        fs::write(
            root.join("pack.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": "talk-yuukei",
                "displayName": "Talk Yuukei",
                "defaultActorId": "yuukei",
                "actors": [
                    {
                        "id": "yuukei",
                        "displayName": "Talk Yuukei",
                        "profile": {}
                    }
                ],
                "signals": {
                    "allow": ["presence.talk_impulse", "surface.attach"]
                },
                "capabilities": {
                    "required": [],
                    "optional": []
                },
                "daihon": {
                    "scripts": ["scripts/random_talk.daihon"]
                },
                "initialVariables": {},
                "uiSpace": {}
            }))?,
        )?;
        Ok(())
    }

    fn write_once_talk_impulse_pack(root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("scripts"))?;
        fs::write(
            root.join("scripts").join("random_talk.daihon"),
            r#"
## 雑談
### talk
合図: ＠雑談_定期
頻度: 一度きり
話者: yuukei
「一度だけしゃべります。」
"#,
        )?;
        fs::write(
            root.join("pack.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": "talk-yuukei",
                "displayName": "Talk Yuukei",
                "defaultActorId": "yuukei",
                "actors": [
                    {
                        "id": "yuukei",
                        "displayName": "Talk Yuukei",
                        "profile": {}
                    }
                ],
                "signals": {
                    "allow": ["presence.talk_impulse", "surface.attach"]
                },
                "capabilities": {
                    "required": [],
                    "optional": []
                },
                "daihon": {
                    "scripts": ["scripts/random_talk.daihon"]
                },
                "initialVariables": {},
                "uiSpace": {}
            }))?,
        )?;
        Ok(())
    }

    fn write_extension_source(
        root: &Path,
        id: &str,
        display_name: &str,
        suffix: &str,
    ) -> Result<()> {
        fs::create_dir_all(root)?;
        fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": id,
                "displayName": display_name,
                "hooks": [
                    {
                        "hookPoint": "beforeCommandEmit",
                        "commandTypes": ["dialogue.say"]
                    }
                ],
                "process": {
                    "command": "node",
                    "args": ["append.js", suffix]
                }
            }))?,
        )?;
        fs::write(
            root.join("append.js"),
            r#"
const fs = require("node:fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const suffix = process.argv[2] ?? "";
const command = input.command;
command.payload.text = String(command.payload.text ?? "") + suffix;
process.stdout.write(JSON.stringify({ action: "replaceCommand", command }));
"#,
        )?;
        Ok(())
    }

    async fn next_runtime_command_of_kind(
        receiver: &mut tokio::sync::broadcast::Receiver<RuntimeCommand>,
        kind: &str,
    ) -> RuntimeCommand {
        loop {
            let command = tokio::time::timeout(std::time::Duration::from_secs(10), receiver.recv())
                .await
                .expect("runtime command timed out")
                .expect("runtime command channel closed");
            if command.kind == kind {
                return command;
            }
        }
    }

    fn write_capability_extension_source(
        root: &Path,
        id: &str,
        display_name: &str,
        capabilities: &[&str],
    ) -> Result<()> {
        fs::create_dir_all(root)?;
        fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": id,
                "displayName": display_name,
                "capabilities": capabilities.iter().map(|capability| json!({
                    "capability": capability,
                    "methods": ["invoke"]
                })).collect::<Vec<_>>(),
                "process": {
                    "command": "node",
                    "args": ["capability.js"]
                }
            }))?,
        )?;
        fs::write(
            root.join("capability.js"),
            format!(
                r#"
const fs = require("node:fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const output = input.capability === "speech.synthesis"
  ? {{ audioPath: "/tmp/user-tts/cmd_1.wav", durationMs: 1200, format: "wav" }}
  : {{}};
process.stdout.write(JSON.stringify({{
  invocationId: input.id,
  extensionId: "{id}",
  capability: input.capability,
  output,
  metadata: {{}}
}}));
"#
            ),
        )?;
        Ok(())
    }

    fn write_extension_data_dir_probe_source(root: &Path) -> Result<()> {
        fs::create_dir_all(root)?;
        fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": "memory-env",
                "displayName": "Memory Env Probe",
                "capabilities": [
                    {
                        "capability": "dialogue.generate",
                        "methods": ["generate"]
                    }
                ],
                "process": {
                    "command": "node",
                    "args": ["probe.js"]
                }
            }))?,
        )?;
        fs::write(
            root.join("probe.js"),
            r#"
const fs = require("node:fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const dataDir = process.env.YUUKEI_EXTENSION_DATA_DIR ?? "";
process.stdout.write(JSON.stringify({
  invocationId: input.id,
  extensionId: "memory-env",
  capability: input.capability,
  output: { speak: true, text: "data dir ok" },
  metadata: {
    dataDir,
    exists: dataDir ? fs.existsSync(dataDir) : false
  }
}));
"#,
        )?;
        Ok(())
    }

    fn write_settings_probe_extension_source(root: &Path) -> Result<()> {
        fs::create_dir_all(root)?;
        fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": "settings-probe",
                "displayName": "Settings Probe",
                "capabilities": [
                    {
                        "capability": "dialogue.generate",
                        "methods": ["generate"]
                    }
                ],
                "settings": {
                    "fields": [
                        {
                            "key": "provider",
                            "type": "select",
                            "label": "Provider",
                            "options": [
                                { "value": "gemini", "label": "Gemini" },
                                { "value": "openai-compatible", "label": "OpenAI compatible" }
                            ],
                            "default": "openai-compatible"
                        },
                        {
                            "key": "timeoutMs",
                            "type": "number",
                            "label": "Timeout",
                            "default": 30000,
                            "min": 1000,
                            "max": 120000
                        },
                        {
                            "key": "gemini.apiKey",
                            "type": "secret",
                            "label": "Gemini API key",
                            "visibleWhen": { "key": "provider", "equals": "gemini" }
                        }
                    ]
                },
                "process": {
                    "command": "node",
                    "args": ["settings.js"]
                }
            }))?,
        )?;
        fs::write(
            root.join("settings.js"),
            r#"
const fs = require("node:fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const settings = JSON.parse(process.env.YUUKEI_EXTENSION_SETTINGS_JSON ?? "{}");
const ok = settings.provider === "gemini"
  && settings.timeoutMs === 45000
  && settings["gemini.apiKey"] === "super-secret";
process.stdout.write(JSON.stringify({
  invocationId: input.id,
  extensionId: "settings-probe",
  capability: input.capability,
  output: { speak: true, text: ok ? "settings ok" : "settings missing" },
  metadata: { provider: settings.provider, hasGeminiApiKey: Boolean(settings["gemini.apiKey"]) }
}));
"#,
        )?;
        Ok(())
    }

    fn write_yuukei_intelligence_extension_source(root: &Path) -> Result<()> {
        fs::create_dir_all(root)?;
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        copy_dir_for_test(
            &repo_root
                .join("packages")
                .join("yuukei-intelligence")
                .join("src"),
            &root.join("src"),
        )?;
        fs::write(
            root.join("fetch-stub.mjs"),
            r#"
globalThis.fetch = async (url, init) => {
  const body = JSON.parse(init.body);
  if (String(url) !== "http://stub.local/v1/chat/completions") {
    throw new Error(`unexpected URL: ${url}`);
  }
  if (body.model !== "stub-model") {
    throw new Error(`unexpected model: ${body.model}`);
  }
  const bodyText = JSON.stringify(body);
  if (bodyText.includes("ユーザーは今日のお出かけに行けますか？")) {
    return new Response(JSON.stringify({
      usage: { prompt_tokens: 21, completion_tokens: 7 },
      choices: [
        {
          message: {
            content: "{\"choice\":\"いいえ\",\"confidence\":0.9}"
          }
        }
      ]
    }), {
      status: 200,
      headers: { "content-type": "application/json" }
    });
  }
  if (bodyText.includes("memory.index provider") || bodyText.includes("newFacts")) {
    return new Response(JSON.stringify({
      usage: { prompt_tokens: 34, completion_tokens: 10 },
      choices: [
        {
          message: {
            content: "{\"diary\":\"ユーザーは唐揚げの話をした。\",\"newFacts\":[\"唐揚げが好き。\"]}"
          }
        }
      ]
    }), {
      status: 200,
      headers: { "content-type": "application/json" }
    });
  }
  if (bodyText.includes("唐揚げのこと覚えてる？")) {
    if (!bodyText.includes("唐揚げが好き。")) {
      throw new Error("prompt did not include retrieved memory");
    }
    return new Response(JSON.stringify({
      usage: { prompt_tokens: 55, completion_tokens: 12 },
      choices: [
        {
          message: {
            content: "{\"speak\":true,\"text\":\"覚えています。唐揚げが好きなんですよね。\"}"
          }
        }
      ]
    }), {
      status: 200,
      headers: { "content-type": "application/json" }
    });
  }
  if (!bodyText.includes("何か言う？")) {
    throw new Error("prompt did not include source text");
  }
  return new Response(JSON.stringify({
    usage: { prompt_tokens: 13, completion_tokens: 5 },
    choices: [
      {
        message: {
          content: "{\"speak\":true,\"text\":\"スタブから返事します。\",\"expression\":\"smile\"}"
        }
      }
    ]
  }), {
    status: 200,
    headers: { "content-type": "application/json" }
  });
};
"#,
        )?;
        fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": "yuukei-intelligence",
                "displayName": "Yuukei Intelligence",
                "runtime": "process",
                "permissions": {
                    "broadEventSubscription": false
                },
                "capabilities": [
                    {
                        "capability": "dialogue.generate",
                        "methods": ["generate"]
                    },
                    {
                        "capability": "dialogue.interpret",
                        "methods": ["interpret"]
                    },
                    {
                        "capability": "memory.index",
                        "methods": ["index"]
                    },
                    {
                        "capability": "memory.retrieve",
                        "methods": ["retrieve"]
                    }
                ],
                "process": {
                    "command": "node",
                    "args": ["--import", "./fetch-stub.mjs", "src/main.mjs"],
                    "timeoutMs": 5000
                },
                "config": {
                    "provider": "openai-compatible",
                    "timeoutMs": 1000,
                    "openaiCompatible": {
                        "baseUrl": "http://stub.local/v1",
                        "model": "stub-model"
                    }
                }
            }))?,
        )?;
        Ok(())
    }

    fn copy_dir_for_test(source: &Path, destination: &Path) -> Result<()> {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let metadata = fs::metadata(&source_path)?;
            if metadata.is_dir() {
                copy_dir_for_test(&source_path, &destination_path)?;
            } else if metadata.is_file() {
                fs::copy(&source_path, &destination_path)?;
            }
        }
        Ok(())
    }
}
