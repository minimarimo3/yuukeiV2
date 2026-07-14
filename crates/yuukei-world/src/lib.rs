use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use thiserror::Error;
use tokio::sync::Mutex;
use yuukei_daihon::{
    has_errors, parse_scripts, validate_script, ActionHandler, ChoiceRequest, DaihonDiagnostic,
    DaihonNumber, DaihonRuntimeError, DaihonValue, ExtractRequest, FunctionRegistry, FunctionSpec,
    GenerateRequest, GeneratedDialogue, InMemoryVariableStore, InterpretHandler, InterpretRequest,
    Interpreter, ParamSpec, ParamType, RunOptions, SceneHistoryStore, Script,
    Severity as DaihonSeverity, Span, Spanned, Stmt, SystemEvent, ValidationMode,
    CHOICE_FUNCTION_NAME, EXTRACT_FUNCTION_NAME, GENERATE_FUNCTION_NAME, INTERPRET_FUNCTION_NAME,
};
use yuukei_protocol::{
    canonical_signal_id, Causality, CommandTarget, JsonMap, RuntimeCommand, RuntimeEvent,
    SignalAliasTable,
};

#[derive(Debug, Error)]
pub enum WorldError {
    #[error("world pack io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("world pack json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("world pack validation error: {0}")]
    Validation(String),
    #[error("daihon error: {0}")]
    Daihon(DaihonDiagnosticReport),
}

pub type Result<T> = std::result::Result<T, WorldError>;

