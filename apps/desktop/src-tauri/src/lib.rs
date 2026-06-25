use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use yuukei_event_log::EventLog;
use yuukei_protocol::{
    JsonMap, ResidentSnapshot, RuntimeCommand, RuntimeEvent, SurfaceKind, SurfacePresentation,
    SurfaceRenderer, SurfaceSession,
};
use yuukei_resident_home::ResidentHome;
use yuukei_world::WorldPack;

const DEVICE_ID: &str = "device-local";
const SURFACE_ID: &str = "surface-main";
const RESIDENT_ID: &str = "resident-default";

pub struct AppState {
    home: Arc<ResidentHome>,
    device_id: String,
    surface_id: String,
}

#[tauri::command]
fn attach_surface(app: AppHandle, state: State<'_, AppState>) -> Result<ResidentSnapshot, String> {
    let session = build_surface_session(&state.device_id, &state.surface_id);
    let snapshot = state.home.attach_surface(session).map_err(to_message)?;
    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    Ok(snapshot)
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> Result<ResidentSnapshot, String> {
    state.home.snapshot().map_err(to_message)
}

#[tauri::command]
async fn send_conversation_text(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> Result<Vec<RuntimeCommand>, String> {
    let home = state.home.clone();
    let device_id = state.device_id.clone();
    let surface_id = state.surface_id.clone();
    let event = build_conversation_text_event(&text, &device_id, &surface_id);
    let commands = home.ingest_event(event).await.map_err(to_message)?;
    let snapshot = home.snapshot().map_err(to_message)?;
    app.emit("yuukei-snapshot", &snapshot).map_err(to_message)?;
    Ok(commands)
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let home = tauri::async_runtime::block_on(create_resident_home())
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            let home = Arc::new(home);
            spawn_command_forwarder(home.clone(), app.handle().clone());
            app.manage(AppState {
                home,
                device_id: DEVICE_ID.to_string(),
                surface_id: SURFACE_ID.to_string(),
            });
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

async fn create_resident_home() -> Result<ResidentHome, String> {
    let world_root = workspace_root().join("packs").join("default-yuukei");
    let world = WorldPack::load_from_dir(&world_root).map_err(to_message)?;
    let data_dir = std::env::temp_dir().join("yuukei-v2");
    std::fs::create_dir_all(&data_dir).map_err(to_message)?;
    let event_log = EventLog::open(data_dir.join("events.sqlite3")).map_err(to_message)?;
    ResidentHome::new(RESIDENT_ID, world, event_log)
        .await
        .map_err(to_message)
}

fn spawn_command_forwarder(home: Arc<ResidentHome>, app: AppHandle) {
    let mut receiver = home.subscribe_commands();
    tauri::async_runtime::spawn(async move {
        while let Ok(command) = receiver.recv().await {
            let _ = app.emit("yuukei-command", &command);
        }
    });
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("src-tauri is nested under apps/desktop")
        .to_path_buf()
}

fn build_surface_session(device_id: &str, surface_id: &str) -> SurfaceSession {
    SurfaceSession {
        surface_id: surface_id.to_string(),
        device_id: device_id.to_string(),
        kind: SurfaceKind::Desktop,
        active: true,
        capabilities: vec![
            "dialogue.say".to_string(),
            "avatar.expression".to_string(),
            "avatar.motion".to_string(),
        ],
        presentation: SurfacePresentation {
            renderer: Some(SurfaceRenderer::Html),
            transparent: Some(false),
            accepts_input: Some(true),
        },
    }
}

fn build_conversation_text_event(text: &str, device_id: &str, surface_id: &str) -> RuntimeEvent {
    RuntimeEvent {
        id: yuukei_protocol::new_id("evt"),
        kind: "conversation.text".to_string(),
        timestamp: yuukei_protocol::now_timestamp(),
        source: "surface".to_string(),
        resident_id: RESIDENT_ID.to_string(),
        payload: JsonMap::from([("text".to_string(), Value::String(text.to_string()))]),
        causality: None,
        device_id: Some(device_id.to_string()),
        surface_id: Some(surface_id.to_string()),
        actor_id: None,
    }
}

fn to_message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn surface_session_contains_required_boundary_fields() {
        let session = build_surface_session("device-local", "surface-main");
        assert_eq!(session.surface_id, "surface-main");
        assert_eq!(session.device_id, "device-local");
        assert!(session.active);
        assert_eq!(session.kind, SurfaceKind::Desktop);
        assert_eq!(session.presentation.renderer, Some(SurfaceRenderer::Html));
        assert_eq!(session.presentation.accepts_input, Some(true));
    }

    #[test]
    fn user_input_is_sent_as_conversation_text_event() {
        let event = build_conversation_text_event("hello", "device-local", "surface-main");
        assert_eq!(event.kind, "conversation.text");
        assert_eq!(event.source, "surface");
        assert_eq!(event.resident_id, RESIDENT_ID);
        assert_eq!(event.device_id.as_deref(), Some("device-local"));
        assert_eq!(event.surface_id.as_deref(), Some("surface-main"));
        assert_eq!(event.payload["text"], json!("hello"));
    }
}
