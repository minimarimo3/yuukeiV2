use crate::DesktopActorSurfaceAssetCatalog;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::RwLock,
};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewWindow};
use yuukei_device_host::{
    DesktopWindowFrame, DesktopWindowObservation, StageFootAnchor, DEFAULT_ACTOR_SCALE_PERCENT,
};
use yuukei_protocol::RuntimeCommand;

mod drag;
mod geometry;
mod layout;
#[cfg(test)]
mod tests;
mod walk;
mod windows;
#[cfg(windows)]
mod windows_caption;
use drag::{
    begin_actor_drag_in_state, move_actor_drag_in_state, take_actor_drag_in_state, ActiveActorDrag,
};
use geometry::*;
pub(crate) use geometry::{command_actor_id, foot_anchor};
use layout::*;
use walk::*;
pub(crate) use windows::enforce_borderless;
#[allow(unused_imports)]
pub use windows::{
    actor_webview_windows, actor_window_label, is_actor_window_label, is_stage_overlay_label,
    stage_overlay_window_label,
};
use windows::{create_actor_window, create_stage_overlay_window, monitor_snapshots};

const ACTOR_WINDOW_LABEL_PREFIX: &str = "actor-";
const STAGE_OVERLAY_LABEL_PREFIX: &str = "stage-overlay-";
const ACTOR_WINDOW_WIDTH: f64 = 420.0;
const ACTOR_WINDOW_HEIGHT: f64 = 560.0;
const ACTOR_WINDOW_MARGIN: f64 = 24.0;
const ACTOR_COLLISION_PADDING: f64 = 16.0;
const DEFAULT_BUBBLE_DURATION_MS: u64 = 9_000;
const MIN_BUBBLE_DURATION_MS: u64 = 2_500;
const MAX_BUBBLE_DURATION_MS: u64 = 30_000;
const STAGE_STATE_EVENT: &str = "yuukei-stage-state";

#[derive(Debug, Default)]
pub struct DesktopStageManager {
    state: RwLock<DesktopStageState>,
}

#[derive(Clone, Debug)]
struct DesktopStageState {
    monitors: Vec<StageMonitor>,
    actors: BTreeMap<String, StageActor>,
    bubbles: BTreeMap<String, StageBubble>,
    perches: BTreeMap<String, StagePerch>,
    terrain_windows: BTreeMap<String, DesktopWindowFrame>,
    persisted_anchors: BTreeMap<String, StageFootAnchor>,
    active_drags: BTreeMap<String, ActiveActorDrag>,
    active_walks: BTreeMap<String, ActiveStageWalk>,
    actor_scale_percent: u16,
    window_observation_enabled: bool,
}

