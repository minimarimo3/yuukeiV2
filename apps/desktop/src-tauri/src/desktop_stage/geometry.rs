pub(super) fn clamp_rect_to_bounds(rect: StageRect, bounds: &StageRect, margin: f64) -> StageRect {
    let max_x = (bounds.x + bounds.width - rect.width - margin).max(bounds.x + margin);
    let max_y = (bounds.y + bounds.height - rect.height - margin).max(bounds.y + margin);
    StageRect {
        x: rect.x.clamp(bounds.x + margin, max_x),
        y: rect.y.clamp(bounds.y + margin, max_y),
        width: rect.width,
        height: rect.height,
    }
}

pub(super) fn overlaps_any(rect: &StageRect, others: &[StageRect]) -> bool {
    others
        .iter()
        .any(|other| rects_overlap(rect, other, ACTOR_COLLISION_PADDING))
}

pub(super) fn rect_overlap_area(a: &StageRect, b: &StageRect) -> f64 {
    let width = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let height = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    width.max(0.0) * height.max(0.0)
}

pub(super) fn rects_overlap(a: &StageRect, b: &StageRect, padding: f64) -> bool {
    a.x < b.x + b.width + padding
        && a.x + a.width + padding > b.x
        && a.y < b.y + b.height + padding
        && a.y + a.height + padding > b.y
}

pub(super) fn center_distance_squared(a: &StageRect, b: &StageRect) -> f64 {
    let ax = a.x + a.width * 0.5;
    let ay = a.y + a.height * 0.5;
    let bx = b.x + b.width * 0.5;
    let by = b.y + b.height * 0.5;
    (ax - bx).powi(2) + (ay - by).powi(2)
}

pub(super) fn same_position(a: &StageRect, b: &StageRect) -> bool {
    (a.x - b.x).abs() <= 0.5 && (a.y - b.y).abs() <= 0.5
}

pub(super) fn same_size(a: &StageRect, b: &StageRect) -> bool {
    (a.width - b.width).abs() <= 0.5 && (a.height - b.height).abs() <= 0.5
}

pub(super) fn actor_window_size(percent: u16) -> ActorWindowSize {
    let scale = f64::from(clamp_actor_scale_percent(percent)) / 100.0;
    ActorWindowSize {
        width: ACTOR_WINDOW_WIDTH * scale,
        height: ACTOR_WINDOW_HEIGHT * scale,
    }
}

pub(super) fn resize_actor_bounds_from_bottom_center(
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

pub(super) fn bounds_from_foot_anchor(anchor: StageFootAnchor, actor_size: ActorWindowSize) -> StageRect {
    StageRect {
        x: anchor.x - actor_size.width * 0.5,
        y: anchor.y - actor_size.height,
        width: actor_size.width,
        height: actor_size.height,
    }
}

pub(super) fn foot_anchor(bounds: &StageRect) -> StageFootAnchor {
    StageFootAnchor {
        x: bounds.x + bounds.width * 0.5,
        y: bounds.y + bounds.height,
    }
}

pub(super) fn update_persisted_anchors_after_scale(state: &mut DesktopStageState) {
    for (actor_id, actor) in &state.actors {
        if !state.perches.contains_key(actor_id) {
            state
                .persisted_anchors
                .insert(actor_id.clone(), foot_anchor(&actor.bounds));
        }
    }
}

pub(super) fn default_actor_anchor(bounds: &StageRect) -> StageAnchor {
    StageAnchor {
        x: bounds.x + bounds.width * 0.5,
        y: bounds.y + bounds.height * 0.28,
        visible: true,
    }
}

pub(super) fn command_actor_id(command: &RuntimeCommand) -> Option<String> {
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

pub(super) fn actor_window_url(actor_id: &str) -> WebviewUrl {
    WebviewUrl::App(format!("index.html?actorId={}", encode_path_segment(actor_id)).into())
}

pub(super) fn stage_overlay_url(monitor_id: &str) -> WebviewUrl {
    WebviewUrl::App(format!("index.html?stageOverlayId={monitor_id}").into())
}

pub(super) fn encode_path_segment(value: &str) -> String {
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

pub(super) fn usable_scale_factor(scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub(super) fn to_message(error: impl std::fmt::Display) -> String {
    error.to_string()
}
use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ActorWindowSize {
    pub(super) width: f64,
    pub(super) height: f64,
}
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::WebviewUrl;
use yuukei_device_host::clamp_actor_scale_percent;
