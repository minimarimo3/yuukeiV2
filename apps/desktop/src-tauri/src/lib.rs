use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};
use yuukei_device_host::{tauri_surface_session, LocalYuukeiRuntime, TAURI_SURFACE_ID};
use yuukei_protocol::{ResidentSnapshot, RuntimeCommand};

pub struct AppState {
    runtime: LocalYuukeiRuntime,
}

#[tauri::command]
fn attach_surface(app: AppHandle, state: State<'_, AppState>) -> Result<ResidentSnapshot, String> {
    let session = tauri_surface_session(state.runtime.device_id());
    let snapshot = state.runtime.attach_surface(session).map_err(to_message)?;
    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    Ok(snapshot)
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> Result<ResidentSnapshot, String> {
    state.runtime.snapshot().map_err(to_message)
}

#[tauri::command]
async fn send_conversation_text(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> Result<Vec<RuntimeCommand>, String> {
    let runtime = state.runtime.clone();
    let commands = runtime
        .send_conversation_text(TAURI_SURFACE_ID, &text)
        .await
        .map_err(to_message)?;
    let snapshot = runtime.snapshot().map_err(to_message)?;
    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    Ok(commands)
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let runtime = tauri::async_runtime::block_on(LocalYuukeiRuntime::open_default())
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            println!("Yuukei app log: {}", runtime.paths().app_log_path.display());
            spawn_command_forwarder(runtime.home(), app.handle().clone());
            app.manage(AppState { runtime });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            attach_surface,
            get_snapshot,
            send_conversation_text
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Yuukei desktop");
}

fn spawn_command_forwarder(home: Arc<yuukei_resident_home::ResidentHome>, app: AppHandle) {
    let mut receiver = home.subscribe_commands();
    tauri::async_runtime::spawn(async move {
        while let Ok(command) = receiver.recv().await {
            let _ = app.emit("yuukei-command", &command);
        }
    });
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
