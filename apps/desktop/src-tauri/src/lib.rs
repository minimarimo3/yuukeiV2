use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;
use yuukei_device_host::{
    tauri_surface_session, ExtensionSettingsChangeResult, ExtensionSettingsState,
    LocalYuukeiRuntime, WorldPackSelectionState, WorldPackSwitchResult, TAURI_SURFACE_ID,
};
use yuukei_protocol::{ExtensionHookPoint, ResidentSnapshot, RuntimeCommand};

mod power_observer;
use power_observer::PowerObserver;

pub struct AppState {
    runtime: Mutex<LocalYuukeiRuntime>,
    command_forwarder: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    presence_loop: Mutex<Option<tokio::task::JoinHandle<()>>>,
    power_observer: Mutex<Option<PowerObserver>>,
}

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
    let runtime = LocalYuukeiRuntime::select_world_pack_directory(path)
        .await
        .map_err(to_message)?;
    replace_runtime(app, state, runtime).await
}

#[tauri::command]
async fn reset_world_pack_to_default(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<WorldPackSwitchResult, String> {
    let runtime = LocalYuukeiRuntime::reset_world_pack_to_default()
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
    LocalYuukeiRuntime::install_extension_directory(path).map_err(to_message)?;
    reload_runtime_for_extension_change(app, state).await
}

#[tauri::command]
async fn uninstall_extension(
    app: AppHandle,
    state: State<'_, AppState>,
    extension_id: String,
) -> Result<ExtensionSettingsChangeResult, String> {
    LocalYuukeiRuntime::uninstall_extension(&extension_id).map_err(to_message)?;
    reload_runtime_for_extension_change(app, state).await
}

#[tauri::command]
async fn set_extension_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    extension_id: String,
    enabled: bool,
) -> Result<ExtensionSettingsChangeResult, String> {
    LocalYuukeiRuntime::set_extension_enabled(&extension_id, enabled).map_err(to_message)?;
    reload_runtime_for_extension_change(app, state).await
}

#[tauri::command]
async fn set_extension_hook_order(
    app: AppHandle,
    state: State<'_, AppState>,
    hook_point: ExtensionHookPoint,
    extension_ids: Vec<String>,
) -> Result<ExtensionSettingsChangeResult, String> {
    LocalYuukeiRuntime::set_extension_hook_order(hook_point, extension_ids).map_err(to_message)?;
    reload_runtime_for_extension_change(app, state).await
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let runtime = tauri::async_runtime::block_on(LocalYuukeiRuntime::open_default())
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            println!("Yuukei app log: {}", runtime.paths().app_log_path.display());
            let command_forwarder = spawn_command_forwarder(runtime.home(), app.handle().clone());
            let power_observer = PowerObserver::new(runtime.clone());
            app.manage(AppState {
                runtime: Mutex::new(runtime),
                command_forwarder: Mutex::new(Some(command_forwarder)),
                presence_loop: Mutex::new(None),
                power_observer: Mutex::new(Some(power_observer)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            attach_surface,
            get_snapshot,
            get_world_pack_status,
            get_extension_settings,
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

async fn replace_runtime(
    app: AppHandle,
    state: State<'_, AppState>,
    runtime: LocalYuukeiRuntime,
) -> Result<WorldPackSwitchResult, String> {
    runtime
        .attach_surface(tauri_surface_session(runtime.device_id()))
        .await
        .map_err(to_message)?;
    runtime.emit_app_startup().await.map_err(to_message)?;
    let snapshot = runtime.snapshot().map_err(to_message)?;
    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    let next_forwarder = spawn_command_forwarder(runtime.home(), app);
    let next_presence_loop = runtime.spawn_presence_loop();
    let next_power_observer = PowerObserver::new(runtime.clone());
    let status = runtime.world_pack_status();

    {
        let mut current = state.runtime.lock().await;
        *current = runtime;
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

    Ok(WorldPackSwitchResult { status, snapshot })
}

async fn reload_runtime_for_extension_change(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ExtensionSettingsChangeResult, String> {
    let runtime = LocalYuukeiRuntime::open_selected()
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
    runtime
        .attach_surface(tauri_surface_session(runtime.device_id()))
        .await
        .map_err(to_message)?;
    runtime.emit_app_startup().await.map_err(to_message)?;
    let snapshot = runtime.snapshot().map_err(to_message)?;
    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    let next_forwarder = spawn_command_forwarder(runtime.home(), app);
    let next_presence_loop = runtime.spawn_presence_loop();
    let next_power_observer = PowerObserver::new(runtime.clone());

    {
        let mut current = state.runtime.lock().await;
        *current = runtime;
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
        assert_eq!(session.presentation.renderer, Some(SurfaceRenderer::Html));
        assert_eq!(session.presentation.accepts_input, Some(true));
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
