use super::*;

#[derive(Clone)]
pub struct LocalYuukeiRuntime {
    home: Arc<ResidentHome>,
    daihon_adapter: Arc<YuukeiDaihonAdapter>,
    logger: AppLogger,
    install_id: String,
    resident_id: String,
    device_id: String,
    paths: RuntimePaths,
    world_pack_status: WorldPackSelectionState,
    actor_surface_assets: ActorSurfaceAssetCatalog,
    presence_state: Arc<Mutex<PresenceState>>,
    session_daihon_diagnostics: Arc<Mutex<Vec<DaihonDiagnosticEntry>>>,
    process_runtime_supervisor: ProcessRuntimeSupervisor,
}

impl LocalYuukeiRuntime {
    pub fn capability_usage(&self) -> Result<CapabilityUsageState> {
        let records = self
            .home
            .event_log()
            .read(EventLogQuery::default())?
            .records;
        Ok(capability_usage_from_records(&records, Utc::now()))
    }

    pub async fn open(config: LocalRuntimeConfig) -> Result<Self> {
        let install = WorldPackInstall {
            install_id: config.install_id.clone(),
            resident_id: config.resident_id.clone(),
            world_pack_id: "custom".to_string(),
            display_name: "Custom World Pack".to_string(),
            canonical_root: config.world_root.clone(),
            source: WorldPackSource::ExternalDirectory,
            last_load_error: None,
        };
        let status = WorldPackSelectionState {
            configured_install_id: install.install_id.clone(),
            running_install_id: install.install_id.clone(),
            active_install: install.clone(),
            installs: vec![install],
            fallback_active: false,
            last_load_error: None,
            daihon_diagnostics: Vec::new(),
            settings_path: config.data_dir.join("settings").join("world-packs.json"),
        };
        Self::open_with_status(config, status).await
    }

