use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};

use chrono::Utc;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::{
    http::{Response, StatusCode},
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, UriSchemeContext, WebviewWindow, WindowEvent,
};
use tokio::sync::Mutex;
use yuukei_device_host::{
    tauri_surface_session, ActorSurfaceHitZoneDefinition, ActorSurfaceRendererKind,
    AppSettingsState, AvatarGesturePoke, CapabilityUsageState, DesktopFolderObservationState,
    DesktopWindowObservationState, EventLogDeleteResult, EventLogPrivacyCategoryFilter,
    ExtensionSettingsChangeResult, ExtensionSettingsState, LocalRuntimeEnvironment,
    LocalYuukeiRuntime, ObservationSettingsState, ObservationSettingsUpdate, OnboardingState,
    ResidentEventLogPage, WorldPackSelectionState, WorldPackSwitchResult, WorldPackZipInspection,
    TAURI_SURFACE_ID,
};
use yuukei_protocol::{
    ExtensionHookPoint, MemoryEntryKind, MemoryForgetEntry, MemoryForgetOutput, MemoryListOutput,
    MemoryUpdateOutput, ResidentSnapshot, RuntimeCommand,
};
use yuukei_world::resolve_pack_relative_path;

mod audio_player;
mod desktop_stage;
mod idle_observer;
mod power_observer;
mod window_observer;
use audio_player::AudioPlayer;
use desktop_stage::{ActorStageAnchorReport, DesktopStageManager};
use idle_observer::seconds_since_last_user_input;
use power_observer::PowerObserver;

