use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use yuukei_event_log::{EventLog, EventLogQuery};
use yuukei_extension::{ProcessExtensionManifest, ProcessHookExtension};
use yuukei_protocol::{
    new_id, now_timestamp, JsonMap, ResidentSnapshot, RuntimeCommand, RuntimeEvent, SurfaceKind,
    SurfacePresentation, SurfaceRenderer, SurfaceSession,
};
use yuukei_resident_home::{ResidentHome, ResidentHomeError};
use yuukei_world::{WorldError, WorldPack};

mod world_pack_registry;

pub use world_pack_registry::{
    LocalRuntimeEnvironment, WorldPackInstall, WorldPackSelectionState, WorldPackSource,
    WorldPackSwitchResult, DEFAULT_WORLD_PACK_INSTALL_ID,
};

pub const DEFAULT_RESIDENT_ID: &str = "resident-default";
pub const DEFAULT_DEVICE_ID: &str = "device-local";
pub const TAURI_SURFACE_ID: &str = "surface-tauri";
pub const CLI_SURFACE_ID: &str = "surface-cli";

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
}

pub type Result<T> = std::result::Result<T, DeviceHostError>;

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
        let workspace_root = workspace_root();
        let data_dir = default_data_dir();
        let world_root = workspace_root.join("packs").join("default-yuukei");
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
                registry.mark_load_error(&requested_install.install_id, error.to_string())?;
                let default_install = registry.default_install()?;
                let status = registry.selection_state(&default_install, true);
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
            settings_path: config.data_dir.join("settings").join("world-packs.json"),
        };
        Self::open_with_status(config, status).await
    }

    async fn open_with_status(
        config: LocalRuntimeConfig,
        world_pack_status: WorldPackSelectionState,
    ) -> Result<Self> {
        fs::create_dir_all(&config.data_dir)?;
        fs::create_dir_all(&config.extension_root)?;
        if let Some(parent) = config.event_log_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let logger = AppLogger::open(&config.app_log_path)?;
        let paths = config.paths();
        logger.record("runtime.open.request", "device-host", paths_payload(&paths))?;

        let world = match WorldPack::load_from_dir(&config.world_root) {
            Ok(world) => world,
            Err(error) => {
                let _ = logger.record(
                    "runtime.open.error",
                    "device-host",
                    error_payload("world-pack", &error),
                );
                return Err(error.into());
            }
        };
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
        let home = match ResidentHome::new(&config.resident_id, world, event_log).await {
            Ok(home) => Arc::new(home),
            Err(error) => {
                let _ = logger.record(
                    "runtime.open.error",
                    "device-host",
                    error_payload("resident-home", &error),
                );
                return Err(error.into());
            }
        };
        let loaded_extensions = load_trusted_extensions(&config.extension_root, &home, &logger)?;

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
        self.world_pack_status.clone()
    }

    pub fn attach_surface(&self, session: SurfaceSession) -> Result<ResidentSnapshot> {
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
        match self.home.attach_surface(session.clone()) {
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
                let _ = self.logger.record(
                    "runtime.commands.error",
                    "resident-home",
                    error_payload("conversation.text", &error),
                );
                Err(error.into())
            }
        }
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
        ],
        presentation: SurfacePresentation {
            renderer: Some(SurfaceRenderer::Html),
            transparent: Some(false),
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

fn load_trusted_extensions(
    extension_root: &Path,
    home: &ResidentHome,
    logger: &AppLogger,
) -> Result<usize> {
    let mut loaded = 0;
    for entry in fs::read_dir(extension_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        match load_trusted_extension_manifest(&path, home) {
            Ok(Some(extension_id)) => {
                loaded += 1;
                logger.record(
                    "extension.load.ready",
                    "device-host",
                    JsonMap::from([
                        ("extensionId".to_string(), json!(extension_id)),
                        ("manifestPath".to_string(), json!(display_path(&path))),
                    ]),
                )?;
            }
            Ok(None) => {
                logger.record(
                    "extension.load.skipped",
                    "device-host",
                    JsonMap::from([("manifestPath".to_string(), json!(display_path(&path)))]),
                )?;
            }
            Err(error) => {
                logger.record(
                    "extension.load.error",
                    "device-host",
                    JsonMap::from([
                        ("manifestPath".to_string(), json!(display_path(&path))),
                        ("message".to_string(), json!(error)),
                    ]),
                )?;
            }
        }
    }
    Ok(loaded)
}

fn load_trusted_extension_manifest(
    path: &Path,
    home: &ResidentHome,
) -> std::result::Result<Option<String>, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let manifest: ProcessExtensionManifest =
        serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "unsupported extension schemaVersion: {}",
            manifest.schema_version
        ));
    }
    if manifest.hooks.is_empty() {
        return Err("extension must declare at least one hook".to_string());
    }
    let extension_id = manifest.id.clone();
    if !manifest.enabled {
        return Ok(None);
    }

    home.register_extension(ProcessHookExtension::from_manifest(manifest))
        .map_err(|error| error.to_string())?;
    Ok(Some(extension_id))
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
    async fn process_extension_manifest_is_loaded_into_snapshot() -> Result<()> {
        let workspace = tempdir()?;
        let data = tempdir()?;
        write_pack(
            &workspace.path().join("packs").join("default-yuukei"),
            "default-yuukei",
            "Default Yuukei",
            &[],
        )?;
        fs::create_dir_all(data.path().join("extensions"))?;
        fs::write(
            data.path().join("extensions").join("nya.json"),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "id": "nya-process",
                "displayName": "Nya Process",
                "enabled": true,
                "hooks": [
                    {
                        "hookPoint": "beforeCommandEmit",
                        "commandTypes": ["dialogue.say"]
                    }
                ],
                "process": {
                    "command": "missing-extension-command",
                    "args": []
                }
            }))?,
        )?;
        let env = test_env(workspace.path(), data.path());

        let runtime = LocalYuukeiRuntime::open_selected_in(env).await?;
        let snapshot = runtime.snapshot()?;

        assert!(runtime.paths().extension_root.ends_with("extensions"));
        assert!(snapshot.extensions.contains_key("nya-process"));
        Ok(())
    }

    fn test_env(workspace_root: &Path, data_dir: &Path) -> LocalRuntimeEnvironment {
        LocalRuntimeEnvironment {
            workspace_root: workspace_root.to_path_buf(),
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
}
