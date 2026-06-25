use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use yuukei_protocol::{Causality, CommandTarget, JsonMap, RuntimeCommand, RuntimeEvent};

#[derive(Debug, Error)]
pub enum WorldError {
    #[error("world pack io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("world pack json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("world pack validation error: {0}")]
    Validation(String),
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
    pub fake_scenes: Vec<FakeScene>,
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
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FakeScene {
    pub id: String,
    pub signal: String,
    pub actor_id: String,
    pub response_template: String,
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

#[async_trait]
pub trait DaihonAdapter: Send + Sync {
    async fn load_world(&self, world: &WorldPack) -> Result<()>;
    async fn dispatch(
        &self,
        event: &RuntimeEvent,
        world: &WorldPack,
    ) -> Result<DaihonDispatchResult>;
}

#[derive(Clone, Debug, Default)]
pub struct FakeDaihonAdapter;

#[async_trait]
impl DaihonAdapter for FakeDaihonAdapter {
    async fn load_world(&self, world: &WorldPack) -> Result<()> {
        world.validate()
    }

    async fn dispatch(
        &self,
        event: &RuntimeEvent,
        world: &WorldPack,
    ) -> Result<DaihonDispatchResult> {
        if !world.allows_signal(&event.kind) {
            return Ok(DaihonDispatchResult::default());
        }

        let scene = world
            .fake_scenes
            .iter()
            .find(|scene| scene.signal == event.kind)
            .or_else(|| {
                world
                    .fake_scenes
                    .iter()
                    .find(|scene| scene.signal == "conversation.text")
            });

        let Some(scene) = scene else {
            return Ok(DaihonDispatchResult::default());
        };

        let text = event
            .payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let response = scene
            .response_template
            .replace("{text}", text)
            .replace("{signal}", &event.kind);

        let mut command = RuntimeCommand::new("dialogue.say", "daihon", event.resident_id.clone());
        command.payload = JsonMap::from([
            ("text".to_string(), Value::String(response)),
            (
                "speakerId".to_string(),
                Value::String(scene.actor_id.clone()),
            ),
            ("emotion".to_string(), Value::String("neutral".to_string())),
        ]);
        command.causality = Some(Causality {
            source_event_id: Some(event.id.clone()),
            source_command_id: None,
            trace_id: event
                .causality
                .as_ref()
                .and_then(|causality| causality.trace_id.clone()),
        });
        command.target = Some(CommandTarget {
            device_id: event.device_id.clone(),
            surface_id: event.surface_id.clone(),
            actor_id: Some(scene.actor_id.clone()),
        });

        Ok(DaihonDispatchResult {
            commands: vec![command],
            executed_scenes: vec![ExecutedScene {
                key: scene.id.clone(),
                event_name: event.kind.clone(),
                scene_name: scene.id.clone(),
            }],
            variable_patches: Vec::new(),
        })
    }
}

impl WorldPack {
    pub fn load_from_dir(path: impl AsRef<Path>) -> Result<Self> {
        let root = path.as_ref();
        let pack_path = root.join("pack.json");
        let raw = fs::read_to_string(pack_path)?;
        let pack: Self = serde_json::from_str(&raw)?;
        pack.validate_paths(root)?;
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

        for scene in &self.fake_scenes {
            require_non_empty("fakeScenes.id", &scene.id)?;
            require_non_empty("fakeScenes.signal", &scene.signal)?;
            if !signals.contains(&scene.signal) {
                return Err(WorldError::Validation(format!(
                    "fake scene uses undeclared signal: {}",
                    scene.signal
                )));
            }
            if !actor_ids.contains(&scene.actor_id) {
                return Err(WorldError::Validation(format!(
                    "fake scene uses unknown actor: {}",
                    scene.actor_id
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

    fn validate_paths(&self, root: &Path) -> Result<()> {
        for script in &self.daihon.scripts {
            validate_relative_pack_path(root, script)?;
        }
        Ok(())
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
    Ok(root.join(relative))
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
            daihon: DaihonConfig::default(),
            fake_scenes: vec![FakeScene {
                id: "echo-conversation".to_string(),
                signal: "conversation.text".to_string(),
                actor_id: "yuukei".to_string(),
                response_template: "聞こえています。「{text}」".to_string(),
            }],
            initial_variables: JsonMap::new(),
            ui_space: JsonMap::new(),
        }
    }

    #[test]
    fn validates_signal_and_actor_references() {
        assert!(pack().validate().is_ok());
        let mut invalid = pack();
        invalid.fake_scenes[0].actor_id = "unknown".to_string();
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

    #[tokio::test]
    async fn fake_adapter_returns_dialogue_say_for_allowed_text() -> Result<()> {
        let world = pack();
        let adapter = FakeDaihonAdapter;
        let mut event = RuntimeEvent::new("conversation.text", "surface", "resident-default");
        event.id = "evt_1".to_string();
        event
            .payload
            .insert("text".to_string(), json!("こんにちは"));

        let result = adapter.dispatch(&event, &world).await?;
        assert_eq!(result.commands.len(), 1);
        assert_eq!(result.commands[0].kind, "dialogue.say");
        assert_eq!(
            result.commands[0].payload["text"],
            "聞こえています。「こんにちは」"
        );
        assert_eq!(result.executed_scenes[0].key, "echo-conversation");
        Ok(())
    }

    #[tokio::test]
    async fn fake_adapter_ignores_disallowed_signal() -> Result<()> {
        let world = pack();
        let adapter = FakeDaihonAdapter;
        let event = RuntimeEvent::new("os.file_browser.focused", "device", "resident-default");
        let result = adapter.dispatch(&event, &world).await?;
        assert!(result.commands.is_empty());
        Ok(())
    }
}
