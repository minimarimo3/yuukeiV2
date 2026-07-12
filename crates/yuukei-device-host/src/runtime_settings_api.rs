use super::*;

impl LocalYuukeiRuntime {
    pub async fn open_default() -> Result<Self> {
        Self::open_selected().await
    }

    pub async fn open_selected() -> Result<Self> {
        Self::open_selected_in(LocalRuntimeEnvironment::default_local()).await
    }

    pub async fn open_selected_in(env: LocalRuntimeEnvironment) -> Result<Self> {
        let mut registry = world_pack_registry::WorldPackRegistry::open(env)?;
        let requested_install = registry.active_install()?;
        let status = registry.selection_state(&requested_install, false);
        match Self::open_with_status(registry.config_for_install(&requested_install), status).await
        {
            Ok(runtime) => Ok(runtime),
            Err(error) if requested_install.install_id != DEFAULT_WORLD_PACK_INSTALL_ID => {
                let daihon_diagnostics =
                    diagnostics_from_error_for_install(&error, &requested_install);
                registry.mark_load_error(&requested_install.install_id, error.to_string())?;
                let default_install = registry.default_install()?;
                let mut status = registry.selection_state(&default_install, true);
                status.daihon_diagnostics = daihon_diagnostics;
                Self::open_with_status(registry.config_for_install(&default_install), status).await
            }
            Err(error) => Err(error),
        }
    }

    pub async fn select_world_pack_directory(path: impl AsRef<Path>) -> Result<Self> {
        Self::select_world_pack_directory_in(LocalRuntimeEnvironment::default_local(), path).await
    }

    pub async fn select_world_pack_directory_in(
        env: LocalRuntimeEnvironment,
        path: impl AsRef<Path>,
    ) -> Result<Self> {
        let mut registry = world_pack_registry::WorldPackRegistry::open(env)?;
        let install = registry.install_from_directory(path)?;
        registry.stage_active_install(install.clone());
        let status = registry.selection_state(&install, false);
        let runtime = Self::open_with_status(registry.config_for_install(&install), status).await?;
        registry.save()?;
        Ok(runtime)
    }

    pub fn inspect_world_pack_zip(path: impl AsRef<Path>) -> Result<WorldPackZipInspection> {
        Self::inspect_world_pack_zip_in(LocalRuntimeEnvironment::default_local(), path)
    }

    pub fn inspect_world_pack_zip_in(
        env: LocalRuntimeEnvironment,
        path: impl AsRef<Path>,
    ) -> Result<WorldPackZipInspection> {
        inspect_world_pack_zip_at(&env.data_dir, path)
    }

    pub async fn import_world_pack_zip(path: impl AsRef<Path>) -> Result<Self> {
        Self::import_world_pack_zip_in(LocalRuntimeEnvironment::default_local(), path).await
    }

    pub async fn import_world_pack_zip_in(
        env: LocalRuntimeEnvironment,
        path: impl AsRef<Path>,
    ) -> Result<Self> {
        let imported_root = import_world_pack_zip_to_dir(&env.data_dir, path)?;
        Self::select_world_pack_directory_in(env, imported_root).await
    }

    pub async fn reset_world_pack_to_default() -> Result<Self> {
        Self::reset_world_pack_to_default_in(LocalRuntimeEnvironment::default_local()).await
    }

    pub async fn reset_world_pack_to_default_in(env: LocalRuntimeEnvironment) -> Result<Self> {
        let mut registry = world_pack_registry::WorldPackRegistry::open(env)?;
        let install = registry.default_install()?;
        registry.stage_active_install(install.clone());
        let status = registry.selection_state(&install, false);
        let runtime = Self::open_with_status(registry.config_for_install(&install), status).await?;
        registry.save()?;
        Ok(runtime)
    }

    pub fn extension_settings_state() -> Result<ExtensionSettingsState> {
        Self::extension_settings_state_in(LocalRuntimeEnvironment::default_local())
    }

    pub fn extension_settings_state_in(
        env: LocalRuntimeEnvironment,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        Ok(config.extension_settings_registry()?.state())
    }

