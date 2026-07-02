use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use yuukei_capability::{DEFAULT_SPEECH_SYNTHESIS_EXTENSION_ID, SPEECH_SYNTHESIS_CAPABILITY};
use yuukei_extension::{
    validate_extension_summary, ProcessExtensionManifest, ProcessHookExtension, YuukeiExtension,
};
use yuukei_protocol::{
    now_timestamp, ExtensionCapabilityDeclaration, ExtensionEventSubscription, ExtensionHookPoint,
    ExtensionHookSubscription, ExtensionPermissions, ExtensionRuntimeKind, ExtensionSignalAlias,
    ResidentSnapshot,
};

use crate::{DeviceHostError, Result};

const EXTENSION_SETTINGS_SCHEMA_VERSION: u32 = 1;
const EXTENSION_MANIFEST_FILE: &str = "manifest.json";

pub const TRUSTED_CODE_NOTICE: &str = "Extensionは信頼したローカルコードとして実行されます。Yuukeiは公開protocolへの入力と出力を検証しますが、OSレベルのファイルアクセス隔離はv1では行いません。";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSettingsState {
    pub installed: Vec<InstalledExtension>,
    pub hook_order: BTreeMap<ExtensionHookPoint, Vec<String>>,
    pub capability_defaults: BTreeMap<String, String>,
    pub settings_path: PathBuf,
    pub extension_root: PathBuf,
    pub trusted_code_notice: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledExtension {
    pub extension_id: String,
    pub display_name: String,
    pub enabled: bool,
    pub runtime: ExtensionRuntimeKind,
    pub permissions: ExtensionPermissions,
    pub hooks: Vec<ExtensionHookSubscription>,
    pub event_subscriptions: Vec<ExtensionEventSubscription>,
    pub emitted_events: Vec<String>,
    pub capabilities: Vec<ExtensionCapabilityDeclaration>,
    pub signal_aliases: Vec<ExtensionSignalAlias>,
    pub installed_path: PathBuf,
    pub manifest_path: PathBuf,
    pub installed_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_load_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSettingsChangeResult {
    pub state: ExtensionSettingsState,
    pub snapshot: ResidentSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredExtensionSettings {
    schema_version: u32,
    #[serde(default)]
    installed_extensions: Vec<StoredInstalledExtension>,
    #[serde(default)]
    hook_order: BTreeMap<ExtensionHookPoint, Vec<String>>,
    #[serde(default)]
    capability_defaults: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredInstalledExtension {
    extension_id: String,
    enabled: bool,
    installed_path: PathBuf,
    installed_at: String,
    updated_at: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ExtensionRuntimeInstall {
    pub extension_id: String,
    pub manifest: ProcessExtensionManifest,
    pub enabled: bool,
    pub install_dir: PathBuf,
    pub manifest_path: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct ExtensionRuntimeLoadError {
    pub extension_id: String,
    pub manifest_path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug)]
pub(crate) enum ExtensionRuntimeEntry {
    Ready(Box<ExtensionRuntimeInstall>),
    Error(ExtensionRuntimeLoadError),
}

#[derive(Clone, Debug)]
pub struct ExtensionSettingsRegistry {
    settings_path: PathBuf,
    extension_root: PathBuf,
    stored: StoredExtensionSettings,
}

impl ExtensionSettingsRegistry {
    pub fn open(data_dir: impl AsRef<Path>, extension_root: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref();
        let settings_path = data_dir.join("settings").join("extensions.json");
        let extension_root = extension_root.as_ref().to_path_buf();
        fs::create_dir_all(&extension_root)?;
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let stored = if settings_path.exists() {
            let raw = fs::read_to_string(&settings_path)?;
            serde_json::from_str(&raw)?
        } else {
            StoredExtensionSettings {
                schema_version: EXTENSION_SETTINGS_SCHEMA_VERSION,
                installed_extensions: Vec::new(),
                hook_order: BTreeMap::new(),
                capability_defaults: BTreeMap::new(),
            }
        };
        if stored.schema_version != EXTENSION_SETTINGS_SCHEMA_VERSION {
            return Err(DeviceHostError::ExtensionSettings(format!(
                "unsupported extension settings schemaVersion: {}",
                stored.schema_version
            )));
        }
        let mut registry = Self {
            settings_path,
            extension_root,
            stored,
        };
        registry.normalize();
        registry.save()?;
        Ok(registry)
    }

    pub fn state(&self) -> ExtensionSettingsState {
        let mut installed = self
            .ordered_installed_extensions()
            .into_iter()
            .map(|stored| self.installed_extension_state(stored))
            .collect::<Vec<_>>();
        installed.sort_by_key(|extension| {
            self.order_index(
                &ExtensionHookPoint::BeforeCommandEmit,
                &extension.extension_id,
            )
            .unwrap_or(usize::MAX)
        });
        ExtensionSettingsState {
            installed,
            hook_order: self.stored.hook_order.clone(),
            capability_defaults: self.stored.capability_defaults.clone(),
            settings_path: self.settings_path.clone(),
            extension_root: self.extension_root.clone(),
            trusted_code_notice: TRUSTED_CODE_NOTICE.to_string(),
        }
    }

    pub fn install_from_directory(
        &mut self,
        source_dir: impl AsRef<Path>,
    ) -> Result<ExtensionSettingsState> {
        let source_dir = fs::canonicalize(source_dir.as_ref())?;
        if !source_dir.is_dir() {
            return Err(DeviceHostError::ExtensionSettings(
                "extension install source must be a directory".to_string(),
            ));
        }
        let source_manifest_path = source_dir.join(EXTENSION_MANIFEST_FILE);
        let manifest = read_manifest(&source_manifest_path)?;
        validate_manifest(&manifest)?;
        validate_extension_id(&manifest.id)?;

        let install_dir = self.extension_root.join(&manifest.id);
        if install_dir.starts_with(&source_dir) && install_dir != source_dir {
            return Err(DeviceHostError::ExtensionSettings(
                "extension install destination cannot be inside the source directory".to_string(),
            ));
        }
        let source_is_install_dir = fs::canonicalize(&install_dir)
            .map(|canonical_install_dir| canonical_install_dir == source_dir)
            .unwrap_or(false);
        if !source_is_install_dir {
            if install_dir.exists() {
                fs::remove_dir_all(&install_dir)?;
            }
            copy_dir_recursively(&source_dir, &install_dir)?;
        }

        let now = now_timestamp();
        let installed_at = self
            .stored
            .installed_extensions
            .iter()
            .find(|extension| extension.extension_id == manifest.id)
            .map(|extension| extension.installed_at.clone())
            .unwrap_or_else(|| now.clone());
        let enabled = self
            .stored
            .installed_extensions
            .iter()
            .find(|extension| extension.extension_id == manifest.id)
            .map(|extension| extension.enabled)
            .unwrap_or(true);
        self.stored
            .installed_extensions
            .retain(|extension| extension.extension_id != manifest.id);
        self.stored
            .installed_extensions
            .push(StoredInstalledExtension {
                extension_id: manifest.id.clone(),
                enabled,
                installed_path: install_dir,
                installed_at,
                updated_at: now,
            });
        self.append_manifest_hooks_to_order(&manifest);
        self.normalize();
        self.save()?;
        Ok(self.state())
    }

    pub fn uninstall(&mut self, extension_id: &str) -> Result<ExtensionSettingsState> {
        validate_extension_id(extension_id)?;
        self.stored
            .installed_extensions
            .retain(|extension| extension.extension_id != extension_id);
        for order in self.stored.hook_order.values_mut() {
            order.retain(|candidate| candidate != extension_id);
        }
        let install_dir = self.extension_root.join(extension_id);
        if install_dir.exists() {
            fs::remove_dir_all(install_dir)?;
        }
        self.normalize();
        self.save()?;
        Ok(self.state())
    }

    pub fn set_enabled(
        &mut self,
        extension_id: &str,
        enabled: bool,
    ) -> Result<ExtensionSettingsState> {
        let Some(extension) = self
            .stored
            .installed_extensions
            .iter_mut()
            .find(|extension| extension.extension_id == extension_id)
        else {
            return Err(DeviceHostError::ExtensionSettings(format!(
                "extension is not installed: {extension_id}"
            )));
        };
        extension.enabled = enabled;
        extension.updated_at = now_timestamp();
        self.save()?;
        Ok(self.state())
    }

    pub fn set_hook_order(
        &mut self,
        hook_point: ExtensionHookPoint,
        extension_ids: Vec<String>,
    ) -> Result<ExtensionSettingsState> {
        let known_ids = self
            .stored
            .installed_extensions
            .iter()
            .map(|extension| extension.extension_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut seen = BTreeSet::new();
        let mut order = extension_ids
            .into_iter()
            .filter(|extension_id| {
                known_ids.contains(extension_id.as_str()) && seen.insert(extension_id.clone())
            })
            .collect::<Vec<_>>();

        for stored in &self.stored.installed_extensions {
            if seen.contains(&stored.extension_id) {
                continue;
            }
            if self
                .read_manifest_for(stored)
                .ok()
                .is_some_and(|manifest| manifest_subscribes_to(&manifest, &hook_point))
            {
                seen.insert(stored.extension_id.clone());
                order.push(stored.extension_id.clone());
            }
        }

        self.stored.hook_order.insert(hook_point, order);
        self.normalize();
        self.save()?;
        Ok(self.state())
    }

    pub fn set_capability_default(
        &mut self,
        capability: &str,
        extension_id: &str,
    ) -> Result<ExtensionSettingsState> {
        validate_capability_name(capability)?;
        validate_extension_id(extension_id)?;
        self.validate_capability_default_target(capability, extension_id)?;
        self.stored
            .capability_defaults
            .insert(capability.to_string(), extension_id.to_string());
        self.normalize();
        self.save()?;
        Ok(self.state())
    }

    pub(crate) fn runtime_entries(&self) -> Vec<ExtensionRuntimeEntry> {
        self.ordered_installed_extensions()
            .into_iter()
            .map(|stored| {
                let manifest_path = self.manifest_path(&stored.extension_id);
                match self.read_manifest_for(stored) {
                    Ok(manifest) => {
                        ExtensionRuntimeEntry::Ready(Box::new(ExtensionRuntimeInstall {
                            extension_id: stored.extension_id.clone(),
                            manifest,
                            enabled: stored.enabled,
                            install_dir: self.install_dir(&stored.extension_id),
                            manifest_path,
                        }))
                    }
                    Err(error) => ExtensionRuntimeEntry::Error(ExtensionRuntimeLoadError {
                        extension_id: stored.extension_id.clone(),
                        manifest_path,
                        message: error.to_string(),
                    }),
                }
            })
            .collect()
    }

    pub(crate) fn hook_order(&self, hook_point: &ExtensionHookPoint) -> Vec<String> {
        self.stored
            .hook_order
            .get(hook_point)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn capability_defaults(&self) -> BTreeMap<String, String> {
        self.stored.capability_defaults.clone()
    }

    fn normalize(&mut self) {
        let mut seen = BTreeSet::new();
        self.stored
            .installed_extensions
            .retain(|extension| seen.insert(extension.extension_id.clone()));
        for extension in &mut self.stored.installed_extensions {
            extension.installed_path = self.extension_root.join(&extension.extension_id);
        }
        for hook_point in [ExtensionHookPoint::BeforeCommandEmit] {
            let current = self
                .stored
                .hook_order
                .remove(&hook_point)
                .unwrap_or_default();
            let mut next = Vec::new();
            let mut ordered = BTreeSet::new();
            let installed_ids = self
                .stored
                .installed_extensions
                .iter()
                .map(|extension| extension.extension_id.as_str())
                .collect::<BTreeSet<_>>();
            for extension_id in current {
                if installed_ids.contains(extension_id.as_str())
                    && ordered.insert(extension_id.clone())
                {
                    next.push(extension_id);
                }
            }
            for stored in &self.stored.installed_extensions {
                if ordered.contains(&stored.extension_id) {
                    continue;
                }
                if self
                    .read_manifest_for(stored)
                    .ok()
                    .is_some_and(|manifest| manifest_subscribes_to(&manifest, &hook_point))
                {
                    ordered.insert(stored.extension_id.clone());
                    next.push(stored.extension_id.clone());
                }
            }
            self.stored.hook_order.insert(hook_point, next);
        }
        let extension_capabilities = self
            .stored
            .installed_extensions
            .iter()
            .filter_map(|stored| {
                self.read_manifest_for(stored).ok().map(|manifest| {
                    (
                        stored.extension_id.clone(),
                        manifest
                            .capabilities
                            .into_iter()
                            .map(|capability| capability.capability)
                            .collect::<BTreeSet<_>>(),
                    )
                })
            })
            .collect::<BTreeMap<_, _>>();
        self.stored
            .capability_defaults
            .retain(|capability, extension_id| {
                if extension_id == DEFAULT_SPEECH_SYNTHESIS_EXTENSION_ID {
                    return capability == SPEECH_SYNTHESIS_CAPABILITY;
                }
                extension_capabilities
                    .get(extension_id)
                    .is_some_and(|capabilities| capabilities.contains(capability))
            });
    }

    fn append_manifest_hooks_to_order(&mut self, manifest: &ProcessExtensionManifest) {
        for hook in &manifest.hooks {
            let order = self
                .stored
                .hook_order
                .entry(hook.hook_point.clone())
                .or_default();
            if !order
                .iter()
                .any(|extension_id| extension_id == &manifest.id)
            {
                order.push(manifest.id.clone());
            }
        }
    }

    fn ordered_installed_extensions(&self) -> Vec<&StoredInstalledExtension> {
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();
        if let Some(before_command_emit) = self
            .stored
            .hook_order
            .get(&ExtensionHookPoint::BeforeCommandEmit)
        {
            for extension_id in before_command_emit {
                if seen.insert(extension_id.as_str()) {
                    if let Some(stored) = self
                        .stored
                        .installed_extensions
                        .iter()
                        .find(|extension| extension.extension_id == *extension_id)
                    {
                        ordered.push(stored);
                    }
                }
            }
        }
        for stored in &self.stored.installed_extensions {
            if seen.insert(stored.extension_id.as_str()) {
                ordered.push(stored);
            }
        }
        ordered
    }

    fn installed_extension_state(&self, stored: &StoredInstalledExtension) -> InstalledExtension {
        let manifest_path = self.manifest_path(&stored.extension_id);
        match self.read_manifest_for(stored) {
            Ok(manifest) => InstalledExtension {
                extension_id: stored.extension_id.clone(),
                display_name: manifest.display_name,
                enabled: stored.enabled,
                runtime: manifest.runtime.unwrap_or(ExtensionRuntimeKind::Process),
                permissions: manifest.permissions,
                hooks: manifest.hooks,
                event_subscriptions: manifest.event_subscriptions,
                emitted_events: manifest.emitted_events,
                capabilities: manifest.capabilities,
                signal_aliases: manifest.signal_aliases,
                installed_path: self.install_dir(&stored.extension_id),
                manifest_path,
                installed_at: stored.installed_at.clone(),
                updated_at: stored.updated_at.clone(),
                last_load_error: None,
            },
            Err(error) => InstalledExtension {
                extension_id: stored.extension_id.clone(),
                display_name: stored.extension_id.clone(),
                enabled: stored.enabled,
                runtime: ExtensionRuntimeKind::Process,
                permissions: ExtensionPermissions::default(),
                hooks: Vec::new(),
                event_subscriptions: Vec::new(),
                emitted_events: Vec::new(),
                capabilities: Vec::new(),
                signal_aliases: Vec::new(),
                installed_path: self.install_dir(&stored.extension_id),
                manifest_path,
                installed_at: stored.installed_at.clone(),
                updated_at: stored.updated_at.clone(),
                last_load_error: Some(error.to_string()),
            },
        }
    }

    fn order_index(&self, hook_point: &ExtensionHookPoint, extension_id: &str) -> Option<usize> {
        self.stored
            .hook_order
            .get(hook_point)
            .and_then(|order| order.iter().position(|candidate| candidate == extension_id))
    }

    fn install_dir(&self, extension_id: &str) -> PathBuf {
        self.extension_root.join(extension_id)
    }

    fn manifest_path(&self, extension_id: &str) -> PathBuf {
        self.install_dir(extension_id).join(EXTENSION_MANIFEST_FILE)
    }

    fn read_manifest_for(
        &self,
        stored: &StoredInstalledExtension,
    ) -> Result<ProcessExtensionManifest> {
        let manifest = read_manifest(self.manifest_path(&stored.extension_id))?;
        validate_manifest(&manifest)?;
        if manifest.id != stored.extension_id {
            return Err(DeviceHostError::ExtensionSettings(format!(
                "manifest id {} does not match installed extension {}",
                manifest.id, stored.extension_id
            )));
        }
        Ok(manifest)
    }

    fn validate_capability_default_target(
        &self,
        capability: &str,
        extension_id: &str,
    ) -> Result<()> {
        if extension_id == DEFAULT_SPEECH_SYNTHESIS_EXTENSION_ID {
            if capability == SPEECH_SYNTHESIS_CAPABILITY {
                return Ok(());
            }
            return Err(DeviceHostError::ExtensionSettings(format!(
                "{DEFAULT_SPEECH_SYNTHESIS_EXTENSION_ID} only provides {SPEECH_SYNTHESIS_CAPABILITY}"
            )));
        }
        let Some(stored) = self
            .stored
            .installed_extensions
            .iter()
            .find(|stored| stored.extension_id == extension_id)
        else {
            return Err(DeviceHostError::ExtensionSettings(format!(
                "extension is not installed: {extension_id}"
            )));
        };
        let manifest = self.read_manifest_for(stored)?;
        if manifest
            .capabilities
            .iter()
            .any(|declared| declared.capability == capability)
        {
            return Ok(());
        }
        Err(DeviceHostError::ExtensionSettings(format!(
            "extension {extension_id} does not provide capability {capability}"
        )))
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

fn read_manifest(path: impl AsRef<Path>) -> Result<ProcessExtensionManifest> {
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn validate_manifest(manifest: &ProcessExtensionManifest) -> Result<()> {
    if manifest.schema_version != 1 {
        return Err(DeviceHostError::ExtensionSettings(format!(
            "unsupported extension schemaVersion: {}",
            manifest.schema_version
        )));
    }
    if manifest.hooks.is_empty()
        && manifest.event_subscriptions.is_empty()
        && manifest.emitted_events.is_empty()
        && manifest.capabilities.is_empty()
        && manifest.signal_aliases.is_empty()
    {
        return Err(DeviceHostError::ExtensionSettings(
            "extension must declare at least one hook, event subscription, emitted event, capability, or signal alias".to_string(),
        ));
    }
    if let Some(runtime) = &manifest.runtime {
        if runtime != &ExtensionRuntimeKind::Process {
            return Err(DeviceHostError::ExtensionSettings(
                "process extension manifest may only declare runtime \"process\"".to_string(),
            ));
        }
    }
    validate_extension_id(&manifest.id)?;
    let summary = ProcessHookExtension::from_manifest(manifest.clone()).registration();
    validate_extension_summary(&summary)
        .map_err(|error| DeviceHostError::ExtensionSettings(error.to_string()))?;
    Ok(())
}

fn validate_capability_name(capability: &str) -> Result<()> {
    if capability.trim().is_empty() {
        return Err(DeviceHostError::ExtensionSettings(
            "capability name must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn validate_extension_id(extension_id: &str) -> Result<()> {
    if extension_id.is_empty()
        || extension_id == "."
        || extension_id == ".."
        || !extension_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(DeviceHostError::ExtensionSettings(format!(
            "invalid extension id: {extension_id}"
        )));
    }
    Ok(())
}

fn manifest_subscribes_to(
    manifest: &ProcessExtensionManifest,
    hook_point: &ExtensionHookPoint,
) -> bool {
    manifest
        .hooks
        .iter()
        .any(|hook| &hook.hook_point == hook_point)
}

fn copy_dir_recursively(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::metadata(&source_path)?;
        if metadata.is_dir() {
            copy_dir_recursively(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path)?;
        }
    }
    Ok(())
}
