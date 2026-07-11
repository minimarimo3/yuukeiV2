use super::*;

pub(crate) fn extension_config_for_env(env: LocalRuntimeEnvironment) -> LocalRuntimeConfig {
    let LocalRuntimeEnvironment {
        workspace_root,
        default_world_root,
        data_dir,
        device_id,
    } = env;
    let world_root = default_world_root;
    let extension_root = data_dir.join("extensions");
    let event_log_path = data_dir
        .join("residents")
        .join(DEFAULT_WORLD_PACK_INSTALL_ID)
        .join("events.sqlite3");
    let scene_history_path = data_dir
        .join("residents")
        .join(DEFAULT_WORLD_PACK_INSTALL_ID)
        .join("scene-history.json");
    let variables_path = data_dir
        .join("residents")
        .join(DEFAULT_WORLD_PACK_INSTALL_ID)
        .join("variables.json");
    let mood_state_path = data_dir
        .join("residents")
        .join(DEFAULT_WORLD_PACK_INSTALL_ID)
        .join("mood.json");
    let app_log_path = data_dir.join("app-activity.jsonl");
    LocalRuntimeConfig {
        install_id: DEFAULT_WORLD_PACK_INSTALL_ID.to_string(),
        resident_id: DEFAULT_RESIDENT_ID.to_string(),
        device_id,
        workspace_root,
        world_root,
        extension_root,
        event_log_path,
        scene_history_path,
        variables_path,
        mood_state_path,
        app_log_path,
        data_dir,
    }
}

pub(crate) fn extension_data_dir(data_dir: &Path, extension_id: &str) -> PathBuf {
    data_dir.join("extension-data").join(extension_id)
}

pub(crate) fn paths_payload(paths: &RuntimePaths) -> JsonMap {
    JsonMap::from([
        (
            "workspaceRoot".to_string(),
            json!(display_path(&paths.workspace_root)),
        ),
        ("dataDir".to_string(), json!(display_path(&paths.data_dir))),
        (
            "worldRoot".to_string(),
            json!(display_path(&paths.world_root)),
        ),
        (
            "extensionRoot".to_string(),
            json!(display_path(&paths.extension_root)),
        ),
        (
            "eventLogPath".to_string(),
            json!(display_path(&paths.event_log_path)),
        ),
        (
            "sceneHistoryPath".to_string(),
            json!(display_path(&paths.scene_history_path)),
        ),
        (
            "variablesPath".to_string(),
            json!(display_path(&paths.variables_path)),
        ),
        (
            "moodStatePath".to_string(),
            json!(display_path(&paths.mood_state_path)),
        ),
        (
            "appLogPath".to_string(),
            json!(display_path(&paths.app_log_path)),
        ),
    ])
}

pub(crate) fn error_payload(stage: &str, error: &dyn std::fmt::Display) -> JsonMap {
    JsonMap::from([
        ("stage".to_string(), json!(stage)),
        ("message".to_string(), json!(error.to_string())),
    ])
}

pub(crate) fn diagnostics_from_error_for_install(
    error: &DeviceHostError,
    install: &WorldPackInstall,
) -> Vec<DaihonDiagnosticEntry> {
    error
        .daihon_report()
        .map(|report| diagnostics_for_install(install, report.diagnostics.clone()))
        .unwrap_or_default()
}

pub(crate) fn diagnostics_for_install(
    install: &WorldPackInstall,
    diagnostics: Vec<DaihonDiagnosticEntry>,
) -> Vec<DaihonDiagnosticEntry> {
    let occurred_at = now_timestamp();
    diagnostics
        .into_iter()
        .map(|diagnostic| enrich_diagnostic_for_install(diagnostic, install, Some(&occurred_at)))
        .collect()
}

pub(crate) fn diagnostics_for_pack_root(
    pack_root: &Path,
    diagnostics: Vec<DaihonDiagnosticEntry>,
) -> Vec<DaihonDiagnosticEntry> {
    let occurred_at = now_timestamp();
    let pack_root = display_path(pack_root);
    diagnostics
        .into_iter()
        .map(|mut diagnostic| {
            if diagnostic.occurred_at.is_none() {
                diagnostic.occurred_at = Some(occurred_at.clone());
            }
            if diagnostic.pack_root.is_none() {
                diagnostic.pack_root = Some(pack_root.clone());
            }
            diagnostic
        })
        .collect()
}

pub(crate) fn enrich_diagnostic_for_install(
    mut diagnostic: DaihonDiagnosticEntry,
    install: &WorldPackInstall,
    occurred_at: Option<&str>,
) -> DaihonDiagnosticEntry {
    if diagnostic.occurred_at.is_none() {
        if let Some(occurred_at) = occurred_at {
            diagnostic.occurred_at = Some(occurred_at.to_string());
        }
    }
    if diagnostic.install_id.is_none() {
        diagnostic.install_id = Some(install.install_id.clone());
    }
    if diagnostic.world_pack_id.is_none() {
        diagnostic.world_pack_id = Some(install.world_pack_id.clone());
    }
    if diagnostic.pack_root.is_none() {
        diagnostic.pack_root = Some(display_path(&install.canonical_root));
    }
    diagnostic
}

pub(crate) fn record_daihon_diagnostics_to_app_log(
    logger: &AppLogger,
    context: &str,
    diagnostics: &[DaihonDiagnosticEntry],
) -> Result<()> {
    if diagnostics.is_empty() {
        return Ok(());
    }
    logger.record(
        "daihon.diagnostics",
        "device-host",
        JsonMap::from([
            ("context".to_string(), json!(context)),
            ("count".to_string(), json!(diagnostics.len())),
            (
                "diagnostics".to_string(),
                serde_json::to_value(diagnostics)?,
            ),
        ]),
    )?;
    Ok(())
}

