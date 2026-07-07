use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Instant,
};

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::{
    io::AsyncWriteExt,
    process::Command,
    time::{timeout, Duration},
};
use yuukei_capability::{CapabilityProvider, CapabilityResult, ProviderRegistration};
use yuukei_protocol::{
    new_id, CapabilityInvocation, EventLogRecord, ExecutionLocation,
    ExtensionCapabilityDeclaration, ExtensionEventInvocation, ExtensionEventLogReadPermission,
    ExtensionEventResult, ExtensionEventSubscription, ExtensionHealth, ExtensionHookAction,
    ExtensionHookInvocation, ExtensionHookPoint, ExtensionHookResult, ExtensionHookSubscription,
    ExtensionPermissions, ExtensionRuntimeKind, ExtensionSettingField, ExtensionSettingsSchema,
    ExtensionSignalAlias, ExtensionSummary, JsonMap, RuntimeCommand,
};

#[derive(Debug, Error)]
pub enum ExtensionError {
    #[error("extension already registered: {0}")]
    DuplicateExtension(String),
    #[error("extension must declare at least one hook, event subscription, emitted event, capability, or signal alias: {0}")]
    EmptyDeclaration(String),
    #[error("extension {0} must explicitly declare broadEventSubscription permission to subscribe to all event types")]
    MissingBroadEventPermission(String),
    #[error("extension {extension_id} signal alias points outside its event namespace: {signal}")]
    InvalidSignalAliasNamespace {
        extension_id: String,
        signal: String,
    },
    #[error(
        "extension {extension_id} signal alias target is not declared in emittedEvents: {signal}"
    )]
    SignalAliasNotEmitted {
        extension_id: String,
        signal: String,
    },
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
    #[error("process extension failed: {message}")]
    ProcessFailed {
        extension_id: String,
        display_name: String,
        kind: ProcessFailureKind,
        message: String,
        suspended: bool,
    },
    #[error("process extension is suspended: {message}")]
    ProcessSuspended {
        extension_id: String,
        display_name: String,
        message: String,
    },
}

