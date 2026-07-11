use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsState {
    pub talk_interval_minutes: u64,
    pub actor_scale_percent: u16,
    pub settings_path: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct AppSettingsRegistry {
    settings_path: PathBuf,
    stored: StoredAppSettings,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredAppSettings {
    schema_version: u32,
    talk_interval_minutes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    actor_scale_percent: Option<u16>,
}

impl Default for StoredAppSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            talk_interval_minutes: DEFAULT_TALK_INTERVAL_MINUTES,
            actor_scale_percent: None,
        }
    }
}

pub fn clamp_actor_scale_percent(percent: u16) -> u16 {
    percent.clamp(MIN_ACTOR_SCALE_PERCENT, MAX_ACTOR_SCALE_PERCENT)
}

impl AppSettingsRegistry {
    pub(crate) fn open(data_dir: &Path) -> Result<Self> {
        let settings_path = data_dir.join("settings").join("app.json");
        let exists = settings_path.exists();
        let stored = if exists {
            let raw = fs::read_to_string(&settings_path)?;
            let stored: StoredAppSettings = serde_json::from_str(&raw)?;
            if stored.schema_version != 1 {
                return Err(DeviceHostError::AppSettings(format!(
                    "unsupported app settings schemaVersion: {}",
                    stored.schema_version
                )));
            }
            stored
        } else {
            StoredAppSettings::default()
        };
        let registry = Self {
            settings_path,
            stored,
        };
        if !exists {
            registry.save()?;
        }
        Ok(registry)
    }

    pub(crate) fn state(&self) -> AppSettingsState {
        AppSettingsState {
            talk_interval_minutes: self.stored.talk_interval_minutes,
            actor_scale_percent: self
                .stored
                .actor_scale_percent
                .map(clamp_actor_scale_percent)
                .unwrap_or(DEFAULT_ACTOR_SCALE_PERCENT),
            settings_path: self.settings_path.clone(),
        }
    }

    pub(crate) fn set_talk_interval_minutes(&mut self, minutes: u64) -> Result<AppSettingsState> {
        self.stored.talk_interval_minutes = minutes;
        self.save()?;
        Ok(self.state())
    }

    pub(crate) fn set_actor_scale_percent(&mut self, percent: u16) -> Result<AppSettingsState> {
        self.stored.actor_scale_percent = Some(clamp_actor_scale_percent(percent));
        self.save()?;
        Ok(self.state())
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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageFootAnchor {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug)]
pub struct StageSettingsRegistry {
    settings_path: PathBuf,
    stored: StoredStageSettings,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredStageSettings {
    #[serde(default = "stage_settings_schema_version")]
    schema_version: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    actors: BTreeMap<String, StageFootAnchor>,
}

impl Default for StoredStageSettings {
    fn default() -> Self {
        Self {
            schema_version: stage_settings_schema_version(),
            actors: BTreeMap::new(),
        }
    }
}

const fn stage_settings_schema_version() -> u32 {
    1
}

impl StageSettingsRegistry {
    pub fn open(data_dir: &Path) -> Result<Self> {
        let settings_path = data_dir.join("settings").join("stage.json");
        let stored = if settings_path.exists() {
            let raw = fs::read_to_string(&settings_path)?;
            let stored: StoredStageSettings = serde_json::from_str(&raw)?;
            if stored.schema_version != stage_settings_schema_version() {
                return Err(DeviceHostError::StageSettings(format!(
                    "unsupported stage settings schemaVersion: {}",
                    stored.schema_version
                )));
            }
            if stored
                .actors
                .values()
                .any(|anchor| !anchor.x.is_finite() || !anchor.y.is_finite())
            {
                return Err(DeviceHostError::StageSettings(
                    "stage actor anchors must be finite numbers".to_string(),
                ));
            }
            stored
        } else {
            StoredStageSettings {
                schema_version: stage_settings_schema_version(),
                actors: BTreeMap::new(),
            }
        };
        Ok(Self {
            settings_path,
            stored,
        })
    }

    pub fn actor_anchors(&self) -> &BTreeMap<String, StageFootAnchor> {
        &self.stored.actors
    }

    pub fn set_actor_anchor(
        &mut self,
        actor_id: impl Into<String>,
        anchor: StageFootAnchor,
    ) -> Result<()> {
        if !anchor.x.is_finite() || !anchor.y.is_finite() {
            return Err(DeviceHostError::StageSettings(
                "stage actor anchors must be finite numbers".to_string(),
            ));
        }
        self.stored.actors.insert(actor_id.into(), anchor);
        self.save()
    }

