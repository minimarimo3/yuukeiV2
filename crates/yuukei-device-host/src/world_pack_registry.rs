use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use yuukei_world::{DaihonDiagnosticEntry, WorldPack};

use crate::{display_path, LocalRuntimeConfig, Result, DEFAULT_DEVICE_ID, DEFAULT_RESIDENT_ID};

pub const DEFAULT_WORLD_PACK_INSTALL_ID: &str = "default-yuukei";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WorldPackSource {
    BundledDefault,
    ExternalDirectory,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldPackInstall {
    pub install_id: String,
    pub resident_id: String,
    pub world_pack_id: String,
    pub display_name: String,
    pub canonical_root: PathBuf,
    pub source: WorldPackSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_load_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldPackSelectionState {
    pub configured_install_id: String,
    pub running_install_id: String,
    pub active_install: WorldPackInstall,
    pub installs: Vec<WorldPackInstall>,
    pub fallback_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_load_error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub daihon_diagnostics: Vec<DaihonDiagnosticEntry>,
    pub settings_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldPackSwitchResult {
    pub status: WorldPackSelectionState,
    pub snapshot: yuukei_protocol::ResidentSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalRuntimeEnvironment {
    pub workspace_root: PathBuf,
    pub default_world_root: PathBuf,
    pub data_dir: PathBuf,
    pub device_id: String,
}

impl LocalRuntimeEnvironment {
    pub fn default_local() -> Self {
        let workspace_root = crate::workspace_root();
        let default_world_root = workspace_root.join("packs").join("default-yuukei");
        Self {
            workspace_root,
            default_world_root,
            data_dir: crate::default_data_dir(),
            device_id: DEFAULT_DEVICE_ID.to_string(),
        }
    }

    pub fn app_log_path(&self) -> PathBuf {
        self.data_dir.join("app-activity.jsonl")
    }
}

#[derive(Clone, Debug)]
pub struct WorldPackRegistry {
    env: LocalRuntimeEnvironment,
    settings_path: PathBuf,
    stored: StoredWorldPackRegistry,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredWorldPackRegistry {
    active_install_id: String,
    installs: Vec<WorldPackInstall>,
}

impl WorldPackRegistry {
    pub fn open(env: LocalRuntimeEnvironment) -> Result<Self> {
        let settings_path = env.data_dir.join("settings").join("world-packs.json");
        let default_install = default_install(&env)?;
        let stored = if settings_path.exists() {
            let raw = fs::read_to_string(&settings_path)?;
            serde_json::from_str(&raw)?
        } else {
            StoredWorldPackRegistry {
                active_install_id: default_install.install_id.clone(),
                installs: Vec::new(),
            }
        };
        let mut registry = Self {
            env,
            settings_path,
            stored,
        };
        registry.upsert_install(default_install);
        if registry
            .stored
            .installs
            .iter()
            .all(|install| install.install_id != registry.stored.active_install_id)
        {
            registry.stored.active_install_id = DEFAULT_WORLD_PACK_INSTALL_ID.to_string();
        }
        registry.save()?;
        Ok(registry)
    }

    pub fn active_install(&self) -> Result<WorldPackInstall> {
        self.install_by_id(&self.stored.active_install_id)
            .or_else(|| self.install_by_id(DEFAULT_WORLD_PACK_INSTALL_ID))
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "default world pack install is missing",
                )
                .into()
            })
    }

    pub fn default_install(&self) -> Result<WorldPackInstall> {
        self.install_by_id(DEFAULT_WORLD_PACK_INSTALL_ID)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "default world pack install is missing",
                )
                .into()
            })
    }

    pub fn install_from_directory(&self, path: impl AsRef<Path>) -> Result<WorldPackInstall> {
        let canonical_root = fs::canonicalize(path.as_ref())?;
        let pack = WorldPack::load_from_dir(&canonical_root)?;
        let existing = self.stored.installs.iter().find(|install| {
            install.source == WorldPackSource::ExternalDirectory
                && install.canonical_root == canonical_root
        });
        let install_id = existing
            .map(|install| install.install_id.clone())
            .unwrap_or_else(|| stable_external_install_id(&canonical_root));
        let resident_id = existing
            .map(|install| install.resident_id.clone())
            .unwrap_or_else(|| format!("resident-{install_id}"));
        Ok(WorldPackInstall {
            install_id,
            resident_id,
            world_pack_id: pack.id,
            display_name: pack.display_name,
            canonical_root,
            source: WorldPackSource::ExternalDirectory,
            last_load_error: None,
        })
    }

    pub fn stage_active_install(&mut self, mut install: WorldPackInstall) {
        install.last_load_error = None;
        self.stored.active_install_id = install.install_id.clone();
        self.upsert_install(install);
    }

    pub fn mark_load_error(&mut self, install_id: &str, error: impl Into<String>) -> Result<()> {
        if let Some(install) = self
            .stored
            .installs
            .iter_mut()
            .find(|install| install.install_id == install_id)
        {
            install.last_load_error = Some(error.into());
        }
        self.save()
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.settings_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            &self.settings_path,
            serde_json::to_vec_pretty(&self.stored)?,
        )?;
        Ok(())
    }

    pub fn config_for_install(&self, install: &WorldPackInstall) -> LocalRuntimeConfig {
        let resident_dir = self
            .env
            .data_dir
            .join("residents")
            .join(&install.install_id);
        LocalRuntimeConfig {
            install_id: install.install_id.clone(),
            resident_id: install.resident_id.clone(),
            device_id: self.env.device_id.clone(),
            workspace_root: self.env.workspace_root.clone(),
            data_dir: self.env.data_dir.clone(),
            world_root: install.canonical_root.clone(),
            extension_root: self.env.data_dir.join("extensions"),
            event_log_path: resident_dir.join("events.sqlite3"),
            scene_history_path: resident_dir.join("scene-history.json"),
            variables_path: resident_dir.join("variables.json"),
            mood_state_path: resident_dir.join("mood.json"),
            app_log_path: self.env.app_log_path(),
        }
    }

    pub fn selection_state(
        &self,
        running_install: &WorldPackInstall,
        fallback_active: bool,
    ) -> WorldPackSelectionState {
        let last_load_error = if fallback_active {
            self.install_by_id(&self.stored.active_install_id)
                .and_then(|install| install.last_load_error.clone())
        } else {
            running_install.last_load_error.clone()
        };
        WorldPackSelectionState {
            configured_install_id: self.stored.active_install_id.clone(),
            running_install_id: running_install.install_id.clone(),
            active_install: running_install.clone(),
            installs: self.stored.installs.clone(),
            fallback_active,
            last_load_error,
            daihon_diagnostics: Vec::new(),
            settings_path: self.settings_path.clone(),
        }
    }

    fn install_by_id(&self, install_id: &str) -> Option<WorldPackInstall> {
        self.stored
            .installs
            .iter()
            .find(|install| install.install_id == install_id)
            .cloned()
    }

    fn upsert_install(&mut self, install: WorldPackInstall) {
        if let Some(existing) = self
            .stored
            .installs
            .iter_mut()
            .find(|existing| existing.install_id == install.install_id)
        {
            *existing = install;
        } else {
            self.stored.installs.push(install);
        }
        self.stored
            .installs
            .sort_by(|a, b| a.install_id.cmp(&b.install_id));
    }
}

fn default_install(env: &LocalRuntimeEnvironment) -> Result<WorldPackInstall> {
    let canonical_root = fs::canonicalize(&env.default_world_root)?;
    let pack = WorldPack::load_from_dir(&canonical_root)?;
    Ok(WorldPackInstall {
        install_id: DEFAULT_WORLD_PACK_INSTALL_ID.to_string(),
        resident_id: DEFAULT_RESIDENT_ID.to_string(),
        world_pack_id: pack.id,
        display_name: pack.display_name,
        canonical_root,
        source: WorldPackSource::BundledDefault,
        last_load_error: None,
    })
}

fn stable_external_install_id(path: &Path) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in display_path(path).as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("pack-{hash:016x}")
}
