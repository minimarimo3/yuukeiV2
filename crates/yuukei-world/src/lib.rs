use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use thiserror::Error;
use tokio::sync::Mutex;
use yuukei_daihon::{
    has_errors, parse_script, validate_script, ActionHandler, DaihonDiagnostic, DaihonNumber,
    DaihonRuntimeError, DaihonValue, FunctionRegistry, FunctionSpec, InMemorySceneHistory,
    InMemoryVariableStore, InterpretHandler, InterpretRequest, Interpreter, ParamSpec, ParamType,
    RunOptions, Script, Severity as DaihonSeverity, Span, Spanned, Stmt, SystemEvent,
    ValidationMode, INTERPRET_FUNCTION_NAME,
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

#[async_trait]
pub trait DaihonInterpretHandler: Send {
    async fn interpret(&mut self, request: DaihonInterpretRequest) -> String;
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
    history: InMemorySceneHistory,
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
            let mut script = parse_script(&source.source).map_err(|diagnostics| {
                WorldError::Daihon(diagnostic_report(
                    &diagnostics,
                    DaihonDiagnosticPhase::LoadParse,
                    Some(&source.path),
                    None,
                ))
            })?;
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

        let variables = world
            .initial_variables
            .iter()
            .filter_map(|(key, value)| {
                json_to_daihon_value(value).map(|value| (key.clone(), value))
            })
            .collect();
        let mut state = self.state.lock().await;
        state.scripts = scripts;
        state.variables = variables;
        state.history = InMemorySceneHistory::new();
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
            let mut action_handler =
                YuukeiActionHandler::new(event, world.default_actor_id.clone());
            let mut interpret_bridge = YuukeiInterpretBridge { interpret_handler };
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
            commands.extend(action_handler.commands);
        }

        let next_variables = variables.into_values();
        let variable_patches = diff_variable_patches(&previous_variables, &next_variables);
        state.variables = next_variables;

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
    commands: Vec<RuntimeCommand>,
}

struct YuukeiInterpretBridge<'a> {
    interpret_handler: &'a mut dyn DaihonInterpretHandler,
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
}

impl YuukeiActionHandler {
    fn new(event: &RuntimeEvent, default_actor_id: String) -> Self {
        Self {
            event: event.clone(),
            default_actor_id,
            commands: Vec::new(),
        }
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
        self.commands.push(command);
        Ok(())
    }

    async fn call_function(
        &mut self,
        speaker_id: Option<&str>,
        name: &str,
        positional: Vec<DaihonValue>,
        named: BTreeMap<String, DaihonValue>,
    ) -> std::result::Result<DaihonValue, DaihonRuntimeError> {
        let Some(value) = function_value(&positional, &named) else {
            return Ok(DaihonValue::None);
        };
        let actor_id = speaker_id.unwrap_or(&self.default_actor_id).to_string();
        match name {
            "表情" | "expression" => {
                let mut command = self.command("avatar.expression", speaker_id);
                command.payload = JsonMap::from([
                    ("expression".to_string(), Value::String(value)),
                    ("speakerId".to_string(), Value::String(actor_id)),
                    (
                        "sourceFunction".to_string(),
                        Value::String(name.to_string()),
                    ),
                ]);
                self.commands.push(command);
            }
            "動作" | "モーション" | "motion" => {
                let mut command = self.command("avatar.motion", speaker_id);
                command.payload = JsonMap::from([
                    ("motion".to_string(), Value::String(value)),
                    ("speakerId".to_string(), Value::String(actor_id)),
                    (
                        "sourceFunction".to_string(),
                        Value::String(name.to_string()),
                    ),
                ]);
                self.commands.push(command);
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
    async fn yuukei_adapter_ignores_disallowed_signal() -> Result<()> {
        let world = pack();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let event = RuntimeEvent::new("os.file_browser.focused", "device", "resident-default");
        let result = adapter.dispatch(&event, &world).await?;
        assert!(result.commands.is_empty());
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