pub struct AppState {
    env: LocalRuntimeEnvironment,
    runtime: Mutex<LocalYuukeiRuntime>,
    asset_index: RwLock<PackAssetIndex>,
    stage: Arc<DesktopStageManager>,
    audio_player: Option<Arc<AudioPlayer>>,
    command_forwarder: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    surface_attached: Mutex<bool>,
    presence_loop: Mutex<Option<tokio::task::JoinHandle<()>>>,
    power_observer: Mutex<Option<PowerObserver>>,
    window_observer: Mutex<Option<tokio::task::JoinHandle<()>>>,
    download_observer: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[derive(Clone, Debug, Default)]
struct PackAssetIndex {
    routes: HashMap<String, PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopActorSurfaceAssetCatalog {
    world_pack_id: String,
    actors: Vec<DesktopActorSurfaceAsset>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopActorSurfaceAsset {
    actor_id: String,
    display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    renderer: Option<DesktopActorSurfaceRendererAsset>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopActorSurfaceRendererAsset {
    kind: &'static str,
    model_url: String,
    motions: HashMap<String, String>,
    hit_zones: Vec<ActorSurfaceHitZoneDefinition>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DesktopAvatarGesturePoke {
    actor_id: String,
    hit_zone_id: String,
    #[serde(default)]
    hit_zone_label: Option<String>,
    #[serde(default)]
    hit_surface: Option<String>,
    #[serde(default)]
    hit_bone: Option<String>,
    input: yuukei_device_host::AvatarGestureInput,
    screen: yuukei_device_host::AvatarGestureScreen,
}

impl From<DesktopAvatarGesturePoke> for AvatarGesturePoke {
    fn from(value: DesktopAvatarGesturePoke) -> Self {
        Self {
            actor_id: value.actor_id,
            hit_zone_id: value.hit_zone_id,
            hit_zone_label: value.hit_zone_label,
            hit_surface: value.hit_surface,
            hit_bone: value.hit_bone,
            input: value.input,
            screen: value.screen,
        }
    }
}

const PACK_ASSET_SCHEME: &str = "yuukei-pack";
const MENU_SETTINGS_ID: &str = "settings";
const MENU_TOGGLE_CHARACTER_ID: &str = "toggle-character";
const MENU_QUIT_ID: &str = "quit";

#[tauri::command]
async fn attach_surface(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ResidentSnapshot, String> {
    let runtime = state.runtime.lock().await.clone();
    {
        let mut surface_attached = state.surface_attached.lock().await;
        if !*surface_attached {
            attach_tauri_surface_or_status(&app, &runtime).await?;
            *surface_attached = true;
        }
    }
    ensure_presence_loop(&app, &state, &runtime).await?;
    let snapshot = runtime.snapshot().map_err(to_message)?;
    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    state.stage.emit_state(&app)?;
    Ok(snapshot)
}

#[tauri::command]
async fn get_snapshot(state: State<'_, AppState>) -> Result<ResidentSnapshot, String> {
    let runtime = state.runtime.lock().await.clone();
    runtime.snapshot().map_err(to_message)
}

#[tauri::command]
async fn get_world_pack_status(
    state: State<'_, AppState>,
) -> Result<WorldPackSelectionState, String> {
    let runtime = state.runtime.lock().await;
    Ok(runtime.world_pack_status())
}

#[tauri::command]
async fn get_extension_settings(
    state: State<'_, AppState>,
) -> Result<ExtensionSettingsState, String> {
    let runtime = state.runtime.lock().await;
    runtime.extension_settings().map_err(to_message)
}

#[tauri::command]
async fn get_app_settings(state: State<'_, AppState>) -> Result<AppSettingsState, String> {
    let runtime = state.runtime.lock().await;
    runtime.app_settings().map_err(to_message)
}

#[tauri::command]
async fn get_observation_settings(
    state: State<'_, AppState>,
) -> Result<ObservationSettingsState, String> {
    let runtime = state.runtime.lock().await;
    runtime.observation_settings().map_err(to_message)
}

#[tauri::command]
async fn get_onboarding_state(state: State<'_, AppState>) -> Result<OnboardingState, String> {
    LocalYuukeiRuntime::onboarding_state_in(state.env.clone()).map_err(to_message)
}

#[tauri::command]
async fn complete_onboarding(state: State<'_, AppState>) -> Result<OnboardingState, String> {
    LocalYuukeiRuntime::complete_onboarding_in(state.env.clone()).map_err(to_message)
}

#[tauri::command]
async fn set_observation_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: ObservationSettingsUpdate,
) -> Result<ObservationSettingsState, String> {
    let next = LocalYuukeiRuntime::set_observation_settings_in(state.env.clone(), settings)
        .map_err(to_message)?;
    reconcile_window_observer(&app, &state).await?;
    reconcile_download_observer(&state).await?;
    Ok(next)
}

#[tauri::command]
async fn get_capability_usage(state: State<'_, AppState>) -> Result<CapabilityUsageState, String> {
    let runtime = state.runtime.lock().await;
    runtime.capability_usage().map_err(to_message)
}

#[tauri::command]
async fn list_resident_memories(
    state: State<'_, AppState>,
    episode_limit: Option<usize>,
    episode_offset: Option<usize>,
) -> Result<MemoryListOutput, String> {
    let runtime = state.runtime.lock().await.clone();
    runtime
        .list_resident_memories(episode_limit, episode_offset)
        .await
        .map_err(to_message)
}

#[tauri::command]
async fn update_resident_memory(
    state: State<'_, AppState>,
    kind: MemoryEntryKind,
    id: String,
    text: String,
) -> Result<MemoryUpdateOutput, String> {
    let runtime = state.runtime.lock().await.clone();
    runtime
        .update_resident_memory(kind, id, text)
        .await
        .map_err(to_message)
}

#[tauri::command]
async fn forget_resident_memories(
    state: State<'_, AppState>,
    entries: Option<Vec<MemoryForgetEntry>>,
    all: Option<bool>,
) -> Result<MemoryForgetOutput, String> {
    let runtime = state.runtime.lock().await.clone();
    runtime
        .forget_resident_memories(entries.unwrap_or_default(), all.unwrap_or(false))
        .await
        .map_err(to_message)
}

#[tauri::command]
async fn read_event_log_page(
    state: State<'_, AppState>,
    kind_prefix: Option<String>,
    privacy_category: EventLogPrivacyCategoryFilter,
    before_sequence: Option<i64>,
    limit: Option<usize>,
) -> Result<ResidentEventLogPage, String> {
    let runtime = state.runtime.lock().await;
    runtime
        .read_event_log_page(kind_prefix, privacy_category, before_sequence, limit)
        .map_err(to_message)
}

#[tauri::command]
async fn count_event_log_delete_before(
    state: State<'_, AppState>,
    timestamp: String,
) -> Result<usize, String> {
    let runtime = state.runtime.lock().await;
    runtime
        .count_event_log_delete_before(timestamp)
        .map_err(to_message)
}

#[tauri::command]
async fn count_event_log_delete_by_kind_prefix(
    state: State<'_, AppState>,
    prefix: String,
) -> Result<usize, String> {
    let runtime = state.runtime.lock().await;
    runtime
        .count_event_log_delete_by_kind_prefix(prefix)
        .map_err(to_message)
}

#[tauri::command]
async fn count_event_log_delete_all(state: State<'_, AppState>) -> Result<usize, String> {
    let runtime = state.runtime.lock().await;
    runtime.count_event_log_delete_all().map_err(to_message)
}

#[tauri::command]
async fn delete_event_log_before(
    state: State<'_, AppState>,
    timestamp: String,
) -> Result<EventLogDeleteResult, String> {
    let runtime = state.runtime.lock().await;
    runtime
        .delete_event_log_before(timestamp)
        .map_err(to_message)
}

#[tauri::command]
async fn delete_event_log_by_kind_prefix(
    state: State<'_, AppState>,
    prefix: String,
) -> Result<EventLogDeleteResult, String> {
    let runtime = state.runtime.lock().await;
    runtime
        .delete_event_log_by_kind_prefix(prefix)
        .map_err(to_message)
}

#[tauri::command]
async fn delete_event_log_all(state: State<'_, AppState>) -> Result<EventLogDeleteResult, String> {
    let runtime = state.runtime.lock().await;
    runtime.delete_event_log_all().map_err(to_message)
}

#[tauri::command]
async fn get_actor_surface_assets(
    state: State<'_, AppState>,
) -> Result<DesktopActorSurfaceAssetCatalog, String> {
    let runtime = state.runtime.lock().await;
    Ok(desktop_actor_surface_assets(&runtime))
}

#[tauri::command]
fn set_actor_window_click_through(window: WebviewWindow, passthrough: bool) -> Result<(), String> {
    window
        .set_ignore_cursor_events(passthrough)
        .map_err(to_message)
}

#[tauri::command]
fn set_stage_overlay_click_through(window: WebviewWindow, passthrough: bool) -> Result<(), String> {
    window
        .set_ignore_cursor_events(passthrough)
        .map_err(to_message)
}

#[tauri::command]
fn get_desktop_stage_state(
    state: State<'_, AppState>,
) -> Result<desktop_stage::DesktopStageSnapshot, String> {
    state.stage.snapshot()
}

#[tauri::command]
fn report_actor_stage_anchor(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    actor_id: String,
    report: ActorStageAnchorReport,
) -> Result<(), String> {
    state
        .stage
        .report_actor_anchor(&app, &window, actor_id, report)
}

#[tauri::command]
fn dismiss_stage_bubble(
    app: AppHandle,
    state: State<'_, AppState>,
    bubble_id: String,
) -> Result<(), String> {
    state.stage.dismiss_bubble(&app, bubble_id)
}

#[tauri::command]
fn open_settings_window(app: AppHandle) -> Result<(), String> {
    show_settings_window(&app)
}

#[tauri::command]
async fn send_conversation_text(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> Result<Vec<RuntimeCommand>, String> {
    let runtime = state.runtime.lock().await.clone();
    let commands = match runtime
        .send_conversation_text(TAURI_SURFACE_ID, &text)
        .await
    {
        Ok(commands) => commands,
        Err(error) => {
            emit_world_pack_status(&app, &runtime.world_pack_status())?;
            return Err(to_message(error));
        }
    };
    let snapshot = runtime.snapshot().map_err(to_message)?;
    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    Ok(commands)
}

#[tauri::command]
async fn send_conversation_choice(
    app: AppHandle,
    state: State<'_, AppState>,
    choice_id: String,
    choice: String,
    index: usize,
) -> Result<Vec<RuntimeCommand>, String> {
    let runtime = state.runtime.lock().await.clone();
    let commands = match runtime
        .send_conversation_choice(TAURI_SURFACE_ID, &choice_id, &choice, index)
        .await
    {
        Ok(commands) => commands,
        Err(error) => {
            emit_world_pack_status(&app, &runtime.world_pack_status())?;
            return Err(to_message(error));
        }
    };
    let snapshot = runtime.snapshot().map_err(to_message)?;
    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    Ok(commands)
}

#[tauri::command]
async fn send_avatar_gesture_poke(
    app: AppHandle,
    state: State<'_, AppState>,
    gesture: DesktopAvatarGesturePoke,
) -> Result<Vec<RuntimeCommand>, String> {
    let runtime = state.runtime.lock().await.clone();
    let commands = match runtime
        .send_avatar_gesture_poke(TAURI_SURFACE_ID, gesture.into())
        .await
    {
        Ok(commands) => commands,
        Err(error) => {
            emit_world_pack_status(&app, &runtime.world_pack_status())?;
            return Err(to_message(error));
        }
    };
    let snapshot = runtime.snapshot().map_err(to_message)?;
    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    Ok(commands)
}

#[tauri::command]
async fn select_world_pack_directory(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<WorldPackSwitchResult, String> {
    match LocalYuukeiRuntime::select_world_pack_directory_in(state.env.clone(), &path).await {
        Ok(runtime) => replace_runtime(app, state, runtime).await,
        Err(error) => {
            let current = state.runtime.lock().await.clone();
            let _ = current
                .record_session_daihon_diagnostics_from_error(&error, Some(Path::new(&path)));
            emit_world_pack_status(&app, &current.world_pack_status())?;
            Err(to_message(error))
        }
    }
}

#[tauri::command]
async fn inspect_world_pack_zip(
    state: State<'_, AppState>,
    path: String,
) -> Result<WorldPackZipInspection, String> {
    LocalYuukeiRuntime::inspect_world_pack_zip_in(state.env.clone(), &path).map_err(to_message)
}

#[tauri::command]
async fn import_world_pack_zip(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<WorldPackSwitchResult, String> {
    match LocalYuukeiRuntime::import_world_pack_zip_in(state.env.clone(), &path).await {
        Ok(runtime) => replace_runtime(app, state, runtime).await,
        Err(error) => {
            let current = state.runtime.lock().await.clone();
            emit_world_pack_status(&app, &current.world_pack_status())?;
            Err(to_message(error))
        }
    }
}

#[tauri::command]
async fn reset_world_pack_to_default(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<WorldPackSwitchResult, String> {
    let runtime = LocalYuukeiRuntime::reset_world_pack_to_default_in(state.env.clone())
        .await
        .map_err(to_message)?;
    replace_runtime(app, state, runtime).await
}

#[tauri::command]
async fn install_extension_directory(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<ExtensionSettingsChangeResult, String> {
    LocalYuukeiRuntime::install_extension_directory_in(state.env.clone(), path)
        .map_err(to_message)?;
    reload_runtime_for_extension_change(app, state).await
}

#[tauri::command]
async fn uninstall_extension(
    app: AppHandle,
    state: State<'_, AppState>,
    extension_id: String,
) -> Result<ExtensionSettingsChangeResult, String> {
    LocalYuukeiRuntime::uninstall_extension_in(state.env.clone(), &extension_id)
        .map_err(to_message)?;
    reload_runtime_for_extension_change(app, state).await
}

#[tauri::command]
async fn set_extension_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    extension_id: String,
    enabled: bool,
) -> Result<ExtensionSettingsChangeResult, String> {
    LocalYuukeiRuntime::set_extension_enabled_in(state.env.clone(), &extension_id, enabled)
        .map_err(to_message)?;
    reload_runtime_for_extension_change(app, state).await
}

#[tauri::command]
async fn set_extension_hook_order(
    app: AppHandle,
    state: State<'_, AppState>,
    hook_point: ExtensionHookPoint,
    extension_ids: Vec<String>,
) -> Result<ExtensionSettingsChangeResult, String> {
    LocalYuukeiRuntime::set_extension_hook_order_in(state.env.clone(), hook_point, extension_ids)
        .map_err(to_message)?;
    reload_runtime_for_extension_change(app, state).await
}

#[tauri::command]
async fn set_extension_setting_values(
    app: AppHandle,
    state: State<'_, AppState>,
    extension_id: String,
    values: Map<String, Value>,
) -> Result<ExtensionSettingsChangeResult, String> {
    LocalYuukeiRuntime::set_extension_setting_values_in(state.env.clone(), &extension_id, values)
        .map_err(to_message)?;
    reload_runtime_for_extension_change(app, state).await
}

#[tauri::command]
async fn set_extension_secret(
    app: AppHandle,
    state: State<'_, AppState>,
    extension_id: String,
    key: String,
    value: Option<String>,
) -> Result<ExtensionSettingsChangeResult, String> {
    LocalYuukeiRuntime::set_extension_secret_in(state.env.clone(), &extension_id, &key, value)
        .map_err(to_message)?;
    reload_runtime_for_extension_change(app, state).await
}

#[tauri::command]
async fn restart_extension_process(
    state: State<'_, AppState>,
    extension_id: String,
) -> Result<ExtensionSettingsState, String> {
    let runtime = state.runtime.lock().await;
    runtime
        .restart_extension_process(&extension_id)
        .map_err(to_message)
}

#[tauri::command]
async fn set_app_talk_interval_minutes(
    state: State<'_, AppState>,
    minutes: u64,
) -> Result<AppSettingsState, String> {
    LocalYuukeiRuntime::set_app_talk_interval_minutes_in(state.env.clone(), minutes)
        .map_err(to_message)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .register_uri_scheme_protocol(PACK_ASSET_SCHEME, pack_asset_protocol_response)
        .setup(|app| {
            let env = local_runtime_environment(app.handle())
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            let runtime =
                tauri::async_runtime::block_on(LocalYuukeiRuntime::open_selected_in(env.clone()))
                    .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            println!("Yuukei app log: {}", runtime.paths().app_log_path.display());
            let asset_index = build_pack_asset_index(&runtime)
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            let asset_catalog = desktop_actor_surface_assets(&runtime);
            configure_app_menu(app.handle())
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            configure_tray(app.handle())
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            let stage = Arc::new(DesktopStageManager::new());
            let audio_player = match AudioPlayer::new() {
                Ok(player) => Some(Arc::new(player)),
                Err(error) => {
                    eprintln!("Yuukei audio output unavailable: {error}");
                    None
                }
            };
            let command_forwarder = spawn_command_forwarder(
                runtime.home(),
                app.handle().clone(),
                stage.clone(),
                audio_player.clone(),
            );
            let power_observer = PowerObserver::new(runtime.clone());
            let should_show_onboarding = LocalYuukeiRuntime::onboarding_state_in(env.clone())
                .map(|state| !state.completed)
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            app.manage(AppState {
                env,
                runtime: Mutex::new(runtime),
                asset_index: RwLock::new(asset_index),
                stage: stage.clone(),
                audio_player,
                command_forwarder: Mutex::new(Some(command_forwarder)),
                surface_attached: Mutex::new(false),
                presence_loop: Mutex::new(None),
                power_observer: Mutex::new(Some(power_observer)),
                window_observer: Mutex::new(None),
                download_observer: Mutex::new(None),
            });
            {
                let state = app.handle().state::<AppState>();
                tauri::async_runtime::block_on(reconcile_window_observer(app.handle(), &state))
                    .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
                tauri::async_runtime::block_on(reconcile_download_observer(&state))
                    .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            }
            stage
                .sync_surfaces(app.handle(), &asset_catalog)
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            if should_show_onboarding {
                show_settings_window(app.handle())
                    .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            }
            Ok(())
        })
        .on_menu_event(|app, event| {
            if let Err(error) = handle_menu_event(app, event.id().as_ref()) {
                eprintln!("Yuukei menu error: {error}");
            }
        })
        .on_window_event(|window, event| {
            if desktop_stage::is_actor_window_label(window.label())
                && matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_))
            {
                let app_handle = window.app_handle().clone();
                let state = app_handle.state::<AppState>();
                if let Some(webview_window) = app_handle.get_webview_window(window.label()) {
                    let _ = state
                        .stage
                        .refresh_actor_window(&app_handle, &webview_window);
                }
            }
            if window.label() == "settings" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.emit("yuukei-onboarding-dismissed", ());
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            attach_surface,
            get_snapshot,
            get_world_pack_status,
            get_extension_settings,
            get_app_settings,
            get_observation_settings,
            get_onboarding_state,
            complete_onboarding,
            set_observation_settings,
            get_capability_usage,
            list_resident_memories,
            update_resident_memory,
            forget_resident_memories,
            read_event_log_page,
            count_event_log_delete_before,
            count_event_log_delete_by_kind_prefix,
            count_event_log_delete_all,
            delete_event_log_before,
            delete_event_log_by_kind_prefix,
            delete_event_log_all,
            get_actor_surface_assets,
            set_actor_window_click_through,
            set_stage_overlay_click_through,
            get_desktop_stage_state,
            report_actor_stage_anchor,
            dismiss_stage_bubble,
            open_settings_window,
            send_conversation_text,
            send_conversation_choice,
            send_avatar_gesture_poke,
            select_world_pack_directory,
            inspect_world_pack_zip,
            import_world_pack_zip,
            reset_world_pack_to_default,
            install_extension_directory,
            uninstall_extension,
            set_extension_enabled,
            set_extension_hook_order,
            set_extension_setting_values,
            set_extension_secret,
            restart_extension_process,
            set_app_talk_interval_minutes
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Yuukei desktop");
}

fn local_runtime_environment(app: &AppHandle) -> Result<LocalRuntimeEnvironment, String> {
    let mut env = LocalRuntimeEnvironment::default_local();
    if let Ok(resource_dir) = app.path().resource_dir() {
        let bundled_default = resource_dir.join("packs").join("default-yuukei");
        if bundled_default.join("pack.json").exists() {
            env.default_world_root = bundled_default;
        }
    }
    Ok(env)
}

fn configure_app_menu(app: &AppHandle) -> Result<(), String> {
    let settings = MenuItem::with_id(app, MENU_SETTINGS_ID, "設定を開く", true, None::<&str>)
        .map_err(to_message)?;
    let toggle = MenuItem::with_id(
        app,
        MENU_TOGGLE_CHARACTER_ID,
        "キャラクター表示を切り替え",
        true,
        None::<&str>,
    )
    .map_err(to_message)?;
    let quit =
        MenuItem::with_id(app, MENU_QUIT_ID, "終了", true, None::<&str>).map_err(to_message)?;
    let separator = PredefinedMenuItem::separator(app).map_err(to_message)?;
    let app_menu = Submenu::with_items(
        app,
        "Yuukei",
        true,
        &[&settings, &toggle, &separator, &quit],
    )
    .map_err(to_message)?;
    let menu = Menu::with_items(app, &[&app_menu]).map_err(to_message)?;
    app.set_menu(menu).map_err(to_message)?;
    Ok(())
}

fn configure_tray(app: &AppHandle) -> Result<(), String> {
    let settings = MenuItem::with_id(app, MENU_SETTINGS_ID, "設定を開く", true, None::<&str>)
        .map_err(to_message)?;
    let toggle = MenuItem::with_id(
        app,
        MENU_TOGGLE_CHARACTER_ID,
        "キャラクター表示を切り替え",
        true,
        None::<&str>,
    )
    .map_err(to_message)?;
    let quit =
        MenuItem::with_id(app, MENU_QUIT_ID, "終了", true, None::<&str>).map_err(to_message)?;
    let separator = PredefinedMenuItem::separator(app).map_err(to_message)?;
    let menu =
        Menu::with_items(app, &[&settings, &toggle, &separator, &quit]).map_err(to_message)?;
    let mut builder = TrayIconBuilder::with_id("yuukei")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Yuukei")
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = show_settings_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app).map_err(to_message)?;
    Ok(())
}

fn handle_menu_event(app: &AppHandle, id: &str) -> Result<(), String> {
    match id {
        MENU_SETTINGS_ID => show_settings_window(app),
        MENU_TOGGLE_CHARACTER_ID => toggle_actor_window(app),
        MENU_QUIT_ID => {
            app.exit(0);
            Ok(())
        }
        _ => Ok(()),
    }
}

fn show_settings_window(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("settings")
        .ok_or_else(|| "settings window is not available".to_string())?;
    window.unminimize().map_err(to_message)?;
    window.show().map_err(to_message)?;
    window.set_focus().map_err(to_message)?;
    Ok(())
}

fn toggle_actor_window(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let windows = state.stage.actor_windows(app);
    if windows.is_empty() {
        return Ok(());
    }
    let any_visible = windows
        .iter()
        .any(|window| window.is_visible().unwrap_or(false));
    if any_visible {
        for window in windows {
            let label = window.label().to_string();
            window.hide().map_err(to_message)?;
            state.stage.set_actor_window_visible(app, &label, false)?;
        }
    } else {
        for window in windows {
            let label = window.label().to_string();
            window.show().map_err(to_message)?;
            state.stage.set_actor_window_visible(app, &label, true)?;
        }
    }
    Ok(())
}

fn pack_asset_protocol_response<R: tauri::Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let route = request.uri().path().trim_start_matches('/').to_string();
    let state = ctx.app_handle().state::<AppState>();
    let path = match state.asset_index.read() {
        Ok(index) => index.routes.get(&route).cloned(),
        Err(_) => {
            return bytes_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "text/plain",
                b"asset index lock error".to_vec(),
            )
        }
    };
    let Some(path) = path else {
        return bytes_response(
            StatusCode::NOT_FOUND,
            "text/plain",
            b"asset not found".to_vec(),
        );
    };
    match fs::read(&path) {
        Ok(bytes) => bytes_response(StatusCode::OK, content_type_for_path(&path), bytes),
        Err(error) => bytes_response(
            StatusCode::NOT_FOUND,
            "text/plain",
            format!("asset read error: {error}").into_bytes(),
        ),
    }
}

fn bytes_response(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Access-Control-Allow-Origin", "*")
        .header("Content-Type", content_type)
        .body(body)
        .expect("failed to build custom protocol response")
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("vrm" | "vrma" | "glb") => "model/gltf-binary",
        _ => "application/octet-stream",
    }
}

fn build_pack_asset_index(runtime: &LocalYuukeiRuntime) -> Result<PackAssetIndex, String> {
    let root = fs::canonicalize(&runtime.paths().world_root).map_err(to_message)?;
    let catalog = runtime.actor_surface_assets();
    let mut routes = HashMap::new();
    for actor in catalog.actors {
        let Some(renderer) = actor.renderer else {
            continue;
        };
        routes.insert(
            actor_model_route(&actor.actor_id),
            resolve_pack_relative_path(&root, &renderer.model).map_err(to_message)?,
        );
        for (motion_id, motion_path) in renderer.motions {
            routes.insert(
                actor_motion_route(&actor.actor_id, &motion_id),
                resolve_pack_relative_path(&root, &motion_path).map_err(to_message)?,
            );
        }
    }
    Ok(PackAssetIndex { routes })
}

fn desktop_actor_surface_assets(runtime: &LocalYuukeiRuntime) -> DesktopActorSurfaceAssetCatalog {
    let catalog = runtime.actor_surface_assets();
    DesktopActorSurfaceAssetCatalog {
        world_pack_id: catalog.world_pack_id,
        actors: catalog
            .actors
            .into_iter()
            .map(|actor| {
                let renderer = actor
                    .renderer
                    .map(|renderer| DesktopActorSurfaceRendererAsset {
                        kind: match renderer.kind {
                            ActorSurfaceRendererKind::Vrm => "vrm",
                        },
                        model_url: pack_asset_url(&actor_model_route(&actor.actor_id)),
                        motions: renderer
                            .motions
                            .into_keys()
                            .map(|motion_id| {
                                let route = actor_motion_route(&actor.actor_id, &motion_id);
                                (motion_id, pack_asset_url(&route))
                            })
                            .collect(),
                        hit_zones: renderer.hit_zones,
                    });
                DesktopActorSurfaceAsset {
                    actor_id: actor.actor_id,
                    display_name: actor.display_name,
                    renderer,
                }
            })
            .collect(),
    }
}

fn pack_asset_url(route: &str) -> String {
    format!("{PACK_ASSET_SCHEME}://localhost/{route}")
}

fn actor_model_route(actor_id: &str) -> String {
    format!("actors/{}/model", encode_path_segment(actor_id))
}

fn actor_motion_route(actor_id: &str, motion_id: &str) -> String {
    format!(
        "actors/{}/motions/{}",
        encode_path_segment(actor_id),
        encode_path_segment(motion_id)
    )
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        let is_unreserved =
            byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'.' | b'_' | b'~');
        if is_unreserved {
            encoded.push(char::from(*byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

async fn replace_runtime(
    app: AppHandle,
    state: State<'_, AppState>,
    runtime: LocalYuukeiRuntime,
) -> Result<WorldPackSwitchResult, String> {
    let asset_index = build_pack_asset_index(&runtime)?;
    let asset_catalog = desktop_actor_surface_assets(&runtime);
    attach_tauri_surface_or_status(&app, &runtime).await?;
    emit_app_startup_or_status(&app, &runtime).await?;
    let snapshot = runtime.snapshot().map_err(to_message)?;
    let next_forwarder = spawn_command_forwarder(
        runtime.home(),
        app.clone(),
        state.stage.clone(),
        state.audio_player.clone(),
    );
    let next_presence_loop = spawn_desktop_presence_loop(&runtime);
    let next_power_observer = PowerObserver::new(runtime.clone());
    let status = runtime.world_pack_status();

    {
        let mut current = state.runtime.lock().await;
        *current = runtime;
    }
    {
        let mut current = state
            .asset_index
            .write()
            .map_err(|_| "asset index lock is poisoned".to_string())?;
        *current = asset_index;
    }
    {
        let mut forwarder = state.command_forwarder.lock().await;
        if let Some(previous) = forwarder.replace(next_forwarder) {
            previous.abort();
        }
    }
    {
        let mut surface_attached = state.surface_attached.lock().await;
        *surface_attached = true;
    }
    {
        let mut presence_loop = state.presence_loop.lock().await;
        if let Some(previous) = presence_loop.replace(next_presence_loop) {
            previous.abort();
        }
    }
    {
        let mut power_observer = state.power_observer.lock().await;
        let _previous = power_observer.replace(next_power_observer);
    }
    reconcile_window_observer(&app, &state).await?;
    reconcile_download_observer(&state).await?;

    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    state.stage.sync_surfaces(&app, &asset_catalog)?;
    app.emit("yuukei-assets-changed", &asset_catalog)
        .map_err(to_message)?;
    emit_world_pack_status(&app, &status)?;

    Ok(WorldPackSwitchResult { status, snapshot })
}

async fn reload_runtime_for_extension_change(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ExtensionSettingsChangeResult, String> {
    let runtime = LocalYuukeiRuntime::open_selected_in(state.env.clone())
        .await
        .map_err(to_message)?;
    let snapshot = replace_runtime_snapshot(app, state, runtime.clone()).await?;
    let extension_state = runtime.extension_settings().map_err(to_message)?;
    Ok(ExtensionSettingsChangeResult {
        state: extension_state,
        snapshot,
    })
}

async fn replace_runtime_snapshot(
    app: AppHandle,
    state: State<'_, AppState>,
    runtime: LocalYuukeiRuntime,
) -> Result<ResidentSnapshot, String> {
    let asset_index = build_pack_asset_index(&runtime)?;
    let asset_catalog = desktop_actor_surface_assets(&runtime);
    attach_tauri_surface_or_status(&app, &runtime).await?;
    emit_app_startup_or_status(&app, &runtime).await?;
    let snapshot = runtime.snapshot().map_err(to_message)?;
    let next_forwarder = spawn_command_forwarder(
        runtime.home(),
        app.clone(),
        state.stage.clone(),
        state.audio_player.clone(),
    );
    let next_presence_loop = spawn_desktop_presence_loop(&runtime);
    let next_power_observer = PowerObserver::new(runtime.clone());
    let status = runtime.world_pack_status();

    {
        let mut current = state.runtime.lock().await;
        *current = runtime;
    }
    {
        let mut current = state
            .asset_index
            .write()
            .map_err(|_| "asset index lock is poisoned".to_string())?;
        *current = asset_index;
    }
    {
        let mut forwarder = state.command_forwarder.lock().await;
        if let Some(previous) = forwarder.replace(next_forwarder) {
            previous.abort();
        }
    }
    {
        let mut surface_attached = state.surface_attached.lock().await;
        *surface_attached = true;
    }
    {
        let mut presence_loop = state.presence_loop.lock().await;
        if let Some(previous) = presence_loop.replace(next_presence_loop) {
            previous.abort();
        }
    }
    {
        let mut power_observer = state.power_observer.lock().await;
        let _previous = power_observer.replace(next_power_observer);
    }
    reconcile_window_observer(&app, &state).await?;
    reconcile_download_observer(&state).await?;

    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    state.stage.sync_surfaces(&app, &asset_catalog)?;
    app.emit("yuukei-assets-changed", &asset_catalog)
        .map_err(to_message)?;
    emit_world_pack_status(&app, &status)?;

    Ok(snapshot)
}

async fn ensure_presence_loop(
    app: &AppHandle,
    state: &State<'_, AppState>,
    runtime: &LocalYuukeiRuntime,
) -> Result<(), String> {
    emit_app_startup_or_status(app, runtime).await?;
    let mut presence_loop = state.presence_loop.lock().await;
    if presence_loop.is_none() {
        *presence_loop = Some(spawn_desktop_presence_loop(runtime));
    }
    Ok(())
}

fn spawn_desktop_presence_loop(runtime: &LocalYuukeiRuntime) -> tokio::task::JoinHandle<()> {
    runtime.spawn_presence_loop_with_idle_sampler(seconds_since_last_user_input)
}

async fn reconcile_window_observer(
    app: &AppHandle,
    state: &State<'_, AppState>,
) -> Result<(), String> {
    let runtime = state.runtime.lock().await.clone();
    let settings = runtime.observation_settings().map_err(to_message)?;
    let enabled = window_observer::observation_loop_enabled(&settings);
    state
        .stage
        .set_window_observation_enabled(settings.windows)?;
    let mut current = state.window_observer.lock().await;
    match (enabled, current.is_some()) {
        (true, false) => {
            *current = Some(spawn_window_observer(
                app.clone(),
                runtime,
                state.stage.clone(),
            ));
        }
        (false, true) => {
            if let Some(previous) = current.take() {
                previous.abort();
            }
        }
        _ => {}
    }
    Ok(())
}

async fn reconcile_download_observer(state: &State<'_, AppState>) -> Result<(), String> {
    let runtime = state.runtime.lock().await.clone();
    let settings = runtime.observation_settings().map_err(to_message)?;
    let mut current = state.download_observer.lock().await;
    match (settings.downloads, current.is_some()) {
        (true, false) => {
            *current = Some(spawn_download_observer(runtime));
        }
        (false, true) => {
            if let Some(previous) = current.take() {
                previous.abort();
            }
        }
        _ => {}
    }
    Ok(())
}

fn spawn_window_observer(
    app: AppHandle,
    runtime: LocalYuukeiRuntime,
    stage: Arc<DesktopStageManager>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut state = DesktopWindowObservationState::default();
        let mut folder_state = DesktopFolderObservationState::default();
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            let settings = match runtime.observation_settings() {
                Ok(settings) => settings,
                Err(_) => continue,
            };
            if settings.windows {
                let observations = window_observer::collect_desktop_windows();
                match stage.apply_window_terrain(&app, &observations) {
                    Ok(ended) => {
                        for event in ended {
                            if let Err(error) = runtime
                                .emit_stage_perch_ended(
                                    &event.actor_id,
                                    &event.window_key,
                                    event.reason,
                                )
                                .await
                            {
                                let _ = runtime.logger().record(
                                    "stage.perch.ended.error",
                                    "device-host",
                                    serde_json::json!({
                                        "actorId": event.actor_id,
                                        "windowKey": event.window_key,
                                        "message": error.to_string()
                                    })
                                    .as_object()
                                    .cloned()
                                    .unwrap_or_default()
                                    .into_iter()
                                    .collect(),
                                );
                            }
                        }
                    }
                    Err(error) => {
                        let _ = runtime.logger().record(
                            "desktop.window.terrain.error",
                            "device-host",
                            serde_json::json!({ "message": error })
                                .as_object()
                                .cloned()
                                .unwrap_or_default()
                                .into_iter()
                                .collect(),
                        );
                    }
                }
                for transition in state.update(Utc::now(), observations) {
                    if let Err(error) = runtime.emit_desktop_window_transition(transition).await {
                        let _ = runtime.logger().record(
                            "desktop.window.event.error",
                            "device-host",
                            serde_json::json!({ "message": error.to_string() })
                                .as_object()
                                .cloned()
                                .unwrap_or_default()
                                .into_iter()
                                .collect(),
                        );
                    }
                }
            } else {
                state = DesktopWindowObservationState::default();
            }
            if settings.folders {
                let observations = window_observer::collect_desktop_folders();
                for transition in folder_state.update(Utc::now(), observations) {
                    if let Err(error) = runtime.emit_desktop_folder_transition(transition).await {
                        let _ = runtime.logger().record(
                            "desktop.folder.event.error",
                            "device-host",
                            serde_json::json!({ "message": error.to_string() })
                                .as_object()
                                .cloned()
                                .unwrap_or_default()
                                .into_iter()
                                .collect(),
                        );
                    }
                }
            } else {
                folder_state = DesktopFolderObservationState::default();
            }
        }
    })
}

fn spawn_download_observer(runtime: LocalYuukeiRuntime) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let Some(downloads_dir) = window_observer::downloads_dir() else {
            let _ = runtime.logger().record(
                "desktop.download.observer.unavailable",
                "device-host",
                std::collections::BTreeMap::new(),
            );
            return;
        };
        if !downloads_dir.is_dir() {
            let _ = runtime.logger().record(
                "desktop.download.observer.unavailable",
                "device-host",
                std::collections::BTreeMap::new(),
            );
            return;
        }
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let watcher_result: notify::Result<RecommendedWatcher> =
            notify::recommended_watcher(move |result| {
                let _ = tx.send(result);
            });
        let mut watcher = match watcher_result {
            Ok(watcher) => watcher,
            Err(error) => {
                let _ = runtime.logger().record(
                    "desktop.download.observer.error",
                    "device-host",
                    serde_json::json!({ "message": error.to_string() })
                        .as_object()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .collect(),
                );
                return;
            }
        };
        if let Err(error) = watcher.watch(&downloads_dir, RecursiveMode::NonRecursive) {
            let _ = runtime.logger().record(
                "desktop.download.observer.error",
                "device-host",
                serde_json::json!({ "message": error.to_string() })
                    .as_object()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
            );
            return;
        }
        while let Some(result) = rx.recv().await {
            let event = match result {
                Ok(event) => event,
                Err(error) => {
                    let _ = runtime.logger().record(
                        "desktop.download.observer.error",
                        "device-host",
                        serde_json::json!({ "message": error.to_string() })
                            .as_object()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .collect(),
                    );
                    continue;
                }
            };
            if !is_download_completion_event(&event.kind) {
                continue;
            }
            for path in event.paths {
                let runtime = runtime.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    if !path.is_file() {
                        return;
                    }
                    let Some(file_name) = path
                        .file_name()
                        .and_then(|file_name| file_name.to_str())
                        .map(str::to_string)
                    else {
                        return;
                    };
                    let Some(observation) =
                        yuukei_device_host::download_file_observation_from_name(&file_name)
                    else {
                        return;
                    };
                    if let Err(error) = runtime.emit_desktop_download_completed(observation).await {
                        let _ = runtime.logger().record(
                            "desktop.download.event.error",
                            "device-host",
                            serde_json::json!({ "message": error.to_string() })
                                .as_object()
                                .cloned()
                                .unwrap_or_default()
                                .into_iter()
                                .collect(),
                        );
                    }
                });
            }
        }
    })
}

fn is_download_completion_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_)
            | EventKind::Modify(notify::event::ModifyKind::Name(
                notify::event::RenameMode::To
                    | notify::event::RenameMode::Both
                    | notify::event::RenameMode::Any
            ))
    )
}