impl WorldError {
    pub fn daihon_report(&self) -> Option<&DaihonDiagnosticReport> {
        match self {
            Self::Daihon(report) => Some(report),
            Self::Io(_) | Self::Json(_) | Self::Validation(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DaihonDiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DaihonDiagnosticPhase {
    LoadParse,
    LoadValidate,
    LoadSpeaker,
    RuntimeValidate,
    RuntimeExecute,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaihonDiagnosticEntry {
    pub phase: DaihonDiagnosticPhase,
    pub severity: DaihonDiagnosticSeverity,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world_pack_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_id: Option<String>,
}

impl DaihonDiagnosticEntry {
    pub fn with_occurred_at(mut self, occurred_at: impl Into<String>) -> Self {
        self.occurred_at = Some(occurred_at.into());
        self
    }

    pub fn with_pack_context(
        mut self,
        install_id: impl Into<String>,
        world_pack_id: impl Into<String>,
        pack_root: impl Into<String>,
    ) -> Self {
        self.install_id = Some(install_id.into());
        self.world_pack_id = Some(world_pack_id.into());
        self.pack_root = Some(pack_root.into());
        self
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaihonDiagnosticReport {
    pub diagnostics: Vec<DaihonDiagnosticEntry>,
}

impl DaihonDiagnosticReport {
    pub fn new(diagnostics: Vec<DaihonDiagnosticEntry>) -> Self {
        Self { diagnostics }
    }

    pub fn single(diagnostic: DaihonDiagnosticEntry) -> Self {
        Self::new(vec![diagnostic])
    }

    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

impl fmt::Display for DaihonDiagnosticReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(first) = self.diagnostics.first() else {
            return formatter.write_str("unknown Daihon diagnostic");
        };
        let location = match (first.line, first.column) {
            (Some(line), Some(column)) => format!(" at {line}:{column}"),
            _ => String::new(),
        };
        if self.diagnostics.len() == 1 {
            write!(formatter, "{}{}: {}", first.code, location, first.message)
        } else {
            write!(
                formatter,
                "{} Daihon diagnostics; first: {}{}: {}",
                self.diagnostics.len(),
                first.code,
                location,
                first.message
            )
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldPack {
    pub schema_version: u32,
    pub id: String,
    pub display_name: String,
    pub default_actor_id: String,
    pub actors: Vec<ActorDefinition>,
    pub signals: SignalAllowlist,
    #[serde(default)]
    pub capabilities: CapabilityDeclarations,
    #[serde(default, skip_serializing_if = "LlmDelegation::is_empty")]
    pub llm_delegation: LlmDelegation,
    #[serde(default)]
    pub daihon: DaihonConfig,
    #[serde(default)]
    pub initial_variables: JsonMap,
    #[serde(default)]
    pub ui_space: JsonMap,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorDefinition {
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speaker_aliases: Vec<String>,
    #[serde(default)]
    pub profile: JsonMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer: Option<ActorRendererDefinition>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorRendererDefinition {
    pub kind: ActorRendererKind,
    pub model: String,
    #[serde(default)]
    pub motions: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hit_zones: Vec<ActorHitZoneDefinition>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorRendererKind {
    Vrm,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorHitZoneDefinition {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub source: ActorHitZoneSource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bones: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ActorHitZoneShape>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorHitZoneSource {
    HumanoidBone,
    NodeName,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorHitZoneShape {
    Auto,
    Mesh,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalAllowlist {
    #[serde(default)]
    pub allow: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDeclarations {
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmDelegation {
    #[serde(default)]
    pub signals: Vec<LlmDelegationSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_budget: Option<u32>,
}

impl LlmDelegation {
    pub fn is_empty(&self) -> bool {
        self.signals.is_empty() && self.daily_budget.is_none()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmDelegationSignal {
    pub signal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_seconds: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaihonConfig {
    #[serde(default)]
    pub scripts: Vec<String>,
    #[serde(skip)]
    pub loaded_scripts: Vec<DaihonScriptSource>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaihonScriptSource {
    pub path: String,
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutedScene {
    pub key: String,
    pub event_name: String,
    pub scene_name: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaihonDispatchResult {
    pub commands: Vec<RuntimeCommand>,
    pub executed_scenes: Vec<ExecutedScene>,
    #[serde(default)]
    pub variable_patches: Vec<Value>,
}

impl DaihonDispatchResult {
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
            && self.executed_scenes.is_empty()
            && self.variable_patches.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaihonInterpretRequest {
    pub input_text: String,
    pub question: String,
    pub choices: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaihonChoiceRequest {
    pub choices: Vec<String>,
    pub timeout_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaihonExtractRequest {
    pub input_text: String,
    pub instruction: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaihonGenerateRequest {
    pub instruction: String,
    pub speaker_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaihonGenerateResponse {
    pub text: String,
    pub expression: Option<String>,
    pub motion: Option<String>,
}

#[async_trait]
pub trait DaihonInterpretHandler: Send {
    async fn interpret(&mut self, request: DaihonInterpretRequest) -> String;

    async fn flush_commands_before_choice(&mut self, _commands: Vec<RuntimeCommand>) -> bool {
        false
    }

    async fn choose(&mut self, _request: DaihonChoiceRequest) -> String {
        yuukei_daihon::UNKNOWN_INTERPRETATION.to_string()
    }

    async fn extract(&mut self, _request: DaihonExtractRequest) -> String {
        yuukei_daihon::UNKNOWN_INTERPRETATION.to_string()
    }

    async fn generate(
        &mut self,
        _request: DaihonGenerateRequest,
    ) -> Option<DaihonGenerateResponse> {
        None
    }
}

#[derive(Debug, Default)]
pub struct NoopDaihonInterpretHandler;

#[async_trait]
impl DaihonInterpretHandler for NoopDaihonInterpretHandler {
    async fn interpret(&mut self, _request: DaihonInterpretRequest) -> String {
        yuukei_daihon::UNKNOWN_INTERPRETATION.to_string()
    }
}

#[async_trait]
pub trait DaihonAdapter: Send + Sync {
    async fn load_world(&self, world: &WorldPack) -> Result<()>;
    async fn load_world_with_signal_aliases(
        &self,
        world: &WorldPack,
        aliases: &SignalAliasTable,
    ) -> Result<()> {
        let _ = aliases;
        self.load_world(world).await
    }
    async fn dispatch(
        &self,
        event: &RuntimeEvent,
        world: &WorldPack,
    ) -> Result<DaihonDispatchResult> {
        let mut interpret_handler = NoopDaihonInterpretHandler;
        self.dispatch_with_interpret(event, world, &mut interpret_handler)
            .await
    }

    async fn dispatch_with_interpret(
        &self,
        event: &RuntimeEvent,
        world: &WorldPack,
        interpret_handler: &mut dyn DaihonInterpretHandler,
    ) -> Result<DaihonDispatchResult>;
}

#[derive(Debug, Default)]
pub struct YuukeiDaihonAdapter {
    state: Mutex<YuukeiDaihonState>,
}

#[derive(Debug, Default)]
struct YuukeiDaihonState {
    scripts: Vec<LoadedDaihonScript>,
    variables: BTreeMap<String, DaihonValue>,
    variables_storage: YuukeiVariableStorage,
    history: YuukeiSceneHistory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SceneHistoryPersistenceError {
    pub path: PathBuf,
    pub message: String,
}

pub type SceneHistoryErrorLogger = Arc<dyn Fn(SceneHistoryPersistenceError) + Send + Sync>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneHistoryEntry {
    pub event_name: String,
    pub scene_name: String,
    pub last_executed_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariablePersistenceError {
    pub path: PathBuf,
    pub message: String,
}

pub type VariableErrorLogger = Arc<dyn Fn(VariablePersistenceError) + Send + Sync>;

impl YuukeiDaihonAdapter {
    pub fn with_persistent_scene_history(path: impl Into<PathBuf>) -> Self {
        Self {
            state: Mutex::new(YuukeiDaihonState {
                history: YuukeiSceneHistory::persistent(path.into(), None),
                ..YuukeiDaihonState::default()
            }),
        }
    }

    pub fn with_persistent_scene_history_logger(
        path: impl Into<PathBuf>,
        error_logger: SceneHistoryErrorLogger,
    ) -> Self {
        Self {
            state: Mutex::new(YuukeiDaihonState {
                history: YuukeiSceneHistory::persistent(path.into(), Some(error_logger)),
                ..YuukeiDaihonState::default()
            }),
        }
    }

    pub fn with_persistent_state(
        scene_history_path: impl Into<PathBuf>,
        variables_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            state: Mutex::new(YuukeiDaihonState {
                history: YuukeiSceneHistory::persistent(scene_history_path.into(), None),
                variables_storage: YuukeiVariableStorage::persistent(variables_path.into(), None),
                ..YuukeiDaihonState::default()
            }),
        }
    }

    pub fn with_persistent_state_loggers(
        scene_history_path: impl Into<PathBuf>,
        scene_history_error_logger: SceneHistoryErrorLogger,
        variables_path: impl Into<PathBuf>,
        variable_error_logger: VariableErrorLogger,
    ) -> Self {
        Self {
            state: Mutex::new(YuukeiDaihonState {
                history: YuukeiSceneHistory::persistent(
                    scene_history_path.into(),
                    Some(scene_history_error_logger),
                ),
                variables_storage: YuukeiVariableStorage::persistent(
                    variables_path.into(),
                    Some(variable_error_logger),
                ),
                ..YuukeiDaihonState::default()
            }),
        }
    }

    pub async fn scene_history_entries(&self) -> Vec<SceneHistoryEntry> {
        let state = self.state.lock().await;
        state.history.entries()
    }

    pub async fn reset_scene_history(&self) {
        let mut state = self.state.lock().await;
        state.history.clear();
    }
}

#[derive(Clone)]
struct YuukeiSceneHistory {
    entries: BTreeMap<(String, String), DateTime<FixedOffset>>,
    last_by_event: BTreeMap<String, String>,
    storage: SceneHistoryStorage,
}

impl fmt::Debug for YuukeiSceneHistory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("YuukeiSceneHistory")
            .field("entries", &self.entries)
            .field("last_by_event", &self.last_by_event)
            .field("storage", &self.storage)
            .finish()
    }
}

impl Default for YuukeiSceneHistory {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            last_by_event: BTreeMap::new(),
            storage: SceneHistoryStorage::Memory,
        }
    }
}

impl YuukeiSceneHistory {
    fn persistent(path: PathBuf, error_logger: Option<SceneHistoryErrorLogger>) -> Self {
        let storage = SceneHistoryStorage::File { path, error_logger };
        let mut history = Self {
            storage,
            ..Self::default()
        };
        history.load_from_storage();
        history
    }

    fn reload(&mut self) {
        if matches!(self.storage, SceneHistoryStorage::File { .. }) {
            self.entries.clear();
            self.last_by_event.clear();
            self.load_from_storage();
        }
    }

    fn load_from_storage(&mut self) {
        let SceneHistoryStorage::File { path, .. } = &self.storage else {
            return;
        };
        match fs::read_to_string(path) {
            Ok(raw) => match serde_json::from_str::<StoredSceneHistory>(&raw) {
                Ok(stored) if stored.schema_version == 1 => {
                    self.entries = stored
                        .executed_scenes
                        .into_iter()
                        .map(|entry| ((entry.event_name, entry.scene_name), entry.last_executed_at))
                        .collect();
                    self.last_by_event = stored
                        .last_scenes
                        .into_iter()
                        .map(|entry| (entry.event_name, entry.scene_name))
                        .collect();
                }
                Ok(stored) => {
                    self.report_storage_error(format!(
                        "unsupported scene history schemaVersion: {}",
                        stored.schema_version
                    ));
                }
                Err(error) => {
                    self.report_storage_error(format!("invalid scene history JSON: {error}"));
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                self.report_storage_error(format!("failed to read scene history: {error}"));
            }
        }
    }

    fn save_to_storage(&self) {
        let SceneHistoryStorage::File { path, .. } = &self.storage else {
            return;
        };
        if let Some(parent) = path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                self.report_storage_error(format!(
                    "failed to create scene history directory: {error}"
                ));
                return;
            }
        }
        let stored = StoredSceneHistory {
            schema_version: 1,
            executed_scenes: self
                .entries
                .iter()
                .map(
                    |((event_name, scene_name), last_executed_at)| StoredSceneExecution {
                        event_name: event_name.clone(),
                        scene_name: scene_name.clone(),
                        last_executed_at: *last_executed_at,
                    },
                )
                .collect(),
            last_scenes: self
                .last_by_event
                .iter()
                .map(|(event_name, scene_name)| StoredLastScene {
                    event_name: event_name.clone(),
                    scene_name: scene_name.clone(),
                })
                .collect(),
        };
        match serde_json::to_vec_pretty(&stored) {
            Ok(bytes) => {
                if let Err(error) = fs::write(path, bytes) {
                    self.report_storage_error(format!("failed to write scene history: {error}"));
                }
            }
            Err(error) => {
                self.report_storage_error(format!("failed to encode scene history: {error}"));
            }
        }
    }

    fn report_storage_error(&self, message: String) {
        if let SceneHistoryStorage::File { path, error_logger } = &self.storage {
            let error = SceneHistoryPersistenceError {
                path: path.clone(),
                message,
            };
            if let Some(error_logger) = error_logger {
                error_logger(error);
            } else {
                eprintln!(
                    "scene history persistence error at {}: {}",
                    error.path.display(),
                    error.message
                );
            }
        }
    }

    fn entries(&self) -> Vec<SceneHistoryEntry> {
        let mut entries = self
            .entries
            .iter()
            .map(
                |((event_name, scene_name), last_executed_at)| SceneHistoryEntry {
                    event_name: event_name.clone(),
                    scene_name: scene_name.clone(),
                    last_executed_at: last_executed_at.to_rfc3339(),
                },
            )
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| b.last_executed_at.cmp(&a.last_executed_at));
        entries
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.last_by_event.clear();
        self.save_to_storage();
    }
}

impl SceneHistoryStore for YuukeiSceneHistory {
    fn last_executed_at(
        &self,
        event_name: &str,
        scene_name: &str,
    ) -> Option<DateTime<FixedOffset>> {
        self.entries
            .get(&(event_name.to_owned(), scene_name.to_owned()))
            .copied()
    }

    fn last_scene_for_event(&self, event_name: &str) -> Option<String> {
        self.last_by_event.get(event_name).cloned()
    }

    fn record_executed(&mut self, event_name: &str, scene_name: &str, at: DateTime<FixedOffset>) {
        self.entries
            .insert((event_name.to_owned(), scene_name.to_owned()), at);
        self.last_by_event
            .insert(event_name.to_owned(), scene_name.to_owned());
        self.save_to_storage();
    }
}

#[derive(Clone)]
enum SceneHistoryStorage {
    Memory,
    File {
        path: PathBuf,
        error_logger: Option<SceneHistoryErrorLogger>,
    },
}

impl fmt::Debug for SceneHistoryStorage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory => formatter.write_str("Memory"),
            Self::File { path, .. } => formatter
                .debug_struct("File")
                .field("path", path)
                .finish_non_exhaustive(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredSceneHistory {
    schema_version: u32,
    #[serde(default)]
    executed_scenes: Vec<StoredSceneExecution>,
    #[serde(default)]
    last_scenes: Vec<StoredLastScene>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredSceneExecution {
    event_name: String,
    scene_name: String,
    last_executed_at: DateTime<FixedOffset>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredLastScene {
    event_name: String,
    scene_name: String,
}

#[derive(Clone, Debug)]
struct YuukeiVariableStorage {
    values: BTreeMap<String, DaihonValue>,
    storage: VariableStorage,
}

impl Default for YuukeiVariableStorage {
    fn default() -> Self {
        Self {
            values: BTreeMap::new(),
            storage: VariableStorage::Memory,
        }
    }
}

impl YuukeiVariableStorage {
    fn persistent(path: PathBuf, error_logger: Option<VariableErrorLogger>) -> Self {
        let storage = VariableStorage::File { path, error_logger };
        let mut variables = Self {
            storage,
            ..Self::default()
        };
        variables.load_from_storage();
        variables
    }

    fn reload(&mut self) {
        if matches!(self.storage, VariableStorage::File { .. }) {
            self.values.clear();
            self.load_from_storage();
        }
    }

    fn load_from_storage(&mut self) {
        let VariableStorage::File { path, .. } = &self.storage else {
            return;
        };
        match fs::read_to_string(path) {
            Ok(raw) => match serde_json::from_str::<StoredVariables>(&raw) {
                Ok(stored) if stored.version == 1 => {
                    self.values = stored.variables;
                }
                Ok(stored) => {
                    self.report_storage_error(format!(
                        "unsupported variables schema version: {}",
                        stored.version
                    ));
                }
                Err(error) => {
                    self.report_storage_error(format!("invalid variables JSON: {error}"));
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                self.report_storage_error(format!("failed to read variables: {error}"));
            }
        }
    }

    fn set_values(&mut self, values: BTreeMap<String, DaihonValue>) {
        self.values = persistent_variables(values);
    }

    fn save_if_changed(&mut self, values: &BTreeMap<String, DaihonValue>) {
        let persistent = persistent_variables(values.clone());
        if persistent == self.values {
            return;
        }
        self.values = persistent;
        self.save_to_storage();
    }

    fn save_to_storage(&self) {
        let VariableStorage::File { path, .. } = &self.storage else {
            return;
        };
        if let Some(parent) = path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                self.report_storage_error(format!("failed to create variables directory: {error}"));
                return;
            }
        }
        let stored = StoredVariables {
            version: 1,
            variables: self.values.clone(),
        };
        let bytes = match serde_json::to_vec_pretty(&stored) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.report_storage_error(format!("failed to encode variables: {error}"));
                return;
            }
        };
        let temporary_path = path.with_extension("json.tmp");
        if let Err(error) = fs::write(&temporary_path, bytes) {
            self.report_storage_error(format!("failed to write temporary variables: {error}"));
            return;
        }
        if let Err(error) = fs::rename(&temporary_path, path) {
            self.report_storage_error(format!("failed to replace variables: {error}"));
        }
    }

    fn report_storage_error(&self, message: String) {
        if let VariableStorage::File { path, error_logger } = &self.storage {
            let error = VariablePersistenceError {
                path: path.clone(),
                message,
            };
            if let Some(error_logger) = error_logger {
                error_logger(error);
            } else {
                eprintln!(
                    "variables persistence error at {}: {}",
                    error.path.display(),
                    error.message
                );
            }
        }
    }
}

#[derive(Clone)]
enum VariableStorage {
    Memory,
    File {
        path: PathBuf,
        error_logger: Option<VariableErrorLogger>,
    },
}

impl fmt::Debug for VariableStorage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory => formatter.write_str("Memory"),
            Self::File { path, .. } => formatter
                .debug_struct("File")
                .field("path", path)
                .finish_non_exhaustive(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredVariables {
    version: u32,
    #[serde(default)]
    variables: BTreeMap<String, DaihonValue>,
}

fn persistent_variables(values: BTreeMap<String, DaihonValue>) -> BTreeMap<String, DaihonValue> {
    values
        .into_iter()
        .filter(|(key, _)| is_persistent_variable_key(key))
        .collect()
}

fn is_persistent_variable_key(key: &str) -> bool {
    let parts = key.split('#').collect::<Vec<_>>();
    match parts.as_slice() {
        ["全体", name] => !name.is_empty(),
        ["住人", actor, name] => !actor.is_empty() && !name.is_empty(),
        ["関係", subject, object, name] => {
            !subject.is_empty() && !object.is_empty() && !name.is_empty()
        }
        _ => false,
    }
}

fn merge_initial_and_persistent_variables(
    initial: BTreeMap<String, DaihonValue>,
    persistent: &BTreeMap<String, DaihonValue>,
    storage: &YuukeiVariableStorage,
) -> BTreeMap<String, DaihonValue> {
    let mut variables = initial.clone();
    for (key, value) in persistent {
        match initial.get(key) {
            Some(initial_value) if initial_value.value_type() != value.value_type() => {
                storage.report_storage_error(format!(
                    "persistent variable {key} has type {:?}, initial value has type {:?}; using initial value",
                    value.value_type(),
                    initial_value.value_type()
                ));
            }
            _ => {
                variables.insert(key.clone(), value.clone());
            }
        }
    }
    variables
}

#[derive(Clone, Debug)]
struct LoadedDaihonScript {
    path: String,
    script: Script,
}

#[async_trait]
impl DaihonAdapter for YuukeiDaihonAdapter {
    async fn load_world(&self, world: &WorldPack) -> Result<()> {
        self.load_world_with_signal_aliases(world, &SignalAliasTable::default())
            .await
    }

    async fn load_world_with_signal_aliases(
        &self,
        world: &WorldPack,
        aliases: &SignalAliasTable,
    ) -> Result<()> {
        world.validate()?;
        if !world.daihon.scripts.is_empty() && world.daihon.loaded_scripts.is_empty() {
            return Err(WorldError::Validation(
                "daihon scripts are declared but no script source is loaded".to_string(),
            ));
        }

        let speaker_aliases = world.speaker_resolution_table()?;
        let function_registry = yuukei_function_registry();
        let mut scripts = Vec::new();
        for source in &world.daihon.loaded_scripts {
            let parsed_scripts = parse_scripts(&source.source).map_err(|diagnostics| {
                WorldError::Daihon(diagnostic_report(
                    &diagnostics,
                    DaihonDiagnosticPhase::LoadParse,
                    Some(&source.path),
                    None,
                ))
            })?;
            for mut script in parsed_scripts {
                canonicalize_daihon_script_signals(&mut script, aliases);
                canonicalize_daihon_script_speakers(&mut script, &speaker_aliases, &source.path)?;
                let diagnostics = validate_script(&script, Some(&function_registry));
                if has_errors(&diagnostics) {
                    return Err(WorldError::Daihon(diagnostic_report(
                        &diagnostics,
                        DaihonDiagnosticPhase::LoadValidate,
                        Some(&source.path),
                        None,
                    )));
                }
                scripts.push(LoadedDaihonScript {
                    path: source.path.clone(),
                    script,
                });
            }
        }

        let initial_variables = world
            .initial_variables
            .iter()
            .filter_map(|(key, value)| {
                json_to_daihon_value(value).map(|value| (key.clone(), value))
            })
            .collect();
        let mut state = self.state.lock().await;
        state.variables_storage.reload();
        let variables = merge_initial_and_persistent_variables(
            initial_variables,
            &state.variables_storage.values,
            &state.variables_storage,
        );
        state.scripts = scripts;
        state.variables = variables;
        let variables_snapshot = state.variables.clone();
        state
            .variables_storage
            .set_values(persistent_variables(variables_snapshot));
        state.history.reload();
        Ok(())
    }

    async fn dispatch_with_interpret(
        &self,
        event: &RuntimeEvent,
        world: &WorldPack,
        interpret_handler: &mut dyn DaihonInterpretHandler,
    ) -> Result<DaihonDispatchResult> {
        let mut state = self.state.lock().await;
        if state.scripts.is_empty() {
            return Ok(DaihonDispatchResult::default());
        }

        let scripts = state.scripts.clone();
        let function_registry = yuukei_function_registry();
        let mut variables = InMemoryVariableStore::from_values(state.variables.clone());
        for (name, value) in event_inputs(event) {
            variables = variables.with_input(name, value);
        }
        let previous_variables = variables.values().clone();
        let mut commands = Vec::new();
        let mut executed_scenes = Vec::new();

        for loaded in scripts
            .iter()
            .filter(|loaded| script_accepts_event(&loaded.script, &event.kind))
        {
            let command_buffer = Arc::new(Mutex::new(Vec::new()));
            let mut action_handler = YuukeiActionHandler::new(
                event,
                world.default_actor_id.clone(),
                command_buffer.clone(),
            );
            let mut interpret_bridge = YuukeiInterpretBridge {
                interpret_handler,
                command_buffer,
            };
            let mut interpreter = Interpreter {
                action_handler: &mut action_handler,
                interpret_handler: &mut interpret_bridge,
                variable_store: &mut variables,
                scene_history: &mut state.history,
                function_registry: &function_registry,
                options: RunOptions {
                    trigger: Some(SystemEvent::new(event.kind.clone(), Span::empty())),
                    default_speaker: Some(world.default_actor_id.clone()),
                    validation_mode: ValidationMode::Strict,
                    ..RunOptions::default()
                },
                interpretation_count: 0,
                generation_count: 0,
                choice_count: 0,
                diagnostics: Vec::new(),
            };
            let run = interpreter
                .run_script(&loaded.script)
                .await
                .map_err(|error| {
                    WorldError::Daihon(diagnostic_report(
                        std::slice::from_ref(error.diagnostic.as_ref()),
                        DaihonDiagnosticPhase::RuntimeExecute,
                        Some(&loaded.path),
                        Some(event),
                    ))
                })?;
            if has_errors(&run.diagnostics) {
                return Err(WorldError::Daihon(diagnostic_report(
                    &run.diagnostics,
                    DaihonDiagnosticPhase::RuntimeValidate,
                    Some(&loaded.path),
                    Some(event),
                )));
            }
            if let Some(scene) = run.selected_scene {
                executed_scenes.push(ExecutedScene {
                    key: format!("{}#{}", loaded.path, scene.name),
                    event_name: run.event_name,
                    scene_name: scene.name,
                });
            }
            commands.extend(action_handler.drain_commands().await);
        }

        let next_variables = variables.into_values();
        let variable_patches = diff_variable_patches(&previous_variables, &next_variables);
        state.variables = next_variables;
        if !variable_patches.is_empty() {
            let variables_snapshot = state.variables.clone();
            state.variables_storage.save_if_changed(&variables_snapshot);
        }

        Ok(DaihonDispatchResult {
            commands,
            executed_scenes,
            variable_patches,
        })
    }
}

struct YuukeiActionHandler {
    event: RuntimeEvent,
    default_actor_id: String,
    commands: Arc<Mutex<Vec<RuntimeCommand>>>,
}

struct YuukeiInterpretBridge<'a> {
    interpret_handler: &'a mut dyn DaihonInterpretHandler,
    command_buffer: Arc<Mutex<Vec<RuntimeCommand>>>,
}

#[async_trait]
impl InterpretHandler for YuukeiInterpretBridge<'_> {
    async fn interpret(
        &mut self,
        request: InterpretRequest,
    ) -> std::result::Result<String, DaihonRuntimeError> {
        Ok(self
            .interpret_handler
            .interpret(DaihonInterpretRequest {
                input_text: request.input_text,
                question: request.question,
                choices: request.choices,
            })
            .await)
    }

    async fn generate(
        &mut self,
        request: GenerateRequest,
    ) -> std::result::Result<Option<GeneratedDialogue>, DaihonRuntimeError> {
        Ok(self
            .interpret_handler
            .generate(DaihonGenerateRequest {
                instruction: request.instruction,
                speaker_id: request.speaker_id,
            })
            .await
            .map(|response| GeneratedDialogue {
                text: response.text,
                expression: response.expression,
                motion: response.motion,
            }))
    }

    async fn extract(
        &mut self,
        request: ExtractRequest,
    ) -> std::result::Result<String, DaihonRuntimeError> {
        Ok(self
            .interpret_handler
            .extract(DaihonExtractRequest {
                input_text: request.input_text,
                instruction: request.instruction,
            })
            .await)
    }

    async fn choose(
        &mut self,
        request: ChoiceRequest,
    ) -> std::result::Result<String, DaihonRuntimeError> {
        let pending_commands: Vec<RuntimeCommand> =
            self.command_buffer.lock().await.drain(..).collect();
        if !pending_commands.is_empty()
            && !self
                .interpret_handler
                .flush_commands_before_choice(pending_commands.clone())
                .await
        {
            self.command_buffer.lock().await.extend(pending_commands);
        }
        Ok(self
            .interpret_handler
            .choose(DaihonChoiceRequest {
                choices: request.choices,
                timeout_seconds: request.timeout_seconds,
            })
            .await)
    }
}

impl YuukeiActionHandler {
    fn new(
        event: &RuntimeEvent,
        default_actor_id: String,
        commands: Arc<Mutex<Vec<RuntimeCommand>>>,
    ) -> Self {
        Self {
            event: event.clone(),
            default_actor_id,
            commands,
        }
    }

    async fn drain_commands(&self) -> Vec<RuntimeCommand> {
        self.commands.lock().await.drain(..).collect()
    }

    fn command(&self, kind: &str, speaker_id: Option<&str>) -> RuntimeCommand {
        let actor_id = speaker_id.unwrap_or(&self.default_actor_id).to_string();
        let mut command = RuntimeCommand::new(kind, "daihon", self.event.resident_id.clone());
        command.causality = Some(Causality {
            source_event_id: Some(self.event.id.clone()),
            source_command_id: None,
            trace_id: self
                .event
                .causality
                .as_ref()
                .and_then(|causality| causality.trace_id.clone()),
        });
        command.target = Some(CommandTarget {
            device_id: self.event.device_id.clone(),
            surface_id: self.event.surface_id.clone(),
            actor_id: Some(actor_id),
        });
        command
    }
}

#[async_trait]
impl ActionHandler for YuukeiActionHandler {
    async fn show_dialogue(
        &mut self,
        speaker_id: Option<&str>,
        text: &str,
    ) -> std::result::Result<(), DaihonRuntimeError> {
        let actor_id = speaker_id.unwrap_or(&self.default_actor_id).to_string();
        let mut command = self.command("dialogue.say", speaker_id);
        command.payload = JsonMap::from([
            ("text".to_string(), Value::String(text.to_string())),
            ("speakerId".to_string(), Value::String(actor_id)),
            ("emotion".to_string(), Value::String("neutral".to_string())),
        ]);
        self.commands.lock().await.push(command);
        Ok(())
    }

    async fn show_generated_dialogue(
        &mut self,
        speaker_id: Option<&str>,
        dialogue: GeneratedDialogue,
        instruction: &str,
    ) -> std::result::Result<(), DaihonRuntimeError> {
        let actor_id = speaker_id.unwrap_or(&self.default_actor_id).to_string();
        if let Some(expression) = dialogue.expression.filter(|value| !value.trim().is_empty()) {
            let mut command = self.command("avatar.expression", speaker_id);
            command.source = "capability.dialogue.generate".to_string();
            command.payload = JsonMap::from([
                ("expression".to_string(), Value::String(expression)),
                ("speakerId".to_string(), Value::String(actor_id.clone())),
                (
                    "sourceFunction".to_string(),
                    Value::String(GENERATE_FUNCTION_NAME.to_string()),
                ),
                (
                    "sourceCapability".to_string(),
                    Value::String("dialogue.generate".to_string()),
                ),
                (
                    "generationInstruction".to_string(),
                    Value::String(instruction.to_string()),
                ),
            ]);
            self.commands.lock().await.push(command);
        }
        if let Some(motion) = dialogue.motion.filter(|value| !value.trim().is_empty()) {
            let mut command = self.command("avatar.motion", speaker_id);
            command.source = "capability.dialogue.generate".to_string();
            command.payload = JsonMap::from([
                ("motion".to_string(), Value::String(motion)),
                ("speakerId".to_string(), Value::String(actor_id.clone())),
                (
                    "sourceFunction".to_string(),
                    Value::String(GENERATE_FUNCTION_NAME.to_string()),
                ),
                (
                    "sourceCapability".to_string(),
                    Value::String("dialogue.generate".to_string()),
                ),
                (
                    "generationInstruction".to_string(),
                    Value::String(instruction.to_string()),
                ),
            ]);
            self.commands.lock().await.push(command);
        }
        let mut command = self.command("dialogue.say", speaker_id);
        command.source = "capability.dialogue.generate".to_string();
        command.payload = JsonMap::from([
            ("text".to_string(), Value::String(dialogue.text)),
            ("speakerId".to_string(), Value::String(actor_id)),
            ("emotion".to_string(), Value::String("neutral".to_string())),
            (
                "sourceFunction".to_string(),
                Value::String(GENERATE_FUNCTION_NAME.to_string()),
            ),
            (
                "sourceCapability".to_string(),
                Value::String("dialogue.generate".to_string()),
            ),
            (
                "generationInstruction".to_string(),
                Value::String(instruction.to_string()),
            ),
        ]);
        self.commands.lock().await.push(command);
        Ok(())
    }

    async fn call_function(
        &mut self,
        speaker_id: Option<&str>,
        name: &str,
        positional: Vec<DaihonValue>,
        named: BTreeMap<String, DaihonValue>,
    ) -> std::result::Result<DaihonValue, DaihonRuntimeError> {
        let actor_id = speaker_id.unwrap_or(&self.default_actor_id).to_string();
        match name {
            "表情" | "expression" => {
                let Some(value) = function_value(&positional, &named) else {
                    return Ok(DaihonValue::None);
                };
                let mut command = self.command("avatar.expression", speaker_id);
                command.payload = JsonMap::from([
                    ("expression".to_string(), Value::String(value)),
                    ("speakerId".to_string(), Value::String(actor_id)),
                    (
                        "sourceFunction".to_string(),
                        Value::String(name.to_string()),
                    ),
                ]);
                self.commands.lock().await.push(command);
            }
            "動作" | "モーション" | "motion" => {
                let Some(value) = function_value(&positional, &named) else {
                    return Ok(DaihonValue::None);
                };
                let mut command = self.command("avatar.motion", speaker_id);
                command.payload = JsonMap::from([
                    ("motion".to_string(), Value::String(value)),
                    ("speakerId".to_string(), Value::String(actor_id)),
                    (
                        "sourceFunction".to_string(),
                        Value::String(name.to_string()),
                    ),
                ]);
                self.commands.lock().await.push(command);
            }
            "場所" | "location" => {
                let Some(location) = positional
                    .first()
                    .map(DaihonValue::to_display_string)
                    .filter(|value| !value.trim().is_empty())
                else {
                    return Ok(DaihonValue::None);
                };
                let mut command = self.command("actor.location.set", speaker_id);
                command.payload = JsonMap::from([
                    ("location".to_string(), Value::String(location)),
                    ("speakerId".to_string(), Value::String(actor_id)),
                    (
                        "sourceFunction".to_string(),
                        Value::String(name.to_string()),
                    ),
                ]);
                self.commands.lock().await.push(command);
            }
            "退場" | "exit" => {
                let mut command = self.command("actor.exit", speaker_id);
                command.payload = JsonMap::from([
                    ("speakerId".to_string(), Value::String(actor_id)),
                    (
                        "sourceFunction".to_string(),
                        Value::String(name.to_string()),
                    ),
                ]);
                if let Some(location) = named
                    .get("行き先")
                    .or_else(|| named.get("destination"))
                    .map(DaihonValue::to_display_string)
                    .filter(|value| !value.trim().is_empty())
                {
                    command
                        .payload
                        .insert("location".to_string(), Value::String(location));
                }
                self.commands.lock().await.push(command);
            }
            "登場" | "enter" => {
                let mut command = self.command("actor.enter", speaker_id);
                command.payload = JsonMap::from([
                    ("speakerId".to_string(), Value::String(actor_id)),
                    (
                        "sourceFunction".to_string(),
                        Value::String(name.to_string()),
                    ),
                ]);
                if let Some(location) = named
                    .get("場所")
                    .or_else(|| named.get("location"))
                    .map(DaihonValue::to_display_string)
                    .filter(|value| !value.trim().is_empty())
                {
                    command
                        .payload
                        .insert("location".to_string(), Value::String(location));
                }
                self.commands.lock().await.push(command);
            }
            "歩く" | "walk" => {
                let Some(destination) =
                    positional
                        .first()
                        .and_then(|value| match value.to_display_string().trim() {
                            "右端" | "right-edge" => Some("right-edge"),
                            "左端" | "left-edge" => Some("left-edge"),
                            _ => None,
                        })
                else {
                    return Ok(DaihonValue::None);
                };
                let motion = named
                    .get("動作")
                    .map(DaihonValue::to_display_string)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "walk".to_string());
                let mut command = self.command("stage.walk", speaker_id);
                command.payload = JsonMap::from([
                    (
                        "destination".to_string(),
                        Value::String(destination.to_string()),
                    ),
                    ("motion".to_string(), Value::String(motion)),
                    ("speakerId".to_string(), Value::String(actor_id)),
                    (
                        "sourceFunction".to_string(),
                        Value::String(name.to_string()),
                    ),
                ]);
                if let Some(speed) = named.get("速さ") {
                    let speed = daihon_value_to_json(speed);
                    if speed.is_number() {
                        command.payload.insert("speedPxPerSec".to_string(), speed);
                    }
                }
                self.commands.lock().await.push(command);
            }
            "枠に座る" => {
                let Some(window_key) = function_value(&positional, &named) else {
                    return Ok(DaihonValue::None);
                };
                let mut command = self.command("stage.perch", speaker_id);
                command.payload = JsonMap::from([
                    ("windowKey".to_string(), Value::String(window_key)),
                    ("speakerId".to_string(), Value::String(actor_id)),
                    (
                        "sourceFunction".to_string(),
                        Value::String(name.to_string()),
                    ),
                ]);
                self.commands.lock().await.push(command);
            }
            "枠から降りる" => {
                let mut command = self.command("stage.perch.release", speaker_id);
                command.payload = JsonMap::from([
                    ("speakerId".to_string(), Value::String(actor_id)),
                    (
                        "sourceFunction".to_string(),
                        Value::String(name.to_string()),
                    ),
                ]);
                self.commands.lock().await.push(command);
            }
            _ => {}
        }
        Ok(DaihonValue::None)
    }
}

fn function_value(
    positional: &[DaihonValue],
    named: &BTreeMap<String, DaihonValue>,
) -> Option<String> {
    positional
        .first()
        .or_else(|| named.get("名前"))
        .or_else(|| named.get("name"))
        .map(DaihonValue::to_display_string)
        .filter(|value| !value.trim().is_empty())
}

fn script_accepts_event(script: &Script, event_kind: &str) -> bool {
    let event_kind = canonical_signal_id(event_kind);
    script.event.name.value == event_kind
        || script.event.scenes.iter().any(|scene| {
            scene
                .metadata
                .signals
                .iter()
                .any(|signal| signal.name.value == event_kind)
        })
}

fn canonicalize_daihon_script_signals(script: &mut Script, aliases: &SignalAliasTable) {
    script.event.name.value = aliases.canonicalize(&script.event.name.value);
    for scene in &mut script.event.scenes {
        for signal in &mut scene.metadata.signals {
            signal.name.value = aliases.canonicalize(&signal.name.value);
        }
    }
}

fn canonicalize_daihon_script_speakers(
    script: &mut Script,
    speakers: &BTreeMap<String, String>,
    path: &str,
) -> Result<()> {
    for scene in &mut script.event.scenes {
        if let Some(speaker) = &mut scene.metadata.speaker {
            canonicalize_speaker(speaker, speakers, path)?;
        }
        canonicalize_statement_speakers(&mut scene.statements, speakers, path)?;
    }
    Ok(())
}

fn canonicalize_statement_speakers(
    statements: &mut [Stmt],
    speakers: &BTreeMap<String, String>,
    path: &str,
) -> Result<()> {
    for statement in statements {
        match statement {
            Stmt::SpeakerDisplay { speaker, .. } => {
                canonicalize_speaker(speaker, speakers, path)?;
            }
            Stmt::Conditional(block) => {
                for branch in &mut block.branches {
                    canonicalize_statement_speakers(&mut branch.statements, speakers, path)?;
                }
                if let Some(else_branch) = &mut block.else_branch {
                    canonicalize_statement_speakers(else_branch, speakers, path)?;
                }
            }
            Stmt::Display(_) | Stmt::Assignment(_) | Stmt::Jump(_) => {}
        }
    }
    Ok(())
}

fn canonicalize_speaker(
    speaker: &mut Spanned<String>,
    speakers: &BTreeMap<String, String>,
    path: &str,
) -> Result<()> {
    let key = speaker.value.trim();
    let Some(actor_id) = speakers.get(key) else {
        return Err(WorldError::Daihon(DaihonDiagnosticReport::single(
            DaihonDiagnosticEntry {
                phase: DaihonDiagnosticPhase::LoadSpeaker,
                severity: DaihonDiagnosticSeverity::Error,
                code: "E-YUKEI-DHN-SPEAKER-001".to_string(),
                message: format!(
                    "unknown Daihon speaker in {path} at {}:{}: {}",
                    speaker.span.line, speaker.span.column, speaker.value
                ),
                script_path: Some(path.to_string()),
                line: Some(speaker.span.line),
                column: Some(speaker.span.column),
                help: Some(
                    "話者名をactor IDにするか、pack.jsonのactors[].speakerAliasesへ追加してください。"
                        .to_string(),
                ),
                occurred_at: None,
                install_id: None,
                world_pack_id: None,
                pack_root: None,
                source_event_type: None,
                source_event_id: None,
            },
        )));
    };
    speaker.value = actor_id.clone();
    Ok(())
}

fn event_inputs(event: &RuntimeEvent) -> Vec<(String, DaihonValue)> {
    let mut inputs = vec![
        ("合図".to_string(), DaihonValue::String(event.kind.clone())),
        (
            "イベント種別".to_string(),
            DaihonValue::String(event.kind.clone()),
        ),
        (
            "イベントID".to_string(),
            DaihonValue::String(event.id.clone()),
        ),
        (
            "source".to_string(),
            DaihonValue::String(event.source.clone()),
        ),
    ];

    if let Some(device_id) = &event.device_id {
        inputs.push((
            "deviceId".to_string(),
            DaihonValue::String(device_id.clone()),
        ));
    }
    if let Some(surface_id) = &event.surface_id {
        inputs.push((
            "surfaceId".to_string(),
            DaihonValue::String(surface_id.clone()),
        ));
    }
    if let Some(actor_id) = &event.actor_id {
        inputs.push(("actorId".to_string(), DaihonValue::String(actor_id.clone())));
    }

    for (key, value) in &event.payload {
        if let Some(value) = json_to_daihon_value(value) {
            inputs.push((key.clone(), value.clone()));
            inputs.push((format!("payload.{key}"), value));
        }
    }
    if let Some(text) = event.payload.get("text").and_then(Value::as_str) {
        inputs.push((
            "ユーザー発言".to_string(),
            DaihonValue::String(text.to_string()),
        ));
    }
    if let Some(hour) = event.payload.get("localHour").and_then(Value::as_i64) {
        inputs.push((
            "現在時".to_string(),
            DaihonValue::Number(DaihonNumber::Integer(hour)),
        ));
    }
    if let Some(minute) = event.payload.get("localMinute").and_then(Value::as_i64) {
        inputs.push((
            "現在分".to_string(),
            DaihonValue::Number(DaihonNumber::Integer(minute)),
        ));
    }
    if let Some(period) = event.payload.get("timePeriod").and_then(Value::as_str) {
        inputs.push((
            "時間帯".to_string(),
            DaihonValue::String(period.to_string()),
        ));
    }
    if let Some(idle_minutes) = event.payload.get("idleMinutes").and_then(Value::as_i64) {
        inputs.push((
            "不在分".to_string(),
            DaihonValue::Number(DaihonNumber::Integer(idle_minutes)),
        ));
    }
    if event.kind == "stage.walk.ended" {
        let reason = match event.payload.get("reason").and_then(Value::as_str) {
            Some("arrived") => Some("到着"),
            Some("user-drag" | "replaced") => Some("中断"),
            _ => None,
        };
        if let Some(reason) = reason {
            inputs.push(("理由".to_string(), DaihonValue::String(reason.to_string())));
        }
    }
    for (payload_key, input_name) in [
        ("app", "アプリ"),
        ("windowKey", "窓ID"),
        ("category", "フォルダ"),
        ("fileName", "ファイル名"),
        ("fileCategory", "ファイル種類"),
        ("recentDownloadFileName", "最近のダウンロード"),
        ("recentDownloadCategory", "最近のダウンロード種類"),
        ("movedDistance", "移動距離"),
        ("actorLocation", "場所"),
        ("actorPresence", "在席"),
        ("aiConnected", "AI接続"),
    ] {
        if let Some(value) = event
            .payload
            .get(payload_key)
            .and_then(json_to_daihon_value)
        {
            inputs.push((input_name.to_string(), value));
        }
    }
    inputs
}

fn json_to_daihon_value(value: &Value) -> Option<DaihonValue> {
    match value {
        Value::Bool(value) => Some(DaihonValue::Boolean(*value)),
        Value::Number(value) => value
            .as_i64()
            .map(|value| DaihonValue::Number(DaihonNumber::Integer(value)))
            .or_else(|| {
                value
                    .as_f64()
                    .map(|value| DaihonValue::Number(DaihonNumber::Float(value)))
            }),
        Value::String(value) => Some(DaihonValue::String(value.clone())),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn daihon_value_to_json(value: &DaihonValue) -> Value {
    match value {
        DaihonValue::None => Value::Null,
        DaihonValue::Number(DaihonNumber::Integer(value)) => Value::Number(Number::from(*value)),
        DaihonValue::Number(DaihonNumber::Float(value)) => Number::from_f64(*value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        DaihonValue::String(value) => Value::String(value.clone()),
        DaihonValue::Boolean(value) => Value::Bool(*value),
    }
}

fn diff_variable_patches(
    previous: &BTreeMap<String, DaihonValue>,
    next: &BTreeMap<String, DaihonValue>,
) -> Vec<Value> {
    let mut patches = Vec::new();
    for (key, value) in next {
        if previous.get(key) != Some(value) {
            let mut patch = Map::new();
            patch.insert("op".to_string(), Value::String("set".to_string()));
            patch.insert("path".to_string(), Value::String(key.clone()));
            patch.insert("value".to_string(), daihon_value_to_json(value));
            patches.push(Value::Object(patch));
        }
    }
    for key in previous.keys().filter(|key| !next.contains_key(*key)) {
        let mut patch = Map::new();
        patch.insert("op".to_string(), Value::String("remove".to_string()));
        patch.insert("path".to_string(), Value::String(key.clone()));
        patches.push(Value::Object(patch));
    }
    patches
}

fn yuukei_function_registry() -> FunctionRegistry {
    let mut registry = FunctionRegistry::new();
    for name in ["表情", "expression"] {
        registry.register(FunctionSpec {
            name: name.to_string(),
            positional: vec![ParamSpec {
                name: Some("名前".to_string()),
                ty: ParamType::String,
                required: true,
            }],
            named: BTreeMap::new(),
            return_type: None,
        });
    }
    for name in ["動作", "モーション", "motion"] {
        registry.register(FunctionSpec {
            name: name.to_string(),
            positional: vec![ParamSpec {
                name: Some("名前".to_string()),
                ty: ParamType::String,
                required: true,
            }],
            named: BTreeMap::new(),
            return_type: None,
        });
    }
    for name in ["場所", "location"] {
        registry.register(FunctionSpec {
            name: name.to_string(),
            positional: vec![ParamSpec {
                name: Some("場所".to_string()),
                ty: ParamType::String,
                required: true,
            }],
            named: BTreeMap::new(),
            return_type: None,
        });
    }
    registry.register(FunctionSpec {
        name: "退場".to_string(),
        positional: Vec::new(),
        named: BTreeMap::from([(
            "行き先".to_string(),
            ParamSpec {
                name: Some("行き先".to_string()),
                ty: ParamType::String,
                required: false,
            },
        )]),
        return_type: None,
    });
    registry.register(FunctionSpec {
        name: "exit".to_string(),
        positional: Vec::new(),
        named: BTreeMap::from([(
            "destination".to_string(),
            ParamSpec {
                name: Some("destination".to_string()),
                ty: ParamType::String,
                required: false,
            },
        )]),
        return_type: None,
    });
    registry.register(FunctionSpec {
        name: "登場".to_string(),
        positional: Vec::new(),
        named: BTreeMap::from([(
            "場所".to_string(),
            ParamSpec {
                name: Some("場所".to_string()),
                ty: ParamType::String,
                required: false,
            },
        )]),
        return_type: None,
    });
    registry.register(FunctionSpec {
        name: "enter".to_string(),
        positional: Vec::new(),
        named: BTreeMap::from([(
            "location".to_string(),
            ParamSpec {
                name: Some("location".to_string()),
                ty: ParamType::String,
                required: false,
            },
        )]),
        return_type: None,
    });
    for name in ["歩く", "walk"] {
        registry.register(FunctionSpec {
            name: name.to_string(),
            positional: vec![ParamSpec {
                name: Some("行き先".to_string()),
                ty: ParamType::String,
                required: false,
            }],
            named: BTreeMap::from([
                (
                    "速さ".to_string(),
                    ParamSpec {
                        name: Some("速さ".to_string()),
                        ty: ParamType::Number,
                        required: false,
                    },
                ),
                (
                    "動作".to_string(),
                    ParamSpec {
                        name: Some("動作".to_string()),
                        ty: ParamType::String,
                        required: false,
                    },
                ),
            ]),
            return_type: None,
        });
    }
    registry.register(FunctionSpec {
        name: "枠に座る".to_string(),
        positional: vec![ParamSpec {
            name: Some("窓ID".to_string()),
            ty: ParamType::Any,
            required: true,
        }],
        named: BTreeMap::new(),
        return_type: None,
    });
    registry.register(FunctionSpec {
        name: "枠から降りる".to_string(),
        positional: Vec::new(),
        named: BTreeMap::new(),
        return_type: None,
    });
    registry.register(FunctionSpec {
        name: INTERPRET_FUNCTION_NAME.to_string(),
        positional: vec![
            ParamSpec {
                name: Some("入力".to_string()),
                ty: ParamType::Any,
                required: true,
            },
            ParamSpec {
                name: Some("質問".to_string()),
                ty: ParamType::String,
                required: true,
            },
            ParamSpec {
                name: Some("選択肢".to_string()),
                ty: ParamType::String,
                required: true,
            },
        ],
        named: BTreeMap::new(),
        return_type: Some(yuukei_daihon::ValueType::String),
    });
    registry.register(FunctionSpec {
        name: CHOICE_FUNCTION_NAME.to_string(),
        positional: (0..6)
            .map(|index| ParamSpec {
                name: Some(format!("選択肢{}", index + 1)),
                ty: ParamType::String,
                required: index == 0,
            })
            .collect(),
        named: BTreeMap::from([(
            "秒数".to_string(),
            ParamSpec {
                name: Some("秒数".to_string()),
                ty: ParamType::Number,
                required: false,
            },
        )]),
        return_type: Some(yuukei_daihon::ValueType::String),
    });
    registry.register(FunctionSpec {
        name: EXTRACT_FUNCTION_NAME.to_string(),
        positional: vec![
            ParamSpec {
                name: Some("入力".to_string()),
                ty: ParamType::Any,
                required: true,
            },
            ParamSpec {
                name: Some("指示".to_string()),
                ty: ParamType::String,
                required: true,
            },
        ],
        named: BTreeMap::new(),
        return_type: Some(yuukei_daihon::ValueType::String),
    });
    registry.register(FunctionSpec {
        name: GENERATE_FUNCTION_NAME.to_string(),
        positional: vec![
            ParamSpec {
                name: Some("指示".to_string()),
                ty: ParamType::String,
                required: true,
            },
            ParamSpec {
                name: Some("フォールバック".to_string()),
                ty: ParamType::String,
                required: false,
            },
        ],
        named: BTreeMap::new(),
        return_type: None,
    });
    registry
}

fn diagnostic_report(
    diagnostics: &[DaihonDiagnostic],
    phase: DaihonDiagnosticPhase,
    script_path: Option<&str>,
    source_event: Option<&RuntimeEvent>,
) -> DaihonDiagnosticReport {
    DaihonDiagnosticReport::new(
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic_entry(diagnostic, phase, script_path, source_event))
            .collect(),
    )
}

fn diagnostic_entry(
    diagnostic: &DaihonDiagnostic,
    phase: DaihonDiagnosticPhase,
    script_path: Option<&str>,
    source_event: Option<&RuntimeEvent>,
) -> DaihonDiagnosticEntry {
    let location = diagnostic.labels.first().map(|label| label.span);
    DaihonDiagnosticEntry {
        phase,
        severity: match &diagnostic.severity {
            DaihonSeverity::Error => DaihonDiagnosticSeverity::Error,
            DaihonSeverity::Warning => DaihonDiagnosticSeverity::Warning,
            DaihonSeverity::Info => DaihonDiagnosticSeverity::Info,
        },
        code: diagnostic.code.clone(),
        message: diagnostic.message.clone(),
        script_path: script_path.map(ToOwned::to_owned),
        line: location.map(|span| span.line),
        column: location.map(|span| span.column),
        help: diagnostic.help.clone(),
        occurred_at: None,
        install_id: None,
        world_pack_id: None,
        pack_root: None,
        source_event_type: source_event.map(|event| event.kind.clone()),
        source_event_id: source_event.map(|event| event.id.clone()),
    }
}

impl WorldPack {
    pub fn load_from_dir(path: impl AsRef<Path>) -> Result<Self> {
        let root = fs::canonicalize(path.as_ref())?;
        if !root.is_dir() {
            return Err(WorldError::Validation(format!(
                "world pack root must be a directory: {}",
                root.display()
            )));
        }
        let pack_path = root.join("pack.json");
        let raw = fs::read_to_string(pack_path)?;
        let mut pack: Self = serde_json::from_str(&raw)?;
        pack.daihon.loaded_scripts = pack.load_daihon_scripts(&root)?;
        pack.validate_renderer_assets(&root)?;
        pack.validate()?;
        Ok(pack)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(WorldError::Validation(format!(
                "unsupported schemaVersion: {}",
                self.schema_version
            )));
        }
        require_non_empty("id", &self.id)?;
        require_non_empty("defaultActorId", &self.default_actor_id)?;

        let mut actor_ids = BTreeSet::new();
        for actor in &self.actors {
            require_non_empty("actor.id", &actor.id)?;
            require_non_empty("actor.displayName", &actor.display_name)?;
            if !actor_ids.insert(actor.id.clone()) {
                return Err(WorldError::Validation(format!(
                    "duplicate actor id: {}",
                    actor.id
                )));
            }
            if let Some(renderer) = &actor.renderer {
                require_non_empty("actor.renderer.model", &renderer.model)?;
                for (motion_id, motion_path) in &renderer.motions {
                    require_non_empty("actor.renderer.motions key", motion_id)?;
                    require_non_empty("actor.renderer.motions value", motion_path)?;
                }
                validate_hit_zones(&actor.id, &renderer.hit_zones)?;
            }
        }
        self.validate_speaker_aliases(&actor_ids)?;
        if !actor_ids.contains(&self.default_actor_id) {
            return Err(WorldError::Validation(format!(
                "defaultActorId is not declared: {}",
                self.default_actor_id
            )));
        }

        let mut signals = BTreeSet::new();
        for signal in &self.signals.allow {
            require_non_empty("signals.allow", signal)?;
            let canonical = canonical_signal_id(signal).to_string();
            if !signals.insert(canonical.clone()) {
                return Err(WorldError::Validation(format!(
                    "duplicate signal after canonicalization: {} ({canonical})",
                    signal
                )));
            }
        }
        let mut delegated_signals = BTreeSet::new();
        for signal in &self.llm_delegation.signals {
            require_non_empty("llmDelegation.signals.signal", &signal.signal)?;
            let canonical = canonical_signal_id(&signal.signal).to_string();
            if !delegated_signals.insert(canonical.clone()) {
                return Err(WorldError::Validation(format!(
                    "duplicate llmDelegation signal after canonicalization: {} ({canonical})",
                    signal.signal
                )));
            }
        }

        Ok(())
    }

    fn validate_speaker_aliases(&self, actor_ids: &BTreeSet<String>) -> Result<()> {
        let mut aliases = BTreeMap::new();
        for actor in &self.actors {
            for alias in &actor.speaker_aliases {
                require_non_empty("actor.speakerAliases", alias)?;
                let alias = alias.trim();
                if actor_ids.contains(alias) {
                    return Err(WorldError::Validation(format!(
                        "speaker alias collides with actor id: {alias}"
                    )));
                }
                if let Some(existing_actor_id) = aliases.insert(alias.to_string(), actor.id.clone())
                {
                    return Err(WorldError::Validation(format!(
                        "duplicate speaker alias: {alias} ({existing_actor_id}, {})",
                        actor.id
                    )));
                }
            }
        }
        Ok(())
    }

    fn speaker_resolution_table(&self) -> Result<BTreeMap<String, String>> {
        let mut table = BTreeMap::new();
        for actor in &self.actors {
            require_non_empty("actor.id", &actor.id)?;
            table.insert(actor.id.trim().to_string(), actor.id.clone());
        }
        for actor in &self.actors {
            for alias in &actor.speaker_aliases {
                require_non_empty("actor.speakerAliases", alias)?;
                table.insert(alias.trim().to_string(), actor.id.clone());
            }
        }
        Ok(table)
    }

    pub fn allows_signal(&self, signal: &str) -> bool {
        self.allows_signal_with_aliases(signal, &SignalAliasTable::default())
    }

    pub fn allows_signal_with_aliases(&self, signal: &str, aliases: &SignalAliasTable) -> bool {
        let signal = aliases.canonicalize(signal);
        self.signals
            .allow
            .iter()
            .any(|allowed| aliases.canonicalize(allowed) == signal)
    }

    pub fn llm_delegation_for_signal_with_aliases(
        &self,
        signal: &str,
        aliases: &SignalAliasTable,
    ) -> Option<&LlmDelegationSignal> {
        let signal = aliases.canonicalize(signal);
        self.llm_delegation
            .signals
            .iter()
            .find(|delegation| aliases.canonicalize(&delegation.signal) == signal)
    }

    pub fn actor_map(&self) -> BTreeMap<String, ActorDefinition> {
        self.actors
            .iter()
            .map(|actor| (actor.id.clone(), actor.clone()))
            .collect()
    }

    fn load_daihon_scripts(&self, root: &Path) -> Result<Vec<DaihonScriptSource>> {
        let mut sources = Vec::new();
        for script in &self.daihon.scripts {
            let path = resolve_pack_relative_path(root, script)?;
            sources.push(DaihonScriptSource {
                path: script.clone(),
                source: fs::read_to_string(path)?,
            });
        }
        Ok(sources)
    }

    fn validate_renderer_assets(&self, root: &Path) -> Result<()> {
        for actor in &self.actors {
            let Some(renderer) = &actor.renderer else {
                continue;
            };
            resolve_pack_relative_path(root, &renderer.model)?;
            for motion in renderer.motions.values() {
                resolve_pack_relative_path(root, motion)?;
            }
        }
        Ok(())
    }
}

fn validate_hit_zones(actor_id: &str, hit_zones: &[ActorHitZoneDefinition]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for hit_zone in hit_zones {
        require_non_empty("actor.renderer.hitZones.id", &hit_zone.id)?;
        if !ids.insert(hit_zone.id.clone()) {
            return Err(WorldError::Validation(format!(
                "duplicate hitZone id for actor {actor_id}: {}",
                hit_zone.id
            )));
        }

        match hit_zone.source {
            ActorHitZoneSource::HumanoidBone => {
                require_non_empty_list(
                    "actor.renderer.hitZones.bones",
                    &hit_zone.bones,
                    &hit_zone.id,
                )?;
            }
            ActorHitZoneSource::NodeName => {
                require_non_empty_list(
                    "actor.renderer.hitZones.nodes",
                    &hit_zone.nodes,
                    &hit_zone.id,
                )?;
            }
        }

        for event in &hit_zone.events {
            require_non_empty("actor.renderer.hitZones.events", event)?;
        }
    }
    Ok(())
}

fn require_non_empty_list(field: &str, values: &[String], hit_zone_id: &str) -> Result<()> {
    if values.is_empty() {
        return Err(WorldError::Validation(format!(
            "{field} must not be empty for hitZone {hit_zone_id}"
        )));
    }
    for value in values {
        require_non_empty(field, value)?;
    }
    Ok(())
}

fn require_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(WorldError::Validation(format!("{field} must not be empty")));
    }
    Ok(())
}

pub fn resolve_pack_relative_path(root: &Path, path: &str) -> Result<PathBuf> {
    let relative = Path::new(path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(WorldError::Validation(format!(
            "pack path must stay inside pack root: {path}"
        )));
    }
    let resolved = fs::canonicalize(root.join(relative))?;
    if !resolved.starts_with(root) {
        return Err(WorldError::Validation(format!(
            "pack path must stay inside pack root: {path}"
        )));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;
    use yuukei_protocol::{RuntimeEvent, SignalAliasTable};

    use super::*;

    fn pack() -> WorldPack {
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
                allow: vec!["conversation.text".to_string()],
            },
            capabilities: CapabilityDeclarations {
                required: Vec::new(),
                optional: vec!["speech.synthesis".to_string()],
            },
            llm_delegation: LlmDelegation::default(),
            daihon: daihon_config(),
            initial_variables: JsonMap::new(),
            ui_space: JsonMap::new(),
        }
    }

    fn daihon_config() -> DaihonConfig {
        DaihonConfig {
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
        }
    }

    fn world_with_script(source: &str) -> WorldPack {
        let mut world = pack();
        world.signals.allow = vec!["presence.talk_impulse".to_string()];
        world.daihon = DaihonConfig {
            scripts: vec!["scripts/random.daihon".to_string()],
            loaded_scripts: vec![DaihonScriptSource {
                path: "scripts/random.daihon".to_string(),
                source: source.to_string(),
            }],
        };
        world
    }

    fn talk_impulse_event(id: &str) -> RuntimeEvent {
        let mut event = RuntimeEvent::new("presence.talk_impulse", "device", "resident-default");
        event.id = id.to_string();
        event
    }

    async fn dispatch_with_history(
        world: &WorldPack,
        history_path: &Path,
        event_id: &str,
    ) -> Result<DaihonDispatchResult> {
        let adapter = YuukeiDaihonAdapter::with_persistent_scene_history(history_path);
        adapter.load_world(world).await?;
        adapter.dispatch(&talk_impulse_event(event_id), world).await
    }

    async fn dispatch_with_state(
        world: &WorldPack,
        history_path: &Path,
        variables_path: &Path,
        event: RuntimeEvent,
    ) -> Result<DaihonDispatchResult> {
        let adapter = YuukeiDaihonAdapter::with_persistent_state(history_path, variables_path);
        adapter.load_world(world).await?;
        adapter.dispatch(&event, world).await
    }

    #[test]
    fn validates_default_actor_reference() {
        assert!(pack().validate().is_ok());
        let mut invalid = pack();
        invalid.default_actor_id = "unknown".to_string();
        assert!(matches!(invalid.validate(), Err(WorldError::Validation(_))));
    }

    #[test]
    fn validates_signal_allowlist_after_alias_canonicalization() {
        let mut world = pack();
        world.signals.allow = vec!["会話_入力".to_string(), "conversation.text".to_string()];
        let error = world.validate().expect_err("duplicate canonical signal");
        assert!(error.to_string().contains("conversation.text"));
    }

    #[test]
    fn validates_llm_delegation_after_alias_canonicalization() {
        let mut world = pack();
        world.llm_delegation.signals = vec![
            LlmDelegationSignal {
                signal: "会話_入力".to_string(),
                cooldown_seconds: Some(60),
            },
            LlmDelegationSignal {
                signal: "conversation.text".to_string(),
                cooldown_seconds: None,
            },
        ];
        let error = world.validate().expect_err("duplicate delegation signal");
        assert!(error.to_string().contains("conversation.text"));
    }

    #[test]
    fn validates_speaker_aliases() {
        let mut world = pack();
        world.actors[0].speaker_aliases = vec!["ゆ".to_string()];
        assert!(world.validate().is_ok());
    }

    #[test]
    fn rejects_empty_speaker_alias() {
        let mut world = pack();
        world.actors[0].speaker_aliases = vec![" ".to_string()];
        let error = world.validate().expect_err("empty alias should fail");
        assert!(error.to_string().contains("actor.speakerAliases"));
    }

    #[test]
    fn rejects_speaker_alias_that_collides_with_actor_id() {
        let mut world = pack();
        world.actors.push(ActorDefinition {
            id: "partner".to_string(),
            display_name: "Partner".to_string(),
            speaker_aliases: Vec::new(),
            profile: JsonMap::new(),
            renderer: None,
        });
        world.actors[0].speaker_aliases = vec!["partner".to_string()];
        let error = world
            .validate()
            .expect_err("actor id collision should fail");
        assert!(error.to_string().contains("collides with actor id"));
    }

    #[test]
    fn rejects_duplicate_speaker_aliases() {
        let mut world = pack();
        world.actors.push(ActorDefinition {
            id: "partner".to_string(),
            display_name: "Partner".to_string(),
            speaker_aliases: vec!["ゆ".to_string()],
            profile: JsonMap::new(),
            renderer: None,
        });
        world.actors[0].speaker_aliases = vec!["ゆ".to_string()];
        let error = world.validate().expect_err("duplicate alias should fail");
        assert!(error.to_string().contains("duplicate speaker alias"));
    }

    #[test]
    fn allows_signal_accepts_standard_aliases() {
        let mut world = pack();
        world.signals.allow = vec!["会話_入力".to_string()];
        assert!(world.allows_signal("conversation.text"));
        assert!(world.allows_signal("会話_入力"));
        assert!(!world.allows_signal("surface.attach"));
    }

    #[test]
    fn llm_delegation_accepts_standard_aliases() {
        let mut world = pack();
        world.llm_delegation.signals = vec![LlmDelegationSignal {
            signal: "会話_入力".to_string(),
            cooldown_seconds: Some(60),
        }];
        assert!(world
            .llm_delegation_for_signal_with_aliases(
                "conversation.text",
                &SignalAliasTable::default()
            )
            .is_some());
        assert!(world
            .llm_delegation_for_signal_with_aliases("surface.attach", &SignalAliasTable::default())
            .is_none());
    }

    #[test]
    fn rejects_pack_path_traversal() -> Result<()> {
        let dir = tempdir()?;
        let mut invalid = pack();
        invalid.daihon.scripts = vec!["../escape.daihon".to_string()];
        let raw = serde_json::to_string(&invalid)?;
        fs::write(dir.path().join("pack.json"), raw)?;
        assert!(matches!(
            WorldPack::load_from_dir(dir.path()),
            Err(WorldError::Validation(_))
        ));
        Ok(())
    }

    #[test]
    fn load_from_dir_validates_renderer_asset_paths() -> Result<()> {
        let dir = tempdir()?;
        fs::create_dir_all(dir.path().join("scripts"))?;
        fs::create_dir_all(dir.path().join("character"))?;
        fs::create_dir_all(dir.path().join("motion"))?;
        fs::write(
            dir.path().join("scripts").join("reactions.daihon"),
            &pack().daihon.loaded_scripts[0].source,
        )?;
        fs::write(dir.path().join("character").join("character_1.vrm"), [])?;
        fs::write(dir.path().join("motion").join("walk.vrma"), [])?;
        let mut raw_pack = pack();
        raw_pack.actors[0].renderer = Some(ActorRendererDefinition {
            kind: ActorRendererKind::Vrm,
            model: "character/character_1.vrm".to_string(),
            motions: BTreeMap::from([("walk".to_string(), "motion/walk.vrma".to_string())]),
            hit_zones: Vec::new(),
        });
        raw_pack.daihon.loaded_scripts.clear();
        fs::write(
            dir.path().join("pack.json"),
            serde_json::to_string(&raw_pack)?,
        )?;

        let loaded = WorldPack::load_from_dir(dir.path())?;
        assert_eq!(
            loaded.actors[0]
                .renderer
                .as_ref()
                .map(|renderer| renderer.model.as_str()),
            Some("character/character_1.vrm")
        );
        Ok(())
    }

    #[test]
    fn rejects_renderer_asset_path_traversal() -> Result<()> {
        let dir = tempdir()?;
        fs::create_dir_all(dir.path().join("scripts"))?;
        fs::write(
            dir.path().join("scripts").join("reactions.daihon"),
            &pack().daihon.loaded_scripts[0].source,
        )?;
        let mut invalid = pack();
        invalid.actors[0].renderer = Some(ActorRendererDefinition {
            kind: ActorRendererKind::Vrm,
            model: "../escape.vrm".to_string(),
            motions: BTreeMap::new(),
            hit_zones: Vec::new(),
        });
        fs::write(
            dir.path().join("pack.json"),
            serde_json::to_string(&invalid)?,
        )?;
        assert!(matches!(
            WorldPack::load_from_dir(dir.path()),
            Err(WorldError::Validation(_))
        ));
        Ok(())
    }

    #[test]
    fn rejects_missing_pack_manifest() -> Result<()> {
        let dir = tempdir()?;
        assert!(matches!(
            WorldPack::load_from_dir(dir.path()),
            Err(WorldError::Io(_))
        ));
        Ok(())
    }

    #[test]
    fn rejects_unsupported_schema_version() -> Result<()> {
        let dir = tempdir()?;
        fs::create_dir_all(dir.path().join("scripts"))?;
        fs::write(
            dir.path().join("scripts").join("reactions.daihon"),
            &pack().daihon.loaded_scripts[0].source,
        )?;
        let mut invalid = pack();
        invalid.schema_version = 2;
        fs::write(
            dir.path().join("pack.json"),
            serde_json::to_string(&invalid)?,
        )?;
        assert!(matches!(
            WorldPack::load_from_dir(dir.path()),
            Err(WorldError::Validation(_))
        ));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape_for_daihon_scripts() -> Result<()> {
        let dir = tempdir()?;
        let outside = tempdir()?;
        fs::write(
            outside.path().join("escape.daihon"),
            &pack().daihon.loaded_scripts[0].source,
        )?;
        std::os::unix::fs::symlink(outside.path(), dir.path().join("scripts"))?;
        let mut invalid = pack();
        invalid.daihon.scripts = vec!["scripts/escape.daihon".to_string()];
        fs::write(
            dir.path().join("pack.json"),
            serde_json::to_string(&invalid)?,
        )?;
        assert!(matches!(
            WorldPack::load_from_dir(dir.path()),
            Err(WorldError::Validation(_))
        ));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape_for_renderer_assets() -> Result<()> {
        let dir = tempdir()?;
        let outside = tempdir()?;
        fs::create_dir_all(dir.path().join("scripts"))?;
        fs::write(
            dir.path().join("scripts").join("reactions.daihon"),
            &pack().daihon.loaded_scripts[0].source,
        )?;
        fs::write(outside.path().join("character_1.vrm"), [])?;
        std::os::unix::fs::symlink(outside.path(), dir.path().join("character"))?;
        let mut invalid = pack();
        invalid.actors[0].renderer = Some(ActorRendererDefinition {
            kind: ActorRendererKind::Vrm,
            model: "character/character_1.vrm".to_string(),
            motions: BTreeMap::new(),
            hit_zones: Vec::new(),
        });
        fs::write(
            dir.path().join("pack.json"),
            serde_json::to_string(&invalid)?,
        )?;
        assert!(matches!(
            WorldPack::load_from_dir(dir.path()),
            Err(WorldError::Validation(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn load_from_dir_reads_daihon_script_sources() -> Result<()> {
        let dir = tempdir()?;
        fs::create_dir_all(dir.path().join("scripts"))?;
        fs::write(
            dir.path().join("scripts").join("reactions.daihon"),
            &pack().daihon.loaded_scripts[0].source,
        )?;
        let mut raw_pack = pack();
        raw_pack.daihon.loaded_scripts.clear();
        let raw = serde_json::to_string(&raw_pack)?;
        fs::write(dir.path().join("pack.json"), raw)?;

        let loaded = WorldPack::load_from_dir(dir.path())?;
        assert_eq!(loaded.daihon.loaded_scripts.len(), 1);
        assert!(loaded.daihon.loaded_scripts[0]
            .source
            .contains("desktop reactions"));
        Ok(())
    }

    #[test]
    fn demo_interpret_pack_loads_from_repo() -> Result<()> {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let loaded = WorldPack::load_from_dir(repo_root.join("packs").join("demo-interpret"))?;
        assert_eq!(loaded.id, "demo-interpret");
        assert_eq!(loaded.daihon.loaded_scripts.len(), 1);
        Ok(())
    }

    #[test]
    fn default_pack_all_authored_daihon_signals_are_allowlisted() -> Result<()> {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repo root");
        let world = WorldPack::load_from_dir(repo_root.join("packs").join("default-yuukei"))?;
        let allowed = world
            .signals
            .allow
            .iter()
            .map(|signal| canonical_signal_id(signal).to_string())
            .collect::<BTreeSet<_>>();
        let mut missing = Vec::new();

        for source in &world.daihon.loaded_scripts {
            for script in parse_scripts(&source.source).expect("default pack script parses") {
                for scene in &script.event.scenes {
                    let signals = if scene.metadata.signals.is_empty() {
                        vec![script.event.name.value.as_str()]
                    } else {
                        scene
                            .metadata
                            .signals
                            .iter()
                            .map(|signal| signal.name.value.as_str())
                            .collect()
                    };
                    for signal in signals {
                        let canonical = canonical_signal_id(signal).to_string();
                        if !allowed.contains(&canonical) {
                            missing.push(format!(
                                "{}:{} uses {signal} ({canonical})",
                                source.path, scene.name.value
                            ));
                        }
                    }
                }
            }
        }

        assert!(
            missing.is_empty(),
            "default pack Daihon signals missing from signals.allow: {}",
            missing.join(", ")
        );
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_runs_loaded_daihon_script_for_allowed_text() -> Result<()> {
        let world = pack();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
        event.id = "evt_1".to_string();
        event
            .payload
            .insert("text".to_string(), json!("こんにちは"));

        let result = adapter.dispatch(&event, &world).await?;
        assert_eq!(result.commands.len(), 2);
        assert_eq!(result.commands[0].kind, "avatar.expression");
        assert_eq!(result.commands[0].payload["expression"], "笑顔");
        assert_eq!(result.commands[1].kind, "dialogue.say");
        assert_eq!(
            result.commands[1].payload["text"],
            "聞こえています。こんにちは"
        );
        assert_eq!(result.executed_scenes[0].scene_name, "conversation");
        assert!(!result.variable_patches.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_resolves_daihon_signal_aliases() -> Result<()> {
        let mut world = pack();
        world.daihon.loaded_scripts[0].source = r#"
## desktop reactions
### conversation
合図: ＠会話_入力
話者: yuukei
ユーザー発言=入力#ユーザー発言
「日本語合図で聞こえています。＜ユーザー発言＞」
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
        event.id = "evt_alias".to_string();
        event
            .payload
            .insert("text".to_string(), json!("こんにちは"));

        let result = adapter.dispatch(&event, &world).await?;
        assert_eq!(result.commands.len(), 1);
        assert_eq!(
            result.commands[0].payload["text"],
            "日本語合図で聞こえています。こんにちは"
        );
        assert_eq!(result.executed_scenes[0].scene_name, "conversation");
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_resolves_top_level_daihon_signal_aliases() -> Result<()> {
        let mut world = pack();
        world.daihon.loaded_scripts[0].source = r#"
## 会話_入力
### conversation
話者: yuukei
「top-level alias」
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
        event.id = "evt_top_level_alias".to_string();

        let result = adapter.dispatch(&event, &world).await?;
        assert_eq!(result.commands.len(), 1);
        assert_eq!(result.commands[0].payload["text"], "top-level alias");
        assert_eq!(result.executed_scenes[0].event_name, "conversation.text");
        assert_eq!(result.executed_scenes[0].scene_name, "conversation");
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_dispatches_conditioned_scene_by_implicit_event_signal() -> Result<()> {
        let mut world = pack();
        world.signals.allow = vec!["desktop.folder.opened".to_string()];
        world.daihon.loaded_scripts[0].source = r#"
## フォルダ_開いた
### downloads
条件:（入力#フォルダ = 「downloads」）
話者: yuukei
「ダウンロードを見ています」
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut event = RuntimeEvent::new("desktop.folder.opened", "device", "resident-default");
        event.id = "evt_folder_implicit".to_string();
        event
            .payload
            .insert("category".to_string(), json!("downloads"));

        let result = adapter.dispatch(&event, &world).await?;

        assert_eq!(result.commands.len(), 1);
        assert_eq!(
            result.commands[0].payload["text"],
            "ダウンロードを見ています"
        );
        assert_eq!(
            result.executed_scenes[0].event_name,
            "desktop.folder.opened"
        );
        assert_eq!(result.executed_scenes[0].scene_name, "downloads");
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_does_not_dispatch_implicit_scene_for_mismatched_event() -> Result<()> {
        let mut world = pack();
        world.signals.allow = vec!["desktop.folder.opened".to_string()];
        world.daihon.loaded_scripts[0].source = r#"
## 会話_入力
### downloads
条件:（入力#フォルダ = 「downloads」）
話者: yuukei
「これは出ない」
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut event = RuntimeEvent::new("desktop.folder.opened", "device", "resident-default");
        event.id = "evt_folder_mismatch".to_string();
        event
            .payload
            .insert("category".to_string(), json!("downloads"));

        let result = adapter.dispatch(&event, &world).await?;

        assert!(result.commands.is_empty());
        assert!(result.executed_scenes.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_loads_multiple_events_from_one_daihon_file() -> Result<()> {
        let mut world = pack();
        world.signals.allow = vec![
            "presence.idle.start".to_string(),
            "presence.idle.end".to_string(),
        ];
        world.daihon.loaded_scripts[0].source = r#"
## 不在_開始
### idle start
話者: yuukei
「いってらっしゃい」

## 復帰
### idle end
話者: yuukei
「おかえり」
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;

        let mut start_event =
            RuntimeEvent::new("presence.idle.start", "device", "resident-default");
        start_event.id = "evt_idle_start_multi".to_string();
        let start = adapter.dispatch(&start_event, &world).await?;

        let mut end_event = RuntimeEvent::new("presence.idle.end", "device", "resident-default");
        end_event.id = "evt_idle_end_multi".to_string();
        let end = adapter.dispatch(&end_event, &world).await?;

        assert_eq!(start.commands[0].payload["text"], "いってらっしゃい");
        assert_eq!(start.executed_scenes[0].event_name, "presence.idle.start");
        assert_eq!(end.commands[0].payload["text"], "おかえり");
        assert_eq!(end.executed_scenes[0].event_name, "presence.idle.end");
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_dispatches_gesture_scene_by_hit_surface_input() -> Result<()> {
        let mut world = pack();
        world.signals.allow = vec!["avatar.gesture.poke".to_string()];
        world.daihon.loaded_scripts[0].source = r#"
## desktop reactions
### cloth poke
合図: ＠avatar.gesture.poke
条件:（入力#hitSurface = 「cloth」）
話者: yuukei
「服だよ」

### skin poke
合図: ＠avatar.gesture.poke
条件:（入力#hitSurface = 「skin」）
話者: yuukei
「肌だよ」
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut event = RuntimeEvent::new("avatar.gesture.poke", "surface", "resident-default");
        event.id = "evt_hit_surface".to_string();
        event
            .payload
            .insert("hitSurface".to_string(), json!("cloth"));

        let result = adapter.dispatch(&event, &world).await?;
        assert_eq!(result.commands.len(), 1);
        assert_eq!(result.commands[0].payload["text"], "服だよ");
        assert_eq!(result.executed_scenes[0].scene_name, "cloth poke");
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_dispatches_drop_distance_with_friendly_input_name() -> Result<()> {
        let mut world = pack();
        world.signals.allow = vec!["avatar.gesture.drop".to_string()];
        world.daihon.loaded_scripts[0].source = r#"
## desktop reactions
### far drop
合図: ＠住人_おろす
条件:（入力#移動距離 >= 100）
話者: yuukei
「遠くまで来たね」
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut event = RuntimeEvent::new("avatar.gesture.drop", "surface", "resident-default");
        event
            .payload
            .insert("movedDistance".to_string(), json!(184));

        let result = adapter.dispatch(&event, &world).await?;

        assert_eq!(result.commands[0].payload["text"], "遠くまで来たね");
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_dispatches_idle_end_scene_by_friendly_idle_minutes_input() -> Result<()>
    {
        let mut world = pack();
        world.signals.allow = vec!["presence.idle.end".to_string()];
        world.daihon.loaded_scripts[0].source = r#"
## desktop reactions
### welcome back
合図: ＠復帰
条件:（入力#不在分 = 7）
話者: yuukei
「おかえり」
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut event = RuntimeEvent::new("presence.idle.end", "device", "resident-default");
        event.id = "evt_idle_end".to_string();
        event.payload.insert("idleMinutes".to_string(), json!(7));
        event.payload.insert("idleSeconds".to_string(), json!(421));

        let result = adapter.dispatch(&event, &world).await?;
        assert_eq!(result.commands.len(), 1);
        assert_eq!(result.commands[0].payload["text"], "おかえり");
        assert_eq!(result.executed_scenes[0].scene_name, "welcome back");
        Ok(())
    }

    #[test]
    fn event_inputs_include_desktop_terrain_friendly_names() {
        let mut event = RuntimeEvent::new("desktop.folder.opened", "device", "resident-default");
        event.payload.insert("app".to_string(), json!("finder"));
        event
            .payload
            .insert("category".to_string(), json!("downloads"));
        event
            .payload
            .insert("fileName".to_string(), json!("report.pdf"));
        event
            .payload
            .insert("fileCategory".to_string(), json!("document"));
        event
            .payload
            .insert("windowKey".to_string(), json!("window-1"));
        event
            .payload
            .insert("recentDownloadFileName".to_string(), json!("photo.png"));
        event
            .payload
            .insert("recentDownloadCategory".to_string(), json!("image"));
        event.payload.insert("aiConnected".to_string(), json!(true));
        event
            .payload
            .insert("actorLocation".to_string(), json!("downloads"));
        event
            .payload
            .insert("actorPresence".to_string(), json!("away"));

        let inputs = event_inputs(&event).into_iter().collect::<BTreeMap<_, _>>();

        assert_eq!(inputs["アプリ"], DaihonValue::String("finder".to_string()));
        assert_eq!(
            inputs["フォルダ"],
            DaihonValue::String("downloads".to_string())
        );
        assert_eq!(
            inputs["ファイル名"],
            DaihonValue::String("report.pdf".to_string())
        );
        assert_eq!(
            inputs["ファイル種類"],
            DaihonValue::String("document".to_string())
        );
        assert_eq!(inputs["窓ID"], DaihonValue::String("window-1".to_string()));
        assert_eq!(
            inputs["最近のダウンロード"],
            DaihonValue::String("photo.png".to_string())
        );
        assert_eq!(
            inputs["最近のダウンロード種類"],
            DaihonValue::String("image".to_string())
        );
        assert_eq!(inputs["AI接続"], DaihonValue::Boolean(true));
        assert_eq!(inputs["場所"], DaihonValue::String("downloads".to_string()));
        assert_eq!(inputs["在席"], DaihonValue::String("away".to_string()));
    }

    #[tokio::test]
    async fn yuukei_adapter_dispatches_stage_perch_functions() -> Result<()> {
        let mut world = pack();
        world.signals.allow = vec!["desktop.window.focused".to_string()];
        world.daihon.loaded_scripts[0].source = r#"
## desktop reactions
### perch on focused window
合図: ＠窓_注目
話者: yuukei
＜枠に座る (入力#窓ID)＞
＜枠から降りる＞
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut event = RuntimeEvent::new("desktop.window.focused", "device", "resident-default");
        event.id = "evt_window_focus".to_string();
        event.device_id = Some("device-local".to_string());
        event
            .payload
            .insert("windowKey".to_string(), json!("win-42"));
        event.payload.insert("app".to_string(), json!("Finder"));

        let result = adapter.dispatch(&event, &world).await?;

        assert_eq!(result.commands.len(), 2);
        assert_eq!(result.commands[0].kind, "stage.perch");
        assert_eq!(result.commands[0].payload["windowKey"], "win-42");
        assert_eq!(result.commands[0].payload["speakerId"], "yuukei");
        assert_eq!(result.commands[0].payload["sourceFunction"], "枠に座る");
        assert_eq!(
            result.commands[0]
                .target
                .as_ref()
                .and_then(|target| target.actor_id.as_deref()),
            Some("yuukei")
        );
        assert_eq!(result.commands[1].kind, "stage.perch.release");
        assert_eq!(result.commands[1].payload["speakerId"], "yuukei");
        assert_eq!(result.commands[1].payload["sourceFunction"], "枠から降りる");
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_dispatches_stage_walk_function() -> Result<()> {
        let mut world = pack();
        world.signals.allow = vec!["app.startup".to_string()];
        world.daihon.loaded_scripts[0].source = r#"
## app.startup
### walk
話者: yuukei
＜歩く 右端＞
＜walk 左端 速さ=120 動作=歩く＞
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let event = RuntimeEvent::new("app.startup", "device", "resident-default");

        let result = adapter.dispatch(&event, &world).await?;

        assert_eq!(result.commands.len(), 2);
        assert_eq!(result.commands[0].kind, "stage.walk");
        assert_eq!(result.commands[0].payload["destination"], "right-edge");
        assert_eq!(result.commands[0].payload["motion"], "walk");
        assert!(!result.commands[0].payload.contains_key("speedPxPerSec"));
        assert_eq!(result.commands[0].payload["speakerId"], "yuukei");
        assert_eq!(result.commands[0].payload["sourceFunction"], "歩く");
        assert_eq!(result.commands[1].payload["destination"], "left-edge");
        assert_eq!(result.commands[1].payload["motion"], "歩く");
        assert_eq!(result.commands[1].payload["speedPxPerSec"], 120);
        assert_eq!(result.commands[1].payload["sourceFunction"], "walk");
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_dispatches_actor_location_exit_and_enter_functions() -> Result<()> {
        let mut world = pack();
        world.signals.allow = vec!["app.startup".to_string()];
        world.daihon.loaded_scripts[0].source = r#"
## app.startup
### explore folders
話者: yuukei
＜場所 「pictures」＞
＜退場 行き先=「downloads」＞
＜登場＞
＜登場 場所=「desktop」＞
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let event = RuntimeEvent::new("app.startup", "device", "resident-default");

        let result = adapter.dispatch(&event, &world).await?;

        assert_eq!(result.commands.len(), 4);
        assert_eq!(result.commands[0].kind, "actor.location.set");
        assert_eq!(result.commands[0].payload["location"], "pictures");
        assert_eq!(result.commands[0].payload["sourceFunction"], "場所");
        assert_eq!(result.commands[1].kind, "actor.exit");
        assert_eq!(result.commands[1].payload["location"], "downloads");
        assert_eq!(result.commands[1].payload["sourceFunction"], "退場");
        assert_eq!(result.commands[2].kind, "actor.enter");
        assert!(!result.commands[2].payload.contains_key("location"));
        assert_eq!(result.commands[3].kind, "actor.enter");
        assert_eq!(result.commands[3].payload["location"], "desktop");
        assert!(result.commands.iter().all(|command| {
            command
                .target
                .as_ref()
                .and_then(|target| target.actor_id.as_deref())
                == Some("yuukei")
        }));
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_ignores_invalid_or_empty_stage_walk_destination() -> Result<()> {
        let mut world = pack();
        world.signals.allow = vec!["app.startup".to_string()];
        world.daihon.loaded_scripts[0].source = r#"
## app.startup
### walk
話者: yuukei
＜歩く 上＞
＜walk＞
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let event = RuntimeEvent::new("app.startup", "device", "resident-default");

        let result = adapter.dispatch(&event, &world).await?;

        assert!(result.commands.is_empty());
        Ok(())
    }

    #[test]
    fn event_inputs_translate_stage_walk_ended_reason_for_daihon() {
        for (reason, expected) in [
            ("arrived", "到着"),
            ("user-drag", "中断"),
            ("replaced", "中断"),
        ] {
            let mut event = RuntimeEvent::new("stage.walk.ended", "device", "resident-default");
            event.payload.insert("reason".to_string(), json!(reason));

            let inputs = event_inputs(&event).into_iter().collect::<BTreeMap<_, _>>();

            assert_eq!(inputs["理由"], DaihonValue::String(expected.to_string()));
        }
    }

    #[tokio::test]
    async fn yuukei_adapter_dispatches_stage_walk_ended_with_japanese_reason() -> Result<()> {
        let mut world = pack();
        world.signals.allow = vec!["stage.walk.ended".to_string()];
        world.daihon.loaded_scripts[0].source = r#"
## stage reactions
### walk ended
合図: ＠住人_歩き終わり
話者: yuukei
「理由は＜入力#理由＞」
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut event = RuntimeEvent::new("stage.walk.ended", "device", "resident-default");
        event
            .payload
            .insert("reason".to_string(), json!("user-drag"));

        let result = adapter.dispatch(&event, &world).await?;

        assert_eq!(result.commands[0].payload["text"], "理由は中断");
        Ok(())
    }

    struct ChoiceHandler {
        choice: String,
        requests: Vec<DaihonChoiceRequest>,
    }

    struct AiMomentHandler {
        interpretation: String,
        extracted: String,
        generated: Option<DaihonGenerateResponse>,
        interpret_requests: Vec<DaihonInterpretRequest>,
        extract_requests: Vec<DaihonExtractRequest>,
        generate_requests: Vec<DaihonGenerateRequest>,
    }

    #[async_trait]
    impl DaihonInterpretHandler for AiMomentHandler {
        async fn interpret(&mut self, request: DaihonInterpretRequest) -> String {
            self.interpret_requests.push(request);
            self.interpretation.clone()
        }

        async fn extract(&mut self, request: DaihonExtractRequest) -> String {
            self.extract_requests.push(request);
            self.extracted.clone()
        }

        async fn generate(
            &mut self,
            request: DaihonGenerateRequest,
        ) -> Option<DaihonGenerateResponse> {
            self.generate_requests.push(request);
            self.generated.clone()
        }
    }

    fn ai_moment_handler() -> AiMomentHandler {
        AiMomentHandler {
            interpretation: yuukei_daihon::UNKNOWN_INTERPRETATION.to_string(),
            extracted: yuukei_daihon::UNKNOWN_INTERPRETATION.to_string(),
            generated: None,
            interpret_requests: Vec::new(),
            extract_requests: Vec::new(),
            generate_requests: Vec::new(),
        }
    }

    #[async_trait]
    impl DaihonInterpretHandler for ChoiceHandler {
        async fn interpret(&mut self, _request: DaihonInterpretRequest) -> String {
            yuukei_daihon::UNKNOWN_INTERPRETATION.to_string()
        }

        async fn choose(&mut self, request: DaihonChoiceRequest) -> String {
            self.requests.push(request);
            self.choice.clone()
        }
    }

    #[tokio::test]
    async fn yuukei_adapter_dispatches_choice_scene_with_mock_handler() -> Result<()> {
        let mut world = pack();
        world.daihon.loaded_scripts[0].source = r#"
## desktop reactions
### invite
合図: ＠conversation.text
話者: yuukei
返事=＜選択 「見る」 「あとで」＞
※（返事 = 「見る」）なら:
「見よう」
※あるいは（返事 = 「不明」）なら:
「わからない」
※それ以外:
「あとでね」
おわり
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut handler = ChoiceHandler {
            choice: "見る".to_string(),
            requests: Vec::new(),
        };
        let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
        event.id = "evt_choice".to_string();

        let result = adapter
            .dispatch_with_interpret(&event, &world, &mut handler)
            .await?;

        assert_eq!(handler.requests.len(), 1);
        assert_eq!(handler.requests[0].choices, vec!["見る", "あとで"]);
        assert_eq!(handler.requests[0].timeout_seconds, 30);
        assert_eq!(result.commands.len(), 1);
        assert_eq!(result.commands[0].payload["text"], "見よう");
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_resolves_daihon_speaker_aliases() -> Result<()> {
        let mut world = pack();
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
＜表情 笑顔＞
パ: ＜動作 歩く＞「次は私です。」
※（入力#ユーザー発言 = 「こんにちは」）なら:
ゆ: 「戻りました。」
おわり
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
        event.id = "evt_speaker_alias".to_string();
        event
            .payload
            .insert("text".to_string(), json!("こんにちは"));

        let result = adapter.dispatch(&event, &world).await?;
        assert_eq!(result.commands.len(), 4);
        assert_eq!(
            result.commands[0]
                .target
                .as_ref()
                .and_then(|target| target.actor_id.as_deref()),
            Some("yuukei")
        );
        assert_eq!(result.commands[0].payload["speakerId"], "yuukei");
        assert_eq!(
            result.commands[1]
                .target
                .as_ref()
                .and_then(|target| target.actor_id.as_deref()),
            Some("partner")
        );
        assert_eq!(result.commands[1].payload["speakerId"], "partner");
        assert_eq!(result.commands[2].payload["text"], "次は私です。");
        assert_eq!(result.commands[2].payload["speakerId"], "partner");
        assert_eq!(result.commands[3].payload["text"], "戻りました。");
        assert_eq!(result.commands[3].payload["speakerId"], "yuukei");
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_rejects_unknown_daihon_speaker() -> Result<()> {
        let mut world = pack();
        world.daihon.loaded_scripts[0].source = r#"
## desktop reactions
### conversation
合図: ＠conversation.text
話者: だれ
「届きません。」
"#
        .to_string();
        let adapter = YuukeiDaihonAdapter::default();
        let error = adapter
            .load_world(&world)
            .await
            .expect_err("unknown speaker should fail");
        assert!(error.to_string().contains("unknown Daihon speaker"));
        assert!(error.to_string().contains("だれ"));
        let report = error.daihon_report().expect("structured Daihon report");
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(
            report.diagnostics[0].phase,
            DaihonDiagnosticPhase::LoadSpeaker
        );
        assert_eq!(
            report.diagnostics[0].script_path.as_deref(),
            Some("scripts/reactions.daihon")
        );
        assert_eq!(report.diagnostics[0].line, Some(5));
        Ok(())
    }

    #[tokio::test]
    async fn default_pack_sample_dispatches_pat_dialogue_from_other_actor() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/default-yuukei");
        let world = WorldPack::load_from_dir(root)?;
        assert!(world.allows_signal("avatar.gesture.pat"));

        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;

        let mut yuukei_pat = RuntimeEvent::new("avatar.gesture.pat", "surface", "resident-default");
        yuukei_pat.id = "evt_pat_yuukei".to_string();
        yuukei_pat.actor_id = Some("yuukei".to_string());
        yuukei_pat
            .payload
            .insert("hitZoneId".to_string(), json!("head"));
        yuukei_pat
            .payload
            .insert("hitSurface".to_string(), json!("skin"));
        let result = adapter.dispatch(&yuukei_pat, &world).await?;
        let dialogue = result
            .commands
            .iter()
            .find(|command| command.kind == "dialogue.say")
            .expect("partner dialogue");
        assert_eq!(dialogue.payload["speakerId"], "partner");
        assert_eq!(
            dialogue
                .target
                .as_ref()
                .and_then(|target| target.actor_id.as_deref()),
            Some("partner")
        );

        let mut partner_pat =
            RuntimeEvent::new("avatar.gesture.pat", "surface", "resident-default");
        partner_pat.id = "evt_pat_partner".to_string();
        partner_pat.actor_id = Some("partner".to_string());
        partner_pat
            .payload
            .insert("hitZoneId".to_string(), json!("head"));
        partner_pat
            .payload
            .insert("hitSurface".to_string(), json!("skin"));
        let result = adapter.dispatch(&partner_pat, &world).await?;
        let dialogue = result
            .commands
            .iter()
            .find(|command| command.kind == "dialogue.say")
            .expect("yuukei dialogue");
        assert_eq!(dialogue.payload["speakerId"], "yuukei");
        assert_eq!(
            dialogue
                .target
                .as_ref()
                .and_then(|target| target.actor_id.as_deref()),
            Some("yuukei")
        );
        Ok(())
    }

    #[tokio::test]
    async fn default_pack_first_life_tick_starts_an_authored_walk() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/default-yuukei");
        let world = WorldPack::load_from_dir(root)?;
        assert!(world.allows_signal("presence.life_tick"));
        assert!(world.allows_signal("stage.walk.ended"));

        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;

        let mut event = RuntimeEvent::new("presence.life_tick", "device", "resident-default");
        event.id = "evt_first_life_tick".to_string();
        event
            .payload
            .insert("actorLocation".to_string(), json!("desktop"));
        event
            .payload
            .insert("actorPresence".to_string(), json!("present"));

        let result = adapter.dispatch(&event, &world).await?;

        assert_eq!(result.executed_scenes[0].scene_name, "最初の見回り");
        let walk = result
            .commands
            .iter()
            .find(|command| command.kind == "stage.walk")
            .expect("authored first walk");
        assert_eq!(walk.payload["destination"], "right-edge");
        assert!(result
            .commands
            .iter()
            .any(|command| command.kind == "dialogue.say"));
        Ok(())
    }

    #[tokio::test]
    async fn default_pack_download_continues_into_folder_and_later_talk() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/default-yuukei");
        let world = WorldPack::load_from_dir(root)?;
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;

        let mut download =
            RuntimeEvent::new("desktop.download.completed", "device", "resident-default");
        download.id = "evt_download_continuity".to_string();
        download
            .payload
            .insert("fileName".to_string(), json!("おやつ.png"));
        download
            .payload
            .insert("fileCategory".to_string(), json!("image"));
        adapter.dispatch(&download, &world).await?;

        let mut folder = RuntimeEvent::new("desktop.folder.opened", "device", "resident-default");
        folder.id = "evt_folder_continuity".to_string();
        folder
            .payload
            .insert("category".to_string(), json!("downloads"));
        folder
            .payload
            .insert("recentDownloadFileName".to_string(), json!("おやつ.png"));
        folder
            .payload
            .insert("recentDownloadCategory".to_string(), json!("image"));
        folder
            .payload
            .insert("actorLocation".to_string(), json!("desktop"));
        folder
            .payload
            .insert("actorPresence".to_string(), json!("present"));
        let folder_result = adapter.dispatch(&folder, &world).await?;
        assert_eq!(
            folder_result.executed_scenes[0].scene_name,
            "新しい届きものを一緒に確認"
        );
        assert!(folder_result.commands.iter().any(|command| {
            command.kind == "dialogue.say"
                && command.payload["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("ここにありました"))
        }));

        let mut talk = talk_impulse_event("evt_download_followup");
        talk.payload.insert("timePeriod".to_string(), json!("昼"));
        talk.payload.insert("気分".to_string(), json!("ふつう"));
        talk.payload.insert("aiConnected".to_string(), json!(false));
        let talk_result = adapter.dispatch(&talk, &world).await?;
        assert_eq!(
            talk_result.executed_scenes[0].scene_name,
            "届きものの後日談"
        );
        assert!(talk_result.commands.iter().any(|command| {
            command.kind == "dialogue.say"
                && command.payload["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("さっき一緒に見た"))
        }));
        Ok(())
    }

    #[tokio::test]
    async fn default_pack_life_tick_brings_away_actor_home() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/default-yuukei");
        let world = WorldPack::load_from_dir(root)?;
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;

        let mut event = RuntimeEvent::new("presence.life_tick", "device", "resident-default");
        event.id = "evt_return_from_downloads".to_string();
        event
            .payload
            .insert("actorLocation".to_string(), json!("downloads"));
        event
            .payload
            .insert("actorPresence".to_string(), json!("away"));

        let result = adapter.dispatch(&event, &world).await?;

        assert_eq!(result.executed_scenes[0].scene_name, "Downloadsから帰る");
        let enter = result
            .commands
            .iter()
            .find(|command| command.kind == "actor.enter")
            .expect("actor returns to the desktop");
        assert_eq!(enter.payload["location"], "desktop");
        Ok(())
    }

    #[tokio::test]
    async fn default_pack_extracts_and_remembers_requested_user_name() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/default-yuukei");
        let world = WorldPack::load_from_dir(root)?;
        assert!(world
            .capabilities
            .optional
            .contains(&"dialogue.extract".to_string()));
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut handler = ai_moment_handler();
        handler.extracted = "ミナ".to_string();

        let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
        event.id = "evt_remember_name".to_string();
        event
            .payload
            .insert("text".to_string(), json!("ミナって呼んで"));
        event.payload.insert("aiConnected".to_string(), json!(true));

        let result = adapter
            .dispatch_with_interpret(&event, &world, &mut handler)
            .await?;

        assert_eq!(handler.extract_requests.len(), 1);
        assert_eq!(handler.extract_requests[0].input_text, "ミナって呼んで");
        assert!(result.commands.iter().any(|command| {
            command.kind == "dialogue.say"
                && command.payload["text"] == "ミナさん、ですね。覚えました。"
        }));
        assert!(result.variable_patches.iter().any(|patch| {
            patch["path"] == "全体#ユーザー呼び名" && patch["value"] == "ミナ"
        }));
        Ok(())
    }

    #[tokio::test]
    async fn default_pack_interprets_user_condition_into_authored_response() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/default-yuukei");
        let world = WorldPack::load_from_dir(root)?;
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut handler = ai_moment_handler();
        handler.interpretation = "疲れた".to_string();

        let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
        event.id = "evt_interpret_condition".to_string();
        event
            .payload
            .insert("text".to_string(), json!("今日はちょっと疲れた"));
        event.payload.insert("aiConnected".to_string(), json!(true));

        let result = adapter
            .dispatch_with_interpret(&event, &world, &mut handler)
            .await?;

        assert_eq!(handler.interpret_requests.len(), 1);
        assert_eq!(
            handler.interpret_requests[0].choices,
            vec!["元気", "疲れた", "つらい"]
        );
        assert!(result.commands.iter().any(|command| {
            command.kind == "dialogue.say"
                && command.payload["text"] == "疲れているんですね。返事は短くて大丈夫です。"
        }));
        Ok(())
    }

    #[tokio::test]
    async fn default_pack_generates_app_observation_with_authored_fallback() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/default-yuukei");
        let world = WorldPack::load_from_dir(root)?;
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let mut handler = ai_moment_handler();
        handler.generated = Some(DaihonGenerateResponse {
            text: "コードの窓から、細い路地がたくさん見えます。".to_string(),
            expression: None,
            motion: None,
        });

        let mut event = RuntimeEvent::new("desktop.window.focused", "device", "resident-default");
        event.id = "evt_generate_app_observation".to_string();
        event
            .payload
            .insert("app".to_string(), json!("Visual Studio Code"));
        event.payload.insert("aiConnected".to_string(), json!(true));

        let generated = adapter
            .dispatch_with_interpret(&event, &world, &mut handler)
            .await?;
        assert_eq!(handler.generate_requests.len(), 1);
        assert!(handler.generate_requests[0]
            .instruction
            .contains("Visual Studio Code"));
        assert!(generated.commands.iter().any(|command| {
            command.kind == "dialogue.say"
                && command.payload["text"] == "コードの窓から、細い路地がたくさん見えます。"
        }));

        let fallback_adapter = YuukeiDaihonAdapter::default();
        fallback_adapter.load_world(&world).await?;
        let fallback = fallback_adapter.dispatch(&event, &world).await?;
        assert!(
            fallback.commands.iter().any(|command| {
                command.kind == "dialogue.say"
                    && command.payload["text"].as_str().is_some_and(|text| {
                        text.contains("Visual Studio Code")
                            && text.contains("外の気配が少し変わりました")
                    })
            }),
            "fallback commands: {:?}",
            fallback.commands
        );
        Ok(())
    }

    #[tokio::test]
    async fn default_pack_sample_dispatches_poke_dialogue_from_touched_actor() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/default-yuukei");
        let world = WorldPack::load_from_dir(root)?;
        assert!(world.allows_signal("avatar.gesture.poke"));

        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;

        let mut yuukei_poke =
            RuntimeEvent::new("avatar.gesture.poke", "surface", "resident-default");
        yuukei_poke.id = "evt_poke_yuukei".to_string();
        yuukei_poke.actor_id = Some("yuukei".to_string());
        yuukei_poke
            .payload
            .insert("hitZoneId".to_string(), json!("head"));
        yuukei_poke
            .payload
            .insert("hitSurface".to_string(), json!("skin"));
        let result = adapter.dispatch(&yuukei_poke, &world).await?;
        let dialogue = result
            .commands
            .iter()
            .find(|command| command.kind == "dialogue.say")
            .expect("yuukei dialogue");
        assert_eq!(dialogue.payload["speakerId"], "yuukei");
        assert_eq!(
            dialogue
                .target
                .as_ref()
                .and_then(|target| target.actor_id.as_deref()),
            Some("yuukei")
        );

        let mut partner_poke =
            RuntimeEvent::new("avatar.gesture.poke", "surface", "resident-default");
        partner_poke.id = "evt_poke_partner".to_string();
        partner_poke.actor_id = Some("partner".to_string());
        partner_poke
            .payload
            .insert("hitZoneId".to_string(), json!("head"));
        partner_poke
            .payload
            .insert("hitSurface".to_string(), json!("skin"));
        let result = adapter.dispatch(&partner_poke, &world).await?;
        let dialogue = result
            .commands
            .iter()
            .find(|command| command.kind == "dialogue.say")
            .expect("partner dialogue");
        assert_eq!(dialogue.payload["speakerId"], "partner");
        assert_eq!(
            dialogue
                .target
                .as_ref()
                .and_then(|target| target.actor_id.as_deref()),
            Some("partner")
        );
        Ok(())
    }

    #[tokio::test]
    async fn default_pack_poke_cloth_dialogue_matches_touched_zone() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/default-yuukei");
        let world = WorldPack::load_from_dir(root)?;

        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;

        let poke_cloth = |event_id: &str, zone: &str| {
            let mut event = RuntimeEvent::new("avatar.gesture.poke", "surface", "resident-default");
            event.id = event_id.to_string();
            event.actor_id = Some("partner".to_string());
            event.payload.insert("hitZoneId".to_string(), json!(zone));
            event
                .payload
                .insert("hitSurface".to_string(), json!("cloth"));
            event
        };

        // 靴(足ゾーン)のclothは袖セリフになってはいけない
        let result = adapter
            .dispatch(&poke_cloth("evt_poke_shoe", "rightFoot"), &world)
            .await?;
        let dialogue = result
            .commands
            .iter()
            .find(|command| command.kind == "dialogue.say")
            .expect("shoe dialogue");
        assert_eq!(
            dialogue.payload["text"],
            "……くつ。ひっぱっても、伸びません。"
        );

        // 腕ゾーンのclothは従来どおり袖セリフ
        let result = adapter
            .dispatch(&poke_cloth("evt_poke_sleeve", "leftArm"), &world)
            .await?;
        let dialogue = result
            .commands
            .iter()
            .find(|command| command.kind == "dialogue.say")
            .expect("sleeve dialogue");
        assert_eq!(dialogue.payload["text"], "……そで、伸びます。");

        // 胴体などその他のclothは汎用セリフ(袖と言わない)
        let result = adapter
            .dispatch(&poke_cloth("evt_poke_torso", "belly"), &world)
            .await?;
        let dialogue = result
            .commands
            .iter()
            .find(|command| command.kind == "dialogue.say")
            .expect("torso dialogue");
        assert_eq!(dialogue.payload["text"], "……服、伸びます。");
        Ok(())
    }

    #[tokio::test]
    async fn default_pack_poke_yuukei_shoe_walks_to_right_edge() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/default-yuukei");
        let world = WorldPack::load_from_dir(root)?;

        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;

        let mut event = RuntimeEvent::new("avatar.gesture.poke", "surface", "resident-default");
        event.actor_id = Some("yuukei".to_string());
        event
            .payload
            .insert("hitZoneId".to_string(), json!("leftFoot"));
        event
            .payload
            .insert("hitSurface".to_string(), json!("cloth"));

        let result = adapter.dispatch(&event, &world).await?;

        let walk = result
            .commands
            .iter()
            .find(|command| command.kind == "stage.walk")
            .expect("stage.walk command");
        assert_eq!(walk.payload["destination"], "right-edge");
        assert_eq!(walk.payload["motion"], "walk");
        let dialogue = result
            .commands
            .iter()
            .find(|command| command.kind == "dialogue.say")
            .expect("shoe walk dialogue");
        assert_eq!(
            dialogue.payload["text"],
            "わっ、くつはだめです。……もう、あっちに行きますからね。"
        );
        Ok(())
    }

    #[tokio::test]
    async fn yuukei_adapter_ignores_disallowed_signal() -> Result<()> {
        let world = pack();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let event = RuntimeEvent::new("os.file_browser.focused", "device", "resident-default");
        let result = adapter.dispatch(&event, &world).await?;
        assert!(result.commands.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn persistent_scene_history_keeps_once_frequency_after_reopen() -> Result<()> {
        let dir = tempdir()?;
        let history_path = dir.path().join("scene-history.json");
        let world = world_with_script(
            r#"
## presence.talk_impulse
### 初回だけ
合図: ＠presence.talk_impulse
頻度: 一度きり
話者: yuukei
「初回だけです。」
"#,
        );

        let first = dispatch_with_history(&world, &history_path, "evt_once_1").await?;
        assert_eq!(first.executed_scenes[0].scene_name, "初回だけ");
        assert!(history_path.exists());

        let second = dispatch_with_history(&world, &history_path, "evt_once_2").await?;
        assert!(second.commands.is_empty());
        assert!(second.executed_scenes.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn persistent_scene_history_keeps_duration_frequency_after_reopen() -> Result<()> {
        let dir = tempdir()?;
        let history_path = dir.path().join("scene-history.json");
        let world = world_with_script(
            r#"
## presence.talk_impulse
### 日に一度
合図: ＠presence.talk_impulse
頻度: 1日に1回
話者: yuukei
「今日はもう話しました。」
"#,
        );

        let first = dispatch_with_history(&world, &history_path, "evt_duration_1").await?;
        assert_eq!(first.executed_scenes[0].scene_name, "日に一度");

        let second = dispatch_with_history(&world, &history_path, "evt_duration_2").await?;
        assert!(second.commands.is_empty());
        assert!(second.executed_scenes.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn persistent_scene_history_keeps_last_scene_after_reopen() -> Result<()> {
        let dir = tempdir()?;
        let history_path = dir.path().join("scene-history.json");
        let world = world_with_script(
            r#"
## presence.talk_impulse
### A
合図: ＠presence.talk_impulse
話者: yuukei
「Aです。」

### B
合図: ＠presence.talk_impulse
話者: yuukei
「Bです。」
"#,
        );

        let first = dispatch_with_history(&world, &history_path, "evt_last_1").await?;
        let first_scene = first.executed_scenes[0].scene_name.clone();

        let second = dispatch_with_history(&world, &history_path, "evt_last_2").await?;
        assert_eq!(second.executed_scenes.len(), 1);
        assert_ne!(second.executed_scenes[0].scene_name, first_scene);
        Ok(())
    }

    #[tokio::test]
    async fn persistent_scene_history_is_separated_by_history_file() -> Result<()> {
        let dir = tempdir()?;
        let pack_a_history = dir.path().join("pack-a").join("scene-history.json");
        let pack_b_history = dir.path().join("pack-b").join("scene-history.json");
        let world = world_with_script(
            r#"
## presence.talk_impulse
### 初回だけ
合図: ＠presence.talk_impulse
頻度: 一度きり
話者: yuukei
「別のPackなら話せます。」
"#,
        );

        let first_a = dispatch_with_history(&world, &pack_a_history, "evt_pack_a_1").await?;
        assert_eq!(first_a.executed_scenes[0].scene_name, "初回だけ");
        let second_a = dispatch_with_history(&world, &pack_a_history, "evt_pack_a_2").await?;
        assert!(second_a.executed_scenes.is_empty());

        let first_b = dispatch_with_history(&world, &pack_b_history, "evt_pack_b_1").await?;
        assert_eq!(first_b.executed_scenes[0].scene_name, "初回だけ");
        Ok(())
    }

    #[tokio::test]
    async fn persistent_scene_history_ignores_corrupt_json() -> Result<()> {
        let dir = tempdir()?;
        let history_path = dir.path().join("scene-history.json");
        fs::write(&history_path, "{broken json")?;
        let world = world_with_script(
            r#"
## presence.talk_impulse
### 初回だけ
合図: ＠presence.talk_impulse
頻度: 一度きり
話者: yuukei
「壊れていても始めます。」
"#,
        );

        let result = dispatch_with_history(&world, &history_path, "evt_corrupt_1").await?;
        assert_eq!(result.executed_scenes[0].scene_name, "初回だけ");
        Ok(())
    }

    #[tokio::test]
    async fn persistent_variables_round_trip_after_reopen() -> Result<()> {
        let dir = tempdir()?;
        let history_path = dir.path().join("scene-history.json");
        let variables_path = dir.path().join("variables.json");
        let world = world_with_script(
            r#"
## presence.talk_impulse
### 保存
合図: ＠presence.talk_impulse
全体#呼び名=入力#ユーザー発言
「保存しました。」
"#,
        );
        let mut event = talk_impulse_event("evt_variables_save");
        event.payload.insert("text".to_string(), json!("ミナ"));

        let first = dispatch_with_state(&world, &history_path, &variables_path, event).await?;
        assert_eq!(first.variable_patches.len(), 1);
        assert!(variables_path.exists());

        let read_world = world_with_script(
            r#"
## presence.talk_impulse
### 読む
合図: ＠presence.talk_impulse
「呼び名は＜全体#呼び名＞です。」
"#,
        );
        let second = dispatch_with_state(
            &read_world,
            &history_path,
            &variables_path,
            talk_impulse_event("evt_variables_read"),
        )
        .await?;
        assert_eq!(second.commands[0].payload["text"], "呼び名はミナです。");
        Ok(())
    }

    #[tokio::test]
    async fn persistent_variables_ignore_corrupt_json() -> Result<()> {
        let dir = tempdir()?;
        let history_path = dir.path().join("scene-history.json");
        let variables_path = dir.path().join("variables.json");
        fs::write(&variables_path, "{broken json")?;
        let world = world_with_script(
            r#"
## presence.talk_impulse
初期値:
全体#呼び名=「初期」
### 初期
合図: ＠presence.talk_impulse
「呼び名は＜全体#呼び名＞です。」
"#,
        );

        let result = dispatch_with_state(
            &world,
            &history_path,
            &variables_path,
            talk_impulse_event("evt_corrupt_variables"),
        )
        .await?;
        assert_eq!(result.commands[0].payload["text"], "呼び名は初期です。");
        Ok(())
    }

    #[tokio::test]
    async fn persistent_variables_win_over_same_type_initial_values() -> Result<()> {
        let dir = tempdir()?;
        let history_path = dir.path().join("scene-history.json");
        let variables_path = dir.path().join("variables.json");
        let save_world = world_with_script(
            r#"
## presence.talk_impulse
### 保存
合図: ＠presence.talk_impulse
全体#呼び名=「ミナ」
「保存しました。」
"#,
        );
        dispatch_with_state(
            &save_world,
            &history_path,
            &variables_path,
            talk_impulse_event("evt_initial_save"),
        )
        .await?;

        let read_world = world_with_script(
            r#"
## presence.talk_impulse
初期値:
全体#呼び名=「初期」
### 読む
合図: ＠presence.talk_impulse
「呼び名は＜全体#呼び名＞です。」
"#,
        );
        let result = dispatch_with_state(
            &read_world,
            &history_path,
            &variables_path,
            talk_impulse_event("evt_initial_read"),
        )
        .await?;
        assert_eq!(result.commands[0].payload["text"], "呼び名はミナです。");
        Ok(())
    }

    #[tokio::test]
    async fn initial_value_wins_when_persistent_variable_type_differs() -> Result<()> {
        let dir = tempdir()?;
        let history_path = dir.path().join("scene-history.json");
        let variables_path = dir.path().join("variables.json");
        let save_world = world_with_script(
            r#"
## presence.talk_impulse
### 保存
合図: ＠presence.talk_impulse
全体#呼び名=「ミナ」
「保存しました。」
"#,
        );
        dispatch_with_state(
            &save_world,
            &history_path,
            &variables_path,
            talk_impulse_event("evt_type_save"),
        )
        .await?;

        let read_world = world_with_script(
            r#"
## presence.talk_impulse
初期値:
全体#呼び名=1
### 読む
合図: ＠presence.talk_impulse
「呼び名は＜全体#呼び名＞です。」
"#,
        );
        let result = dispatch_with_state(
            &read_world,
            &history_path,
            &variables_path,
            talk_impulse_event("evt_type_read"),
        )
        .await?;
        assert_eq!(result.commands[0].payload["text"], "呼び名は1です。");
        Ok(())
    }

    #[test]
    fn hit_zones_are_optional_for_renderer_assets() {
        let mut world = pack();
        world.actors[0].renderer = Some(ActorRendererDefinition {
            kind: ActorRendererKind::Vrm,
            model: "character/character_1.vrm".to_string(),
            motions: BTreeMap::new(),
            hit_zones: Vec::new(),
        });

        assert!(world.validate().is_ok());
    }

    #[test]
    fn rejects_humanoid_hit_zone_without_bones() {
        let mut world = pack();
        world.actors[0].renderer = Some(ActorRendererDefinition {
            kind: ActorRendererKind::Vrm,
            model: "character/character_1.vrm".to_string(),
            motions: BTreeMap::new(),
            hit_zones: vec![ActorHitZoneDefinition {
                id: "head".to_string(),
                label: Some("頭".to_string()),
                source: ActorHitZoneSource::HumanoidBone,
                bones: Vec::new(),
                nodes: Vec::new(),
                shape: Some(ActorHitZoneShape::Auto),
                events: vec!["avatar.gesture.poke".to_string()],
                priority: None,
            }],
        });

        let error = world.validate().expect_err("empty bones should fail");
        assert!(error.to_string().contains("hitZone head"));
    }

    #[test]
    fn rejects_node_hit_zone_without_nodes() {
        let mut world = pack();
        world.actors[0].renderer = Some(ActorRendererDefinition {
            kind: ActorRendererKind::Vrm,
            model: "character/character_1.vrm".to_string(),
            motions: BTreeMap::new(),
            hit_zones: vec![ActorHitZoneDefinition {
                id: "tail".to_string(),
                label: Some("しっぽ".to_string()),
                source: ActorHitZoneSource::NodeName,
                bones: Vec::new(),
                nodes: Vec::new(),
                shape: Some(ActorHitZoneShape::Mesh),
                events: vec!["avatar.gesture.poke".to_string()],
                priority: None,
            }],
        });

        let error = world.validate().expect_err("empty nodes should fail");
        assert!(error.to_string().contains("hitZone tail"));
    }

    #[test]
    fn rejects_duplicate_hit_zone_ids_per_actor() {
        let mut world = pack();
        let hit_zone = ActorHitZoneDefinition {
            id: "head".to_string(),
            label: Some("頭".to_string()),
            source: ActorHitZoneSource::HumanoidBone,
            bones: vec!["head".to_string()],
            nodes: Vec::new(),
            shape: Some(ActorHitZoneShape::Auto),
            events: vec!["avatar.gesture.poke".to_string()],
            priority: None,
        };
        world.actors[0].renderer = Some(ActorRendererDefinition {
            kind: ActorRendererKind::Vrm,
            model: "character/character_1.vrm".to_string(),
            motions: BTreeMap::new(),
            hit_zones: vec![hit_zone.clone(), hit_zone],
        });

        let error = world.validate().expect_err("duplicate hitZone should fail");
        assert!(error.to_string().contains("duplicate hitZone id"));
    }
}