    pub fn app_settings_state() -> Result<AppSettingsState> {
        Self::app_settings_state_in(LocalRuntimeEnvironment::default_local())
    }

    pub fn app_settings_state_in(env: LocalRuntimeEnvironment) -> Result<AppSettingsState> {
        let registry = AppSettingsRegistry::open(&env.data_dir)?;
        Ok(registry.state())
    }

    pub fn runtime_settings_state() -> Result<RuntimeSettingsState> {
        Self::runtime_settings_state_in(LocalRuntimeEnvironment::default_local())
    }

    pub fn runtime_settings_state_in(env: LocalRuntimeEnvironment) -> Result<RuntimeSettingsState> {
        let registry = RuntimeSettingsRegistry::open(&env.data_dir)?;
        Ok(registry.state())
    }

    pub fn observation_settings_state() -> Result<ObservationSettingsState> {
        Self::observation_settings_state_in(LocalRuntimeEnvironment::default_local())
    }

    pub fn observation_settings_state_in(
        env: LocalRuntimeEnvironment,
    ) -> Result<ObservationSettingsState> {
        let registry = ObservationSettingsRegistry::open(&env.data_dir)?;
        Ok(registry.state())
    }

    pub fn set_observation_settings(
        settings: ObservationSettingsUpdate,
    ) -> Result<ObservationSettingsState> {
        Self::set_observation_settings_in(LocalRuntimeEnvironment::default_local(), settings)
    }

    pub fn set_observation_settings_in(
        env: LocalRuntimeEnvironment,
        settings: ObservationSettingsUpdate,
    ) -> Result<ObservationSettingsState> {
        let mut registry = ObservationSettingsRegistry::open(&env.data_dir)?;
        registry.set(settings)
    }

    pub fn onboarding_state() -> Result<OnboardingState> {
        Self::onboarding_state_in(LocalRuntimeEnvironment::default_local())
    }

    pub fn onboarding_state_in(env: LocalRuntimeEnvironment) -> Result<OnboardingState> {
        let registry = OnboardingRegistry::open(&env.data_dir)?;
        Ok(registry.state())
    }

    pub fn complete_onboarding() -> Result<OnboardingState> {
        Self::complete_onboarding_in(LocalRuntimeEnvironment::default_local())
    }

    pub fn complete_onboarding_in(env: LocalRuntimeEnvironment) -> Result<OnboardingState> {
        let mut registry = OnboardingRegistry::open(&env.data_dir)?;
        registry.complete()
    }

    pub fn dismiss_onboarding() -> Result<OnboardingState> {
        Self::dismiss_onboarding_in(LocalRuntimeEnvironment::default_local())
    }

    pub fn dismiss_onboarding_in(env: LocalRuntimeEnvironment) -> Result<OnboardingState> {
        let mut registry = OnboardingRegistry::open(&env.data_dir)?;
        registry.dismiss()
    }

    pub fn set_app_talk_interval_minutes(minutes: u64) -> Result<AppSettingsState> {
        Self::set_app_talk_interval_minutes_in(LocalRuntimeEnvironment::default_local(), minutes)
    }

    pub fn set_app_talk_interval_minutes_in(
        env: LocalRuntimeEnvironment,
        minutes: u64,
    ) -> Result<AppSettingsState> {
        let mut registry = AppSettingsRegistry::open(&env.data_dir)?;
        registry.set_talk_interval_minutes(minutes)
    }

    pub fn set_app_actor_scale_percent(percent: u16) -> Result<AppSettingsState> {
        Self::set_app_actor_scale_percent_in(LocalRuntimeEnvironment::default_local(), percent)
    }

    pub fn set_app_actor_scale_percent_in(
        env: LocalRuntimeEnvironment,
        percent: u16,
    ) -> Result<AppSettingsState> {
        let mut registry = AppSettingsRegistry::open(&env.data_dir)?;
        registry.set_actor_scale_percent(percent)
    }

