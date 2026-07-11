pub(super) fn reconcile_actor_windows(
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

pub(super) fn actor_window_specs(catalog: &DesktopActorSurfaceAssetCatalog) -> Vec<ActorWindowSpec> {
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

pub(super) fn actor_from_spec(spec: &ActorWindowSpec, bounds: StageRect, visible: bool) -> StageActor {
    StageActor {
        actor_id: spec.actor_id.clone(),
        display_name: spec.display_name.clone(),
        window_label: spec.label.clone(),
        anchor: default_actor_anchor(&bounds),
        bounds,
        visible,
    }
}

pub(super) fn window_bounds(window: &WebviewWindow) -> Result<StageRect, String> {
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

pub(super) fn apply_actor_window_bounds(window: &WebviewWindow, bounds: &StageRect) -> Result<(), String> {
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

pub(super) fn resolve_actor_window_layout(
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

pub(super) fn normalize_actor_window_bounds(
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

pub(super) fn perch_actor_bounds(
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

pub(super) fn apply_window_terrain_to_state(
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

pub(super) fn apply_actor_scale_to_state(
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

pub(super) fn reapply_perches_to_state(
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

pub(super) fn restore_actor_to_desktop(
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

pub(super) fn best_monitor_bounds_for_rect(rect: &StageRect, monitors: &[StageMonitor]) -> StageRect {
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

pub(super) fn place_actor_window(
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

pub(super) fn actor_grid_candidates(monitor: &StageRect, size: &StageRect) -> Vec<StageRect> {
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

pub(super) fn actor_collision_candidates(
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
use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActorWindowReconcile {
    pub(super) close_labels: Vec<String>,
    pub(super) create_specs: Vec<ActorWindowSpec>,
    pub(super) desired_specs: Vec<ActorWindowSpec>,
}
use yuukei_device_host::clamp_actor_scale_percent;