pub type Result<T> = std::result::Result<T, ExtensionError>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessFailureKind {
    Crash,
    Timeout,
    InvalidJson,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessFailureReport {
    pub extension_id: String,
    pub display_name: String,
    pub kind: ProcessFailureKind,
    pub message: String,
    pub suspended: bool,
    pub suspension_started: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessRuntimeStatus {
    pub health: ExtensionHealth,
    pub failure_count: usize,
    pub suspended: bool,
    pub message: Option<String>,
}

#[derive(Clone, Default)]
pub struct ProcessRuntimeSupervisor {
    states: Arc<Mutex<BTreeMap<String, Arc<Mutex<ProcessRuntimeState>>>>>,
}

#[derive(Debug, Default)]
struct ProcessRuntimeState {
    crash_failures: Vec<Instant>,
    consecutive_kind: Option<ProcessFailureKind>,
    consecutive_count: usize,
    suspended: bool,
    message: Option<String>,
}

const PROCESS_FAILURE_LIMIT: usize = 3;
const PROCESS_CRASH_FAILURE_WINDOW: Duration = Duration::from_secs(30);

impl ProcessRuntimeSupervisor {
    pub fn new() -> Self {
        Self::default()
    }

    fn state_for(&self, extension_id: &str) -> Arc<Mutex<ProcessRuntimeState>> {
        let mut states = self
            .states
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        states
            .entry(extension_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(ProcessRuntimeState::default())))
            .clone()
    }

    pub fn status(&self, extension_id: &str) -> Option<ProcessRuntimeStatus> {
        let states = self.states.lock().ok()?;
        let state = states.get(extension_id)?.lock().ok()?;
        Some(state.status())
    }

    pub fn statuses(&self) -> BTreeMap<String, ProcessRuntimeStatus> {
        let Ok(states) = self.states.lock() else {
            return BTreeMap::new();
        };
        states
            .iter()
            .filter_map(|(extension_id, state)| {
                state
                    .lock()
                    .ok()
                    .map(|state| (extension_id.clone(), state.status()))
            })
            .collect()
    }

    pub fn restart(&self, extension_id: &str) -> bool {
        let Some(state) = self
            .states
            .lock()
            .ok()
            .and_then(|states| states.get(extension_id).cloned())
        else {
            return false;
        };
        let Ok(mut state) = state.lock() else {
            return false;
        };
        state.reset();
        true
    }
}

impl ProcessRuntimeState {
    fn status(&self) -> ProcessRuntimeStatus {
        ProcessRuntimeStatus {
            health: if self.suspended {
                ExtensionHealth::Unavailable
            } else if self.consecutive_count > 0 {
                ExtensionHealth::Degraded
            } else {
                ExtensionHealth::Ready
            },
            failure_count: self.consecutive_count,
            suspended: self.suspended,
            message: self.message.clone(),
        }
    }

    fn reset(&mut self) {
        self.crash_failures.clear();
        self.consecutive_kind = None;
        self.consecutive_count = 0;
        self.suspended = false;
        self.message = None;
    }

    fn record_success(&mut self) {
        self.reset();
    }

    fn record_failure(&mut self, kind: ProcessFailureKind, message: String, now: Instant) -> bool {
        if self.consecutive_kind.as_ref() == Some(&kind) {
            self.consecutive_count += 1;
        } else {
            self.consecutive_kind = Some(kind.clone());
            self.consecutive_count = 1;
        }
        if kind == ProcessFailureKind::Crash {
            self.crash_failures
                .retain(|at| now.duration_since(*at) <= PROCESS_CRASH_FAILURE_WINDOW);
            self.crash_failures.push(now);
            if self.crash_failures.len() >= PROCESS_FAILURE_LIMIT {
                self.suspended = true;
            }
        } else if self.consecutive_count >= PROCESS_FAILURE_LIMIT {
            self.suspended = true;
        }
        self.message = Some(message);
        self.suspended
    }
}

impl ExtensionError {
    pub fn process_failure_report(&self) -> Option<ProcessFailureReport> {
        match self {
            ExtensionError::ProcessFailed {
                extension_id,
                display_name,
                kind,
                message,
                suspended,
            } => Some(ProcessFailureReport {
                extension_id: extension_id.clone(),
                display_name: display_name.clone(),
                kind: kind.clone(),
                message: message.clone(),
                suspended: *suspended,
                suspension_started: *suspended,
            }),
            ExtensionError::ProcessSuspended {
                extension_id,
                display_name,
                message,
            } => Some(ProcessFailureReport {
                extension_id: extension_id.clone(),
                display_name: display_name.clone(),
                kind: ProcessFailureKind::Crash,
                message: message.clone(),
                suspended: true,
                suspension_started: false,
            }),
            _ => None,
        }
    }
}

#[async_trait]
pub trait YuukeiExtension: Send + Sync {
    fn registration(&self) -> ExtensionSummary;
    async fn invoke(&self, invocation: ExtensionHookInvocation) -> Result<ExtensionHookResult>;
    async fn on_event_appended(
        &self,
        _invocation: ExtensionEventInvocation,
    ) -> Result<ExtensionEventResult> {
        Ok(ExtensionEventResult::default())
    }
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
    pub process_failure: Option<ProcessFailureReport>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionEventReport {
    pub invocation: ExtensionEventInvocation,
    pub result: ExtensionEventResult,
    pub error: Option<String>,
    pub process_failure: Option<ProcessFailureReport>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionPipelineResult {
    pub command: RuntimeCommand,
    pub reports: Vec<ExtensionHookReport>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionEventPipelineResult {
    pub reports: Vec<ExtensionEventReport>,
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
        validate_extension_summary(&registration)?;
        if registration.hooks.is_empty()
            && registration.event_subscriptions.is_empty()
            && registration.emitted_events.is_empty()
            && registration.capabilities.is_empty()
            && registration.signal_aliases.is_empty()
        {
            return Err(ExtensionError::EmptyDeclaration(registration.extension_id));
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

    pub fn capability_declarations(&self) -> Vec<(String, ExtensionCapabilityDeclaration)> {
        self.registrations
            .values()
            .filter(|registration| registration.enabled)
            .flat_map(|registration| {
                registration
                    .capabilities
                    .iter()
                    .cloned()
                    .map(|capability| (registration.extension_id.clone(), capability))
            })
            .collect()
    }

    pub fn event_log_read_permission(
        &self,
        extension_id: &str,
    ) -> Option<ExtensionEventLogReadPermission> {
        self.registrations
            .get(extension_id)
            .filter(|registration| registration.enabled)
            .and_then(|registration| registration.permissions.event_log_read.clone())
    }

    pub fn signal_aliases(&self) -> Vec<ExtensionSignalAlias> {
        self.registrations
            .values()
            .filter(|registration| registration.enabled)
            .flat_map(|registration| registration.signal_aliases.clone())
            .collect()
    }

    pub fn can_emit_event(&self, extension_id: &str, event_type: &str) -> bool {
        self.registrations
            .get(extension_id)
            .filter(|registration| registration.enabled)
            .is_some_and(|registration| {
                event_type_matches(&registration.emitted_events, event_type)
            })
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
            let (result, output_command, error, process_failure) =
                match extension.invoke(invocation.clone()).await {
                    Ok(result) => {
                        match apply_hook_result(extension_id, &input_command, result.clone()) {
                            Ok(output_command) => (result, output_command, None, None),
                            Err(error) => (
                                error_result(error.to_string()),
                                input_command.clone(),
                                Some(error.to_string()),
                                None,
                            ),
                        }
                    }
                    Err(error) => {
                        let process_failure = error.process_failure_report();
                        (
                            error_result(error.to_string()),
                            input_command.clone(),
                            Some(error.to_string()),
                            process_failure,
                        )
                    }
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
                process_failure,
            });
        }

        Ok(ExtensionPipelineResult { command, reports })
    }

    pub async fn notify_event_appended(
        &self,
        event: EventLogRecord,
        context: ExtensionEventContext,
    ) -> Result<ExtensionEventPipelineResult> {
        let mut reports = Vec::new();

        for extension_id in self.registrations.keys() {
            let Some(registration) = self.registrations.get(extension_id) else {
                continue;
            };
            if !registration.enabled
                || is_self_emitted_event(&event, extension_id)
                || !matches_on_event_appended(registration, &event.kind)
            {
                continue;
            }
            let Some(extension) = self.extensions.get(extension_id) else {
                continue;
            };

            let invocation = ExtensionEventInvocation {
                id: new_id("ext_evt"),
                extension_id: extension_id.clone(),
                resident_id: event.resident_id.clone(),
                world_pack_id: context.world_pack_id.clone(),
                event: event.clone(),
            };
            let (result, error, process_failure) =
                match extension.on_event_appended(invocation.clone()).await {
                    Ok(result) => (result, None, None),
                    Err(error) => {
                        let process_failure = error.process_failure_report();
                        (
                            ExtensionEventResult {
                                proposed_events: Vec::new(),
                                metadata: Some(JsonMap::from([(
                                    "error".to_string(),
                                    json!(error.to_string()),
                                )])),
                            },
                            Some(error.to_string()),
                            process_failure,
                        )
                    }
                };
            reports.push(ExtensionEventReport {
                invocation,
                result,
                error,
                process_failure,
            });
        }

        Ok(ExtensionEventPipelineResult { reports })
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
pub struct ExtensionEventContext {
    pub world_pack_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionCommandContext {
    pub world_pack_id: String,
}

pub fn event_type_matches(patterns: &[String], event_type: &str) -> bool {
    patterns.iter().any(|pattern| {
        let pattern = pattern.trim();
        if pattern == "*" {
            return true;
        }
        if let Some(prefix) = pattern.strip_suffix('*') {
            return event_type.starts_with(prefix);
        }
        pattern == event_type
    })
}

fn matches_before_command_emit(registration: &ExtensionSummary, command_kind: &str) -> bool {
    registration
        .hooks
        .iter()
        .any(|hook| hook.matches_command(&ExtensionHookPoint::BeforeCommandEmit, command_kind))
}

fn matches_on_event_appended(registration: &ExtensionSummary, event_kind: &str) -> bool {
    registration
        .event_subscriptions
        .iter()
        .any(|subscription| event_type_matches(&subscription.event_types, event_kind))
}

pub fn validate_extension_summary(registration: &ExtensionSummary) -> Result<()> {
    let subscribes_all = registration.event_subscriptions.iter().any(|subscription| {
        subscription
            .event_types
            .iter()
            .any(|event_type| event_type.trim() == "*")
    });
    if subscribes_all && !registration.permissions.broad_event_subscription {
        return Err(ExtensionError::MissingBroadEventPermission(
            registration.extension_id.clone(),
        ));
    }
    let required_prefix = format!("ext.{}.", registration.extension_id);
    for alias in &registration.signal_aliases {
        if !alias.signal.starts_with(&required_prefix) {
            return Err(ExtensionError::InvalidSignalAliasNamespace {
                extension_id: registration.extension_id.clone(),
                signal: alias.signal.clone(),
            });
        }
        if !event_type_matches(&registration.emitted_events, &alias.signal) {
            return Err(ExtensionError::SignalAliasNotEmitted {
                extension_id: registration.extension_id.clone(),
                signal: alias.signal.clone(),
            });
        }
    }
    Ok(())
}

fn is_self_emitted_event(event: &EventLogRecord, extension_id: &str) -> bool {
    event.source == "extension"
        && event
            .payload
            .get("yuukeiExtension")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("extensionId"))
            .and_then(Value::as_str)
            .is_some_and(|source_extension_id| source_extension_id == extension_id)
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
            runtime: ExtensionRuntimeKind::Bundled,
            permissions: ExtensionPermissions::default(),
            hooks: vec![ExtensionHookSubscription {
                hook_point: ExtensionHookPoint::BeforeCommandEmit,
                command_types: vec!["dialogue.say".to_string()],
            }],
            event_subscriptions: Vec::new(),
            emitted_events: Vec::new(),
            capabilities: Vec::new(),
            signal_aliases: Vec::new(),
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
    #[serde(default)]
    pub runtime: Option<ExtensionRuntimeKind>,
    #[serde(default)]
    pub permissions: ExtensionPermissions,
    #[serde(default)]
    pub hooks: Vec<ExtensionHookSubscription>,
    #[serde(default)]
    pub event_subscriptions: Vec<ExtensionEventSubscription>,
    #[serde(default)]
    pub emitted_events: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<ExtensionCapabilityDeclaration>,
    #[serde(default)]
    pub signal_aliases: Vec<ExtensionSignalAlias>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<ExtensionSettingsSchema>,
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

#[derive(Clone)]
pub struct ProcessHookExtension {
    manifest: ProcessExtensionManifest,
    install_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    settings_json: Option<String>,
    enabled: bool,
    runtime_supervisor: ProcessRuntimeSupervisor,
}

impl ProcessHookExtension {
    pub fn from_manifest(manifest: ProcessExtensionManifest) -> Self {
        Self {
            manifest,
            install_dir: None,
            data_dir: None,
            settings_json: None,
            enabled: true,
            runtime_supervisor: ProcessRuntimeSupervisor::new(),
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
            data_dir: None,
            settings_json: None,
            enabled,
            runtime_supervisor: ProcessRuntimeSupervisor::new(),
        }
    }

    pub fn with_data_dir(mut self, data_dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(data_dir.into());
        self
    }

    pub fn with_settings_json(mut self, settings_json: impl Into<String>) -> Self {
        self.settings_json = Some(settings_json.into());
        self
    }

    pub fn with_runtime_supervisor(mut self, supervisor: ProcessRuntimeSupervisor) -> Self {
        self.runtime_supervisor = supervisor;
        self
    }
}

#[async_trait]
impl YuukeiExtension for ProcessHookExtension {
    fn registration(&self) -> ExtensionSummary {
        ExtensionSummary {
            extension_id: self.manifest.id.clone(),
            display_name: self.manifest.display_name.clone(),
            runtime: ExtensionRuntimeKind::Process,
            permissions: self.manifest.permissions.clone(),
            hooks: self.manifest.hooks.clone(),
            event_subscriptions: self.manifest.event_subscriptions.clone(),
            emitted_events: self.manifest.emitted_events.clone(),
            capabilities: self.manifest.capabilities.clone(),
            signal_aliases: self.manifest.signal_aliases.clone(),
            location: ExecutionLocation::DeviceHost,
            enabled: self.enabled,
        }
    }

    async fn invoke(&self, invocation: ExtensionHookInvocation) -> Result<ExtensionHookResult> {
        self.run_process_json(&invocation).await
    }

    async fn on_event_appended(
        &self,
        invocation: ExtensionEventInvocation,
    ) -> Result<ExtensionEventResult> {
        self.run_process_json(&invocation).await
    }
}

#[async_trait]
impl CapabilityProvider for ProcessHookExtension {
    fn registration(&self) -> ProviderRegistration {
        let mut methods = BTreeSet::new();
        let mut required_permissions = BTreeSet::new();
        for capability in &self.manifest.capabilities {
            methods.extend(capability.methods.iter().cloned());
            required_permissions.extend(capability.required_permissions.iter().cloned());
        }
        ProviderRegistration {
            extension_id: self.manifest.id.clone(),
            capabilities: self
                .manifest
                .capabilities
                .iter()
                .map(|capability| capability.capability.clone())
                .collect(),
            methods: methods.into_iter().collect(),
            required_permissions: required_permissions.into_iter().collect(),
            location: ExecutionLocation::DeviceHost,
            health: if self.enabled {
                ExtensionHealth::Ready
            } else {
                ExtensionHealth::Unavailable
            },
            enabled: self.enabled,
            config_schema: JsonMap::new(),
            runtime_settings: self.public_runtime_settings(),
        }
    }

    async fn invoke(
        &self,
        invocation: CapabilityInvocation,
    ) -> yuukei_capability::Result<CapabilityResult> {
        self.run_process_json(&invocation)
            .await
            .map_err(capability_error_from_extension_error)
    }
}

impl ProcessHookExtension {
    fn public_runtime_settings(&self) -> JsonMap {
        let Some(settings_json) = &self.settings_json else {
            return JsonMap::new();
        };
        let Ok(serde_json::Value::Object(settings)) = serde_json::from_str(settings_json) else {
            return JsonMap::new();
        };
        let Some(schema) = &self.manifest.settings else {
            return JsonMap::new();
        };
        settings
            .into_iter()
            .filter(|(key, _)| {
                schema.fields.iter().any(|field| {
                    field.key() == key && !matches!(field, ExtensionSettingField::Secret { .. })
                })
            })
            .collect()
    }
}

impl ProcessHookExtension {
    async fn run_process_json<T, R>(&self, invocation: &T) -> Result<R>
    where
        T: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        self.ensure_not_suspended()?;
        match self.execute_process(invocation).await {
            Ok(output) => match serde_json::from_slice(&output) {
                Ok(result) => {
                    self.record_process_success();
                    Ok(result)
                }
                Err(error) => {
                    Err(self
                        .record_process_failure(ProcessFailureKind::InvalidJson, error.to_string()))
                }
            },
            Err(error) => Err(self.record_raw_process_error(error)),
        }
    }

    async fn execute_process<T>(&self, invocation: &T) -> Result<Vec<u8>>
    where
        T: Serialize + ?Sized,
    {
        let command_path = self.resolved_command_path();
        let mut command = Command::new(command_path);
        command.args(&self.manifest.process.args);
        command.kill_on_drop(true);
        if let Some(data_dir) = &self.data_dir {
            command.env("YUUKEI_EXTENSION_DATA_DIR", data_dir);
        }
        if let Some(settings_json) = &self.settings_json {
            command.env("YUUKEI_EXTENSION_SETTINGS_JSON", settings_json);
        }
        if let Some(cwd) = self.resolved_cwd() {
            command.current_dir(cwd);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&serde_json::to_vec(invocation)?).await?;
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
        Ok(output.stdout)
    }

    fn ensure_not_suspended(&self) -> Result<()> {
        let state = self.runtime_supervisor.state_for(&self.manifest.id);
        let Ok(state) = state.lock() else {
            return Ok(());
        };
        if state.suspended {
            return Err(ExtensionError::ProcessSuspended {
                extension_id: self.manifest.id.clone(),
                display_name: self.manifest.display_name.clone(),
                message: state.message.clone().unwrap_or_else(|| {
                    "このExtensionは連続した失敗により休止しています".to_string()
                }),
            });
        }
        Ok(())
    }

    fn record_process_success(&self) {
        let state = self.runtime_supervisor.state_for(&self.manifest.id);
        match state.lock() {
            Ok(mut state) => state.record_success(),
            Err(error) => error.into_inner().record_success(),
        };
    }

    fn record_raw_process_error(&self, error: ExtensionError) -> ExtensionError {
        match error {
            ExtensionError::ProcessTimeout { timeout_ms } => self.record_process_failure(
                ProcessFailureKind::Timeout,
                format!("process extension timed out after {timeout_ms}ms"),
            ),
            ExtensionError::ProcessExit { status, stderr } => self.record_process_failure(
                ProcessFailureKind::Crash,
                format!("process exited with {status}: {}", stderr.trim()),
            ),
            ExtensionError::ProcessIo(error) => {
                self.record_process_failure(ProcessFailureKind::Crash, error.to_string())
            }
            other => other,
        }
    }

    fn record_process_failure(&self, kind: ProcessFailureKind, message: String) -> ExtensionError {
        let state = self.runtime_supervisor.state_for(&self.manifest.id);
        let suspended = state
            .lock()
            .map(|mut state| state.record_failure(kind.clone(), message.clone(), Instant::now()))
            .unwrap_or(false);
        ExtensionError::ProcessFailed {
            extension_id: self.manifest.id.clone(),
            display_name: self.manifest.display_name.clone(),
            kind,
            message,
            suspended,
        }
    }

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

fn capability_error_from_extension_error(
    error: ExtensionError,
) -> yuukei_capability::CapabilityError {
    if let Some(report) = error.process_failure_report() {
        if report.suspended {
            return yuukei_capability::CapabilityError::ExtensionProcessSuspended {
                extension_id: report.extension_id,
                display_name: report.display_name,
                message: report.message,
                suspension_started: report.suspension_started,
            };
        }
    }
    yuukei_capability::CapabilityError::Extension(error.to_string())
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
    use std::fs;

    fn process_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yuukei-extension-{name}-{}",
            new_id("test").replace(':', "_")
        ));
        fs::create_dir_all(&dir).expect("create process test dir");
        dir
    }

    fn process_manifest(id: &str, script: &str) -> ProcessExtensionManifest {
        ProcessExtensionManifest {
            schema_version: 1,
            id: id.to_string(),
            display_name: id.to_string(),
            runtime: Some(ExtensionRuntimeKind::Process),
            permissions: ExtensionPermissions::default(),
            hooks: vec![ExtensionHookSubscription {
                hook_point: ExtensionHookPoint::BeforeCommandEmit,
                command_types: vec!["dialogue.say".to_string()],
            }],
            event_subscriptions: Vec::new(),
            emitted_events: Vec::new(),
            capabilities: Vec::new(),
            signal_aliases: Vec::new(),
            settings: None,
            process: ProcessCommandSpec {
                command: "node".to_string(),
                args: vec![script.to_string()],
                cwd: None,
                timeout_ms: Some(1_000),
            },
        }
    }

    fn dialogue_command(text: &str) -> RuntimeCommand {
        let mut command = RuntimeCommand::new("dialogue.say", "daihon", "resident-default");
        command.payload.insert("text".to_string(), json!(text));
        command
    }

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
            runtime: None,
            permissions: ExtensionPermissions::default(),
            hooks: vec![ExtensionHookSubscription {
                hook_point: ExtensionHookPoint::BeforeCommandEmit,
                command_types: vec!["dialogue.say".to_string()],
            }],
            event_subscriptions: Vec::new(),
            emitted_events: Vec::new(),
            capabilities: Vec::new(),
            signal_aliases: Vec::new(),
            settings: None,
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
                    runtime: ExtensionRuntimeKind::Bundled,
                    permissions: ExtensionPermissions::default(),
                    hooks: vec![ExtensionHookSubscription {
                        hook_point: ExtensionHookPoint::BeforeCommandEmit,
                        command_types: Vec::new(),
                    }],
                    event_subscriptions: Vec::new(),
                    emitted_events: Vec::new(),
                    capabilities: Vec::new(),
                    signal_aliases: Vec::new(),
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

    #[tokio::test]
    async fn process_extension_crash_is_restarted_on_next_invocation() -> Result<()> {
        let dir = process_test_dir("crash-once");
        fs::write(
            dir.join("crash-once.js"),
            r#"
const fs = require("node:fs");
const marker = "crashed-once";
if (!fs.existsSync(marker)) {
  fs.writeFileSync(marker, "yes");
  process.exit(2);
}
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const command = input.command;
command.payload.text = `${command.payload.text} ok`;
process.stdout.write(JSON.stringify({ action: "replaceCommand", command }));
"#,
        )?;
        let extension = ProcessHookExtension::from_installed_manifest(
            process_manifest("crash-once", "crash-once.js"),
            &dir,
            true,
        );
        let mut registry = ExtensionRegistry::new();
        registry.register(extension)?;
        registry.set_hook_order(
            ExtensionHookPoint::BeforeCommandEmit,
            vec!["crash-once".to_string()],
        );

        let first = registry
            .apply_before_command_emit(
                dialogue_command("hello"),
                ExtensionCommandContext {
                    world_pack_id: "default-yuukei".to_string(),
                },
            )
            .await?;
        assert_eq!(first.command.payload["text"], "hello");
        assert!(first.reports[0].process_failure.is_some());

        let second = registry
            .apply_before_command_emit(
                dialogue_command("hello"),
                ExtensionCommandContext {
                    world_pack_id: "default-yuukei".to_string(),
                },
            )
            .await?;
        assert_eq!(second.command.payload["text"], "hello ok");
        assert!(second.reports[0].process_failure.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn process_extension_suspends_after_three_invalid_json_failures_and_restarts(
    ) -> Result<()> {
        let dir = process_test_dir("invalid-json");
        fs::write(
            dir.join("invalid.js"),
            r#"
process.stdout.write("{not json");
"#,
        )?;
        let supervisor = ProcessRuntimeSupervisor::new();
        let extension = ProcessHookExtension::from_installed_manifest(
            process_manifest("invalid-json", "invalid.js"),
            &dir,
            true,
        )
        .with_runtime_supervisor(supervisor.clone());
        let mut registry = ExtensionRegistry::new();
        registry.register(extension)?;
        registry.set_hook_order(
            ExtensionHookPoint::BeforeCommandEmit,
            vec!["invalid-json".to_string()],
        );

        for index in 0..3 {
            let result = registry
                .apply_before_command_emit(
                    dialogue_command("hello"),
                    ExtensionCommandContext {
                        world_pack_id: "default-yuukei".to_string(),
                    },
                )
                .await?;
            let failure = result.reports[0]
                .process_failure
                .as_ref()
                .expect("process failure");
            assert_eq!(failure.kind, ProcessFailureKind::InvalidJson);
            assert_eq!(failure.suspension_started, index == 2);
        }
        let status = supervisor.status("invalid-json").expect("status");
        assert!(status.suspended);
        assert_eq!(status.failure_count, 3);

        let suspended = registry
            .apply_before_command_emit(
                dialogue_command("hello"),
                ExtensionCommandContext {
                    world_pack_id: "default-yuukei".to_string(),
                },
            )
            .await?;
        assert!(
            !suspended.reports[0]
                .process_failure
                .as_ref()
                .expect("suspended failure")
                .suspension_started
        );

        assert!(supervisor.restart("invalid-json"));
        let status = supervisor.status("invalid-json").expect("status");
        assert!(!status.suspended);
        assert_eq!(status.failure_count, 0);
        Ok(())
    }

    #[test]
    fn validation_rejects_signal_alias_outside_extension_namespace() {
        let summary = ExtensionSummary {
            extension_id: "activity".to_string(),
            display_name: "Activity".to_string(),
            runtime: ExtensionRuntimeKind::Bundled,
            permissions: ExtensionPermissions::default(),
            hooks: Vec::new(),
            event_subscriptions: Vec::new(),
            emitted_events: vec!["ext.activity.*".to_string()],
            capabilities: Vec::new(),
            signal_aliases: vec![ExtensionSignalAlias {
                alias: "会話_別名".to_string(),
                signal: "conversation.text".to_string(),
            }],
            location: ExecutionLocation::ResidentHome,
            enabled: true,
        };

        let error = validate_extension_summary(&summary).unwrap_err();
        assert!(matches!(
            error,
            ExtensionError::InvalidSignalAliasNamespace { .. }
        ));
    }

    #[test]
    fn validation_rejects_signal_alias_not_declared_in_emitted_events() {
        let summary = ExtensionSummary {
            extension_id: "activity".to_string(),
            display_name: "Activity".to_string(),
            runtime: ExtensionRuntimeKind::Bundled,
            permissions: ExtensionPermissions::default(),
            hooks: Vec::new(),
            event_subscriptions: Vec::new(),
            emitted_events: vec!["ext.activity.other".to_string()],
            capabilities: Vec::new(),
            signal_aliases: vec![ExtensionSignalAlias {
                alias: "活動時間_開始".to_string(),
                signal: "ext.activity.active-period.start".to_string(),
            }],
            location: ExecutionLocation::ResidentHome,
            enabled: true,
        };

        let error = validate_extension_summary(&summary).unwrap_err();
        assert!(matches!(
            error,
            ExtensionError::SignalAliasNotEmitted { .. }
        ));
    }
}
