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
use yuukei_protocol::{
    new_id, now_timestamp, JsonMap, ResidentSnapshot, RuntimeCommand, RuntimeEvent, SurfaceKind,
    SurfacePresentation, SurfaceRenderer, SurfaceSession,
};
use yuukei_resident_home::{ResidentHome, ResidentHomeError};
use yuukei_world::{WorldError, WorldPack};

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
    pub event_log_path: PathBuf,
    pub app_log_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalRuntimeConfig {
    pub resident_id: String,
    pub device_id: String,
    pub workspace_root: PathBuf,
    pub data_dir: PathBuf,
    pub world_root: PathBuf,
    pub event_log_path: PathBuf,
    pub app_log_path: PathBuf,
}

impl LocalRuntimeConfig {
    pub fn default_local() -> Self {
        let workspace_root = workspace_root();
        let data_dir = default_data_dir();
        let world_root = workspace_root.join("packs").join("default-yuukei");
        Self {
            resident_id: DEFAULT_RESIDENT_ID.to_string(),
            device_id: DEFAULT_DEVICE_ID.to_string(),
            event_log_path: data_dir.join("events.sqlite3"),
            app_log_path: data_dir.join("app-activity.jsonl"),
            workspace_root,
            data_dir,
            world_root,
        }
    }

    pub fn paths(&self) -> RuntimePaths {
        RuntimePaths {
            workspace_root: self.workspace_root.clone(),
            data_dir: self.data_dir.clone(),
            world_root: self.world_root.clone(),
            event_log_path: self.event_log_path.clone(),
            app_log_path: self.app_log_path.clone(),
        }
    }
}

#[derive(Clone)]
pub struct LocalYuukeiRuntime {
    home: Arc<ResidentHome>,
    logger: AppLogger,
    resident_id: String,
    device_id: String,
    paths: RuntimePaths,
}

impl LocalYuukeiRuntime {
    pub async fn open_default() -> Result<Self> {
        Self::open(LocalRuntimeConfig::default_local()).await
    }

    pub async fn open(config: LocalRuntimeConfig) -> Result<Self> {
        fs::create_dir_all(&config.data_dir)?;
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
                    "appLogPath".to_string(),
                    json!(display_path(&config.app_log_path)),
                ),
            ]),
        )?;

        Ok(Self {
            home,
            logger,
            resident_id: config.resident_id,
            device_id: config.device_id,
            paths,
        })
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

    pub fn resident_id(&self) -> &str {
        &self.resident_id
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn snapshot(&self) -> Result<ResidentSnapshot> {
        self.home.snapshot().map_err(Into::into)
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

fn display_path(path: impl AsRef<Path>) -> String {
    path.as_ref().display().to_string()
}

#[cfg(test)]
mod tests {
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
}
