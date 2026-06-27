use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
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
    InMemoryVariableStore, Interpreter, ParamSpec, ParamType, RunOptions, Script, Span,
    SystemEvent, ValidationMode,
};
use yuukei_protocol::{Causality, CommandTarget, JsonMap, RuntimeCommand, RuntimeEvent};

#[derive(Debug, Error)]
pub enum WorldError {
    #[error("world pack io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("world pack json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("world pack validation error: {0}")]
    Validation(String),
    #[error("daihon error: {0}")]
    Daihon(String),
}

pub type Result<T> = std::result::Result<T, WorldError>;

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
    #[serde(default)]
    pub profile: JsonMap,
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

#[async_trait]
pub trait DaihonAdapter: Send + Sync {
    async fn load_world(&self, world: &WorldPack) -> Result<()>;
    async fn dispatch(
        &self,
        event: &RuntimeEvent,
        world: &WorldPack,
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
        world.validate()?;
        if !world.daihon.scripts.is_empty() && world.daihon.loaded_scripts.is_empty() {
            return Err(WorldError::Validation(
                "daihon scripts are declared but no script source is loaded".to_string(),
            ));
        }

        let function_registry = yuukei_function_registry();
        let mut scripts = Vec::new();
        for source in &world.daihon.loaded_scripts {
            let script = parse_script(&source.source)
                .map_err(|diagnostics| WorldError::Daihon(format_diagnostics(&diagnostics)))?;
            let diagnostics = validate_script(&script, Some(&function_registry));
            if has_errors(&diagnostics) {
                return Err(WorldError::Daihon(format_diagnostics(&diagnostics)));
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

    async fn dispatch(
        &self,
        event: &RuntimeEvent,
        world: &WorldPack,
    ) -> Result<DaihonDispatchResult> {
        if !world.allows_signal(&event.kind) {
            return Ok(DaihonDispatchResult::default());
        }

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
            let mut interpreter = Interpreter {
                action_handler: &mut action_handler,
                variable_store: &mut variables,
                scene_history: &mut state.history,
                function_registry: &function_registry,
                options: RunOptions {
                    trigger: Some(SystemEvent::new(event.kind.clone(), Span::empty())),
                    default_speaker: Some(world.default_actor_id.clone()),
                    validation_mode: ValidationMode::Strict,
                    ..RunOptions::default()
                },
            };
            let run = interpreter
                .run_script(&loaded.script)
                .await
                .map_err(|error| WorldError::Daihon(error.to_string()))?;
            if has_errors(&run.diagnostics) {
                return Err(WorldError::Daihon(format_diagnostics(&run.diagnostics)));
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
    script.event.name.value == event_kind
        || script.event.scenes.iter().any(|scene| {
            scene
                .metadata
                .signals
                .iter()
                .any(|signal| signal.name.value == event_kind)
        })
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
    registry
}

fn format_diagnostics(diagnostics: &[DaihonDiagnostic]) -> String {
    if diagnostics.is_empty() {
        return "unknown Daihon diagnostic".to_string();
    }
    diagnostics
        .iter()
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>()
        .join("; ")
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
        }
        if !actor_ids.contains(&self.default_actor_id) {
            return Err(WorldError::Validation(format!(
                "defaultActorId is not declared: {}",
                self.default_actor_id
            )));
        }

        let mut signals = BTreeSet::new();
        for signal in &self.signals.allow {
            require_non_empty("signals.allow", signal)?;
            if !signals.insert(signal.clone()) {
                return Err(WorldError::Validation(format!(
                    "duplicate signal: {}",
                    signal
                )));
            }
        }

        Ok(())
    }

    pub fn allows_signal(&self, signal: &str) -> bool {
        self.signals.allow.iter().any(|allowed| allowed == signal)
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
            let path = validate_relative_pack_path(root, script)?;
            sources.push(DaihonScriptSource {
                path: script.clone(),
                source: fs::read_to_string(path)?,
            });
        }
        Ok(sources)
    }
}

fn require_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(WorldError::Validation(format!("{field} must not be empty")));
    }
    Ok(())
}

fn validate_relative_pack_path(root: &Path, path: &str) -> Result<PathBuf> {
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
    use yuukei_protocol::RuntimeEvent;

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
                profile: JsonMap::new(),
            }],
            signals: SignalAllowlist {
                allow: vec!["conversation.text".to_string()],
            },
            capabilities: CapabilityDeclarations {
                required: Vec::new(),
                optional: vec!["speech.synthesis".to_string()],
            },
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
    async fn yuukei_adapter_ignores_disallowed_signal() -> Result<()> {
        let world = pack();
        let adapter = YuukeiDaihonAdapter::default();
        adapter.load_world(&world).await?;
        let event = RuntimeEvent::new("os.file_browser.focused", "device", "resident-default");
        let result = adapter.dispatch(&event, &world).await?;
        assert!(result.commands.is_empty());
        Ok(())
    }
}