async fn attach_tauri_surface_or_status(
    app: &AppHandle,
    runtime: &LocalYuukeiRuntime,
) -> Result<(), String> {
    match runtime
        .attach_surface(tauri_surface_session(runtime.device_id()))
        .await
    {
        Ok(_) => Ok(()),
        Err(error) if error.daihon_report().is_some() => {
            emit_world_pack_status(app, &runtime.world_pack_status())?;
            Ok(())
        }
        Err(error) => Err(to_message(error)),
    }
}

async fn emit_app_startup_or_status(
    app: &AppHandle,
    runtime: &LocalYuukeiRuntime,
) -> Result<(), String> {
    match runtime.emit_app_startup().await {
        Ok(_) => Ok(()),
        Err(error) if error.daihon_report().is_some() => {
            emit_world_pack_status(app, &runtime.world_pack_status())?;
            Ok(())
        }
        Err(error) => Err(to_message(error)),
    }
}

fn emit_world_pack_status(app: &AppHandle, status: &WorldPackSelectionState) -> Result<(), String> {
    app.emit("yuukei-world-pack-status", status)
        .map_err(to_message)
}

fn spawn_command_forwarder(
    home: Arc<yuukei_resident_home::ResidentHome>,
    app: AppHandle,
    stage: Arc<DesktopStageManager>,
    audio_player: Option<Arc<AudioPlayer>>,
) -> tauri::async_runtime::JoinHandle<()> {
    let mut receiver = home.subscribe_commands();
    tauri::async_runtime::spawn(async move {
        while let Ok(command) = receiver.recv().await {
            if command.kind == "audio.play" {
                if let Some(player) = &audio_player {
                    if let Err(error) = player.play_command(&command) {
                        eprintln!("Yuukei audio.play ignored: {error}");
                    }
                } else {
                    eprintln!("Yuukei audio.play ignored: audio output unavailable");
                }
            }
            let _ = stage.handle_runtime_command(&app, &command);
            let _ = app.emit("yuukei-command", &command);
        }
    })
}