pub(crate) async fn load_trusted_extensions(
    extension_settings: &ExtensionSettingsRegistry,
    home: &ResidentHome,
    logger: &AppLogger,
    data_dir: &Path,
    process_runtime_supervisor: &ProcessRuntimeSupervisor,
) -> Result<usize> {
    let mut loaded = 0;
    let hook_order = extension_settings.hook_order(&ExtensionHookPoint::BeforeCommandEmit);
    for entry in extension_settings.runtime_entries() {
        match entry {
            ExtensionRuntimeEntry::Ready(install) => {
                let extension_id = install.extension_id.clone();
                let manifest_path = install.manifest_path.clone();
                let extension_data_dir = extension_data_dir(data_dir, &extension_id);
                fs::create_dir_all(&extension_data_dir)?;
                home.register_extension(extension_with_runtime_environment(
                    install.manifest,
                    install.install_dir,
                    install.enabled,
                    extension_data_dir,
                    install.settings_json,
                    process_runtime_supervisor.clone(),
                ))
                .await?;
                loaded += 1;
                logger.record(
                    "extension.load.ready",
                    "device-host",
                    JsonMap::from([
                        ("extensionId".to_string(), json!(extension_id)),
                        (
                            "manifestPath".to_string(),
                            json!(display_path(&manifest_path)),
                        ),
                        ("enabled".to_string(), json!(install.enabled)),
                    ]),
                )?;
            }
            ExtensionRuntimeEntry::Error(error) => {
                logger.record(
                    "extension.load.error",
                    "device-host",
                    JsonMap::from([
                        ("extensionId".to_string(), json!(error.extension_id)),
                        (
                            "manifestPath".to_string(),
                            json!(display_path(&error.manifest_path)),
                        ),
                        ("message".to_string(), json!(error.message)),
                    ]),
                )?;
            }
        }
    }
    home.set_extension_hook_order(ExtensionHookPoint::BeforeCommandEmit, hook_order)?;
    Ok(loaded)
}

pub(crate) fn build_extension_capability_router(
    extension_settings: &ExtensionSettingsRegistry,
    logger: &AppLogger,
    data_dir: &Path,
    process_runtime_supervisor: &ProcessRuntimeSupervisor,
) -> Result<CapabilityRouter> {
    let mut router = CapabilityRouter::new();
    for entry in extension_settings.runtime_entries() {
        match entry {
            ExtensionRuntimeEntry::Ready(install) => {
                if install.manifest.capabilities.is_empty() {
                    continue;
                }
                let extension_data_dir = extension_data_dir(data_dir, &install.extension_id);
                fs::create_dir_all(&extension_data_dir)?;
                router
                    .register(extension_with_runtime_environment(
                        install.manifest,
                        install.install_dir,
                        install.enabled,
                        extension_data_dir,
                        install.settings_json,
                        process_runtime_supervisor.clone(),
                    ))
                    .map_err(|error| DeviceHostError::ExtensionSettings(error.to_string()))?;
            }
            ExtensionRuntimeEntry::Error(error) => {
                logger.record(
                    "extension.load.error",
                    "device-host",
                    JsonMap::from([
                        ("extensionId".to_string(), json!(error.extension_id)),
                        (
                            "manifestPath".to_string(),
                            json!(display_path(&error.manifest_path)),
                        ),
                        ("message".to_string(), json!(error.message)),
                    ]),
                )?;
            }
        }
    }
    for (capability, extension_id) in extension_settings.capability_defaults() {
        router.set_default_extension(capability, extension_id);
    }
    Ok(router)
}

pub(crate) fn extension_with_runtime_environment(
    manifest: yuukei_extension::ProcessExtensionManifest,
    install_dir: impl Into<PathBuf>,
    enabled: bool,
    data_dir: impl Into<PathBuf>,
    settings_json: Option<String>,
    process_runtime_supervisor: ProcessRuntimeSupervisor,
) -> ProcessHookExtension {
    let extension = ProcessHookExtension::from_installed_manifest(manifest, install_dir, enabled)
        .with_data_dir(data_dir)
        .with_runtime_supervisor(process_runtime_supervisor);
    if let Some(settings_json) = settings_json {
        extension.with_settings_json(settings_json)
    } else {
        extension
    }
}

pub(crate) fn trim_event_log_if_needed(home: &ResidentHome, logger: &AppLogger) {
    match home.trim_event_log_to_record_limit(
        DEFAULT_MAX_EVENT_LOG_RECORDS,
        DEFAULT_EVENT_LOG_TRIM_FRACTION_DIVISOR,
    ) {
        Ok(summary) if summary.deleted > 0 => {
            let _ = logger.record(
                "event_log.trimmed",
                "device-host",
                JsonMap::from([
                    ("deleted".to_string(), json!(summary.deleted)),
                    (
                        "oldestTimestamp".to_string(),
                        summary
                            .oldest_timestamp
                            .map(Value::String)
                            .unwrap_or(Value::Null),
                    ),
                    (
                        "newestTimestamp".to_string(),
                        summary
                            .newest_timestamp
                            .map(Value::String)
                            .unwrap_or(Value::Null),
                    ),
                ]),
            );
        }
        Ok(_) => {}
        Err(error) => {
            let _ = logger.record(
                "event_log.trim.error",
                "device-host",
                error_payload("event-log", &error),
            );
        }
    }
}