impl Default for DesktopStageState {
    fn default() -> Self {
        Self {
            monitors: Vec::new(),
            actors: BTreeMap::new(),
            bubbles: BTreeMap::new(),
            perches: BTreeMap::new(),
            terrain_windows: BTreeMap::new(),
            persisted_anchors: BTreeMap::new(),
            active_drags: BTreeMap::new(),
            active_walks: BTreeMap::new(),
            actor_scale_percent: DEFAULT_ACTOR_SCALE_PERCENT,
            window_observation_enabled: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageAnchor {
    pub x: f64,
    pub y: f64,
    pub visible: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorStageAnchorReport {
    pub anchor: StageAnchor,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageMonitor {
    pub id: String,
    pub label: String,
    pub name: Option<String>,
    pub bounds: StageRect,
    pub scale_factor: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageActor {
    pub actor_id: String,
    pub display_name: String,
    pub window_label: String,
    pub bounds: StageRect,
    pub anchor: StageAnchor,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageBubble {
    pub bubble_id: String,
    pub actor_id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choice: Option<StageBubbleChoice>,
    pub created_at_ms: u64,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageBubbleChoice {
    pub choice_id: String,
    pub choices: Vec<String>,
    pub timeout_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopStageSnapshot {
    pub monitors: Vec<StageMonitor>,
    pub actors: Vec<StageActor>,
    pub bubbles: Vec<StageBubble>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StagePerchEnded {
    pub actor_id: String,
    pub window_key: String,
    pub reason: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageDragFinished {
    pub actor_id: String,
    pub anchor: StageFootAnchor,
    pub moved_distance: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorWindowDragStarted {
    pub session_id: String,
}

#[derive(Clone, Debug, PartialEq)]
struct StagePerch {
    window_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorWindowSpec {
    pub actor_id: String,
    pub display_name: String,
    pub label: String,
    pub index: usize,
}

impl DesktopStageManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Result<DesktopStageSnapshot, String> {
        let state = self
            .state
            .read()
            .map_err(|_| "desktop stage lock is poisoned".to_string())?;
        Ok(state.snapshot())
    }

    pub fn emit_state(&self, app: &AppHandle) -> Result<(), String> {
        let snapshot = self.snapshot()?;
        self.raise_overlay_windows(app, &snapshot.monitors)?;
        app.emit(STAGE_STATE_EVENT, &snapshot).map_err(to_message)
    }

    pub fn set_persisted_actor_anchors(
        &self,
        anchors: BTreeMap<String, StageFootAnchor>,
    ) -> Result<(), String> {
        let mut state = self
            .state
            .write()
            .map_err(|_| "desktop stage lock is poisoned".to_string())?;
        state.persisted_anchors = anchors;
        Ok(())
    }

    pub fn sync_surfaces(
        &self,
        app: &AppHandle,
        catalog: &DesktopActorSurfaceAssetCatalog,
    ) -> Result<(), String> {
        let monitors = monitor_snapshots(app)?;
        self.sync_overlay_windows(app, &monitors)?;
        let existing_labels = app.webview_windows().into_keys().collect::<Vec<_>>();
        let reconcile = reconcile_actor_windows(existing_labels, catalog);

        for label in &reconcile.close_labels {
            if let Some(window) = app.get_webview_window(label) {
                window.close().map_err(to_message)?;
            }
        }

        let mut current_bounds = BTreeMap::new();
        for spec in &reconcile.desired_specs {
            if let Some(window) = app.get_webview_window(&spec.label) {
                if let Ok(bounds) = window_bounds(&window) {
                    current_bounds.insert(spec.actor_id.clone(), bounds);
                }
            }
        }
        let actor_size = {
            let state = self
                .state
                .read()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            actor_window_size(state.actor_scale_percent)
        };
        let persisted_anchors = {
            let state = self
                .state
                .read()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            state.persisted_anchors.clone()
        };
        let resolved_bounds = resolve_actor_window_layout(
            &reconcile.desired_specs,
            &current_bounds,
            &persisted_anchors,
            &monitors,
            actor_size,
        );
        let mut next_actors = BTreeMap::new();
        for spec in &reconcile.desired_specs {
            let bounds = resolved_bounds
                .get(&spec.actor_id)
                .cloned()
                .unwrap_or_else(|| place_actor_window(spec.index, &monitors, &[], actor_size));
            if let Some(window) = app.get_webview_window(&spec.label) {
                apply_actor_window_bounds(&window, &bounds)?;
                let visible = window.is_visible().unwrap_or(true);
                next_actors.insert(
                    spec.actor_id.clone(),
                    actor_from_spec(spec, bounds, visible),
                );
            } else {
                create_actor_window(app, spec, &bounds)?;
                next_actors.insert(spec.actor_id.clone(), actor_from_spec(spec, bounds, true));
            }
        }

        {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            state.monitors = monitors;
            state.actors = next_actors;
            let terrain_windows = state.terrain_windows.clone();
            reapply_perches_to_state(&mut state, &terrain_windows);
            let actor_ids = state.actors.keys().cloned().collect::<BTreeSet<_>>();
            state
                .bubbles
                .retain(|_, bubble| actor_ids.contains(&bubble.actor_id));
            state
                .perches
                .retain(|actor_id, _| actor_ids.contains(actor_id));
            state
                .active_walks
                .retain(|actor_id, _| actor_ids.contains(actor_id));
        }
        self.emit_state(app)
    }

    pub fn apply_actor_scale_percent(&self, app: &AppHandle, percent: u16) -> Result<(), String> {
        let apply_bounds = {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            let apply_bounds = apply_actor_scale_to_state(&mut state, percent);
            update_persisted_anchors_after_scale(&mut state);
            apply_bounds
        };
        for (label, bounds) in apply_bounds {
            if let Some(window) = app.get_webview_window(&label) {
                apply_actor_window_bounds(&window, &bounds)?;
            }
        }
        self.emit_state(app)
    }

    pub fn persisted_actor_anchors(&self) -> Result<BTreeMap<String, StageFootAnchor>, String> {
        let state = self
            .state
            .read()
            .map_err(|_| "desktop stage lock is poisoned".to_string())?;
        Ok(state.persisted_anchors.clone())
    }

    pub fn cancel_actor_walk(&self, actor_id: &str) -> Result<Option<String>, String> {
        let mut state = self
            .state
            .write()
            .map_err(|_| "desktop stage lock is poisoned".to_string())?;
        Ok(cancel_actor_walk_in_state(&mut state, actor_id))
    }

    pub fn start_actor_walk(
        &self,
        app: &AppHandle,
        command: &RuntimeCommand,
    ) -> Result<Option<StageWalkStarted>, String> {
        let Some(actor_id) = command_actor_id(command) else {
            return Ok(None);
        };
        let Some(destination) = command
            .payload
            .get("destination")
            .and_then(Value::as_str)
            .and_then(WalkDestination::parse)
        else {
            return Ok(None);
        };
        let speed_px_per_sec = command
            .payload
            .get("speedPxPerSec")
            .and_then(Value::as_f64)
            .unwrap_or(DEFAULT_WALK_SPEED_PX_PER_SEC);
        let started = {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            if !state.actors.contains_key(&actor_id) {
                return Ok(None);
            }
            start_actor_walk_in_state(
                &mut state,
                &actor_id,
                &command.id,
                destination,
                speed_px_per_sec,
            )?
        };
        if let Some(window) = app.get_webview_window(&started.window_label) {
            apply_actor_window_bounds(&window, &started.bounds)?;
        }
        self.emit_state(app)?;
        Ok(Some(started))
    }

    pub fn advance_actor_walk(
        &self,
        app: &AppHandle,
        actor_id: &str,
        walk_id: &str,
        delta_seconds: f64,
    ) -> Result<Option<WalkStep>, String> {
        let progress = {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            advance_actor_walk_in_state(&mut state, actor_id, walk_id, delta_seconds)
        };
        let Some(progress) = progress else {
            return Ok(None);
        };
        if let Some(window) = app.get_webview_window(&actor_window_label(actor_id)) {
            apply_actor_window_bounds(&window, &progress.bounds)?;
        }
        self.emit_state(app)?;
        Ok(Some(progress))
    }

    pub fn begin_actor_drag(
        &self,
        app: &AppHandle,
        window: &WebviewWindow,
        actor_id: &str,
    ) -> Result<(ActorWindowDragStarted, Option<StagePerchEnded>, Option<String>), String> {
        let bounds = window_bounds(window)?;
        let session_id = uuid::Uuid::new_v4().to_string();
        let (ended, cancelled_walk_id) = {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            begin_actor_drag_in_state(&mut state, actor_id, &session_id, bounds)?
        };
        if ended.is_some() || cancelled_walk_id.is_some() {
            self.emit_state(app)?;
        }
        Ok((ActorWindowDragStarted { session_id }, ended, cancelled_walk_id))
    }

    pub fn finish_actor_drag(
        &self,
        app: &AppHandle,
        window: &WebviewWindow,
        actor_id: &str,
        session_id: &str,
    ) -> Result<StageDragFinished, String> {
        let actual_bounds = window_bounds(window)?;
        let (bounds, result) = {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            let start = take_actor_drag_in_state(&mut state, actor_id, session_id)?.start_bounds;
            let size = actor_window_size(state.actor_scale_percent);
            let bounds = normalize_actor_window_bounds(actual_bounds, &state.monitors, size);
            let anchor = foot_anchor(&bounds);
            let start_anchor = foot_anchor(&start);
            let moved_distance =
                ((anchor.x - start_anchor.x).hypot(anchor.y - start_anchor.y)).round() as u64;
            let actor = state
                .actors
                .get_mut(actor_id)
                .ok_or_else(|| format!("unknown stage actor: {actor_id}"))?;
            actor.bounds = bounds.clone();
            actor.anchor = default_actor_anchor(&bounds);
            state.persisted_anchors.insert(actor_id.to_string(), anchor);
            (
                bounds,
                StageDragFinished {
                    actor_id: actor_id.to_string(),
                    anchor,
                    moved_distance,
                },
            )
        };
        apply_actor_window_bounds(window, &bounds)?;
        self.emit_state(app)?;
        Ok(result)
    }

    pub fn cancel_actor_drag(
        &self,
        app: &AppHandle,
        window: &WebviewWindow,
        actor_id: &str,
        session_id: &str,
    ) -> Result<(), String> {
        let actual_bounds = window_bounds(window)?;
        let bounds = {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            take_actor_drag_in_state(&mut state, actor_id, session_id)?;
            let size = actor_window_size(state.actor_scale_percent);
            let bounds = normalize_actor_window_bounds(actual_bounds, &state.monitors, size);
            let actor = state
                .actors
                .get_mut(actor_id)
                .ok_or_else(|| format!("unknown stage actor: {actor_id}"))?;
            actor.bounds = bounds.clone();
            actor.anchor = default_actor_anchor(&bounds);
            bounds
        };
        apply_actor_window_bounds(window, &bounds)?;
        self.emit_state(app)
    }

    pub fn move_actor_drag(
        &self,
        app: &AppHandle,
        window: &WebviewWindow,
        actor_id: &str,
        session_id: &str,
        dx: f64,
        dy: f64,
    ) -> Result<(), String> {
        if !dx.is_finite() || !dy.is_finite() {
            return Err("actor drag delta must be finite".to_string());
        }
        let bounds = {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            move_actor_drag_in_state(&mut state, actor_id, session_id, dx, dy)?
        };
        apply_actor_window_bounds(window, &bounds)?;
        self.emit_state(app)
    }

    pub fn set_window_observation_enabled(&self, enabled: bool) -> Result<(), String> {
        let mut state = self
            .state
            .write()
            .map_err(|_| "desktop stage lock is poisoned".to_string())?;
        state.window_observation_enabled = enabled;
        if !enabled {
            state.perches.clear();
            state.terrain_windows.clear();
        }
        Ok(())
    }

    pub fn apply_window_terrain(
        &self,
        app: &AppHandle,
        observations: &[DesktopWindowObservation],
    ) -> Result<Vec<StagePerchEnded>, String> {
        let (apply_bounds, ended) = {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            if !state.window_observation_enabled {
                return Ok(Vec::new());
            }
            apply_window_terrain_to_state(&mut state, observations)
        };
        for (label, bounds) in apply_bounds {
            if let Some(window) = app.get_webview_window(&label) {
                apply_actor_window_bounds(&window, &bounds)?;
            }
        }
        if !ended.is_empty() || !observations.is_empty() {
            self.emit_state(app)?;
        }
        Ok(ended)
    }

    pub fn set_actor_window_visible(
        &self,
        app: &AppHandle,
        window_label: &str,
        visible: bool,
    ) -> Result<(), String> {
        if !is_actor_window_label(window_label) {
            return Ok(());
        }
        if self.set_actor_visibility_for_window(window_label, visible)? {
            self.emit_state(app)?;
        }
        Ok(())
    }

    pub fn refresh_actor_window(
        &self,
        app: &AppHandle,
        window: &WebviewWindow,
    ) -> Result<(), String> {
        if !is_actor_window_label(window.label()) {
            return Ok(());
        }
        let Some(actor_id) = self.actor_id_for_window_label(window.label())? else {
            return Ok(());
        };
        let bounds = window_bounds(window)?;
        {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            if let Some(actor) = state.actors.get_mut(&actor_id) {
                actor.bounds = bounds.clone();
                actor.anchor = default_actor_anchor(&bounds);
                actor.visible = window.is_visible().unwrap_or(true);
            }
        }
        self.emit_state(app)
    }

    pub fn report_actor_anchor(
        &self,
        app: &AppHandle,
        window: &WebviewWindow,
        actor_id: String,
        report: ActorStageAnchorReport,
    ) -> Result<(), String> {
        let bounds = window_bounds(window)?;
        let anchor = StageAnchor {
            x: bounds.x + report.anchor.x,
            y: bounds.y + report.anchor.y,
            visible: report.anchor.visible,
        };
        {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            if let Some(actor) = state.actors.get_mut(&actor_id) {
                actor.bounds = bounds;
                actor.anchor = anchor;
                actor.visible = window.is_visible().unwrap_or(true);
            }
        }
        self.emit_state(app)
    }

    pub fn dismiss_bubble(&self, app: &AppHandle, bubble_id: String) -> Result<(), String> {
        {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            state.bubbles.remove(&bubble_id);
        }
        self.emit_state(app)
    }

    pub fn handle_runtime_command(
        &self,
        app: &AppHandle,
        command: &RuntimeCommand,
    ) -> Result<(), String> {
        let Some(actor_id) = command_actor_id(command) else {
            return Ok(());
        };
        if command.kind == "stage.perch" {
            let Some(window_key) = command.payload.get("windowKey").and_then(Value::as_str) else {
                return Ok(());
            };
            {
                let mut state = self
                    .state
                    .write()
                    .map_err(|_| "desktop stage lock is poisoned".to_string())?;
                if !state.window_observation_enabled {
                    eprintln!("Yuukei stage.perch ignored: window observation is disabled");
                    return Ok(());
                }
                if !state.actors.contains_key(&actor_id) {
                    return Ok(());
                }
                state.perches.insert(
                    actor_id,
                    StagePerch {
                        window_key: window_key.to_string(),
                    },
                );
            }
            return self.emit_state(app);
        }
        if command.kind == "stage.perch.release" {
            let apply_bounds = {
                let mut state = self
                    .state
                    .write()
                    .map_err(|_| "desktop stage lock is poisoned".to_string())?;
                state.perches.remove(&actor_id);
                restore_actor_to_desktop(&mut state, &actor_id)
            };
            if let Some((label, bounds)) = apply_bounds {
                if let Some(window) = app.get_webview_window(&label) {
                    apply_actor_window_bounds(&window, &bounds)?;
                }
                return self.emit_state(app);
            }
            return Ok(());
        }
        if command.kind == "dialogue.choices.clear" {
            let Some(choice_id) = command.payload.get("choiceId").and_then(Value::as_str) else {
                return Ok(());
            };
            {
                let mut state = self
                    .state
                    .write()
                    .map_err(|_| "desktop stage lock is poisoned".to_string())?;
                for bubble in state.bubbles.values_mut() {
                    if bubble.actor_id == actor_id
                        && bubble
                            .choice
                            .as_ref()
                            .is_some_and(|choice| choice.choice_id == choice_id)
                    {
                        bubble.choice = None;
                    }
                }
            }
            return self.emit_state(app);
        }

        if command.kind == "dialogue.choices" {
            let Some(choice_id) = command.payload.get("choiceId").and_then(Value::as_str) else {
                return Ok(());
            };
            let Some(choices) = command.payload.get("choices").and_then(Value::as_array) else {
                return Ok(());
            };
            let choices = choices
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if choices.is_empty() {
                return Ok(());
            }
            let timeout_seconds = command
                .payload
                .get("timeoutSeconds")
                .and_then(Value::as_u64)
                .unwrap_or(30);
            {
                let mut state = self
                    .state
                    .write()
                    .map_err(|_| "desktop stage lock is poisoned".to_string())?;
                if !state.actors.contains_key(&actor_id) {
                    return Ok(());
                }
                for bubble in state.bubbles.values_mut() {
                    if bubble.actor_id == actor_id {
                        bubble.choice = None;
                    }
                }
                let latest_bubble_id = state
                    .bubbles
                    .values()
                    .filter(|bubble| bubble.actor_id == actor_id)
                    .max_by_key(|bubble| bubble.created_at_ms)
                    .map(|bubble| bubble.bubble_id.clone());
                let choice = StageBubbleChoice {
                    choice_id: choice_id.to_string(),
                    choices,
                    timeout_seconds,
                };
                if let Some(bubble_id) = latest_bubble_id {
                    if let Some(bubble) = state.bubbles.get_mut(&bubble_id) {
                        bubble.choice = Some(choice);
                        bubble.duration_ms =
                            bubble.duration_ms.max(timeout_seconds.saturating_mul(1000));
                    }
                } else {
                    state.bubbles.insert(
                        command.id.clone(),
                        StageBubble {
                            bubble_id: command.id.clone(),
                            actor_id,
                            text: String::new(),
                            choice: Some(choice),
                            created_at_ms: now_ms(),
                            duration_ms: timeout_seconds
                                .saturating_mul(1000)
                                .clamp(MIN_BUBBLE_DURATION_MS, MAX_BUBBLE_DURATION_MS),
                        },
                    );
                }
            }
            return self.emit_state(app);
        }

        if command.kind != "dialogue.say" {
            return Ok(());
        }
        let Some(text) = command.payload.get("text").and_then(Value::as_str) else {
            return Ok(());
        };
        let duration_ms = command
            .payload
            .get("durationMs")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_BUBBLE_DURATION_MS)
            .clamp(MIN_BUBBLE_DURATION_MS, MAX_BUBBLE_DURATION_MS);
        {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            if !state.actors.contains_key(&actor_id) {
                return Ok(());
            }
            state.bubbles.insert(
                command.id.clone(),
                StageBubble {
                    bubble_id: command.id.clone(),
                    actor_id,
                    text: text.to_string(),
                    choice: None,
                    created_at_ms: now_ms(),
                    duration_ms,
                },
            );
        }
        self.emit_state(app)
    }

    pub fn actor_windows(&self, app: &AppHandle) -> Vec<WebviewWindow> {
        actor_webview_windows(app)
    }

    fn actor_id_for_window_label(&self, label: &str) -> Result<Option<String>, String> {
        let state = self
            .state
            .read()
            .map_err(|_| "desktop stage lock is poisoned".to_string())?;
        Ok(state
            .actors
            .values()
            .find(|actor| actor.window_label == label)
            .map(|actor| actor.actor_id.clone()))
    }

    fn set_actor_visibility_for_window(&self, label: &str, visible: bool) -> Result<bool, String> {
        let mut state = self
            .state
            .write()
            .map_err(|_| "desktop stage lock is poisoned".to_string())?;
        let Some(actor) = state
            .actors
            .values_mut()
            .find(|actor| actor.window_label == label)
        else {
            return Ok(false);
        };
        if actor.visible == visible {
            return Ok(false);
        }
        actor.visible = visible;
        Ok(true)
    }

    fn sync_overlay_windows(
        &self,
        app: &AppHandle,
        monitors: &[StageMonitor],
    ) -> Result<(), String> {
        let desired_labels = monitors
            .iter()
            .map(|monitor| monitor.label.clone())
            .collect::<BTreeSet<_>>();
        let existing_labels = app.webview_windows().into_keys().collect::<Vec<_>>();
        for label in existing_labels {
            if is_stage_overlay_label(&label) && !desired_labels.contains(&label) {
                if let Some(window) = app.get_webview_window(&label) {
                    window.close().map_err(to_message)?;
                }
            }
        }
        for monitor in monitors {
            match app.get_webview_window(&monitor.label) {
                Some(window) => {
                    window
                        .set_position(LogicalPosition::new(monitor.bounds.x, monitor.bounds.y))
                        .map_err(to_message)?;
                    window
                        .set_size(LogicalSize::new(
                            monitor.bounds.width,
                            monitor.bounds.height,
                        ))
                        .map_err(to_message)?;
                    window.set_ignore_cursor_events(true).map_err(to_message)?;
                    window.show().map_err(to_message)?;
                    enforce_borderless(&window);
                }
                None => {
                    create_stage_overlay_window(app, monitor)?;
                }
            }
        }
        Ok(())
    }

    fn raise_overlay_windows(
        &self,
        app: &AppHandle,
        monitors: &[StageMonitor],
    ) -> Result<(), String> {
        for monitor in monitors {
            if let Some(window) = app.get_webview_window(&monitor.label) {
                window.set_always_on_top(true).map_err(to_message)?;
                window.show().map_err(to_message)?;
                enforce_borderless(&window);
            }
        }
        Ok(())
    }
}

impl DesktopStageState {
    fn snapshot(&self) -> DesktopStageSnapshot {
        DesktopStageSnapshot {
            monitors: self.monitors.clone(),
            actors: self.actors.values().cloned().collect(),
            bubbles: self.bubbles.values().cloned().collect(),
        }
    }
}
