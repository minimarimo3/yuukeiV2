use super::*;

pub(super) const DEFAULT_WALK_SPEED_PX_PER_SEC: f64 = 240.0;
const MIN_WALK_SPEED_PX_PER_SEC: f64 = 60.0;
const MAX_WALK_SPEED_PX_PER_SEC: f64 = 960.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WalkDestination {
    RightEdge,
    LeftEdge,
}

impl WalkDestination {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "right-edge" => Some(Self::RightEdge),
            "left-edge" => Some(Self::LeftEdge),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WalkStep {
    pub(crate) bounds: StageRect,
    pub(crate) arrived: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ActiveStageWalk {
    pub(super) walk_id: String,
    pub(super) target_bounds: StageRect,
    pub(super) speed_px_per_sec: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StageWalkStarted {
    pub(crate) actor_id: String,
    pub(crate) walk_id: String,
    pub(crate) window_label: String,
    pub(crate) bounds: StageRect,
    pub(crate) perch_ended: Option<StagePerchEnded>,
}

pub(super) fn clamp_walk_speed(speed_px_per_sec: f64) -> f64 {
    if speed_px_per_sec.is_finite() {
        speed_px_per_sec.clamp(MIN_WALK_SPEED_PX_PER_SEC, MAX_WALK_SPEED_PX_PER_SEC)
    } else {
        DEFAULT_WALK_SPEED_PX_PER_SEC
    }
}

pub(super) fn walk_target_bounds(
    current: &StageRect,
    monitors: &[StageMonitor],
    destination: WalkDestination,
) -> StageRect {
    let monitor = best_monitor_bounds_for_rect(current, monitors);
    let x = match destination {
        WalkDestination::RightEdge => {
            (monitor.x + monitor.width - current.width - ACTOR_WINDOW_MARGIN)
                .max(monitor.x + ACTOR_WINDOW_MARGIN)
        }
        WalkDestination::LeftEdge => monitor.x + ACTOR_WINDOW_MARGIN,
    };
    StageRect {
        x,
        ..current.clone()
    }
}

pub(super) fn advance_walk_bounds(
    current: &StageRect,
    target: &StageRect,
    speed_px_per_sec: f64,
    delta_seconds: f64,
) -> WalkStep {
    let remaining = target.x - current.x;
    let distance = clamp_walk_speed(speed_px_per_sec)
        * if delta_seconds.is_finite() {
            delta_seconds.max(0.0)
        } else {
            0.0
        };
    if remaining.abs() <= distance {
        return WalkStep {
            bounds: target.clone(),
            arrived: true,
        };
    }
    WalkStep {
        bounds: StageRect {
            x: current.x + remaining.signum() * distance,
            ..current.clone()
        },
        arrived: false,
    }
}

pub(super) fn start_actor_walk_in_state(
    state: &mut DesktopStageState,
    actor_id: &str,
    walk_id: &str,
    destination: WalkDestination,
    speed_px_per_sec: f64,
) -> Result<StageWalkStarted, String> {
    if !state.actors.contains_key(actor_id) {
        return Err(format!("unknown stage actor: {actor_id}"));
    }
    let perch_ended = state.perches.remove(actor_id).map(|perch| StagePerchEnded {
        actor_id: actor_id.to_string(),
        window_key: perch.window_key,
        reason: "walk",
    });
    if perch_ended.is_some() {
        restore_actor_to_desktop(state, actor_id);
    }
    let actor = state
        .actors
        .get(actor_id)
        .ok_or_else(|| format!("unknown stage actor: {actor_id}"))?;
    let target_bounds = walk_target_bounds(&actor.bounds, &state.monitors, destination);
    let window_label = actor.window_label.clone();
    let bounds = actor.bounds.clone();
    state.active_walks.insert(
        actor_id.to_string(),
        ActiveStageWalk {
            walk_id: walk_id.to_string(),
            target_bounds,
            speed_px_per_sec: clamp_walk_speed(speed_px_per_sec),
        },
    );
    Ok(StageWalkStarted {
        actor_id: actor_id.to_string(),
        walk_id: walk_id.to_string(),
        window_label,
        bounds,
        perch_ended,
    })
}

pub(super) fn cancel_actor_walk_in_state(
    state: &mut DesktopStageState,
    actor_id: &str,
) -> Option<String> {
    state
        .active_walks
        .remove(actor_id)
        .map(|walk| walk.walk_id)
}

pub(super) fn advance_actor_walk_in_state(
    state: &mut DesktopStageState,
    actor_id: &str,
    walk_id: &str,
    delta_seconds: f64,
) -> Option<WalkStep> {
    let active = state.active_walks.get(actor_id)?.clone();
    if active.walk_id != walk_id {
        return None;
    }
    let current = state.actors.get(actor_id)?.bounds.clone();
    let progress = advance_walk_bounds(
        &current,
        &active.target_bounds,
        active.speed_px_per_sec,
        delta_seconds,
    );
    let actor = state.actors.get_mut(actor_id)?;
    actor.bounds = progress.bounds.clone();
    actor.anchor = default_actor_anchor(&progress.bounds);
    if progress.arrived {
        state.active_walks.remove(actor_id);
        state
            .persisted_anchors
            .insert(actor_id.to_string(), foot_anchor(&progress.bounds));
    }
    Some(progress)
}
