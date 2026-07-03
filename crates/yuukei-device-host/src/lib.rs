use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{DateTime, Local, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::task::JoinHandle;
use yuukei_capability::CapabilityRouter;
use yuukei_event_log::{EventLog, EventLogQuery};
use yuukei_extension::ProcessHookExtension;
use yuukei_protocol::{
    new_id, now_timestamp, ExtensionHookPoint, JsonMap, ResidentSnapshot, RuntimeCommand,
    RuntimeEvent, SurfaceKind, SurfacePresentation, SurfaceRenderer, SurfaceSession,
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
            app_log_path: self.app_log_path.clone(),
        }
    }

    fn extension_settings_registry(&self) -> Result<ExtensionSettingsRegistry> {
        ExtensionSettingsRegistry::open(&self.data_dir, &self.extension_root)
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarGesturePoke {
    pub actor_id: String,
    pub hit_zone_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_zone_label: Option<String>,
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
        let capabilities = build_extension_capability_router(&extension_settings, &logger)?;
        let home = match ResidentHome::with_parts(
            &config.resident_id,
            world,
            event_log,
            Arc::new(YuukeiDaihonAdapter::default()),
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
            load_trusted_extensions(&extension_settings, &home, &logger).await?;

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

    pub async fn send_avatar_gesture_poke(
        &self,
        surface_id: &str,
        gesture: AvatarGesturePoke,
    ) -> Result<Vec<RuntimeCommand>> {
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

    pub async fn emit_device_sleep_before(&self) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event("device.sleep.before", current_presence_payload())
            .await
    }

    pub async fn emit_device_wake(&self) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event("device.wake", current_presence_payload())
            .await
    }

    pub fn spawn_presence_loop(&self) -> JoinHandle<()> {
        let runtime = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(PRESENCE_LIFE_TICK_INTERVAL).await;
                if let Err(error) = runtime.emit_presence_tick().await {
                    let _ = runtime.logger.record(
                        "presence.loop.error",
                        "device-host",
                        error_payload("presence", &error),
                    );
                }
            }
        })
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
    let app_log_path = data_dir.join("app-activity.jsonl");
    LocalRuntimeConfig {
        install_id: DEFAULT_WORLD_PACK_INSTALL_ID.to_string(),
        resident_id: DEFAULT_RESIDENT_ID.to_string(),
        device_id,
        workspace_root,
        world_root,
        extension_root,
        event_log_path,
        app_log_path,
        data_dir,
    }
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
) -> Result<usize> {
    let mut loaded = 0;
    let hook_order = extension_settings.hook_order(&ExtensionHookPoint::BeforeCommandEmit);
    for entry in extension_settings.runtime_entries() {
        match entry {
            ExtensionRuntimeEntry::Ready(install) => {
                let extension_id = install.extension_id.clone();
                let manifest_path = install.manifest_path.clone();
                home.register_extension(ProcessHookExtension::from_installed_manifest(
                    install.manifest,
                    install.install_dir,
                    install.enabled,
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
) -> Result<CapabilityRouter> {
    let mut router = CapabilityRouter::new();
    for entry in extension_settings.runtime_entries() {
        match entry {
            ExtensionRuntimeEntry::Ready(install) => {
                if install.manifest.capabilities.is_empty() {
                    continue;
                }
                router
                    .register(ProcessHookExtension::from_installed_manifest(
                        install.manifest,
                        install.install_dir,
                        install.enabled,
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

fn display_path(path: impl AsRef<Path>) -> String {
    path.as_ref().display().to_string()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;
    use tempfile::tempdir;

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
    fn avatar_gesture_poke_event_uses_surface_boundary_fields() {
        let event = build_avatar_gesture_poke_event(
            "resident-test",
            "device-test",
            "surface-test",
            AvatarGesturePoke {
                actor_id: "yuukei".to_string(),
                hit_zone_id: "head".to_string(),
                hit_zone_label: Some("頭".to_string()),
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
        let commands = runtime
            .send_conversation_text(CLI_SURFACE_ID, "こんにちは")
            .await?;
        assert!(commands
            .iter()
            .any(|command| command.payload.get("speechRef") == Some(&json!("user-tts://cmd_1"))));
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
        let commands = reopened
            .send_conversation_text(CLI_SURFACE_ID, "もう一度")
            .await?;
        assert!(commands.iter().any(|command| command
            .payload
            .get("speechRef")
            .and_then(Value::as_str)
            .is_some_and(|speech_ref| speech_ref.starts_with("yuukei-default-tts://"))));

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
        let commands = reopened
            .send_conversation_text(CLI_SURFACE_ID, "さらに")
            .await?;
        assert!(commands
            .iter()
            .any(|command| command.payload.get("speechRef") == Some(&json!("user-tts://cmd_1"))));
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
  ? {{ speechRef: "user-tts://cmd_1" }}
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
}
