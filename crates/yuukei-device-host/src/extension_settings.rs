use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use yuukei_capability::{DEFAULT_SPEECH_SYNTHESIS_EXTENSION_ID, SPEECH_SYNTHESIS_CAPABILITY};
use yuukei_extension::{
    validate_extension_summary, ProcessExtensionManifest, ProcessHookExtension, YuukeiExtension,
};
use yuukei_protocol::{
    now_timestamp, ExtensionCapabilityDeclaration, ExtensionEventSubscription, ExtensionHookPoint,
    ExtensionHookSubscription, ExtensionPermissions, ExtensionRuntimeKind, ExtensionSettingField,
    ExtensionSettingsSchema, ExtensionSignalAlias, ResidentSnapshot,
};

use crate::{DeviceHostError, Result};

const EXTENSION_SETTINGS_SCHEMA_VERSION: u32 = 1;
const EXTENSION_MANIFEST_FILE: &str = "manifest.json";

pub const TRUSTED_CODE_NOTICE: &str = "Extensionは信頼したローカルコードとして実行されます。Yuukeiは公開protocolへの入力と出力を検証しますが、OSレベルのファイルアクセス隔離はv1では行いません。";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSettingsState {
    pub installed: Vec<InstalledExtension>,
    pub hook_order: BTreeMap<ExtensionHookPoint, Vec<String>>,
    pub capability_defaults: BTreeMap<String, String>,
    pub settings_path: PathBuf,
    pub extension_root: PathBuf,
    pub trusted_code_notice: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_schema: Option<ExtensionSettingsSchema>,
    #[serde(default)]
    pub setting_values: Map<String, Value>,
    #[serde(default)]
    pub secrets_set: Vec<String>,
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredExtensionSettings {
    schema_version: u32,
    #[serde(default)]
    installed_extensions: Vec<StoredInstalledExtension>,
    #[serde(default)]
    hook_order: BTreeMap<ExtensionHookPoint, Vec<String>>,
    #[serde(default)]
    capability_defaults: BTreeMap<String, String>,
    #[serde(default)]
    extension_values: BTreeMap<String, Map<String, Value>>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredExtensionSecrets {
    #[serde(flatten)]
    extensions: BTreeMap<String, BTreeMap<String, String>>,
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
    pub settings_json: Option<String>,
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
    secrets_path: PathBuf,
    extension_root: PathBuf,
    stored: StoredExtensionSettings,
    secrets: StoredExtensionSecrets,
}

impl ExtensionSettingsRegistry {
    pub fn open(data_dir: impl AsRef<Path>, extension_root: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref();
        let settings_path = data_dir.join("settings").join("extensions.json");
        let secrets_path = data_dir.join("settings").join("extension-secrets.json");
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
                extension_values: BTreeMap::new(),
            }
        };
        let secrets = if secrets_path.exists() {
            let raw = fs::read_to_string(&secrets_path)?;
            serde_json::from_str(&raw)?
        } else {
            StoredExtensionSecrets::default()
        };
        if stored.schema_version != EXTENSION_SETTINGS_SCHEMA_VERSION {
            return Err(DeviceHostError::ExtensionSettings(format!(
                "unsupported extension settings schemaVersion: {}",
                stored.schema_version
            )));
        }
        let mut registry = Self {
            settings_path,
            secrets_path,
            extension_root,
            stored,
            secrets,
        };
        registry.normalize();
        registry.save()?;
        registry.save_secrets()?;
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
        self.stored.extension_values.remove(extension_id);
        self.secrets.extensions.remove(extension_id);
        for order in self.stored.hook_order.values_mut() {
            order.retain(|candidate| candidate != extension_id);
        }
        let install_dir = self.extension_root.join(extension_id);
        if install_dir.exists() {
            fs::remove_dir_all(install_dir)?;
        }
        self.normalize();
        self.save()?;
        self.save_secrets()?;
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

    pub fn set_extension_setting_values(
        &mut self,
        extension_id: &str,
        values: Map<String, Value>,
    ) -> Result<ExtensionSettingsState> {
        validate_extension_id(extension_id)?;
        let manifest = self.manifest_for_installed_id(extension_id)?;
        let Some(schema) = manifest.settings.as_ref() else {
            return Err(DeviceHostError::ExtensionSettings(format!(
                "extension does not declare settings: {extension_id}"
            )));
        };
        let known = schema_fields_by_key(schema);
        let mut next = self
            .stored
            .extension_values
            .get(extension_id)
            .cloned()
            .unwrap_or_default();

        for (key, value) in values {
            let Some(field) = known.get(key.as_str()) else {
                return Err(DeviceHostError::ExtensionSettings(format!(
                    "unknown setting key for {extension_id}: {key}"
                )));
            };
            if matches!(field, ExtensionSettingField::Secret { .. }) {
                return Err(DeviceHostError::ExtensionSettings(format!(
                    "secret setting must be updated through set_extension_secret: {key}"
                )));
            }
            if value.is_null() {
                next.remove(&key);
                continue;
            }
            validate_setting_value(field, &value)?;
            next.insert(key, value);
        }

        if next.is_empty() {
            self.stored.extension_values.remove(extension_id);
        } else {
            self.stored
                .extension_values
                .insert(extension_id.to_string(), next);
        }
        self.save()?;
        Ok(self.state())
    }

    pub fn set_extension_secret(
        &mut self,
        extension_id: &str,
        key: &str,
        value: Option<String>,
    ) -> Result<ExtensionSettingsState> {
        validate_extension_id(extension_id)?;
        let manifest = self.manifest_for_installed_id(extension_id)?;
        let Some(schema) = manifest.settings.as_ref() else {
            return Err(DeviceHostError::ExtensionSettings(format!(
                "extension does not declare settings: {extension_id}"
            )));
        };
        let Some(field) = schema_fields_by_key(schema).remove(key) else {
            return Err(DeviceHostError::ExtensionSettings(format!(
                "unknown setting key for {extension_id}: {key}"
            )));
        };
        if !matches!(field, ExtensionSettingField::Secret { .. }) {
            return Err(DeviceHostError::ExtensionSettings(format!(
                "setting is not secret: {key}"
            )));
        }

        match value {
            Some(value) if !value.is_empty() => {
                self.secrets
                    .extensions
                    .entry(extension_id.to_string())
                    .or_default()
                    .insert(key.to_string(), value);
            }
            _ => {
                if let Some(secrets) = self.secrets.extensions.get_mut(extension_id) {
                    secrets.remove(key);
                    if secrets.is_empty() {
                        self.secrets.extensions.remove(extension_id);
                    }
                }
            }
        }
        self.save_secrets()?;
        Ok(self.state())
    }

    pub(crate) fn runtime_entries(&self) -> Vec<ExtensionRuntimeEntry> {
        self.ordered_installed_extensions()
            .into_iter()
            .map(|stored| {
                let manifest_path = self.manifest_path(&stored.extension_id);
                match self.read_manifest_for(stored).and_then(|manifest| {
                    let settings_json =
                        self.resolved_settings_json(&stored.extension_id, &manifest)?;
                    Ok((manifest, settings_json))
                }) {
                    Ok((manifest, settings_json)) => {
                        ExtensionRuntimeEntry::Ready(Box::new(ExtensionRuntimeInstall {
                            extension_id: stored.extension_id.clone(),
                            manifest,
                            enabled: stored.enabled,
                            install_dir: self.install_dir(&stored.extension_id),
                            manifest_path,
                            settings_json,
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
        let installed_ids = self
            .stored
            .installed_extensions
            .iter()
            .map(|extension| extension.extension_id.clone())
            .collect::<BTreeSet<_>>();
        self.stored
            .extension_values
            .retain(|extension_id, _| installed_ids.contains(extension_id));
        self.secrets
            .extensions
            .retain(|extension_id, _| installed_ids.contains(extension_id));
        for hook_point in [ExtensionHookPoint::BeforeCommandEmit] {
            let current = self
                .stored
                .hook_order
                .remove(&hook_point)
                .unwrap_or_default();
            let mut next = Vec::new();
            let mut ordered = BTreeSet::new();
            for extension_id in current {
                if installed_ids.contains(&extension_id) && ordered.insert(extension_id.clone()) {
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
                settings_schema: manifest.settings.clone(),
                setting_values: self
                    .visible_setting_values(&stored.extension_id, manifest.settings.as_ref()),
                secrets_set: self.secret_keys_set(&stored.extension_id, manifest.settings.as_ref()),
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
                settings_schema: None,
                setting_values: Map::new(),
                secrets_set: Vec::new(),
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

    fn manifest_for_installed_id(&self, extension_id: &str) -> Result<ProcessExtensionManifest> {
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
        self.read_manifest_for(stored)
    }

    fn visible_setting_values(
        &self,
        extension_id: &str,
        schema: Option<&ExtensionSettingsSchema>,
    ) -> Map<String, Value> {
        let Some(schema) = schema else {
            return Map::new();
        };
        let known = schema_fields_by_key(schema);
        self.stored
            .extension_values
            .get(extension_id)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|(key, value)| {
                        let field = known.get(key.as_str())?;
                        if matches!(field, ExtensionSettingField::Secret { .. })
                            || validate_setting_value(field, value).is_err()
                        {
                            return None;
                        }
                        Some((key.clone(), value.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn secret_keys_set(
        &self,
        extension_id: &str,
        schema: Option<&ExtensionSettingsSchema>,
    ) -> Vec<String> {
        let Some(schema) = schema else {
            return Vec::new();
        };
        let secret_keys = schema
            .fields
            .iter()
            .filter(|field| matches!(field, ExtensionSettingField::Secret { .. }))
            .map(|field| field.key().to_string())
            .collect::<BTreeSet<_>>();
        let mut keys = self
            .secrets
            .extensions
            .get(extension_id)
            .map(|values| {
                values
                    .keys()
                    .filter(|key| secret_keys.contains(*key))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        keys.sort();
        keys
    }

    fn resolved_settings_json(
        &self,
        extension_id: &str,
        manifest: &ProcessExtensionManifest,
    ) -> Result<Option<String>> {
        let Some(schema) = manifest.settings.as_ref() else {
            return Ok(None);
        };
        // スキーマのdefaultはGUI表示用。ここで実効値へ焼き込むと、Extension自身の
        // デフォルトや環境変数フォールバックを常に上書きしてしまうため、
        // ユーザーが明示的に保存した値とsecretだけを渡す。
        let mut effective = Map::new();
        let known = schema_fields_by_key(schema);
        if let Some(values) = self.stored.extension_values.get(extension_id) {
            for (key, value) in values {
                let Some(field) = known.get(key.as_str()) else {
                    continue;
                };
                if matches!(field, ExtensionSettingField::Secret { .. })
                    || validate_setting_value(field, value).is_err()
                {
                    continue;
                }
                effective.insert(key.clone(), value.clone());
            }
        }
        if let Some(secrets) = self.secrets.extensions.get(extension_id) {
            for (key, value) in secrets {
                if known
                    .get(key.as_str())
                    .is_some_and(|field| matches!(field, ExtensionSettingField::Secret { .. }))
                {
                    effective.insert(key.clone(), Value::String(value.clone()));
                }
            }
        }
        Ok(Some(serde_json::to_string(&effective)?))
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

    fn save_secrets(&self) -> Result<()> {
        if let Some(parent) = self.secrets_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&self.secrets)?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.secrets_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.secrets_path, fs::Permissions::from_mode(0o600))?;
        }
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
    if let Some(schema) = &manifest.settings {
        validate_settings_schema(schema)?;
    }
    validate_extension_id(&manifest.id)?;
    let summary = ProcessHookExtension::from_manifest(manifest.clone()).registration();
    validate_extension_summary(&summary)
        .map_err(|error| DeviceHostError::ExtensionSettings(error.to_string()))?;
    Ok(())
}

fn validate_settings_schema(schema: &ExtensionSettingsSchema) -> Result<()> {
    let mut keys = BTreeSet::new();
    for field in &schema.fields {
        validate_setting_key(field.key())?;
        if !keys.insert(field.key().to_string()) {
            return Err(DeviceHostError::ExtensionSettings(format!(
                "duplicate setting key: {}",
                field.key()
            )));
        }
        match field {
            ExtensionSettingField::Select {
                key,
                options,
                default,
                ..
            } => {
                if options.is_empty() {
                    return Err(DeviceHostError::ExtensionSettings(format!(
                        "select setting must declare options: {key}"
                    )));
                }
                let option_values = options
                    .iter()
                    .map(|option| option.value.as_str())
                    .collect::<BTreeSet<_>>();
                if let Some(default) = default {
                    if !option_values.contains(default.as_str()) {
                        return Err(DeviceHostError::ExtensionSettings(format!(
                            "select setting default must be one of options: {key}"
                        )));
                    }
                }
            }
            ExtensionSettingField::Number { key, min, max, .. } => {
                if let (Some(min), Some(max)) = (*min, *max) {
                    if min > max {
                        return Err(DeviceHostError::ExtensionSettings(format!(
                            "number setting min must be <= max: {key}"
                        )));
                    }
                }
            }
            ExtensionSettingField::Secret { key, default, .. } => {
                if default.is_some() {
                    return Err(DeviceHostError::ExtensionSettings(format!(
                        "secret setting cannot declare default: {key}"
                    )));
                }
            }
            ExtensionSettingField::String { .. } | ExtensionSettingField::Boolean { .. } => {}
        }
    }
    for field in &schema.fields {
        if let Some(visible_when) = field.visible_when() {
            if !keys.contains(&visible_when.key) {
                return Err(DeviceHostError::ExtensionSettings(format!(
                    "visibleWhen references unknown setting key: {}",
                    visible_when.key
                )));
            }
        }
    }
    Ok(())
}

fn schema_fields_by_key(
    schema: &ExtensionSettingsSchema,
) -> BTreeMap<&str, &ExtensionSettingField> {
    schema
        .fields
        .iter()
        .map(|field| (field.key(), field))
        .collect()
}

fn validate_setting_key(key: &str) -> Result<()> {
    if key.is_empty()
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return Err(DeviceHostError::ExtensionSettings(format!(
            "invalid setting key: {key}"
        )));
    }
    Ok(())
}

fn validate_setting_value(field: &ExtensionSettingField, value: &Value) -> Result<()> {
    match field {
        ExtensionSettingField::String { key, .. } => {
            if value.is_string() {
                Ok(())
            } else {
                Err(setting_type_error(key, "string"))
            }
        }
        ExtensionSettingField::Number { key, min, max, .. } => {
            let Some(number) = value.as_f64() else {
                return Err(setting_type_error(key, "number"));
            };
            if let Some(min) = *min {
                if number < min {
                    return Err(DeviceHostError::ExtensionSettings(format!(
                        "setting {key} is below minimum {min}"
                    )));
                }
            }
            if let Some(max) = *max {
                if number > max {
                    return Err(DeviceHostError::ExtensionSettings(format!(
                        "setting {key} is above maximum {max}"
                    )));
                }
            }
            Ok(())
        }
        ExtensionSettingField::Boolean { key, .. } => {
            if value.is_boolean() {
                Ok(())
            } else {
                Err(setting_type_error(key, "boolean"))
            }
        }
        ExtensionSettingField::Select { key, options, .. } => {
            let Some(selected) = value.as_str() else {
                return Err(setting_type_error(key, "select"));
            };
            if options.iter().any(|option| option.value == selected) {
                Ok(())
            } else {
                Err(DeviceHostError::ExtensionSettings(format!(
                    "setting {key} must be one of declared options"
                )))
            }
        }
        ExtensionSettingField::Secret { key, .. } => Err(DeviceHostError::ExtensionSettings(
            format!("secret setting must be updated through set_extension_secret: {key}"),
        )),
    }
}

fn setting_type_error(key: &str, expected: &str) -> DeviceHostError {
    DeviceHostError::ExtensionSettings(format!("setting {key} must be {expected}"))
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