    pub fn set_app_conversation_send_shortcut_in(
        env: LocalRuntimeEnvironment,
        shortcut: ConversationSendShortcut,
    ) -> Result<AppSettingsState> {
        let mut registry = AppSettingsRegistry::open(&env.data_dir)?;
        registry.set_conversation_send_shortcut(shortcut)
    }

    pub fn set_runtime_settings(settings: RuntimeSettingsUpdate) -> Result<RuntimeSettingsState> {
        Self::set_runtime_settings_in(LocalRuntimeEnvironment::default_local(), settings)
    }

    pub fn set_runtime_settings_in(
        env: LocalRuntimeEnvironment,
        settings: RuntimeSettingsUpdate,
    ) -> Result<RuntimeSettingsState> {
        let mut registry = RuntimeSettingsRegistry::open(&env.data_dir)?;
        registry.set(settings)
    }

    pub fn install_extension_directory(path: impl AsRef<Path>) -> Result<ExtensionSettingsState> {
        Self::install_extension_directory_in(LocalRuntimeEnvironment::default_local(), path)
    }

    pub fn install_extension_directory_in(
        env: LocalRuntimeEnvironment,
        path: impl AsRef<Path>,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.install_from_directory(path)
    }

    pub fn uninstall_extension(extension_id: &str) -> Result<ExtensionSettingsState> {
        Self::uninstall_extension_in(LocalRuntimeEnvironment::default_local(), extension_id)
    }

    pub fn uninstall_extension_in(
        env: LocalRuntimeEnvironment,
        extension_id: &str,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.uninstall(extension_id)
    }

    pub fn set_extension_enabled(
        extension_id: &str,
        enabled: bool,
    ) -> Result<ExtensionSettingsState> {
        Self::set_extension_enabled_in(
            LocalRuntimeEnvironment::default_local(),
            extension_id,
            enabled,
        )
    }

    pub fn set_extension_enabled_in(
        env: LocalRuntimeEnvironment,
        extension_id: &str,
        enabled: bool,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.set_enabled(extension_id, enabled)
    }

    pub fn set_extension_hook_order(
        hook_point: ExtensionHookPoint,
        extension_ids: Vec<String>,
    ) -> Result<ExtensionSettingsState> {
        Self::set_extension_hook_order_in(
            LocalRuntimeEnvironment::default_local(),
            hook_point,
            extension_ids,
        )
    }

    pub fn set_extension_hook_order_in(
        env: LocalRuntimeEnvironment,
        hook_point: ExtensionHookPoint,
        extension_ids: Vec<String>,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.set_hook_order(hook_point, extension_ids)
    }

    pub fn set_capability_default(
        capability: &str,
        extension_id: &str,
    ) -> Result<ExtensionSettingsState> {
        Self::set_capability_default_in(
            LocalRuntimeEnvironment::default_local(),
            capability,
            extension_id,
        )
    }

    pub fn set_capability_default_in(
        env: LocalRuntimeEnvironment,
        capability: &str,
        extension_id: &str,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.set_capability_default(capability, extension_id)
    }

    pub fn set_extension_setting_values(
        extension_id: &str,
        values: Map<String, Value>,
    ) -> Result<ExtensionSettingsState> {
        Self::set_extension_setting_values_in(
            LocalRuntimeEnvironment::default_local(),
            extension_id,
            values,
        )
    }

    pub fn set_extension_setting_values_in(
        env: LocalRuntimeEnvironment,
        extension_id: &str,
        values: Map<String, Value>,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.set_extension_setting_values(extension_id, values)
    }

    pub fn set_extension_secret(
        extension_id: &str,
        key: &str,
        value: Option<String>,
    ) -> Result<ExtensionSettingsState> {
        Self::set_extension_secret_in(
            LocalRuntimeEnvironment::default_local(),
            extension_id,
            key,
            value,
        )
    }

    pub fn set_extension_secret_in(
        env: LocalRuntimeEnvironment,
        extension_id: &str,
        key: &str,
        value: Option<String>,
    ) -> Result<ExtensionSettingsState> {
        let config = extension_config_for_env(env);
        let mut registry = config.extension_settings_registry()?;
        registry.set_extension_secret(extension_id, key, value)
    }
}
