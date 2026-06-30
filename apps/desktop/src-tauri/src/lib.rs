use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use serde::Serialize;
use tauri::{
    http::{Response, StatusCode},
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, UriSchemeContext, WindowEvent,
};
use tokio::sync::Mutex;
use yuukei_device_host::{
    tauri_surface_session, ActorSurfaceRendererKind, ExtensionSettingsChangeResult,
    ExtensionSettingsState, LocalRuntimeEnvironment, LocalYuukeiRuntime, WorldPackSelectionState,
    WorldPackSwitchResult, TAURI_SURFACE_ID,
};
use yuukei_protocol::{ExtensionHookPoint, ResidentSnapshot, RuntimeCommand};
use yuukei_world::resolve_pack_relative_path;

mod power_observer;
use power_observer::PowerObserver;

pub struct AppState {
    env: LocalRuntimeEnvironment,
    runtime: Mutex<LocalYuukeiRuntime>,
    asset_index: RwLock<PackAssetIndex>,
    command_forwarder: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    presence_loop: Mutex<Option<tokio::task::JoinHandle<()>>>,
    power_observer: Mutex<Option<PowerObserver>>,
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
    let session = tauri_surface_session(runtime.device_id());
    runtime.attach_surface(session).await.map_err(to_message)?;
    ensure_presence_loop(&state, &runtime).await?;
    let snapshot = runtime.snapshot().map_err(to_message)?;
    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
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
async fn get_actor_surface_assets(
    state: State<'_, AppState>,
) -> Result<DesktopActorSurfaceAssetCatalog, String> {
    let runtime = state.runtime.lock().await;
    Ok(desktop_actor_surface_assets(&runtime))
}

#[tauri::command]
fn set_actor_window_click_through(app: AppHandle, passthrough: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("actor")
        .ok_or_else(|| "actor window is not available".to_string())?;
    window
        .set_ignore_cursor_events(passthrough)
        .map_err(to_message)
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
    let commands = runtime
        .send_conversation_text(TAURI_SURFACE_ID, &text)
        .await
        .map_err(to_message)?;
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
    let runtime = LocalYuukeiRuntime::select_world_pack_directory_in(state.env.clone(), path)
        .await
        .map_err(to_message)?;
    replace_runtime(app, state, runtime).await
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
            configure_app_menu(app.handle())
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            configure_tray(app.handle())
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            let command_forwarder = spawn_command_forwarder(runtime.home(), app.handle().clone());
            let power_observer = PowerObserver::new(runtime.clone());
            app.manage(AppState {
                env,
                runtime: Mutex::new(runtime),
                asset_index: RwLock::new(asset_index),
                command_forwarder: Mutex::new(Some(command_forwarder)),
                presence_loop: Mutex::new(None),
                power_observer: Mutex::new(Some(power_observer)),
            });
            Ok(())
        })
        .on_menu_event(|app, event| {
            if let Err(error) = handle_menu_event(app, event.id().as_ref()) {
                eprintln!("Yuukei menu error: {error}");
            }
        })
        .on_window_event(|window, event| {
            if window.label() == "settings" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            attach_surface,
            get_snapshot,
            get_world_pack_status,
            get_extension_settings,
            get_actor_surface_assets,
            set_actor_window_click_through,
            open_settings_window,
            send_conversation_text,
            select_world_pack_directory,
            reset_world_pack_to_default,
            install_extension_directory,
            uninstall_extension,
            set_extension_enabled,
            set_extension_hook_order
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
    window.show().map_err(to_message)?;
    window.set_focus().map_err(to_message)?;
    Ok(())
}

fn toggle_actor_window(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("actor")
        .ok_or_else(|| "actor window is not available".to_string())?;
    if window.is_visible().map_err(to_message)? {
        window.hide().map_err(to_message)
    } else {
        window.show().map_err(to_message)
    }
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
    runtime
        .attach_surface(tauri_surface_session(runtime.device_id()))
        .await
        .map_err(to_message)?;
    runtime.emit_app_startup().await.map_err(to_message)?;
    let snapshot = runtime.snapshot().map_err(to_message)?;
    let next_forwarder = spawn_command_forwarder(runtime.home(), app.clone());
    let next_presence_loop = runtime.spawn_presence_loop();
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
        let mut presence_loop = state.presence_loop.lock().await;
        if let Some(previous) = presence_loop.replace(next_presence_loop) {
            previous.abort();
        }
    }
    {
        let mut power_observer = state.power_observer.lock().await;
        let _previous = power_observer.replace(next_power_observer);
    }

    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    app.emit("yuukei-assets-changed", &asset_catalog)
        .map_err(to_message)?;

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
    runtime
        .attach_surface(tauri_surface_session(runtime.device_id()))
        .await
        .map_err(to_message)?;
    runtime.emit_app_startup().await.map_err(to_message)?;
    let snapshot = runtime.snapshot().map_err(to_message)?;
    let next_forwarder = spawn_command_forwarder(runtime.home(), app.clone());
    let next_presence_loop = runtime.spawn_presence_loop();
    let next_power_observer = PowerObserver::new(runtime.clone());

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
        let mut presence_loop = state.presence_loop.lock().await;
        if let Some(previous) = presence_loop.replace(next_presence_loop) {
            previous.abort();
        }
    }
    {
        let mut power_observer = state.power_observer.lock().await;
        let _previous = power_observer.replace(next_power_observer);
    }

    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    app.emit("yuukei-assets-changed", &asset_catalog)
        .map_err(to_message)?;

    Ok(snapshot)
}

async fn ensure_presence_loop(
    state: &State<'_, AppState>,
    runtime: &LocalYuukeiRuntime,
) -> Result<(), String> {
    runtime.emit_app_startup().await.map_err(to_message)?;
    let mut presence_loop = state.presence_loop.lock().await;
    if presence_loop.is_none() {
        *presence_loop = Some(runtime.spawn_presence_loop());
    }
    Ok(())
}

fn spawn_command_forwarder(
    home: Arc<yuukei_resident_home::ResidentHome>,
    app: AppHandle,
) -> tauri::async_runtime::JoinHandle<()> {
    let mut receiver = home.subscribe_commands();
    tauri::async_runtime::spawn(async move {
        while let Ok(command) = receiver.recv().await {
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
    use yuukei_device_host::{build_conversation_text_event, DEFAULT_RESIDENT_ID};
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
}