    pub fn replace_actor_anchors(
        &mut self,
        anchors: BTreeMap<String, StageFootAnchor>,
    ) -> Result<()> {
        if anchors
            .values()
            .any(|anchor| !anchor.x.is_finite() || !anchor.y.is_finite())
        {
            return Err(DeviceHostError::StageSettings(
                "stage actor anchors must be finite numbers".to_string(),
            ));
        }
        self.stored.actors = anchors;
        self.save()
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSettingsState {
    pub llm_timeout_ms: u64,
    pub recent_context_count: usize,
    pub talk_desire_low: u8,
    pub talk_desire_high: u8,
    pub settings_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSettingsUpdate {
    pub llm_timeout_ms: u64,
    pub recent_context_count: usize,
    pub talk_desire_low: u8,
    pub talk_desire_high: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneHistoryState {
    pub install_id: String,
    pub history_path: PathBuf,
    pub entries: Vec<SceneHistoryEntry>,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeSettingsRegistry {
    settings_path: PathBuf,
    stored: StoredRuntimeSettings,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredRuntimeSettings {
    #[serde(default = "default_llm_timeout_ms")]
    llm_timeout_ms: u64,
    #[serde(default = "default_recent_context_count")]
    recent_context_count: usize,
    #[serde(default = "default_talk_desire_low")]
    talk_desire_low: u8,
    #[serde(default = "default_talk_desire_high")]
    talk_desire_high: u8,
}

impl Default for StoredRuntimeSettings {
    fn default() -> Self {
        Self {
            llm_timeout_ms: DEFAULT_LLM_TIMEOUT_MS,
            recent_context_count: DEFAULT_RECENT_CONTEXT_COUNT,
            talk_desire_low: DEFAULT_TALK_DESIRE_LOW,
            talk_desire_high: DEFAULT_TALK_DESIRE_HIGH,
        }
    }
}

impl StoredRuntimeSettings {
    fn normalized(mut self) -> Self {
        self.llm_timeout_ms = self
            .llm_timeout_ms
            .clamp(MIN_LLM_TIMEOUT_MS, MAX_LLM_TIMEOUT_MS);
        self.recent_context_count = self.recent_context_count.min(MAX_RECENT_CONTEXT_COUNT);
        self.talk_desire_low = self.talk_desire_low.min(100);
        self.talk_desire_high = self.talk_desire_high.min(100);
        if self.talk_desire_low >= self.talk_desire_high {
            self.talk_desire_high = self.talk_desire_low.saturating_add(1).min(100);
            if self.talk_desire_low >= self.talk_desire_high {
                self.talk_desire_low = self.talk_desire_high.saturating_sub(1);
            }
        }
        self
    }
}

impl RuntimeSettingsRegistry {
    pub(crate) fn open(data_dir: &Path) -> Result<Self> {
        let settings_path = data_dir.join("settings").join("runtime.json");
        let exists = settings_path.exists();
        let stored = if exists {
            let raw = fs::read_to_string(&settings_path)?;
            serde_json::from_str::<StoredRuntimeSettings>(&raw)?.normalized()
        } else {
            StoredRuntimeSettings::default()
        };
        let registry = Self {
            settings_path,
            stored,
        };
        if !exists {
            registry.save()?;
        }
        Ok(registry)
    }

    pub(crate) fn state(&self) -> RuntimeSettingsState {
        RuntimeSettingsState {
            llm_timeout_ms: self.stored.llm_timeout_ms,
            recent_context_count: self.stored.recent_context_count,
            talk_desire_low: self.stored.talk_desire_low,
            talk_desire_high: self.stored.talk_desire_high,
            settings_path: self.settings_path.clone(),
        }
    }

    pub(crate) fn set(&mut self, next: RuntimeSettingsUpdate) -> Result<RuntimeSettingsState> {
        self.stored = StoredRuntimeSettings {
            llm_timeout_ms: next.llm_timeout_ms,
            recent_context_count: next.recent_context_count,
            talk_desire_low: next.talk_desire_low,
            talk_desire_high: next.talk_desire_high,
        }
        .normalized();
        self.save()?;
        Ok(self.state())
    }

    pub(crate) fn resident_runtime_settings(&self) -> ResidentRuntimeSettings {
        ResidentRuntimeSettings {
            llm_timeout: Duration::from_millis(self.stored.llm_timeout_ms),
            recent_context_count: self.stored.recent_context_count,
            talk_desire_low: self.stored.talk_desire_low,
            talk_desire_high: self.stored.talk_desire_high,
            mood_state_path: None,
        }
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

fn default_llm_timeout_ms() -> u64 {
    DEFAULT_LLM_TIMEOUT_MS
}

fn default_recent_context_count() -> usize {
    DEFAULT_RECENT_CONTEXT_COUNT
}

fn default_talk_desire_low() -> u8 {
    DEFAULT_TALK_DESIRE_LOW
}

fn default_talk_desire_high() -> u8 {
    DEFAULT_TALK_DESIRE_HIGH
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationSettingsState {
    pub windows: bool,
    pub folders: bool,
    pub downloads: bool,
    pub settings_path: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct ObservationSettingsRegistry {
    settings_path: PathBuf,
    stored: StoredObservationSettings,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredObservationSettings {
    #[serde(default)]
    windows: bool,
    #[serde(default)]
    folders: bool,
    #[serde(default)]
    downloads: bool,
}

impl ObservationSettingsRegistry {
    pub(crate) fn open(data_dir: &Path) -> Result<Self> {
        let settings_path = data_dir.join("settings").join("observations.json");
        let exists = settings_path.exists();
        let stored = if exists {
            let raw = fs::read_to_string(&settings_path)?;
            serde_json::from_str(&raw)?
        } else {
            StoredObservationSettings::default()
        };
        let registry = Self {
            settings_path,
            stored,
        };
        if !exists {
            registry.save()?;
        }
        Ok(registry)
    }

    pub(crate) fn state(&self) -> ObservationSettingsState {
        ObservationSettingsState {
            windows: self.stored.windows,
            folders: self.stored.folders,
            downloads: self.stored.downloads,
            settings_path: self.settings_path.clone(),
        }
    }

    pub(crate) fn set(&mut self, next: ObservationSettingsUpdate) -> Result<ObservationSettingsState> {
        self.stored.windows = next.windows;
        self.stored.folders = next.folders;
        self.stored.downloads = next.downloads;
        self.save()?;
        Ok(self.state())
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationSettingsUpdate {
    pub windows: bool,
    pub folders: bool,
    pub downloads: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingState {
    pub completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    pub settings_path: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct OnboardingRegistry {
    settings_path: PathBuf,
    stored: StoredOnboardingState,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredOnboardingState {
    #[serde(default)]
    completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed_at: Option<String>,
}

impl OnboardingRegistry {
    pub(crate) fn open(data_dir: &Path) -> Result<Self> {
        let settings_path = data_dir.join("settings").join("onboarding.json");
        let stored = if settings_path.exists() {
            let raw = fs::read_to_string(&settings_path)?;
            serde_json::from_str(&raw)?
        } else {
            StoredOnboardingState::default()
        };
        Ok(Self {
            settings_path,
            stored,
        })
    }

    pub(crate) fn state(&self) -> OnboardingState {
        OnboardingState {
            completed: self.stored.completed,
            completed_at: self.stored.completed_at.clone(),
            settings_path: self.settings_path.clone(),
        }
    }

    pub(crate) fn complete(&mut self) -> Result<OnboardingState> {
        self.stored.completed = true;
        self.stored.completed_at = Some(now_timestamp());
        self.save()?;
        Ok(self.state())
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


#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn app_settings_persist_talk_interval_minutes(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let data = tempdir()?;
        let mut registry = AppSettingsRegistry::open(data.path())?;

        assert_eq!(
            registry.state().talk_interval_minutes,
            DEFAULT_TALK_INTERVAL_MINUTES
        );
        registry.set_talk_interval_minutes(12)?;

        let reopened = AppSettingsRegistry::open(data.path())?;
        assert_eq!(reopened.state().talk_interval_minutes, 12);
        assert!(data.path().join("settings").join("app.json").exists());
        Ok(())
    }

    #[test]
    fn app_settings_default_missing_actor_scale_percent(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let data = tempdir()?;
        let settings_dir = data.path().join("settings");
        fs::create_dir_all(&settings_dir)?;
        fs::write(
            settings_dir.join("app.json"),
            r#"{"schemaVersion":1,"talkIntervalMinutes":7}"#,
        )?;

        let registry = AppSettingsRegistry::open(data.path())?;

        assert_eq!(registry.state().talk_interval_minutes, 7);
        assert_eq!(
            registry.state().actor_scale_percent,
            DEFAULT_ACTOR_SCALE_PERCENT
        );
        Ok(())
    }

    #[test]
    fn app_settings_persist_and_clamp_actor_scale_percent(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let data = tempdir()?;
        let mut registry = AppSettingsRegistry::open(data.path())?;

        assert_eq!(
            registry.state().actor_scale_percent,
            DEFAULT_ACTOR_SCALE_PERCENT
        );
        assert_eq!(
            registry.set_actor_scale_percent(20)?.actor_scale_percent,
            50
        );
        assert_eq!(
            registry.set_actor_scale_percent(240)?.actor_scale_percent,
            200
        );

        let reopened = AppSettingsRegistry::open(data.path())?;
        assert_eq!(reopened.state().actor_scale_percent, 200);
        Ok(())
    }

    #[test]
    fn stage_settings_only_persist_explicit_actor_anchors(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let data = tempdir()?;
        let settings_path = data.path().join("settings").join("stage.json");
        let mut registry = StageSettingsRegistry::open(data.path())?;

        assert!(registry.actor_anchors().is_empty());
        assert!(!settings_path.exists());

        registry.set_actor_anchor("yuukei", StageFootAnchor { x: 321.5, y: 654.0 })?;
        let reopened = StageSettingsRegistry::open(data.path())?;

        assert_eq!(
            reopened.actor_anchors().get("yuukei"),
            Some(&StageFootAnchor { x: 321.5, y: 654.0 })
        );
        assert!(settings_path.exists());
        Ok(())
    }

    #[test]
    fn runtime_settings_persist_and_clamp_llm_and_context(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let data = tempdir()?;
        let mut registry = RuntimeSettingsRegistry::open(data.path())?;
        let settings_path = data.path().join("settings").join("runtime.json");

        assert_eq!(
            registry.state(),
            RuntimeSettingsState {
                llm_timeout_ms: DEFAULT_LLM_TIMEOUT_MS,
                recent_context_count: DEFAULT_RECENT_CONTEXT_COUNT,
                talk_desire_low: DEFAULT_TALK_DESIRE_LOW,
                talk_desire_high: DEFAULT_TALK_DESIRE_HIGH,
                settings_path: settings_path.clone(),
            }
        );
        let updated = registry.set(RuntimeSettingsUpdate {
            llm_timeout_ms: 999_999,
            recent_context_count: 999,
            talk_desire_low: 100,
            talk_desire_high: 5,
        })?;
        assert_eq!(updated.llm_timeout_ms, MAX_LLM_TIMEOUT_MS);
        assert_eq!(updated.recent_context_count, MAX_RECENT_CONTEXT_COUNT);
        assert_eq!(updated.talk_desire_low, 99);
        assert_eq!(updated.talk_desire_high, 100);

        fs::write(
            &settings_path,
            r#"{"llmTimeoutMs":500,"recentContextCount":101,"talkDesireLow":80,"talkDesireHigh":80}"#,
        )?;
        let reopened = RuntimeSettingsRegistry::open(data.path())?;
        assert_eq!(reopened.state().llm_timeout_ms, MIN_LLM_TIMEOUT_MS);
        assert_eq!(
            reopened.state().recent_context_count,
            MAX_RECENT_CONTEXT_COUNT
        );
        assert_eq!(reopened.state().talk_desire_low, 80);
        assert_eq!(reopened.state().talk_desire_high, 81);
        assert_eq!(
            reopened.resident_runtime_settings(),
            ResidentRuntimeSettings {
                llm_timeout: Duration::from_millis(MIN_LLM_TIMEOUT_MS),
                recent_context_count: MAX_RECENT_CONTEXT_COUNT,
                talk_desire_low: 80,
                talk_desire_high: 81,
                mood_state_path: None,
            }
        );
        Ok(())
    }

    #[test]
    fn observation_settings_default_off_and_persist(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let data = tempdir()?;
        let mut registry = ObservationSettingsRegistry::open(data.path())?;

        assert_eq!(
            registry.state(),
            ObservationSettingsState {
                windows: false,
                folders: false,
                downloads: false,
                settings_path: data.path().join("settings").join("observations.json"),
            }
        );
        registry.set(ObservationSettingsUpdate {
            windows: true,
            folders: false,
            downloads: true,
        })?;

        let reopened = ObservationSettingsRegistry::open(data.path())?;
        assert!(reopened.state().windows);
        assert!(!reopened.state().folders);
        assert!(reopened.state().downloads);
        let raw = fs::read_to_string(data.path().join("settings").join("observations.json"))?;
        assert!(raw.contains("\"windows\": true"));
        Ok(())
    }

    #[test]
    fn onboarding_state_missing_file_is_initial_and_completion_persists(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let data = tempdir()?;
        let settings_path = data.path().join("settings").join("onboarding.json");
        let mut registry = OnboardingRegistry::open(data.path())?;

        assert_eq!(
            registry.state(),
            OnboardingState {
                completed: false,
                completed_at: None,
                settings_path: settings_path.clone(),
            }
        );
        assert!(!settings_path.exists());

        let completed = registry.complete()?;
        assert!(completed.completed);
        assert!(completed.completed_at.is_some());
        assert!(settings_path.exists());

        let raw = fs::read_to_string(&settings_path)?;
        assert!(raw.contains("\"completed\": true"));
        assert!(raw.contains("\"completedAt\""));

        let reopened = OnboardingRegistry::open(data.path())?;
        assert!(reopened.state().completed);
        assert_eq!(reopened.state().settings_path, settings_path);
        Ok(())
    }

}
