use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::{
    io::AsyncWriteExt,
    process::Command,
    time::{timeout, Duration},
};
use yuukei_protocol::{
    new_id, ExecutionLocation, ExtensionHookAction, ExtensionHookInvocation, ExtensionHookPoint,
    ExtensionHookResult, ExtensionHookSubscription, ExtensionSummary, JsonMap, RuntimeCommand,
};

#[derive(Debug, Error)]
pub enum ExtensionError {
    #[error("extension already registered: {0}")]
    DuplicateExtension(String),
    #[error("extension must declare at least one hook: {0}")]
    EmptyHooks(String),
    #[error("replaceCommand result must include command")]
    MissingReplacementCommand,
    #[error("extension {extension_id} attempted to replace immutable command field {field}")]
    InvalidReplacement {
        extension_id: String,
        field: &'static str,
    },
    #[error("process extension failed to start or communicate: {0}")]
    ProcessIo(#[from] std::io::Error),
    #[error("process extension exited unsuccessfully: status={status}, stderr={stderr}")]
    ProcessExit { status: String, stderr: String },
    #[error("process extension timed out after {timeout_ms}ms")]
    ProcessTimeout { timeout_ms: u64 },
    #[error("process extension returned invalid json: {0}")]
    ProcessJson(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ExtensionError>;

#[async_trait]
pub trait YuukeiExtension: Send + Sync {
    fn registration(&self) -> ExtensionSummary;
    async fn invoke(&self, invocation: ExtensionHookInvocation) -> Result<ExtensionHookResult>;
}

#[derive(Clone, Default)]
pub struct ExtensionRegistry {
    extensions: BTreeMap<String, Arc<dyn YuukeiExtension>>,
    registrations: BTreeMap<String, ExtensionSummary>,
    hook_order: BTreeMap<ExtensionHookPoint, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionHookReport {
    pub invocation: ExtensionHookInvocation,
    pub result: ExtensionHookResult,
    pub input_command: RuntimeCommand,
    pub output_command: RuntimeCommand,
    pub changed: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionPipelineResult {
    pub command: RuntimeCommand,
    pub reports: Vec<ExtensionHookReport>,
}

impl ExtensionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<E>(&mut self, extension: E) -> Result<()>
    where
        E: YuukeiExtension + 'static,
    {
        let registration = extension.registration();
        if registration.hooks.is_empty() {
            return Err(ExtensionError::EmptyHooks(registration.extension_id));
        }
        if self.extensions.contains_key(&registration.extension_id) {
            return Err(ExtensionError::DuplicateExtension(
                registration.extension_id,
            ));
        }

        self.extensions
            .insert(registration.extension_id.clone(), Arc::new(extension));
        self.registrations
            .insert(registration.extension_id.clone(), registration);
        Ok(())
    }

    pub fn set_hook_order(&mut self, hook_point: ExtensionHookPoint, extension_ids: Vec<String>) {
        let mut seen = BTreeSet::new();
        let order = extension_ids
            .into_iter()
            .filter(|extension_id| {
                self.registrations.contains_key(extension_id) && seen.insert(extension_id.clone())
            })
            .collect::<Vec<_>>();
        self.hook_order.insert(hook_point, order);
    }

    pub fn summaries(&self) -> BTreeMap<String, ExtensionSummary> {
        self.registrations.clone()
    }

    pub async fn apply_before_command_emit(
        &self,
        command: RuntimeCommand,
        context: ExtensionCommandContext,
    ) -> Result<ExtensionPipelineResult> {
        let mut command = command;
        let mut reports = Vec::new();

        for extension_id in self.ordered_extension_ids(&ExtensionHookPoint::BeforeCommandEmit) {
            let Some(registration) = self.registrations.get(extension_id) else {
                continue;
            };
            if !registration.enabled || !matches_before_command_emit(registration, &command.kind) {
                continue;
            }
            let Some(extension) = self.extensions.get(extension_id) else {
                continue;
            };

            let input_command = command.clone();
            let invocation = ExtensionHookInvocation {
                id: new_id("hook"),
                hook_point: ExtensionHookPoint::BeforeCommandEmit,
                extension_id: extension_id.clone(),
                resident_id: command.resident_id.clone(),
                world_pack_id: context.world_pack_id.clone(),
                command: command.clone(),
            };
            let (result, output_command, error) = match extension.invoke(invocation.clone()).await {
                Ok(result) => match apply_hook_result(extension_id, &input_command, result.clone())
                {
                    Ok(output_command) => (result, output_command, None),
                    Err(error) => (
                        error_result(error.to_string()),
                        input_command.clone(),
                        Some(error.to_string()),
                    ),
                },
                Err(error) => (
                    error_result(error.to_string()),
                    input_command.clone(),
                    Some(error.to_string()),
                ),
            };
            let changed = output_command != input_command;

            command = output_command.clone();
            reports.push(ExtensionHookReport {
                invocation,
                result,
                input_command,
                output_command,
                changed,
                error,
            });
        }

        Ok(ExtensionPipelineResult { command, reports })
    }

    fn ordered_extension_ids(&self, hook_point: &ExtensionHookPoint) -> Vec<&String> {
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();
        if let Some(configured_order) = self.hook_order.get(hook_point) {
            for extension_id in configured_order {
                if self.registrations.contains_key(extension_id) && seen.insert(extension_id) {
                    ordered.push(extension_id);
                }
            }
        }
        for extension_id in self.registrations.keys() {
            if seen.insert(extension_id) {
                ordered.push(extension_id);
            }
        }
        ordered
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionCommandContext {
    pub world_pack_id: String,
}

fn matches_before_command_emit(registration: &ExtensionSummary, command_kind: &str) -> bool {
    registration
        .hooks
        .iter()
        .any(|hook| hook.matches_command(&ExtensionHookPoint::BeforeCommandEmit, command_kind))
}

fn apply_hook_result(
    extension_id: &str,
    input: &RuntimeCommand,
    result: ExtensionHookResult,
) -> Result<RuntimeCommand> {
    match result.action {
        ExtensionHookAction::Unchanged => Ok(input.clone()),
        ExtensionHookAction::ReplaceCommand => {
            let Some(output) = result.command else {
                return Err(ExtensionError::MissingReplacementCommand);
            };
            validate_replacement(extension_id, input, &output)?;
            Ok(output)
        }
    }
}

fn validate_replacement(
    extension_id: &str,
    input: &RuntimeCommand,
    output: &RuntimeCommand,
) -> Result<()> {
    for (field, before, after) in [
        ("id", input.id.as_str(), output.id.as_str()),
        ("type", input.kind.as_str(), output.kind.as_str()),
        (
            "residentId",
            input.resident_id.as_str(),
            output.resident_id.as_str(),
        ),
    ] {
        if before != after {
            return Err(ExtensionError::InvalidReplacement {
                extension_id: extension_id.to_string(),
                field,
            });
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogueSuffixExtension {
    extension_id: String,
    display_name: String,
    suffix: String,
}

impl DialogueSuffixExtension {
    pub fn new(extension_id: impl Into<String>, suffix: impl Into<String>) -> Self {
        let extension_id = extension_id.into();
        Self {
            display_name: extension_id.clone(),
            extension_id,
            suffix: suffix.into(),
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = display_name.into();
        self
    }
}

#[async_trait]
impl YuukeiExtension for DialogueSuffixExtension {
    fn registration(&self) -> ExtensionSummary {
        ExtensionSummary {
            extension_id: self.extension_id.clone(),
            display_name: self.display_name.clone(),
            hooks: vec![ExtensionHookSubscription {
                hook_point: ExtensionHookPoint::BeforeCommandEmit,
                command_types: vec!["dialogue.say".to_string()],
            }],
            location: ExecutionLocation::ResidentHome,
            enabled: true,
        }
    }

    async fn invoke(&self, invocation: ExtensionHookInvocation) -> Result<ExtensionHookResult> {
        let Some(text) = invocation
            .command
            .payload
            .get("text")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        else {
            return Ok(unchanged_result());
        };
        if text.is_empty() || text.ends_with(&self.suffix) {
            return Ok(unchanged_result());
        }

        let mut command = invocation.command;
        command.payload.insert(
            "text".to_string(),
            Value::String(format!("{text}{}", self.suffix)),
        );
        Ok(ExtensionHookResult {
            action: ExtensionHookAction::ReplaceCommand,
            command: Some(command),
            metadata: Some(JsonMap::from([("suffix".to_string(), json!(self.suffix))])),
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessExtensionManifest {
    pub schema_version: u32,
    pub id: String,
    pub display_name: String,
    pub hooks: Vec<ExtensionHookSubscription>,
    pub process: ProcessCommandSpec,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessCommandSpec {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProcessHookExtension {
    manifest: ProcessExtensionManifest,
    install_dir: Option<PathBuf>,
    enabled: bool,
}

impl ProcessHookExtension {
    pub fn from_manifest(manifest: ProcessExtensionManifest) -> Self {
        Self {
            manifest,
            install_dir: None,
            enabled: true,
        }
    }

    pub fn from_installed_manifest(
        manifest: ProcessExtensionManifest,
        install_dir: impl Into<PathBuf>,
        enabled: bool,
    ) -> Self {
        Self {
            manifest,
            install_dir: Some(install_dir.into()),
            enabled,
        }
    }
}

#[async_trait]
impl YuukeiExtension for ProcessHookExtension {
    fn registration(&self) -> ExtensionSummary {
        ExtensionSummary {
            extension_id: self.manifest.id.clone(),
            display_name: self.manifest.display_name.clone(),
            hooks: self.manifest.hooks.clone(),
            location: ExecutionLocation::DeviceHost,
            enabled: self.enabled,
        }
    }

    async fn invoke(&self, invocation: ExtensionHookInvocation) -> Result<ExtensionHookResult> {
        let command_path = self.resolved_command_path();
        let mut command = Command::new(command_path);
        command.args(&self.manifest.process.args);
        command.kill_on_drop(true);
        if let Some(cwd) = self.resolved_cwd() {
            command.current_dir(cwd);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&serde_json::to_vec(&invocation)?).await?;
            stdin.write_all(b"\n").await?;
        }

        let timeout_ms = self.manifest.process.timeout_ms.unwrap_or(5_000);
        let output = timeout(Duration::from_millis(timeout_ms), child.wait_with_output())
            .await
            .map_err(|_| ExtensionError::ProcessTimeout { timeout_ms })??;
        if !output.status.success() {
            return Err(ExtensionError::ProcessExit {
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let result = serde_json::from_slice(&output.stdout)?;
        Ok(result)
    }
}

impl ProcessHookExtension {
    fn resolved_command_path(&self) -> PathBuf {
        let command = PathBuf::from(&self.manifest.process.command);
        if command.is_absolute() || command.components().count() == 1 {
            return command;
        }
        self.install_dir
            .as_ref()
            .map(|install_dir| install_dir.join(&command))
            .unwrap_or(command)
    }

    fn resolved_cwd(&self) -> Option<PathBuf> {
        let Some(install_dir) = &self.install_dir else {
            return self.manifest.process.cwd.as_ref().map(PathBuf::from);
        };
        match &self.manifest.process.cwd {
            Some(cwd) => {
                let cwd = Path::new(cwd);
                if cwd.is_absolute() {
                    Some(cwd.to_path_buf())
                } else {
                    Some(install_dir.join(cwd))
                }
            }
            None => Some(install_dir.clone()),
        }
    }
}

fn unchanged_result() -> ExtensionHookResult {
    ExtensionHookResult {
        action: ExtensionHookAction::Unchanged,
        command: None,
        metadata: None,
    }
}

fn error_result(message: String) -> ExtensionHookResult {
    ExtensionHookResult {
        action: ExtensionHookAction::Unchanged,
        command: None,
        metadata: Some(JsonMap::from([("error".to_string(), json!(message))])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn suffix_extension_updates_dialogue_command() -> Result<()> {
        let mut command = RuntimeCommand::new("dialogue.say", "daihon", "resident-default");
        command
            .payload
            .insert("text".to_string(), json!("こんにちは"));

        let mut registry = ExtensionRegistry::new();
        registry.register(DialogueSuffixExtension::new("nya-suffix", "にゃ"))?;
        let result = registry
            .apply_before_command_emit(
                command,
                ExtensionCommandContext {
                    world_pack_id: "default-yuukei".to_string(),
                },
            )
            .await?;

        assert_eq!(result.command.payload["text"], "こんにちはにゃ");
        assert_eq!(result.reports.len(), 1);
        assert!(result.reports[0].changed);
        assert_eq!(
            result.reports[0].invocation.hook_point,
            ExtensionHookPoint::BeforeCommandEmit
        );
        Ok(())
    }

    #[tokio::test]
    async fn registry_uses_configured_hook_order() -> Result<()> {
        let mut command = RuntimeCommand::new("dialogue.say", "daihon", "resident-default");
        command
            .payload
            .insert("text".to_string(), json!("こんにちは"));

        let mut registry = ExtensionRegistry::new();
        registry.register(DialogueSuffixExtension::new("nya-suffix", "にゃ"))?;
        registry.register(DialogueSuffixExtension::new("english-marker", " EN"))?;
        registry.set_hook_order(
            ExtensionHookPoint::BeforeCommandEmit,
            vec!["english-marker".to_string(), "nya-suffix".to_string()],
        );

        let result = registry
            .apply_before_command_emit(
                command,
                ExtensionCommandContext {
                    world_pack_id: "default-yuukei".to_string(),
                },
            )
            .await?;

        assert_eq!(result.command.payload["text"], "こんにちは ENにゃ");
        assert_eq!(
            result
                .reports
                .iter()
                .map(|report| report.invocation.extension_id.as_str())
                .collect::<Vec<_>>(),
            vec!["english-marker", "nya-suffix"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn reversing_hook_order_changes_pipeline_output() -> Result<()> {
        let mut command = RuntimeCommand::new("dialogue.say", "daihon", "resident-default");
        command
            .payload
            .insert("text".to_string(), json!("こんにちは"));

        let mut registry = ExtensionRegistry::new();
        registry.register(DialogueSuffixExtension::new("nya-suffix", "にゃ"))?;
        registry.register(DialogueSuffixExtension::new("english-marker", " EN"))?;
        registry.set_hook_order(
            ExtensionHookPoint::BeforeCommandEmit,
            vec!["nya-suffix".to_string(), "english-marker".to_string()],
        );

        let result = registry
            .apply_before_command_emit(
                command,
                ExtensionCommandContext {
                    world_pack_id: "default-yuukei".to_string(),
                },
            )
            .await?;

        assert_eq!(result.command.payload["text"], "こんにちはにゃ EN");
        Ok(())
    }

    #[tokio::test]
    async fn disabled_extension_is_preserved_in_summaries_but_skipped() -> Result<()> {
        let mut command = RuntimeCommand::new("dialogue.say", "daihon", "resident-default");
        command
            .payload
            .insert("text".to_string(), json!("こんにちは"));
        let manifest = ProcessExtensionManifest {
            schema_version: 1,
            id: "disabled-process".to_string(),
            display_name: "Disabled Process".to_string(),
            hooks: vec![ExtensionHookSubscription {
                hook_point: ExtensionHookPoint::BeforeCommandEmit,
                command_types: vec!["dialogue.say".to_string()],
            }],
            process: ProcessCommandSpec {
                command: "missing-extension-command".to_string(),
                args: Vec::new(),
                cwd: None,
                timeout_ms: None,
            },
        };

        let mut registry = ExtensionRegistry::new();
        registry.register(ProcessHookExtension::from_installed_manifest(
            manifest, ".", false,
        ))?;
        let result = registry
            .apply_before_command_emit(
                command.clone(),
                ExtensionCommandContext {
                    world_pack_id: "default-yuukei".to_string(),
                },
            )
            .await?;

        assert_eq!(result.command, command);
        assert!(result.reports.is_empty());
        assert!(!registry.summaries()["disabled-process"].enabled);
        Ok(())
    }

    #[tokio::test]
    async fn registry_reports_invalid_replacement_and_keeps_original_command() -> Result<()> {
        #[derive(Clone)]
        struct BadExtension;

        #[async_trait]
        impl YuukeiExtension for BadExtension {
            fn registration(&self) -> ExtensionSummary {
                ExtensionSummary {
                    extension_id: "bad".to_string(),
                    display_name: "Bad".to_string(),
                    hooks: vec![ExtensionHookSubscription {
                        hook_point: ExtensionHookPoint::BeforeCommandEmit,
                        command_types: Vec::new(),
                    }],
                    location: ExecutionLocation::ResidentHome,
                    enabled: true,
                }
            }

            async fn invoke(
                &self,
                invocation: ExtensionHookInvocation,
            ) -> Result<ExtensionHookResult> {
                let mut command = invocation.command;
                command.id = "cmd_other".to_string();
                Ok(ExtensionHookResult {
                    action: ExtensionHookAction::ReplaceCommand,
                    command: Some(command),
                    metadata: None,
                })
            }
        }

        let command = RuntimeCommand::new("dialogue.say", "daihon", "resident-default");
        let mut registry = ExtensionRegistry::new();
        registry.register(BadExtension)?;
        let result = registry
            .apply_before_command_emit(
                command.clone(),
                ExtensionCommandContext {
                    world_pack_id: "default-yuukei".to_string(),
                },
            )
            .await?;

        assert_eq!(result.command.id, command.id);
        assert_eq!(result.reports.len(), 1);
        assert!(!result.reports[0].changed);
        assert!(result.reports[0]
            .error
            .as_deref()
            .is_some_and(|message| message.contains("immutable command field id")));
        Ok(())
    }
}