    pub(crate) async fn open_with_status(
        config: LocalRuntimeConfig,
        mut world_pack_status: WorldPackSelectionState,
    ) -> Result<Self> {
        fs::create_dir_all(&config.data_dir)?;
        fs::create_dir_all(&config.extension_root)?;
        if let Some(parent) = config.event_log_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Some(parent) = config.scene_history_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let logger = AppLogger::open(&config.app_log_path)?;
        let paths = config.paths();
        let initial_session_daihon_diagnostics =
            std::mem::take(&mut world_pack_status.daihon_diagnostics);
        if !initial_session_daihon_diagnostics.is_empty() {
            let _ = record_daihon_diagnostics_to_app_log(
                &logger,
                "world-pack.fallback-load",
                &initial_session_daihon_diagnostics,
            );
        }
        logger.record("runtime.open.request", "device-host", paths_payload(&paths))?;

        let world = match WorldPack::load_from_dir(&config.world_root) {
            Ok(world) => world,
            Err(error) => {
                if let Some(report) = error.daihon_report() {
                    let diagnostics = diagnostics_for_install(
                        &world_pack_status.active_install,
                        report.diagnostics.clone(),
                    );
                    let _ = record_daihon_diagnostics_to_app_log(
                        &logger,
                        "world-pack.load",
                        &diagnostics,
                    );
                }
                let _ = logger.record(
                    "runtime.open.error",
                    "device-host",
                    error_payload("world-pack", &error),
                );
                return Err(error.into());
            }
        };
        let actor_surface_assets = actor_surface_asset_catalog(&world);
        let event_log = match EventLog::open(&config.event_log_path) {
            Ok(event_log) => event_log,
            Err(error) => {
                let _ = logger.record(
                    "runtime.open.error",
                    "device-host",
                    error_payload("event-log", &error),
                );
                return Err(error.into());
            }
        };
        let extension_settings = config.extension_settings_registry()?;
        let mut resident_runtime_settings =
            RuntimeSettingsRegistry::open(&config.data_dir)?.resident_runtime_settings();
        resident_runtime_settings.mood_state_path = Some(config.mood_state_path.clone());
        let process_runtime_supervisor = ProcessRuntimeSupervisor::new();
        let capabilities = build_extension_capability_router(
            &extension_settings,
            &logger,
            &config.data_dir,
            &process_runtime_supervisor,
        )?;
        let scene_history_logger = {
            let logger = logger.clone();
            Arc::new(move |error: yuukei_world::SceneHistoryPersistenceError| {
                let _ = logger.record(
                    "scene-history.persistence-error",
                    "device-host",
                    JsonMap::from([
                        ("path".to_string(), json!(display_path(&error.path))),
                        ("message".to_string(), json!(error.message)),
                    ]),
                );
            })
        };
        let variable_logger = {
            let logger = logger.clone();
            Arc::new(move |error: yuukei_world::VariablePersistenceError| {
                let _ = logger.record(
                    "variables.persistence-error",
                    "device-host",
                    JsonMap::from([
                        ("path".to_string(), json!(display_path(&error.path))),
                        ("message".to_string(), json!(error.message)),
                    ]),
                );
            })
        };
        let daihon_adapter = Arc::new(YuukeiDaihonAdapter::with_persistent_state_loggers(
            config.scene_history_path.clone(),
            scene_history_logger,
            config.variables_path.clone(),
            variable_logger,
        ));
        let home = match ResidentHome::with_parts_and_runtime_settings(
            &config.resident_id,
            world,
            event_log,
            daihon_adapter.clone(),
            capabilities,
            resident_runtime_settings,
        )
        .await
        {
            Ok(home) => Arc::new(home),
            Err(error) => {
                if let Some(report) = error.daihon_report() {
                    let diagnostics = diagnostics_for_install(
                        &world_pack_status.active_install,
                        report.diagnostics.clone(),
                    );
                    let _ = record_daihon_diagnostics_to_app_log(
                        &logger,
                        "resident-home.load",
                        &diagnostics,
                    );
                }
                let _ = logger.record(
                    "runtime.open.error",
                    "device-host",
                    error_payload("resident-home", &error),
                );
                return Err(error.into());
            }
        };
        let loaded_extensions = load_trusted_extensions(
            &extension_settings,
            &home,
            &logger,
            &config.data_dir,
            &process_runtime_supervisor,
        )
        .await?;
        trim_event_log_if_needed(&home, &logger);

        logger.record(
            "runtime.open.ready",
            "device-host",
            JsonMap::from([
                ("residentId".to_string(), json!(config.resident_id)),
                ("deviceId".to_string(), json!(config.device_id)),
                (
                    "worldRoot".to_string(),
                    json!(display_path(&config.world_root)),
                ),
                (
                    "eventLogPath".to_string(),
                    json!(display_path(&config.event_log_path)),
                ),
                (
                    "extensionRoot".to_string(),
                    json!(display_path(&config.extension_root)),
                ),
                ("loadedExtensions".to_string(), json!(loaded_extensions)),
                (
                    "appLogPath".to_string(),
                    json!(display_path(&config.app_log_path)),
                ),
            ]),
        )?;

        let runtime = Self {
            home,
            daihon_adapter,
            logger,
            install_id: config.install_id,
            resident_id: config.resident_id,
            device_id: config.device_id,
            paths,
            world_pack_status,
            actor_surface_assets,
            presence_state: Arc::new(Mutex::new(PresenceState::default())),
            session_daihon_diagnostics: Arc::new(Mutex::new(initial_session_daihon_diagnostics)),
            process_runtime_supervisor,
        };
        runtime.record_world_pack_activated().await?;
        runtime.spawn_event_log_trim_loop();
        Ok(runtime)
    }

    pub fn home(&self) -> Arc<ResidentHome> {
        self.home.clone()
    }

    pub fn logger(&self) -> AppLogger {
        self.logger.clone()
    }

