use super::*;
use tauri::WebviewWindowBuilder;

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

// Auto-hidden Windows taskbars keep a two-physical-pixel reveal strip inside
// the monitor even though the reported work area spans the whole monitor.
const AUTO_HIDE_TASKBAR_EDGE_RESERVE_PX: u32 = 2;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct AutoHideTaskbarEdges {
    pub(super) left: bool,
    pub(super) top: bool,
    pub(super) right: bool,
    pub(super) bottom: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PhysicalStageBounds {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: u32,
    pub(super) height: u32,
}

impl PhysicalStageBounds {
    fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub(super) fn right(self) -> i32 {
        (i64::from(self.x) + i64::from(self.width)).clamp(i64::from(i32::MIN), i64::from(i32::MAX))
            as i32
    }

    pub(super) fn bottom(self) -> i32 {
        (i64::from(self.y) + i64::from(self.height)).clamp(i64::from(i32::MIN), i64::from(i32::MAX))
            as i32
    }
}

fn reserve_auto_hide_taskbar_edges(
    mut work_area: PhysicalStageBounds,
    monitor: PhysicalStageBounds,
    edges: AutoHideTaskbarEdges,
) -> PhysicalStageBounds {
    let touches_left = work_area.x == monitor.x;
    let touches_top = work_area.y == monitor.y;
    let touches_right = work_area.right() == monitor.right();
    let touches_bottom = work_area.bottom() == monitor.bottom();

    if edges.left && touches_left {
        let reserve = AUTO_HIDE_TASKBAR_EDGE_RESERVE_PX.min(work_area.width.saturating_sub(1));
        work_area.x = work_area.x.saturating_add(reserve as i32);
        work_area.width -= reserve;
    }
    if edges.top && touches_top {
        let reserve = AUTO_HIDE_TASKBAR_EDGE_RESERVE_PX.min(work_area.height.saturating_sub(1));
        work_area.y = work_area.y.saturating_add(reserve as i32);
        work_area.height -= reserve;
    }
    if edges.right && touches_right {
        let reserve = AUTO_HIDE_TASKBAR_EDGE_RESERVE_PX.min(work_area.width.saturating_sub(1));
        work_area.width -= reserve;
    }
    if edges.bottom && touches_bottom {
        let reserve = AUTO_HIDE_TASKBAR_EDGE_RESERVE_PX.min(work_area.height.saturating_sub(1));
        work_area.height -= reserve;
    }

    work_area
}

fn auto_hide_taskbar_edges(monitor: PhysicalStageBounds) -> AutoHideTaskbarEdges {
    #[cfg(windows)]
    {
        windows_caption::auto_hide_taskbar_edges(monitor)
    }
    #[cfg(not(windows))]
    {
        let _ = monitor;
        AutoHideTaskbarEdges::default()
    }
}

pub(super) fn monitor_snapshots(app: &AppHandle) -> Result<Vec<StageMonitor>, String> {
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
            let monitor_position = monitor.position();
            let monitor_size = monitor.size();
            let physical_monitor = PhysicalStageBounds::new(
                monitor_position.x,
                monitor_position.y,
                monitor_size.width,
                monitor_size.height,
            );
            // Leave the Shell-owned reveal strip outside every topmost stage
            // window. Click-through alone does not remove an HWND from AppBar
            // edge handling while another application is fullscreen.
            let physical_work_area = reserve_auto_hide_taskbar_edges(
                PhysicalStageBounds::new(
                    work_area.position.x,
                    work_area.position.y,
                    work_area.size.width,
                    work_area.size.height,
                ),
                physical_monitor,
                auto_hide_taskbar_edges(physical_monitor),
            );
            StageMonitor {
                id: format!("monitor-{index}"),
                label: stage_overlay_window_label(index),
                name: monitor.name().cloned(),
                bounds: StageRect {
                    x: physical_work_area.x as f64 / scale_factor,
                    y: physical_work_area.y as f64 / scale_factor,
                    width: physical_work_area.width as f64 / scale_factor,
                    height: physical_work_area.height as f64 / scale_factor,
                },
                scale_factor,
            }
        })
        .collect())
}

pub(super) fn create_stage_overlay_window(
    app: &AppHandle,
    monitor: &StageMonitor,
) -> Result<(), String> {
    crate::track_surface_window_loading(app, &monitor.label)?;
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
        .visible(false)
        .build()
        .map_err(to_message)?;
    enforce_borderless(&window);
    window.set_ignore_cursor_events(true).map_err(to_message)?;
    Ok(())
}

pub(super) fn create_actor_window(
    app: &AppHandle,
    spec: &ActorWindowSpec,
    bounds: &StageRect,
    _visible: bool,
) -> Result<(), String> {
    crate::track_surface_window_loading(app, &spec.label)?;
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
        .visible(false)
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
/// New actor and overlay windows are built hidden, passed through this function,
/// and only then shown. This prevents their initial decorated native frame from
/// becoming visible before the Windows-specific style fix is installed.
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

#[cfg(test)]
mod tests {
    use super::{
        reserve_auto_hide_taskbar_edges, AutoHideTaskbarEdges, PhysicalStageBounds,
        AUTO_HIDE_TASKBAR_EDGE_RESERVE_PX,
    };

    #[test]
    fn reserves_bottom_edge_when_auto_hide_taskbar_uses_full_work_area() {
        let monitor = PhysicalStageBounds::new(0, 0, 1920, 1080);
        let work_area = monitor;

        let reserved = reserve_auto_hide_taskbar_edges(
            work_area,
            monitor,
            AutoHideTaskbarEdges {
                bottom: true,
                ..AutoHideTaskbarEdges::default()
            },
        );

        assert_eq!(reserved.x, 0);
        assert_eq!(reserved.y, 0);
        assert_eq!(reserved.width, 1920);
        assert_eq!(reserved.height, 1080 - AUTO_HIDE_TASKBAR_EDGE_RESERVE_PX);
    }

    #[test]
    fn does_not_double_reserve_an_edge_already_excluded_from_work_area() {
        let monitor = PhysicalStageBounds::new(0, 0, 1920, 1080);
        let work_area = PhysicalStageBounds::new(0, 0, 1920, 1078);

        let reserved = reserve_auto_hide_taskbar_edges(
            work_area,
            monitor,
            AutoHideTaskbarEdges {
                bottom: true,
                ..AutoHideTaskbarEdges::default()
            },
        );

        assert_eq!(reserved, work_area);
    }

    #[test]
    fn reserves_each_reported_edge_without_moving_unrelated_edges() {
        let monitor = PhysicalStageBounds::new(-1920, 6, 1920, 1080);
        let work_area = monitor;

        let reserved = reserve_auto_hide_taskbar_edges(
            work_area,
            monitor,
            AutoHideTaskbarEdges {
                left: true,
                top: true,
                right: true,
                bottom: true,
            },
        );

        assert_eq!(reserved.x, -1920 + AUTO_HIDE_TASKBAR_EDGE_RESERVE_PX as i32);
        assert_eq!(reserved.y, 6 + AUTO_HIDE_TASKBAR_EDGE_RESERVE_PX as i32);
        assert_eq!(reserved.width, 1920 - AUTO_HIDE_TASKBAR_EDGE_RESERVE_PX * 2);
        assert_eq!(
            reserved.height,
            1080 - AUTO_HIDE_TASKBAR_EDGE_RESERVE_PX * 2
        );
    }
}
