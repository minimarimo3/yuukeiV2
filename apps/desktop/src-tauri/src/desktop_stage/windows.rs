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

pub(super) fn create_stage_overlay_window(
    app: &AppHandle,
    monitor: &StageMonitor,
) -> Result<(), String> {
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

pub(super) fn create_actor_window(
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
use super::*;
use tauri::WebviewWindowBuilder;
