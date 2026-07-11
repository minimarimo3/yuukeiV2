use std::{
    collections::{BTreeMap, BTreeSet},
    sync::RwLock,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use yuukei_device_host::{
    clamp_actor_scale_percent, DesktopWindowFrame, DesktopWindowObservation, StageFootAnchor,
    DEFAULT_ACTOR_SCALE_PERCENT,
};
use yuukei_protocol::RuntimeCommand;

use crate::DesktopActorSurfaceAssetCatalog;

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

#[derive(Clone, Debug, PartialEq)]
pub struct StageDragFinished {
    pub actor_id: String,
    pub anchor: StageFootAnchor,
    pub moved_distance: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct StagePerch {
    window_key: String,
}

#[derive(Clone, Debug, PartialEq)]
struct ActiveActorDrag {
    start_bounds: StageRect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ActorWindowSize {
    width: f64,
    height: f64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorWindowSpec {
    pub actor_id: String,
    pub display_name: String,
    pub label: String,
    pub index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActorWindowReconcile {
    close_labels: Vec<String>,
    create_specs: Vec<ActorWindowSpec>,
    desired_specs: Vec<ActorWindowSpec>,
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

    pub fn begin_actor_drag(
        &self,
        app: &AppHandle,
        window: &WebviewWindow,
        actor_id: &str,
    ) -> Result<Option<StagePerchEnded>, String> {
        let bounds = window_bounds(window)?;
        let ended = {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            begin_actor_drag_in_state(&mut state, actor_id, bounds)?
        };
        if ended.is_some() {
            self.emit_state(app)?;
        }
        Ok(ended)
    }

    pub fn finish_actor_drag(
        &self,
        app: &AppHandle,
        window: &WebviewWindow,
        actor_id: &str,
    ) -> Result<StageDragFinished, String> {
        let actual_bounds = window_bounds(window)?;
        let (bounds, result) = {
            let mut state = self
                .state
                .write()
                .map_err(|_| "desktop stage lock is poisoned".to_string())?;
            let start = state
                .active_drags
                .remove(actor_id)
                .ok_or_else(|| format!("actor drag was not active: {actor_id}"))?
                .start_bounds;
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

    pub fn move_actor_drag(
        &self,
        app: &AppHandle,
        window: &WebviewWindow,
        actor_id: &str,
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
            move_actor_drag_in_state(&mut state, actor_id, dx, dy)?
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

fn begin_actor_drag_in_state(
    state: &mut DesktopStageState,
    actor_id: &str,
    bounds: StageRect,
) -> Result<Option<StagePerchEnded>, String> {
    let actor = state
        .actors
        .get_mut(actor_id)
        .ok_or_else(|| format!("unknown stage actor: {actor_id}"))?;
    actor.bounds = bounds.clone();
    actor.anchor = default_actor_anchor(&bounds);
    state.active_drags.insert(
        actor_id.to_string(),
        ActiveActorDrag {
            start_bounds: bounds,
        },
    );
    Ok(state.perches.remove(actor_id).map(|perch| StagePerchEnded {
        actor_id: actor_id.to_string(),
        window_key: perch.window_key,
        reason: "user-drag",
    }))
}

fn move_actor_drag_in_state(
    state: &mut DesktopStageState,
    actor_id: &str,
    dx: f64,
    dy: f64,
) -> Result<StageRect, String> {
    let start_bounds = state
        .active_drags
        .get(actor_id)
        .ok_or_else(|| format!("actor drag was not active: {actor_id}"))?
        .start_bounds
        .clone();
    let bounds = StageRect {
        x: start_bounds.x + dx,
        y: start_bounds.y + dy,
        width: start_bounds.width,
        height: start_bounds.height,
    };
    let actor = state
        .actors
        .get_mut(actor_id)
        .ok_or_else(|| format!("unknown stage actor: {actor_id}"))?;
    actor.bounds = bounds.clone();
    actor.anchor = default_actor_anchor(&bounds);
    Ok(bounds)
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

pub fn actor_webview_windows(app: &AppHandle) -> Vec<WebviewWindow> {
    app.webview_windows()
        .into_iter()
        .filter_map(|(label, window)| {
            if is_actor_window_label(&label) {
                Some(window)
            } else {
                None
            }
        })
        .collect()
}

pub fn is_actor_window_label(label: &str) -> bool {
    label.starts_with(ACTOR_WINDOW_LABEL_PREFIX)
}

pub fn is_stage_overlay_label(label: &str) -> bool {
    label.starts_with(STAGE_OVERLAY_LABEL_PREFIX)
}

pub fn actor_window_label(actor_id: &str) -> String {
    let mut label = String::from(ACTOR_WINDOW_LABEL_PREFIX);
    for byte in actor_id.as_bytes() {
        label.push_str(&format!("{byte:02x}"));
    }
    label
}

pub fn stage_overlay_window_label(index: usize) -> String {
    format!("{STAGE_OVERLAY_LABEL_PREFIX}{index}")
}

fn monitor_snapshots(app: &AppHandle) -> Result<Vec<StageMonitor>, String> {
    let monitors = app.available_monitors().map_err(to_message)?;
    if monitors.is_empty() {
        return Ok(vec![StageMonitor {
            id: "fallback".to_string(),
            label: stage_overlay_window_label(0),
            name: None,
            bounds: StageRect {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 800.0,
            },
            scale_factor: 1.0,
        }]);
    }
    Ok(monitors
        .into_iter()
        .enumerate()
        .map(|(index, monitor)| {
            let scale_factor = usable_scale_factor(monitor.scale_factor());
            let work_area = monitor.work_area();
            StageMonitor {
                id: format!("monitor-{index}"),
                label: stage_overlay_window_label(index),
                name: monitor.name().cloned(),
                bounds: StageRect {
                    x: work_area.position.x as f64 / scale_factor,
                    y: work_area.position.y as f64 / scale_factor,
                    width: work_area.size.width as f64 / scale_factor,
                    height: work_area.size.height as f64 / scale_factor,
                },
                scale_factor,
            }
        })
        .collect())
}

fn create_stage_overlay_window(app: &AppHandle, monitor: &StageMonitor) -> Result<(), String> {
    let window = WebviewWindowBuilder::new(app, &monitor.label, stage_overlay_url(&monitor.id))
        .title("")
        .inner_size(monitor.bounds.width, monitor.bounds.height)
        .position(monitor.bounds.x, monitor.bounds.y)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .build()
        .map_err(to_message)?;
    enforce_borderless(&window);
    window.set_ignore_cursor_events(true).map_err(to_message)?;
    Ok(())
}

fn create_actor_window(
    app: &AppHandle,
    spec: &ActorWindowSpec,
    bounds: &StageRect,
) -> Result<(), String> {
    let window = WebviewWindowBuilder::new(app, &spec.label, actor_window_url(&spec.actor_id))
        .title("")
        .inner_size(bounds.width, bounds.height)
        .position(bounds.x, bounds.y)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .build()
        .map_err(to_message)?;
    enforce_borderless(&window);
    Ok(())
}

/// Drop the native window caption on Windows and keep it from flashing back.
///
/// tao keeps the `WS_CAPTION` style on these top-level windows at all times and
/// merely hides the caption by returning 0 from `WM_NCCALCSIZE` while its internal
/// decorations flag is off (`to_window_styles` only strips `WS_CAPTION` for child
/// windows). Two consequences on Windows 11:
///
/// 1. The builder's `decorations(false)` does not reliably take for these
///    runtime-created transparent windows, so we re-assert `set_decorations(false)`
///    to force the flag off and hide the caption in the steady state.
/// 2. Because `WS_CAPTION` is still present — tao re-adds it via `SetWindowLongW`
///    on every style update, e.g. each cursor-passthrough toggle — `DefWindowProc`
///    repaints the caption on every activation change (clicking the actor, or the
///    Start menu stealing focus), flashing the "Yuukei" title bar for a frame. We
///    install a window subclass that forwards `WM_NCACTIVATE` with `lParam = -1`,
///    the documented signal telling `DefWindowProc` not to redraw the non-client
///    area, which stops the flicker while leaving tao's focus bookkeeping intact.
///
/// No-op on platforms where the builder already produced a borderless window.
pub(crate) fn enforce_borderless(window: &WebviewWindow) {
    #[cfg(windows)]
    {
        let _ = window.set_decorations(false);
        windows_caption::suppress_activation_flicker(window);
    }
    #[cfg(not(windows))]
    {
        let _ = window;
    }
}

#[cfg(windows)]
mod windows_caption {
    use tauri::WebviewWindow;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_STYLE, STYLESTRUCT,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_NCACTIVATE,
        WM_NCDESTROY, WM_STYLECHANGED, WM_STYLECHANGING, WS_CAPTION, WS_MAXIMIZEBOX,
        WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
    };

    /// Stable id for our single caption subclass on each window.
    const SUBCLASS_ID: usize = 0x594B_00AC;

    /// Styles that can make Windows draw a native caption or resize frame.
    const CAPTION_STYLE_MASK: u32 =
        WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0;

    /// Install a subclass that stops the native caption from flashing on activation
    /// and style changes. See [`super::enforce_borderless`] for the full rationale.
    pub(super) fn suppress_activation_flicker(window: &WebviewWindow) {
        // Take the raw handle as an `isize` so the value is `Send` for the closure
        // below (Tauri's `HWND` newtype wraps a non-`Send` pointer).
        let hwnd_value = match window.hwnd() {
            Ok(hwnd) => hwnd.0 as isize,
            Err(_) => return,
        };
        // The subclass and style changes must run on the thread that owns the window.
        let _ = window.run_on_main_thread(move || unsafe {
            let hwnd = HWND(hwnd_value as *mut core::ffi::c_void);
            strip_caption_styles(hwnd);
            let _ = SetWindowSubclass(hwnd, Some(caption_subclass_proc), SUBCLASS_ID, 0);
        });
    }

    unsafe fn strip_caption_styles(hwnd: HWND) {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let next = style & !CAPTION_STYLE_MASK;
        if next == style {
            return;
        }

        let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, next as isize);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }

    unsafe extern "system" fn caption_subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _uid_subclass: usize,
        _ref_data: usize,
    ) -> LRESULT {
        match msg {
            // Prevent tao/Tauri style updates from reintroducing native caption styles.
            WM_STYLECHANGING => {
                if wparam.0 as i32 == GWL_STYLE.0 && lparam.0 != 0 {
                    let styles = &mut *(lparam.0 as *mut STYLESTRUCT);
                    styles.styleNew &= !CAPTION_STYLE_MASK;
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_STYLECHANGED => {
                if wparam.0 as i32 == GWL_STYLE.0 {
                    strip_caption_styles(hwnd);
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            // `lParam = -1` tells `DefWindowProc` to skip repainting the non-client
            // area, so any still-pending activation frame repaint is suppressed.
            WM_NCACTIVATE => DefSubclassProc(hwnd, msg, wparam, LPARAM(-1)),
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(caption_subclass_proc), SUBCLASS_ID);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

fn reconcile_actor_windows(
    existing_labels: impl IntoIterator<Item = String>,
    catalog: &DesktopActorSurfaceAssetCatalog,
) -> ActorWindowReconcile {
    let specs = actor_window_specs(catalog);
    let desired_labels = specs
        .iter()
        .map(|spec| spec.label.clone())
        .collect::<BTreeSet<_>>();
    let existing_labels = existing_labels.into_iter().collect::<BTreeSet<_>>();
    let close_labels = existing_labels
        .iter()
        .filter(|label| is_actor_window_label(label) && !desired_labels.contains(*label))
        .cloned()
        .collect();
    let create_specs = specs
        .iter()
        .filter(|spec| !existing_labels.contains(&spec.label))
        .cloned()
        .collect();

    ActorWindowReconcile {
        close_labels,
        create_specs,
        desired_specs: specs,
    }
}

fn actor_window_specs(catalog: &DesktopActorSurfaceAssetCatalog) -> Vec<ActorWindowSpec> {
    catalog
        .actors
        .iter()
        .filter(|actor| actor.renderer.is_some())
        .enumerate()
        .map(|(index, actor)| ActorWindowSpec {
            actor_id: actor.actor_id.clone(),
            display_name: actor.display_name.clone(),
            label: actor_window_label(&actor.actor_id),
            index,
        })
        .collect()
}

fn actor_from_spec(spec: &ActorWindowSpec, bounds: StageRect, visible: bool) -> StageActor {
    StageActor {
        actor_id: spec.actor_id.clone(),
        display_name: spec.display_name.clone(),
        window_label: spec.label.clone(),
        anchor: default_actor_anchor(&bounds),
        bounds,
        visible,
    }
}

fn window_bounds(window: &WebviewWindow) -> Result<StageRect, String> {
    let scale_factor = usable_scale_factor(window.scale_factor().map_err(to_message)?);
    let position = window.outer_position().map_err(to_message)?;
    let size = window.inner_size().map_err(to_message)?;
    Ok(StageRect {
        x: position.x as f64 / scale_factor,
        y: position.y as f64 / scale_factor,
        width: size.width as f64 / scale_factor,
        height: size.height as f64 / scale_factor,
    })
}

fn apply_actor_window_bounds(window: &WebviewWindow, bounds: &StageRect) -> Result<(), String> {
    let current = window_bounds(window).ok();
    if current
        .as_ref()
        .map(|rect| !same_position(rect, bounds))
        .unwrap_or(true)
    {
        window
            .set_position(LogicalPosition::new(bounds.x, bounds.y))
            .map_err(to_message)?;
    }
    if current
        .as_ref()
        .map(|rect| !same_size(rect, bounds))
        .unwrap_or(true)
    {
        window
            .set_size(LogicalSize::new(bounds.width, bounds.height))
            .map_err(to_message)?;
    }
    Ok(())
}

fn resolve_actor_window_layout(
    specs: &[ActorWindowSpec],
    current_bounds: &BTreeMap<String, StageRect>,
    persisted_anchors: &BTreeMap<String, StageFootAnchor>,
    monitors: &[StageMonitor],
    actor_size: ActorWindowSize,
) -> BTreeMap<String, StageRect> {
    let mut occupied = Vec::new();
    let mut resolved = BTreeMap::new();
    for spec in specs {
        let preferred = persisted_anchors
            .get(&spec.actor_id)
            .map(|anchor| bounds_from_foot_anchor(*anchor, actor_size))
            .or_else(|| current_bounds.get(&spec.actor_id).cloned())
            .map(|bounds| normalize_actor_window_bounds(bounds, monitors, actor_size))
            .unwrap_or_else(|| place_actor_window(spec.index, monitors, &occupied, actor_size));
        let bounds = if overlaps_any(&preferred, &occupied) {
            place_actor_window(spec.index, monitors, &occupied, actor_size)
        } else {
            preferred
        };
        let bounds = normalize_actor_window_bounds(bounds, monitors, actor_size);
        occupied.push(bounds.clone());
        resolved.insert(spec.actor_id.clone(), bounds);
    }
    resolved
}

fn normalize_actor_window_bounds(
    bounds: StageRect,
    monitors: &[StageMonitor],
    actor_size: ActorWindowSize,
) -> StageRect {
    let bounds = StageRect {
        width: actor_size.width,
        height: actor_size.height,
        ..bounds
    };
    let monitor = best_monitor_bounds_for_rect(&bounds, monitors);
    clamp_rect_to_bounds(bounds, &monitor, ACTOR_WINDOW_MARGIN)
}

fn perch_actor_bounds(
    actor_bounds: &StageRect,
    target: &DesktopWindowFrame,
    monitors: &[StageMonitor],
) -> StageRect {
    let target = StageRect {
        x: target.x,
        y: target.y,
        width: target.width,
        height: target.height,
    };
    let width = if actor_bounds.width > 0.0 {
        actor_bounds.width
    } else {
        actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT).width
    };
    let height = if actor_bounds.height > 0.0 {
        actor_bounds.height
    } else {
        actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT).height
    };
    let desired = StageRect {
        x: target.x + (target.width / 2.0) - (width / 2.0),
        y: target.y - height,
        width,
        height,
    };
    let monitor = best_monitor_bounds_for_rect(&target, monitors);
    clamp_rect_to_bounds(desired, &monitor, 0.0)
}

fn apply_window_terrain_to_state(
    state: &mut DesktopStageState,
    observations: &[DesktopWindowObservation],
) -> (Vec<(String, StageRect)>, Vec<StagePerchEnded>) {
    state.terrain_windows = observations
        .iter()
        .map(|window| (window.window_key.clone(), window.frame.clone()))
        .collect();
    let windows = observations
        .iter()
        .map(|window| (window.window_key.as_str(), &window.frame))
        .collect::<BTreeMap<_, _>>();
    let mut apply_bounds = Vec::new();
    let mut ended = Vec::new();
    let actor_ids = state.perches.keys().cloned().collect::<Vec<_>>();
    for actor_id in actor_ids {
        let Some(perch) = state.perches.get(&actor_id).cloned() else {
            continue;
        };
        if let Some(target) = windows.get(perch.window_key.as_str()) {
            let monitors = state.monitors.clone();
            let Some(actor) = state.actors.get_mut(&actor_id) else {
                state.perches.remove(&actor_id);
                continue;
            };
            let next_bounds = perch_actor_bounds(&actor.bounds, target, &monitors);
            actor.bounds = next_bounds.clone();
            actor.anchor = default_actor_anchor(&next_bounds);
            apply_bounds.push((actor.window_label.clone(), next_bounds));
        } else {
            state.perches.remove(&actor_id);
            if let Some((label, bounds)) = restore_actor_to_desktop(state, &actor_id) {
                apply_bounds.push((label, bounds));
            }
            ended.push(StagePerchEnded {
                actor_id,
                window_key: perch.window_key,
                reason: "window-closed",
            });
        }
    }
    (apply_bounds, ended)
}

fn apply_actor_scale_to_state(
    state: &mut DesktopStageState,
    percent: u16,
) -> Vec<(String, StageRect)> {
    state.actor_scale_percent = clamp_actor_scale_percent(percent);
    let actor_size = actor_window_size(state.actor_scale_percent);
    let terrain_windows = state.terrain_windows.clone();
    let mut apply_bounds = Vec::new();
    for actor in state.actors.values_mut() {
        let resized = resize_actor_bounds_from_bottom_center(&actor.bounds, actor_size);
        actor.bounds = normalize_actor_window_bounds(resized, &state.monitors, actor_size);
        actor.anchor = default_actor_anchor(&actor.bounds);
    }
    reapply_perches_to_state(state, &terrain_windows);
    for actor in state.actors.values() {
        apply_bounds.push((actor.window_label.clone(), actor.bounds.clone()));
    }
    apply_bounds
}

fn reapply_perches_to_state(
    state: &mut DesktopStageState,
    terrain_windows: &BTreeMap<String, DesktopWindowFrame>,
) {
    let actor_ids = state.perches.keys().cloned().collect::<Vec<_>>();
    for actor_id in actor_ids {
        let Some(perch) = state.perches.get(&actor_id).cloned() else {
            continue;
        };
        let Some(target) = terrain_windows.get(&perch.window_key) else {
            continue;
        };
        let Some(actor) = state.actors.get_mut(&actor_id) else {
            state.perches.remove(&actor_id);
            continue;
        };
        let bounds = perch_actor_bounds(&actor.bounds, target, &state.monitors);
        actor.bounds = bounds.clone();
        actor.anchor = default_actor_anchor(&bounds);
    }
}

fn restore_actor_to_desktop(
    state: &mut DesktopStageState,
    actor_id: &str,
) -> Option<(String, StageRect)> {
    let label = state.actors.get(actor_id)?.window_label.clone();
    let index = state
        .actors
        .keys()
        .position(|key| key == actor_id)
        .unwrap_or_default();
    let occupied = state
        .actors
        .iter()
        .filter(|(other_actor_id, _)| other_actor_id.as_str() != actor_id)
        .map(|(_, actor)| actor.bounds.clone())
        .collect::<Vec<_>>();
    let bounds = place_actor_window(
        index,
        &state.monitors,
        &occupied,
        actor_window_size(state.actor_scale_percent),
    );
    let actor = state.actors.get_mut(actor_id)?;
    actor.bounds = bounds.clone();
    actor.anchor = default_actor_anchor(&bounds);
    Some((label, bounds))
}

fn best_monitor_bounds_for_rect(rect: &StageRect, monitors: &[StageMonitor]) -> StageRect {
    monitors
        .iter()
        .max_by(|a, b| {
            let overlap_order = rect_overlap_area(rect, &a.bounds)
                .partial_cmp(&rect_overlap_area(rect, &b.bounds))
                .unwrap_or(std::cmp::Ordering::Equal);
            if overlap_order != std::cmp::Ordering::Equal {
                return overlap_order;
            }
            center_distance_squared(rect, &b.bounds)
                .partial_cmp(&center_distance_squared(rect, &a.bounds))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|monitor| monitor.bounds.clone())
        .unwrap_or(StageRect {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 800.0,
        })
}

fn place_actor_window(
    index: usize,
    monitors: &[StageMonitor],
    occupied: &[StageRect],
    actor_size: ActorWindowSize,
) -> StageRect {
    let monitor = monitors
        .first()
        .map(|monitor| monitor.bounds.clone())
        .unwrap_or(StageRect {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 800.0,
        });
    let size = StageRect {
        x: 0.0,
        y: 0.0,
        width: actor_size.width,
        height: actor_size.height,
    };
    let initial = clamp_rect_to_bounds(
        StageRect {
            x: monitor.x + 48.0 + index as f64 * 54.0,
            y: monitor.y + 96.0 + index as f64 * 36.0,
            width: size.width,
            height: size.height,
        },
        &monitor,
        ACTOR_WINDOW_MARGIN,
    );
    if !overlaps_any(&initial, occupied) {
        return initial;
    }

    let candidates = actor_collision_candidates(&monitor, &size, occupied)
        .into_iter()
        .chain(actor_grid_candidates(&monitor, &size));
    for candidate in candidates {
        if !overlaps_any(&candidate, occupied) {
            return candidate;
        }
    }

    clamp_rect_to_bounds(
        StageRect {
            x: initial.x + index as f64 * 32.0,
            y: initial.y + index as f64 * 28.0,
            width: size.width,
            height: size.height,
        },
        &monitor,
        ACTOR_WINDOW_MARGIN,
    )
}

fn actor_grid_candidates(monitor: &StageRect, size: &StageRect) -> Vec<StageRect> {
    let mut candidates = Vec::new();
    let min_x = monitor.x + ACTOR_WINDOW_MARGIN;
    let min_y = monitor.y + ACTOR_WINDOW_MARGIN;
    let max_x = (monitor.x + monitor.width - size.width - ACTOR_WINDOW_MARGIN).max(min_x);
    let max_y = (monitor.y + monitor.height - size.height - ACTOR_WINDOW_MARGIN).max(min_y);
    let step_x = size.width + ACTOR_WINDOW_MARGIN + ACTOR_COLLISION_PADDING;
    let step_y = size.height + ACTOR_WINDOW_MARGIN + ACTOR_COLLISION_PADDING;
    let mut y = min_y;
    while y <= max_y + 0.5 {
        let mut x = min_x;
        while x <= max_x + 0.5 {
            candidates.push(StageRect {
                x,
                y,
                width: size.width,
                height: size.height,
            });
            x += step_x;
        }
        y += step_y;
    }
    candidates
}

fn actor_collision_candidates(
    monitor: &StageRect,
    size: &StageRect,
    occupied: &[StageRect],
) -> Vec<StageRect> {
    let mut candidates = Vec::new();
    for other in occupied {
        let right = StageRect {
            x: other.x + other.width + ACTOR_COLLISION_PADDING,
            y: other.y,
            width: size.width,
            height: size.height,
        };
        let left = StageRect {
            x: other.x - size.width - ACTOR_COLLISION_PADDING,
            y: other.y,
            width: size.width,
            height: size.height,
        };
        let below = StageRect {
            x: other.x,
            y: other.y + other.height + ACTOR_COLLISION_PADDING,
            width: size.width,
            height: size.height,
        };
        let above = StageRect {
            x: other.x,
            y: other.y - size.height - ACTOR_COLLISION_PADDING,
            width: size.width,
            height: size.height,
        };
        candidates.extend(
            [right, left, below, above]
                .into_iter()
                .map(|candidate| clamp_rect_to_bounds(candidate, monitor, ACTOR_WINDOW_MARGIN)),
        );
    }
    candidates
}

fn clamp_rect_to_bounds(rect: StageRect, bounds: &StageRect, margin: f64) -> StageRect {
    let max_x = (bounds.x + bounds.width - rect.width - margin).max(bounds.x + margin);
    let max_y = (bounds.y + bounds.height - rect.height - margin).max(bounds.y + margin);
    StageRect {
        x: rect.x.clamp(bounds.x + margin, max_x),
        y: rect.y.clamp(bounds.y + margin, max_y),
        width: rect.width,
        height: rect.height,
    }
}

fn overlaps_any(rect: &StageRect, others: &[StageRect]) -> bool {
    others
        .iter()
        .any(|other| rects_overlap(rect, other, ACTOR_COLLISION_PADDING))
}

fn rect_overlap_area(a: &StageRect, b: &StageRect) -> f64 {
    let width = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let height = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    width.max(0.0) * height.max(0.0)
}

fn rects_overlap(a: &StageRect, b: &StageRect, padding: f64) -> bool {
    a.x < b.x + b.width + padding
        && a.x + a.width + padding > b.x
        && a.y < b.y + b.height + padding
        && a.y + a.height + padding > b.y
}

fn center_distance_squared(a: &StageRect, b: &StageRect) -> f64 {
    let ax = a.x + a.width * 0.5;
    let ay = a.y + a.height * 0.5;
    let bx = b.x + b.width * 0.5;
    let by = b.y + b.height * 0.5;
    (ax - bx).powi(2) + (ay - by).powi(2)
}

fn same_position(a: &StageRect, b: &StageRect) -> bool {
    (a.x - b.x).abs() <= 0.5 && (a.y - b.y).abs() <= 0.5
}

fn same_size(a: &StageRect, b: &StageRect) -> bool {
    (a.width - b.width).abs() <= 0.5 && (a.height - b.height).abs() <= 0.5
}

fn actor_window_size(percent: u16) -> ActorWindowSize {
    let scale = f64::from(clamp_actor_scale_percent(percent)) / 100.0;
    ActorWindowSize {
        width: ACTOR_WINDOW_WIDTH * scale,
        height: ACTOR_WINDOW_HEIGHT * scale,
    }
}

fn resize_actor_bounds_from_bottom_center(
    bounds: &StageRect,
    actor_size: ActorWindowSize,
) -> StageRect {
    let bottom_center_x = bounds.x + bounds.width * 0.5;
    let bottom_y = bounds.y + bounds.height;
    StageRect {
        x: bottom_center_x - actor_size.width * 0.5,
        y: bottom_y - actor_size.height,
        width: actor_size.width,
        height: actor_size.height,
    }
}

fn bounds_from_foot_anchor(anchor: StageFootAnchor, actor_size: ActorWindowSize) -> StageRect {
    StageRect {
        x: anchor.x - actor_size.width * 0.5,
        y: anchor.y - actor_size.height,
        width: actor_size.width,
        height: actor_size.height,
    }
}

fn foot_anchor(bounds: &StageRect) -> StageFootAnchor {
    StageFootAnchor {
        x: bounds.x + bounds.width * 0.5,
        y: bounds.y + bounds.height,
    }
}

fn update_persisted_anchors_after_scale(state: &mut DesktopStageState) {
    for (actor_id, actor) in &state.actors {
        if !state.perches.contains_key(actor_id) {
            state
                .persisted_anchors
                .insert(actor_id.clone(), foot_anchor(&actor.bounds));
        }
    }
}

fn default_actor_anchor(bounds: &StageRect) -> StageAnchor {
    StageAnchor {
        x: bounds.x + bounds.width * 0.5,
        y: bounds.y + bounds.height * 0.28,
        visible: true,
    }
}

fn command_actor_id(command: &RuntimeCommand) -> Option<String> {
    command
        .target
        .as_ref()
        .and_then(|target| target.actor_id.clone())
        .or_else(|| {
            command
                .payload
                .get("speakerId")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn actor_window_url(actor_id: &str) -> WebviewUrl {
    WebviewUrl::App(format!("index.html?actorId={}", encode_path_segment(actor_id)).into())
}

fn stage_overlay_url(monitor_id: &str) -> WebviewUrl {
    WebviewUrl::App(format!("index.html?stageOverlayId={monitor_id}").into())
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

fn usable_scale_factor(scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn to_message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_window_labels_hex_encode_actor_ids() {
        assert_eq!(actor_window_label("yuukei"), "actor-7975756b6569");
        assert_eq!(actor_window_label("partner"), "actor-706172746e6572");
        assert_eq!(
            actor_window_label("actor/with/slash"),
            "actor-6163746f722f776974682f736c617368"
        );
    }

    #[test]
    fn actor_window_specs_include_only_renderable_actors() {
        let catalog = test_catalog(vec![
            test_actor("yuukei", true),
            test_actor("headless", false),
            test_actor("partner", true),
        ]);

        let specs = actor_window_specs(&catalog);

        assert_eq!(
            specs
                .iter()
                .map(|spec| (&spec.actor_id, spec.index))
                .collect::<Vec<_>>(),
            vec![(&"yuukei".to_string(), 0), (&"partner".to_string(), 1)]
        );
    }

    #[test]
    fn actor_window_reconcile_closes_stale_and_creates_missing_windows() {
        let catalog = test_catalog(vec![
            test_actor("yuukei", true),
            test_actor("partner", true),
        ]);

        let reconcile = reconcile_actor_windows(
            vec![
                "settings".to_string(),
                actor_window_label("yuukei"),
                actor_window_label("old"),
            ],
            &catalog,
        );

        assert_eq!(reconcile.close_labels, vec![actor_window_label("old")]);
        assert_eq!(
            reconcile
                .create_specs
                .iter()
                .map(|spec| spec.actor_id.as_str())
                .collect::<Vec<_>>(),
            vec!["partner"]
        );
    }

    #[test]
    fn actor_placement_avoids_existing_bounds_when_space_allows() {
        let monitors = vec![StageMonitor {
            id: "monitor-0".to_string(),
            label: stage_overlay_window_label(0),
            name: None,
            bounds: StageRect {
                x: 0.0,
                y: 0.0,
                width: 1000.0,
                height: 700.0,
            },
            scale_factor: 1.0,
        }];
        let size = actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT);
        let first = place_actor_window(0, &monitors, &[], size);
        let second = place_actor_window(1, &monitors, std::slice::from_ref(&first), size);

        assert!(!rects_overlap(&first, &second, 16.0));
    }

    #[test]
    fn actor_layout_resolves_overlapping_current_bounds() {
        let monitors = vec![test_monitor(1000.0, 700.0)];
        let specs = test_specs(&["yuukei", "partner"]);
        let mut current_bounds = BTreeMap::new();
        current_bounds.insert(
            "yuukei".to_string(),
            StageRect {
                x: 80.0,
                y: 80.0,
                width: ACTOR_WINDOW_WIDTH,
                height: ACTOR_WINDOW_HEIGHT,
            },
        );
        current_bounds.insert(
            "partner".to_string(),
            StageRect {
                x: 90.0,
                y: 90.0,
                width: ACTOR_WINDOW_WIDTH,
                height: ACTOR_WINDOW_HEIGHT,
            },
        );

        let resolved = resolve_actor_window_layout(
            &specs,
            &current_bounds,
            &BTreeMap::new(),
            &monitors,
            actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT),
        );
        let first = resolved.get("yuukei").expect("yuukei bounds");
        let second = resolved.get("partner").expect("partner bounds");

        assert!(!rects_overlap(first, second, ACTOR_COLLISION_PADDING));
    }

    #[test]
    fn actor_layout_spreads_three_or_four_actors_when_space_allows() {
        let monitors = vec![test_monitor(1900.0, 700.0)];
        let specs = test_specs(&["yuukei", "partner", "third", "fourth"]);
        let mut current_bounds = BTreeMap::new();
        for spec in &specs {
            current_bounds.insert(
                spec.actor_id.clone(),
                StageRect {
                    x: 64.0,
                    y: 64.0,
                    width: ACTOR_WINDOW_WIDTH,
                    height: ACTOR_WINDOW_HEIGHT,
                },
            );
        }

        let resolved = resolve_actor_window_layout(
            &specs,
            &current_bounds,
            &BTreeMap::new(),
            &monitors,
            actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT),
        );
        let bounds = specs
            .iter()
            .map(|spec| resolved.get(&spec.actor_id).expect("actor bounds"))
            .collect::<Vec<_>>();

        for (index, first) in bounds.iter().enumerate() {
            for second in bounds.iter().skip(index + 1) {
                assert!(!rects_overlap(first, second, ACTOR_COLLISION_PADDING));
            }
        }
    }

    #[test]
    fn actor_layout_clamps_current_bounds_inside_monitor() {
        let monitor = test_monitor(1000.0, 700.0);
        let specs = test_specs(&["yuukei"]);
        let mut current_bounds = BTreeMap::new();
        current_bounds.insert(
            "yuukei".to_string(),
            StageRect {
                x: 920.0,
                y: 620.0,
                width: ACTOR_WINDOW_WIDTH,
                height: ACTOR_WINDOW_HEIGHT,
            },
        );

        let resolved = resolve_actor_window_layout(
            &specs,
            &current_bounds,
            &BTreeMap::new(),
            std::slice::from_ref(&monitor),
            actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT),
        );
        let bounds = resolved.get("yuukei").expect("yuukei bounds");

        assert!(bounds.x >= monitor.bounds.x + ACTOR_WINDOW_MARGIN);
        assert!(bounds.y >= monitor.bounds.y + ACTOR_WINDOW_MARGIN);
        assert!(
            bounds.x + bounds.width
                <= monitor.bounds.x + monitor.bounds.width - ACTOR_WINDOW_MARGIN + 0.5
        );
        assert!(
            bounds.y + bounds.height
                <= monitor.bounds.y + monitor.bounds.height - ACTOR_WINDOW_MARGIN + 0.5
        );
    }

    #[test]
    fn actor_layout_restores_persisted_foot_anchor_with_current_scale_and_clamps_it() {
        let monitor = test_monitor(1000.0, 700.0);
        let specs = test_specs(&["yuukei"]);
        let persisted = BTreeMap::from([(
            "yuukei".to_string(),
            StageFootAnchor {
                x: 4_000.0,
                y: 3_000.0,
            },
        )]);

        let resolved = resolve_actor_window_layout(
            &specs,
            &BTreeMap::new(),
            &persisted,
            std::slice::from_ref(&monitor),
            actor_window_size(50),
        );
        let bounds = resolved.get("yuukei").expect("restored bounds");

        assert_eq!(bounds.width, ACTOR_WINDOW_WIDTH * 0.5);
        assert_eq!(bounds.height, ACTOR_WINDOW_HEIGHT * 0.5);
        assert!(bounds.x + bounds.width <= monitor.bounds.width - ACTOR_WINDOW_MARGIN + 0.5);
        assert!(bounds.y + bounds.height <= monitor.bounds.height - ACTOR_WINDOW_MARGIN + 0.5);
    }

    #[test]
    fn actor_layout_avoids_collisions_between_persisted_anchors() {
        let specs = test_specs(&["yuukei", "partner"]);
        let persisted = BTreeMap::from([
            ("yuukei".to_string(), StageFootAnchor { x: 500.0, y: 680.0 }),
            (
                "partner".to_string(),
                StageFootAnchor { x: 500.0, y: 680.0 },
            ),
        ]);
        let resolved = resolve_actor_window_layout(
            &specs,
            &BTreeMap::new(),
            &persisted,
            &[test_monitor(1400.0, 800.0)],
            actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT),
        );

        assert!(!rects_overlap(
            resolved.get("yuukei").expect("yuukei bounds"),
            resolved.get("partner").expect("partner bounds"),
            ACTOR_COLLISION_PADDING,
        ));
    }

    #[test]
    fn actor_layout_returns_best_effort_when_space_is_tight() {
        let monitors = vec![test_monitor(460.0, 600.0)];
        let specs = test_specs(&["yuukei", "partner", "third"]);

        let resolved = resolve_actor_window_layout(
            &specs,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &monitors,
            actor_window_size(DEFAULT_ACTOR_SCALE_PERCENT),
        );

        assert_eq!(resolved.len(), 3);
        for bounds in resolved.values() {
            assert!(bounds.x.is_finite());
            assert!(bounds.y.is_finite());
            assert_eq!(bounds.width, ACTOR_WINDOW_WIDTH);
            assert_eq!(bounds.height, ACTOR_WINDOW_HEIGHT);
        }
    }

    #[test]
    fn perch_actor_bounds_centers_on_top_edge_and_clamps_to_monitor() {
        let monitors = vec![test_monitor(1000.0, 800.0)];
        let actor = StageRect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 100.0,
        };
        let target = yuukei_device_host::DesktopWindowFrame {
            x: 300.0,
            y: 300.0,
            width: 400.0,
            height: 300.0,
        };

        let perched = perch_actor_bounds(&actor, &target, &monitors);

        assert_eq!(
            perched,
            StageRect {
                x: 400.0,
                y: 200.0,
                width: 200.0,
                height: 100.0,
            }
        );

        let near_edge = yuukei_device_host::DesktopWindowFrame {
            x: 20.0,
            y: 40.0,
            width: 80.0,
            height: 200.0,
        };
        let clamped = perch_actor_bounds(&actor, &near_edge, &monitors);
        assert_eq!(clamped.x, 0.0);
        assert_eq!(clamped.y, 0.0);
    }

    #[test]
    fn window_terrain_loss_restores_actor_and_reports_perch_ended() {
        let spec = test_specs(&["yuukei"]).remove(0);
        let mut state = DesktopStageState {
            monitors: vec![test_monitor(1000.0, 800.0)],
            actors: BTreeMap::from([(
                "yuukei".to_string(),
                actor_from_spec(
                    &spec,
                    StageRect {
                        x: 400.0,
                        y: 200.0,
                        width: 200.0,
                        height: 100.0,
                    },
                    true,
                ),
            )]),
            bubbles: BTreeMap::new(),
            perches: BTreeMap::from([(
                "yuukei".to_string(),
                StagePerch {
                    window_key: "window-1".to_string(),
                },
            )]),
            terrain_windows: BTreeMap::new(),
            persisted_anchors: BTreeMap::new(),
            active_drags: BTreeMap::new(),
            actor_scale_percent: DEFAULT_ACTOR_SCALE_PERCENT,
            window_observation_enabled: true,
        };

        let (apply_bounds, ended) = apply_window_terrain_to_state(&mut state, &[]);

        assert_eq!(
            ended,
            vec![StagePerchEnded {
                actor_id: "yuukei".to_string(),
                window_key: "window-1".to_string(),
                reason: "window-closed",
            }]
        );
        assert!(state.perches.is_empty());
        assert_eq!(apply_bounds.len(), 1);
        assert_eq!(
            state.actors.get("yuukei").expect("actor").bounds.width,
            ACTOR_WINDOW_WIDTH
        );
    }

    #[test]
    fn beginning_user_drag_releases_perch_with_user_drag_reason() {
        let spec = test_specs(&["yuukei"]).remove(0);
        let bounds = StageRect {
            x: 120.0,
            y: 80.0,
            width: ACTOR_WINDOW_WIDTH,
            height: ACTOR_WINDOW_HEIGHT,
        };
        let mut state = DesktopStageState {
            actors: BTreeMap::from([(
                "yuukei".to_string(),
                actor_from_spec(&spec, bounds.clone(), true),
            )]),
            perches: BTreeMap::from([(
                "yuukei".to_string(),
                StagePerch {
                    window_key: "window-1".to_string(),
                },
            )]),
            ..DesktopStageState::default()
        };

        let ended = begin_actor_drag_in_state(&mut state, "yuukei", bounds)
            .expect("begin actor drag")
            .expect("perch ended");

        assert_eq!(ended.reason, "user-drag");
        assert_eq!(ended.window_key, "window-1");
        assert!(state.perches.is_empty());
        assert_eq!(
            state.active_drags.get("yuukei"),
            Some(&ActiveActorDrag {
                start_bounds: StageRect {
                    x: 120.0,
                    y: 80.0,
                    width: ACTOR_WINDOW_WIDTH,
                    height: ACTOR_WINDOW_HEIGHT,
                },
            })
        );
    }

    #[test]
    fn moving_actor_drag_uses_start_bounds_without_clamping_or_persisting() {
        let spec = test_specs(&["yuukei"]).remove(0);
        let start_bounds = StageRect {
            x: 120.0,
            y: 80.0,
            width: ACTOR_WINDOW_WIDTH,
            height: ACTOR_WINDOW_HEIGHT,
        };
        let mut state = DesktopStageState {
            monitors: vec![test_monitor(1000.0, 700.0)],
            actors: BTreeMap::from([(
                "yuukei".to_string(),
                actor_from_spec(&spec, start_bounds.clone(), true),
            )]),
            persisted_anchors: BTreeMap::from([(
                "yuukei".to_string(),
                StageFootAnchor { x: 330.0, y: 640.0 },
            )]),
            ..DesktopStageState::default()
        };
        begin_actor_drag_in_state(&mut state, "yuukei", start_bounds).expect("begin actor drag");

        let moved = move_actor_drag_in_state(&mut state, "yuukei", 2_000.0, -300.0)
            .expect("move actor drag");

        assert_eq!(moved.x, 2_120.0);
        assert_eq!(moved.y, -220.0);
        assert_eq!(
            state.persisted_anchors.get("yuukei"),
            Some(&StageFootAnchor { x: 330.0, y: 640.0 })
        );
    }

    #[test]
    fn actor_scale_recomputes_perched_actor_with_scaled_size() {
        let spec = test_specs(&["yuukei"]).remove(0);
        let target = DesktopWindowFrame {
            x: 300.0,
            y: 900.0,
            width: 400.0,
            height: 300.0,
        };
        let mut state = DesktopStageState {
            monitors: vec![test_monitor(1400.0, 1200.0)],
            actors: BTreeMap::from([(
                "yuukei".to_string(),
                actor_from_spec(
                    &spec,
                    StageRect {
                        x: 390.0,
                        y: 160.0,
                        width: ACTOR_WINDOW_WIDTH,
                        height: ACTOR_WINDOW_HEIGHT,
                    },
                    true,
                ),
            )]),
            bubbles: BTreeMap::new(),
            perches: BTreeMap::from([(
                "yuukei".to_string(),
                StagePerch {
                    window_key: "window-1".to_string(),
                },
            )]),
            terrain_windows: BTreeMap::from([("window-1".to_string(), target)]),
            persisted_anchors: BTreeMap::new(),
            active_drags: BTreeMap::new(),
            actor_scale_percent: DEFAULT_ACTOR_SCALE_PERCENT,
            window_observation_enabled: true,
        };

        let apply_bounds = apply_actor_scale_to_state(&mut state, 150);
        let bounds = state.actors.get("yuukei").expect("actor").bounds.clone();

        assert_eq!(apply_bounds.len(), 1);
        assert_eq!(bounds.width, ACTOR_WINDOW_WIDTH * 1.5);
        assert_eq!(bounds.height, ACTOR_WINDOW_HEIGHT * 1.5);
        assert_eq!(bounds.x, 300.0 + 200.0 - bounds.width / 2.0);
        assert_eq!(bounds.y, 900.0 - bounds.height);
    }

    #[test]
    fn actor_visibility_updates_matching_stage_actor() {
        let manager = DesktopStageManager::new();
        let spec = test_specs(&["yuukei"]).remove(0);
        {
            let mut state = manager.state.write().expect("stage lock");
            state.actors.insert(
                spec.actor_id.clone(),
                actor_from_spec(
                    &spec,
                    StageRect {
                        x: 64.0,
                        y: 64.0,
                        width: ACTOR_WINDOW_WIDTH,
                        height: ACTOR_WINDOW_HEIGHT,
                    },
                    true,
                ),
            );
        }

        assert!(manager
            .set_actor_visibility_for_window(&spec.label, false)
            .expect("hide visibility"));
        assert!(
            !manager
                .snapshot()
                .expect("snapshot")
                .actors
                .first()
                .expect("actor")
                .visible
        );
        assert!(manager
            .set_actor_visibility_for_window(&spec.label, true)
            .expect("show visibility"));
        assert!(
            manager
                .snapshot()
                .expect("snapshot")
                .actors
                .first()
                .expect("actor")
                .visible
        );
    }

    #[test]
    fn command_actor_id_prefers_explicit_target() {
        let mut command = RuntimeCommand::new("dialogue.say", "daihon", "resident-default");
        command.target = Some(yuukei_protocol::CommandTarget {
            device_id: None,
            surface_id: None,
            actor_id: Some("targeted".to_string()),
        });
        command.payload.insert(
            "speakerId".to_string(),
            Value::String("payload".to_string()),
        );

        assert_eq!(command_actor_id(&command).as_deref(), Some("targeted"));
    }

    fn test_catalog(
        actors: Vec<crate::DesktopActorSurfaceAsset>,
    ) -> DesktopActorSurfaceAssetCatalog {
        DesktopActorSurfaceAssetCatalog {
            world_pack_id: "world-test".to_string(),
            actors,
        }
    }

    fn test_actor(actor_id: &str, renderable: bool) -> crate::DesktopActorSurfaceAsset {
        crate::DesktopActorSurfaceAsset {
            actor_id: actor_id.to_string(),
            display_name: actor_id.to_string(),
            renderer: renderable.then(|| crate::DesktopActorSurfaceRendererAsset {
                kind: "vrm",
                model_url: format!("yuukei-pack://localhost/actors/{actor_id}/model"),
                motions: Default::default(),
                hit_zones: Vec::new(),
            }),
        }
    }

    fn test_monitor(width: f64, height: f64) -> StageMonitor {
        StageMonitor {
            id: "monitor-0".to_string(),
            label: stage_overlay_window_label(0),
            name: None,
            bounds: StageRect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
            scale_factor: 1.0,
        }
    }

    fn test_specs(actor_ids: &[&str]) -> Vec<ActorWindowSpec> {
        actor_ids
            .iter()
            .enumerate()
            .map(|(index, actor_id)| ActorWindowSpec {
                actor_id: (*actor_id).to_string(),
                display_name: (*actor_id).to_string(),
                label: actor_window_label(actor_id),
                index,
            })
            .collect()
    }
}
