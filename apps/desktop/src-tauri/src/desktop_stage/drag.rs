pub(super) fn begin_actor_drag_in_state(
    state: &mut DesktopStageState,
    actor_id: &str,
    session_id: &str,
    bounds: StageRect,
) -> Result<(Option<StagePerchEnded>, Option<String>), String> {
    let actor = state
        .actors
        .get_mut(actor_id)
        .ok_or_else(|| format!("unknown stage actor: {actor_id}"))?;
    actor.bounds = bounds.clone();
    actor.anchor = default_actor_anchor(&bounds);
    state.active_drags.insert(
        actor_id.to_string(),
        ActiveActorDrag {
            session_id: session_id.to_string(),
            start_bounds: bounds,
        },
    );
    let perch_ended = state.perches.remove(actor_id).map(|perch| StagePerchEnded {
        actor_id: actor_id.to_string(),
        window_key: perch.window_key,
        reason: "user-drag",
    });
    let cancelled_walk_id = cancel_actor_walk_in_state(state, actor_id);
    Ok((perch_ended, cancelled_walk_id))
}

pub(super) fn move_actor_drag_in_state(
    state: &mut DesktopStageState,
    actor_id: &str,
    session_id: &str,
    dx: f64,
    dy: f64,
) -> Result<StageRect, String> {
    let active = state
        .active_drags
        .get(actor_id)
        .ok_or_else(|| format!("actor drag was not active: {actor_id}"))?;
    if active.session_id != session_id {
        return Err(format!("stale actor drag session: {actor_id}"));
    }
    let start_bounds = active.start_bounds.clone();
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

pub(super) fn take_actor_drag_in_state(
    state: &mut DesktopStageState,
    actor_id: &str,
    session_id: &str,
) -> Result<ActiveActorDrag, String> {
    let active = state
        .active_drags
        .get(actor_id)
        .ok_or_else(|| format!("actor drag was not active: {actor_id}"))?;
    if active.session_id != session_id {
        return Err(format!("stale actor drag session: {actor_id}"));
    }
    state
        .active_drags
        .remove(actor_id)
        .ok_or_else(|| format!("actor drag was not active: {actor_id}"))
}
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ActiveActorDrag {
    pub(super) session_id: String,
    pub(super) start_bounds: StageRect,
}