fn to_message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use yuukei_device_host::{
        build_avatar_gesture_poke_event, build_conversation_text_event, AvatarGestureInput,
        AvatarGesturePoke, AvatarGestureScreen, DEFAULT_RESIDENT_ID,
    };
    use yuukei_protocol::{SurfaceKind, SurfaceRenderer};

    #[test]
    fn surface_session_contains_required_boundary_fields() {
        let session = tauri_surface_session("device-local");
        assert_eq!(session.surface_id, TAURI_SURFACE_ID);
        assert_eq!(session.device_id, "device-local");
        assert!(session.active);
        assert_eq!(session.kind, SurfaceKind::Desktop);
        assert_eq!(session.presentation.renderer, Some(SurfaceRenderer::Vrm));
        assert_eq!(session.presentation.transparent, Some(true));
        assert_eq!(session.presentation.accepts_input, Some(true));
        assert!(session
            .capabilities
            .iter()
            .any(|capability| capability == "avatar.gesture.poke"));
        assert!(session
            .capabilities
            .iter()
            .any(|capability| capability == "actor.place"));
    }

    #[test]
    fn pack_asset_routes_percent_encode_path_segments() {
        assert_eq!(actor_model_route("yuukei"), "actors/yuukei/model");
        assert_eq!(
            actor_motion_route("yuukei", "歩く"),
            "actors/yuukei/motions/%E6%AD%A9%E3%81%8F"
        );
        assert_eq!(
            actor_motion_route("actor/with/slash", "walk now"),
            "actors/actor%2Fwith%2Fslash/motions/walk%20now"
        );
    }

    #[test]
    fn user_input_is_sent_as_conversation_text_event() {
        let event = build_conversation_text_event(
            DEFAULT_RESIDENT_ID,
            "device-local",
            TAURI_SURFACE_ID,
            "hello",
        );
        assert_eq!(event.kind, "conversation.text");
        assert_eq!(event.source, "surface");
        assert_eq!(event.resident_id, DEFAULT_RESIDENT_ID);
        assert_eq!(event.device_id.as_deref(), Some("device-local"));
        assert_eq!(event.surface_id.as_deref(), Some(TAURI_SURFACE_ID));
        assert_eq!(event.payload["text"], json!("hello"));
    }

    #[test]
    fn avatar_gesture_poke_is_sent_as_surface_event() {
        let event = build_avatar_gesture_poke_event(
            DEFAULT_RESIDENT_ID,
            "device-local",
            TAURI_SURFACE_ID,
            AvatarGesturePoke {
                actor_id: "yuukei".to_string(),
                hit_zone_id: "head".to_string(),
                hit_zone_label: Some("頭".to_string()),
                hit_surface: Some("face".to_string()),
                hit_bone: Some("head".to_string()),
                input: AvatarGestureInput {
                    kind: "pointer".to_string(),
                    button: "primary".to_string(),
                },
                screen: AvatarGestureScreen { x: 12.0, y: 34.0 },
            },
        );
        assert_eq!(event.kind, "avatar.gesture.poke");
        assert_eq!(event.source, "surface");
        assert_eq!(event.resident_id, DEFAULT_RESIDENT_ID);
        assert_eq!(event.device_id.as_deref(), Some("device-local"));
        assert_eq!(event.surface_id.as_deref(), Some(TAURI_SURFACE_ID));
        assert_eq!(event.actor_id.as_deref(), Some("yuukei"));
        assert_eq!(event.payload["hitZoneId"], json!("head"));
        assert_eq!(event.payload["hitZoneLabel"], json!("頭"));
        assert_eq!(event.payload["hitSurface"], json!("face"));
        assert_eq!(event.payload["hitBone"], json!("head"));
    }
}