    fn spawn_event_log_trim_loop(&self) {
        let home = self.home.clone();
        let logger = self.logger.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(EVENT_LOG_TRIM_CHECK_INTERVAL);
            loop {
                interval.tick().await;
                trim_event_log_if_needed(&home, &logger);
            }
        });
    }

    pub fn paths(&self) -> &RuntimePaths {
        &self.paths
    }

    pub async fn scene_history_state(&self) -> Result<SceneHistoryState> {
        Ok(SceneHistoryState {
            install_id: self.install_id.clone(),
            history_path: self.paths.scene_history_path.clone(),
            entries: self.daihon_adapter.scene_history_entries().await,
        })
    }

    pub async fn reset_scene_history(&self) -> Result<SceneHistoryState> {
        self.daihon_adapter.reset_scene_history().await;
        self.logger.record(
            "scene-history.reset",
            "device-host",
            JsonMap::from([
                ("installId".to_string(), json!(self.install_id)),
                (
                    "historyPath".to_string(),
                    json!(display_path(&self.paths.scene_history_path)),
                ),
            ]),
        )?;
        self.scene_history_state().await
    }

    pub fn install_id(&self) -> &str {
        &self.install_id
    }

    pub fn resident_id(&self) -> &str {
        &self.resident_id
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn snapshot(&self) -> Result<ResidentSnapshot> {
        self.home.snapshot().map_err(Into::into)
    }

    pub fn world_pack_status(&self) -> WorldPackSelectionState {
        let mut status = self.world_pack_status.clone();
        status.daihon_diagnostics = self.session_daihon_diagnostics();
        status
    }

    pub fn actor_surface_assets(&self) -> ActorSurfaceAssetCatalog {
        self.actor_surface_assets.clone()
    }

    pub fn extension_settings(&self) -> Result<ExtensionSettingsState> {
        let mut state =
            ExtensionSettingsRegistry::open(&self.paths.data_dir, &self.paths.extension_root)
                .map(|registry| registry.state())?;
        let statuses = self.process_runtime_supervisor.statuses();
        for extension in &mut state.installed {
            extension.runtime_status = statuses.get(&extension.extension_id).cloned();
        }
        Ok(state)
    }

    pub fn restart_extension_process(&self, extension_id: &str) -> Result<ExtensionSettingsState> {
        self.process_runtime_supervisor.restart(extension_id);
        self.logger.record(
            "extension.process.restart",
            "device-host",
            JsonMap::from([("extensionId".to_string(), json!(extension_id))]),
        )?;
        self.extension_settings()
    }

    pub fn app_settings(&self) -> Result<AppSettingsState> {
        AppSettingsRegistry::open(&self.paths.data_dir).map(|registry| registry.state())
    }

    pub fn runtime_settings(&self) -> Result<RuntimeSettingsState> {
        RuntimeSettingsRegistry::open(&self.paths.data_dir).map(|registry| registry.state())
    }

    pub fn observation_settings(&self) -> Result<ObservationSettingsState> {
        ObservationSettingsRegistry::open(&self.paths.data_dir).map(|registry| registry.state())
    }

    pub async fn list_resident_memories(
        &self,
        episode_limit: Option<usize>,
        episode_offset: Option<usize>,
    ) -> Result<MemoryListOutput> {
        self.home
            .list_memories(episode_limit, episode_offset)
            .await
            .map_err(Into::into)
    }

    pub async fn update_resident_memory(
        &self,
        kind: MemoryEntryKind,
        id: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<MemoryUpdateOutput> {
        self.home
            .update_memory(kind, id, text)
            .await
            .map_err(Into::into)
    }

    pub async fn forget_resident_memories(
        &self,
        entries: Vec<MemoryForgetEntry>,
        all: bool,
    ) -> Result<MemoryForgetOutput> {
        self.home
            .forget_memories(entries, all)
            .await
            .map_err(Into::into)
    }

    pub fn read_event_log_page(
        &self,
        kind_prefix: Option<String>,
        privacy_category: EventLogPrivacyCategoryFilter,
        before_sequence: Option<i64>,
        limit: Option<usize>,
    ) -> Result<ResidentEventLogPage> {
        self.home
            .read_event_log_page(ResidentEventLogReadOptions {
                kind_prefix,
                privacy_category: privacy_category.into(),
                before_sequence,
                limit,
            })
            .map_err(Into::into)
    }

    pub fn count_event_log_delete_before(&self, timestamp: impl Into<String>) -> Result<usize> {
        self.home
            .count_event_log_delete_before(timestamp)
            .map_err(Into::into)
    }

    pub fn count_event_log_delete_by_kind_prefix(
        &self,
        prefix: impl Into<String>,
    ) -> Result<usize> {
        self.home
            .count_event_log_delete_by_kind_prefix(prefix)
            .map_err(Into::into)
    }

    pub fn count_event_log_delete_all(&self) -> Result<usize> {
        self.home.count_event_log_delete_all().map_err(Into::into)
    }

    pub fn delete_event_log_before(
        &self,
        timestamp: impl Into<String>,
    ) -> Result<EventLogDeleteResult> {
        self.home
            .delete_event_log_before(timestamp)
            .map(EventLogDeleteResult::from)
            .map_err(Into::into)
    }

    pub fn delete_event_log_by_kind_prefix(
        &self,
        prefix: impl Into<String>,
    ) -> Result<EventLogDeleteResult> {
        self.home
            .delete_event_log_by_kind_prefix(prefix)
            .map(EventLogDeleteResult::from)
            .map_err(Into::into)
    }

    pub fn delete_event_log_all(&self) -> Result<EventLogDeleteResult> {
        self.home
            .delete_event_log_all()
            .map(EventLogDeleteResult::from)
            .map_err(Into::into)
    }

    pub fn record_session_daihon_diagnostics_from_error(
        &self,
        error: &DeviceHostError,
        pack_root: Option<&Path>,
    ) -> Result<usize> {
        let Some(report) = error.daihon_report() else {
            return Ok(0);
        };
        let diagnostics = match pack_root {
            Some(pack_root) => diagnostics_for_pack_root(pack_root, report.diagnostics.clone()),
            None => diagnostics_for_install(
                &self.world_pack_status.active_install,
                report.diagnostics.clone(),
            ),
        };
        let count = diagnostics.len();
        if count == 0 {
            return Ok(0);
        }
        {
            let mut session = self
                .session_daihon_diagnostics
                .lock()
                .map_err(|_| DeviceHostError::DaihonDiagnosticState)?;
            session.extend(diagnostics.clone());
        }
        record_daihon_diagnostics_to_app_log(&self.logger, "world-pack.selection", &diagnostics)?;
        Ok(count)
    }

    pub async fn attach_surface(&self, session: SurfaceSession) -> Result<ResidentSnapshot> {
        self.logger.record(
            "surface.attach.request",
            "device-host",
            JsonMap::from([
                ("surfaceId".to_string(), json!(session.surface_id)),
                ("deviceId".to_string(), json!(session.device_id)),
                ("kind".to_string(), json!(session.kind)),
                ("presentation".to_string(), json!(session.presentation)),
            ]),
        )?;
        match self.home.attach_surface(session.clone()).await {
            Ok(snapshot) => {
                self.logger.record(
                    "surface.attach.ready",
                    "resident-home",
                    JsonMap::from([
                        ("surfaceId".to_string(), json!(session.surface_id)),
                        (
                            "activeSurfaceId".to_string(),
                            json!(snapshot.active_surface_id),
                        ),
                        (
                            "recentEventCursor".to_string(),
                            json!(snapshot.recent_event_cursor),
                        ),
                    ]),
                )?;
                Ok(snapshot)
            }
            Err(error) => {
                self.record_runtime_daihon_diagnostics("surface.attach", &error);
                let _ = self.logger.record(
                    "surface.attach.error",
                    "resident-home",
                    error_payload("surface.attach", &error),
                );
                Err(error.into())
            }
        }
    }

    pub async fn send_conversation_text(
        &self,
        surface_id: &str,
        text: &str,
    ) -> Result<Vec<RuntimeCommand>> {
        self.mark_user_activity(Utc::now())?;
        let event = build_conversation_text_event(
            self.resident_id(),
            self.device_id(),
            surface_id,
            text.trim(),
        );
        self.logger.record(
            "surface.input.conversation_text",
            "surface",
            JsonMap::from([
                ("eventId".to_string(), json!(event.id)),
                ("surfaceId".to_string(), json!(surface_id)),
                ("textLength".to_string(), json!(text.chars().count())),
            ]),
        )?;

        match self.home.ingest_event(event.clone()).await {
            Ok(commands) => {
                self.logger.record(
                    "runtime.commands.emitted",
                    "resident-home",
                    JsonMap::from([
                        ("sourceEventId".to_string(), json!(event.id)),
                        (
                            "commandIds".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.id)
                                .collect::<Vec<_>>()),
                        ),
                        (
                            "commandTypes".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.kind)
                                .collect::<Vec<_>>()),
                        ),
                        ("count".to_string(), json!(commands.len())),
                    ]),
                )?;
                Ok(commands)
            }
            Err(error) => {
                self.record_runtime_daihon_diagnostics("conversation.text", &error);
                let _ = self.logger.record(
                    "runtime.commands.error",
                    "resident-home",
                    error_payload("conversation.text", &error),
                );
                Err(error.into())
            }
        }
    }

    pub async fn send_conversation_choice(
        &self,
        surface_id: &str,
        choice_id: &str,
        choice: &str,
        index: usize,
    ) -> Result<Vec<RuntimeCommand>> {
        self.mark_user_activity(Utc::now())?;
        let event = build_conversation_choice_event(
            self.resident_id(),
            self.device_id(),
            surface_id,
            choice_id,
            choice,
            index,
        );
        self.logger.record(
            "surface.input.conversation_choice",
            "surface",
            JsonMap::from([
                ("eventId".to_string(), json!(event.id)),
                ("surfaceId".to_string(), json!(surface_id)),
                ("choiceId".to_string(), json!(choice_id)),
                ("choice".to_string(), json!(choice)),
                ("index".to_string(), json!(index)),
            ]),
        )?;

        match self.home.ingest_event(event.clone()).await {
            Ok(commands) => {
                self.logger.record(
                    "runtime.commands.emitted",
                    "resident-home",
                    JsonMap::from([
                        ("sourceEventId".to_string(), json!(event.id)),
                        (
                            "commandIds".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.id)
                                .collect::<Vec<_>>()),
                        ),
                        (
                            "commandTypes".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.kind)
                                .collect::<Vec<_>>()),
                        ),
                        ("count".to_string(), json!(commands.len())),
                    ]),
                )?;
                Ok(commands)
            }
            Err(error) => {
                self.record_runtime_daihon_diagnostics("conversation.choice", &error);
                let _ = self.logger.record(
                    "runtime.commands.error",
                    "resident-home",
                    error_payload("conversation.choice", &error),
                );
                Err(error.into())
            }
        }
    }

    pub async fn send_avatar_gesture_poke(
        &self,
        surface_id: &str,
        gesture: AvatarGesturePoke,
    ) -> Result<Vec<RuntimeCommand>> {
        self.mark_user_activity(Utc::now())?;
        let event = build_avatar_gesture_poke_event(
            self.resident_id(),
            self.device_id(),
            surface_id,
            gesture,
        );
        self.logger.record(
            "surface.input.avatar_gesture",
            "surface",
            JsonMap::from([
                ("eventId".to_string(), json!(event.id)),
                ("surfaceId".to_string(), json!(surface_id)),
                ("actorId".to_string(), json!(event.actor_id.clone())),
                (
                    "hitZoneId".to_string(),
                    event
                        .payload
                        .get("hitZoneId")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
            ]),
        )?;

        match self.home.ingest_event(event.clone()).await {
            Ok(commands) => {
                self.logger.record(
                    "runtime.commands.emitted",
                    "resident-home",
                    JsonMap::from([
                        ("sourceEventId".to_string(), json!(event.id)),
                        (
                            "commandIds".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.id)
                                .collect::<Vec<_>>()),
                        ),
                        (
                            "commandTypes".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.kind)
                                .collect::<Vec<_>>()),
                        ),
                        ("count".to_string(), json!(commands.len())),
                    ]),
                )?;
                Ok(commands)
            }
            Err(error) => {
                self.record_runtime_daihon_diagnostics("avatar.gesture.poke", &error);
                let _ = self.logger.record(
                    "runtime.commands.error",
                    "resident-home",
                    error_payload("avatar.gesture.poke", &error),
                );
                Err(error.into())
            }
        }
    }

    pub async fn send_avatar_gesture_grab(
        &self,
        surface_id: &str,
        actor_id: &str,
    ) -> Result<Vec<RuntimeCommand>> {
        self.send_avatar_drag_event(surface_id, actor_id, "avatar.gesture.grab", None)
            .await
    }

    pub async fn send_avatar_gesture_drop(
        &self,
        surface_id: &str,
        actor_id: &str,
        moved_distance: u64,
    ) -> Result<Vec<RuntimeCommand>> {
        self.send_avatar_drag_event(
            surface_id,
            actor_id,
            "avatar.gesture.drop",
            Some(moved_distance),
        )
        .await
    }

    async fn send_avatar_drag_event(
        &self,
        surface_id: &str,
        actor_id: &str,
        kind: &str,
        moved_distance: Option<u64>,
    ) -> Result<Vec<RuntimeCommand>> {
        self.mark_user_activity(Utc::now())?;
        let event = build_avatar_drag_event(
            self.resident_id(),
            self.device_id(),
            surface_id,
            actor_id,
            kind,
            moved_distance,
        );
        self.logger.record(
            "surface.input.avatar_gesture",
            "surface",
            JsonMap::from([
                ("eventId".to_string(), json!(event.id)),
                ("eventType".to_string(), json!(event.kind)),
                ("surfaceId".to_string(), json!(surface_id)),
                ("actorId".to_string(), json!(actor_id)),
            ]),
        )?;
        match self.home.ingest_event(event.clone()).await {
            Ok(commands) => {
                self.logger.record(
                    "runtime.commands.emitted",
                    "resident-home",
                    JsonMap::from([
                        ("sourceEventId".to_string(), json!(event.id)),
                        (
                            "commandTypes".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.kind)
                                .collect::<Vec<_>>()),
                        ),
                        ("count".to_string(), json!(commands.len())),
                    ]),
                )?;
                Ok(commands)
            }
            Err(error) => {
                self.record_runtime_daihon_diagnostics(kind, &error);
                let _ = self.logger.record(
                    "runtime.commands.error",
                    "resident-home",
                    error_payload(kind, &error),
                );
                Err(error.into())
            }
        }
    }

    pub async fn emit_app_startup(&self) -> Result<Vec<RuntimeCommand>> {
        let snapshot = current_presence_snapshot();
        {
            let mut state = self
                .presence_state
                .lock()
                .map_err(|_| DeviceHostError::PresenceState)?;
            if state.startup_emitted {
                return Ok(Vec::new());
            }
            state.startup_emitted = true;
            state.last_time_period = Some(snapshot.time_period.to_string());
        }
        let result = self
            .emit_runtime_event("app.startup", snapshot.into_payload())
            .await;
        if result.is_err() {
            if let Ok(mut state) = self.presence_state.lock() {
                state.startup_emitted = false;
                state.last_time_period = None;
            }
        }
        result
    }

    pub async fn emit_presence_tick(&self) -> Result<Vec<RuntimeCommand>> {
        let snapshot = current_presence_snapshot();
        let time_period_changed = {
            let mut state = self
                .presence_state
                .lock()
                .map_err(|_| DeviceHostError::PresenceState)?;
            let changed = state.last_time_period.as_deref() != Some(snapshot.time_period);
            if changed {
                state.last_time_period = Some(snapshot.time_period.to_string());
            }
            changed
        };

        let mut commands = Vec::new();
        if time_period_changed {
            commands.extend(
                self.emit_runtime_event("presence.time_period", snapshot.clone().into_payload())
                    .await?,
            );
        }
        commands.extend(
            self.emit_runtime_event("presence.life_tick", snapshot.into_payload())
                .await?,
        );
        Ok(commands)
    }

    pub async fn emit_talk_impulse(&self) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event("presence.talk_impulse", current_presence_payload())
            .await
    }

    pub async fn emit_device_sleep_before(&self) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event("device.sleep.before", current_presence_payload())
            .await
    }

    pub async fn emit_device_wake(&self) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event("device.wake", current_presence_payload())
            .await
    }

    pub async fn emit_desktop_window_transition(
        &self,
        transition: DesktopWindowTransition,
    ) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event_with_options(
            transition.signal(),
            transition.payload(),
            Some(desktop_observation_privacy()),
            None,
        )
        .await
    }

    pub async fn emit_desktop_folder_transition(
        &self,
        transition: DesktopFolderTransition,
    ) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event_with_options(
            transition.signal(),
            transition.payload(),
            Some(desktop_observation_privacy()),
            None,
        )
        .await
    }

    pub async fn emit_desktop_download_completed(
        &self,
        observation: DesktopDownloadObservation,
    ) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event_with_options(
            observation.signal(),
            observation.payload(),
            Some(desktop_observation_privacy()),
            None,
        )
        .await
    }

    pub async fn emit_stage_perch_ended(
        &self,
        actor_id: &str,
        window_key: &str,
        reason: &str,
    ) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event_with_options(
            "stage.perch.ended",
            JsonMap::from([
                ("windowKey".to_string(), json!(window_key)),
                ("reason".to_string(), json!(reason)),
            ]),
            None,
            Some(actor_id.to_string()),
        )
        .await
    }

    pub async fn emit_stage_walk_ended(
        &self,
        actor_id: &str,
        reason: &str,
    ) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event_with_options(
            "stage.walk.ended",
            JsonMap::from([("reason".to_string(), json!(reason))]),
            None,
            Some(actor_id.to_string()),
        )
        .await
    }

    pub async fn emit_stage_walk_ended_for_command(
        &self,
        actor_id: &str,
        reason: &str,
        command_id: &str,
    ) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event_with_options_and_causality(
            "stage.walk.ended",
            JsonMap::from([("reason".to_string(), json!(reason))]),
            None,
            Some(actor_id.to_string()),
            Some(Causality {
                source_event_id: None,
                source_command_id: Some(command_id.to_string()),
                trace_id: None,
            }),
        )
        .await
    }

    pub fn spawn_presence_loop(&self) -> JoinHandle<()> {
        self.spawn_presence_loop_with_idle_sampler(|| None)
    }

    pub fn spawn_presence_loop_with_idle_sampler<F>(&self, idle_sampler: F) -> JoinHandle<()>
    where
        F: Fn() -> Option<f64> + Send + Sync + 'static,
    {
        let runtime = self.clone();
        let idle_sampler = Arc::new(idle_sampler);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(PRESENCE_LOOP_POLL_INTERVAL).await;
                let idle_seconds_since_last_input = idle_sampler();
                if let Err(error) = runtime
                    .run_presence_loop_step(Utc::now(), idle_seconds_since_last_input)
                    .await
                {
                    let _ = runtime.logger.record(
                        "presence.loop.error",
                        "device-host",
                        error_payload("presence", &error),
                    );
                }
            }
        })
    }

    async fn run_presence_loop_step(
        &self,
        now: DateTime<Utc>,
        idle_seconds_since_last_input: Option<f64>,
    ) -> Result<()> {
        let settings = self.app_settings()?;
        let random = {
            let mut state = self
                .presence_state
                .lock()
                .map_err(|_| DeviceHostError::PresenceState)?;
            next_rng_permyriad(&mut state.talk_rng_state, now)
        };

        let (emit_life_tick, talk_decision, idle_decision) = {
            let mut state = self
                .presence_state
                .lock()
                .map_err(|_| DeviceHostError::PresenceState)?;
            let emit_life_tick = evaluate_life_tick(now, &mut state);
            let talk_decision =
                evaluate_talk_impulse(now, settings.talk_interval_minutes, random, &mut state);
            let idle_decision = evaluate_idle_presence(idle_seconds_since_last_input, &mut state);
            (emit_life_tick, talk_decision, idle_decision)
        };

        match idle_decision {
            Some(IdlePresenceDecision::Start { threshold_seconds }) => {
                self.emit_runtime_event(
                    "presence.idle.start",
                    JsonMap::from([("thresholdSeconds".to_string(), json!(threshold_seconds))]),
                )
                .await?;
            }
            Some(IdlePresenceDecision::End {
                idle_seconds,
                idle_minutes,
            }) => {
                self.emit_runtime_event(
                    "presence.idle.end",
                    JsonMap::from([
                        ("idleMinutes".to_string(), json!(idle_minutes)),
                        ("idleSeconds".to_string(), json!(idle_seconds)),
                    ]),
                )
                .await?;
            }
            None => {}
        }
        if emit_life_tick {
            self.emit_presence_tick().await?;
        }
        match talk_decision {
            TalkImpulseDecision::Emit => {
                self.emit_talk_impulse().await?;
            }
            TalkImpulseDecision::SkippedRecentActivity => {
                self.logger.record(
                    "presence.talk_impulse.skipped",
                    "device-host",
                    JsonMap::from([("reason".to_string(), json!("recent-user-activity"))]),
                )?;
            }
            TalkImpulseDecision::Disabled | TalkImpulseDecision::Waiting => {}
        }
        Ok(())
    }

    fn mark_user_activity(&self, at: DateTime<Utc>) -> Result<()> {
        let mut state = self
            .presence_state
            .lock()
            .map_err(|_| DeviceHostError::PresenceState)?;
        state.last_user_activity_at = Some(at);
        Ok(())
    }

    async fn emit_runtime_event(
        &self,
        kind: &str,
        payload: JsonMap,
    ) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event_with_options(kind, payload, None, None)
            .await
    }

    async fn emit_runtime_event_with_options(
        &self,
        kind: &str,
        payload: JsonMap,
        privacy: Option<Privacy>,
        actor_id: Option<String>,
    ) -> Result<Vec<RuntimeCommand>> {
        self.emit_runtime_event_with_options_and_causality(kind, payload, privacy, actor_id, None)
            .await
    }

    async fn emit_runtime_event_with_options_and_causality(
        &self,
        kind: &str,
        payload: JsonMap,
        privacy: Option<Privacy>,
        actor_id: Option<String>,
        causality: Option<Causality>,
    ) -> Result<Vec<RuntimeCommand>> {
        let active_surface_id = self.home.snapshot()?.active_surface_id;
        let event = RuntimeEvent {
            id: new_id("evt"),
            kind: kind.to_string(),
            timestamp: now_timestamp(),
            source: "device".to_string(),
            resident_id: self.resident_id.clone(),
            payload,
            causality,
            device_id: Some(self.device_id.clone()),
            surface_id: active_surface_id,
            actor_id,
            privacy,
        };
        self.logger.record(
            "runtime.event.emit",
            "device-host",
            JsonMap::from([
                ("eventId".to_string(), json!(event.id)),
                ("eventType".to_string(), json!(event.kind)),
            ]),
        )?;
        match self.home.ingest_event(event.clone()).await {
            Ok(commands) => {
                self.logger.record(
                    "runtime.commands.emitted",
                    "resident-home",
                    JsonMap::from([
                        ("sourceEventId".to_string(), json!(event.id)),
                        (
                            "commandIds".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.id)
                                .collect::<Vec<_>>()),
                        ),
                        (
                            "commandTypes".to_string(),
                            json!(commands
                                .iter()
                                .map(|command| &command.kind)
                                .collect::<Vec<_>>()),
                        ),
                        ("count".to_string(), json!(commands.len())),
                    ]),
                )?;
                Ok(commands)
            }
            Err(error) => {
                self.record_runtime_daihon_diagnostics(kind, &error);
                let _ = self.logger.record(
                    "runtime.commands.error",
                    "resident-home",
                    error_payload(kind, &error),
                );
                Err(error.into())
            }
        }
    }

    fn session_daihon_diagnostics(&self) -> Vec<DaihonDiagnosticEntry> {
        let mut diagnostics = self
            .session_daihon_diagnostics
            .lock()
            .map(|diagnostics| diagnostics.clone())
            .unwrap_or_default();
        diagnostics.extend(
            self.home
                .daihon_diagnostics()
                .unwrap_or_default()
                .into_iter()
                .map(|diagnostic| {
                    enrich_diagnostic_for_install(
                        diagnostic,
                        &self.world_pack_status.active_install,
                        None,
                    )
                }),
        );
        diagnostics.sort_by(|left, right| {
            left.occurred_at
                .cmp(&right.occurred_at)
                .then_with(|| left.script_path.cmp(&right.script_path))
                .then_with(|| left.line.cmp(&right.line))
                .then_with(|| left.column.cmp(&right.column))
        });
        diagnostics
    }

    fn record_runtime_daihon_diagnostics(&self, context: &str, error: &ResidentHomeError) {
        let Some(report) = error.daihon_report() else {
            return;
        };
        let diagnostics = diagnostics_for_install(
            &self.world_pack_status.active_install,
            report.diagnostics.clone(),
        );
        let _ = record_daihon_diagnostics_to_app_log(&self.logger, context, &diagnostics);
    }

    pub fn export_event_log_jsonl(&self, path: impl AsRef<Path>) -> Result<usize> {
        let path = path.as_ref();
        let summary = self
            .home
            .event_log()
            .export_jsonl(EventLogQuery::default(), path)?;
        self.logger.record(
            "event_log.export",
            "device-host",
            JsonMap::from([
                ("path".to_string(), json!(display_path(path))),
                ("exported".to_string(), json!(summary.exported)),
            ]),
        )?;
        Ok(summary.exported)
    }

    async fn record_world_pack_activated(&self) -> Result<()> {
        let source = match &self.world_pack_status.active_install.source {
            WorldPackSource::BundledDefault => "bundledDefault",
            WorldPackSource::ExternalDirectory => "externalDirectory",
        };
        let event = RuntimeEvent {
            id: new_id("evt"),
            kind: "world_pack.activated".to_string(),
            timestamp: now_timestamp(),
            source: "device-host".to_string(),
            resident_id: self.resident_id.clone(),
            payload: JsonMap::from([
                ("installId".to_string(), json!(self.install_id.clone())),
                (
                    "worldPackId".to_string(),
                    json!(self.world_pack_status.active_install.world_pack_id.clone()),
                ),
                (
                    "displayName".to_string(),
                    json!(self.world_pack_status.active_install.display_name.clone()),
                ),
                ("source".to_string(), json!(source)),
                (
                    "configuredInstallId".to_string(),
                    json!(self.world_pack_status.configured_install_id.clone()),
                ),
                (
                    "fallbackActive".to_string(),
                    json!(self.world_pack_status.fallback_active),
                ),
            ]),
            causality: None,
            device_id: Some(self.device_id.clone()),
            surface_id: None,
            actor_id: None,
            privacy: None,
        };
        self.home.ingest_event(event).await?;
        self.logger.record(
            "world_pack.activated",
            "device-host",
            JsonMap::from([
                ("installId".to_string(), json!(self.install_id.clone())),
                (
                    "worldPackId".to_string(),
                    json!(self.world_pack_status.active_install.world_pack_id.clone()),
                ),
                (
                    "worldRoot".to_string(),
                    json!(display_path(&self.paths.world_root)),
                ),
                (
                    "fallbackActive".to_string(),
                    json!(self.world_pack_status.fallback_active),
                ),
            ]),
        )?;
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EventLogPrivacyCategoryFilter {
    All,
    DesktopObservation,
    None,
}

impl From<EventLogPrivacyCategoryFilter> for EventLogPrivacyFilter {
    fn from(value: EventLogPrivacyCategoryFilter) -> Self {
        match value {
            EventLogPrivacyCategoryFilter::All => EventLogPrivacyFilter::All,
            EventLogPrivacyCategoryFilter::DesktopObservation => {
                EventLogPrivacyFilter::Category("desktop-observation".to_string())
            }
            EventLogPrivacyCategoryFilter::None => EventLogPrivacyFilter::None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventLogDeleteResult {
    pub deleted: usize,
}

impl From<DeleteSummary> for EventLogDeleteResult {
    fn from(value: DeleteSummary) -> Self {
        Self {
            deleted: value.deleted,
        }
    }
}
