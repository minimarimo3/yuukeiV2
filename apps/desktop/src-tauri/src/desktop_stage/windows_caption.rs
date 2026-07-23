use tauri::WebviewWindow;
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::Shell::{
    DefSubclassProc, RemoveWindowSubclass, SHAppBarMessage, SetWindowSubclass, ABE_BOTTOM,
    ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_GETAUTOHIDEBAREX, APPBARDATA,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE,
    GWL_STYLE, STYLESTRUCT, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    WINDOWPOS, WM_NCACTIVATE, WM_NCDESTROY, WM_STYLECHANGED, WM_STYLECHANGING,
    WM_WINDOWPOSCHANGING, WS_CAPTION, WS_EX_TOPMOST, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU,
    WS_THICKFRAME,
};

/// Stable id for our single caption subclass on each window.
const SUBCLASS_ID: usize = 0x594B_00AC;

/// Styles that can make Windows draw a native caption or resize frame.
const CAPTION_STYLE_MASK: u32 =
    WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0;

const TASKBAR_WINDOW_CLASSES: [&str; 2] = ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

pub(super) fn auto_hide_taskbar_edges(
    monitor: super::windows::PhysicalStageBounds,
) -> super::windows::AutoHideTaskbarEdges {
    unsafe {
        super::windows::AutoHideTaskbarEdges {
            left: has_auto_hide_appbar(monitor, ABE_LEFT),
            top: has_auto_hide_appbar(monitor, ABE_TOP),
            right: has_auto_hide_appbar(monitor, ABE_RIGHT),
            bottom: has_auto_hide_appbar(monitor, ABE_BOTTOM),
        }
    }
}

unsafe fn has_auto_hide_appbar(monitor: super::windows::PhysicalStageBounds, edge: u32) -> bool {
    let mut data = APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        uEdge: edge,
        rc: windows::Win32::Foundation::RECT {
            left: monitor.x,
            top: monitor.y,
            right: monitor.right(),
            bottom: monitor.bottom(),
        },
        ..APPBARDATA::default()
    };
    SHAppBarMessage(ABM_GETAUTOHIDEBAREX, &mut data) != 0
}

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
        place_behind_shell_taskbars(hwnd);
    });
}

/// Keep a Yuukei window in the topmost band but immediately behind Explorer's
/// taskbar on the same monitor. `EnumWindows` is in front-to-back Z order, so
/// the last matching taskbar for that monitor is the one after which the window
/// must be placed.
///
/// This does not clear `WS_EX_TOPMOST`: using a topmost taskbar as the insertion
/// target preserves the window's topmost status while leaving that monitor's
/// taskbar in front. A taskbar on another monitor must not be used as the
/// insertion target: doing so can demote the taskbar on this window's monitor
/// and prevent its auto-hide reveal gesture. The caller already owns the window
/// thread.
unsafe fn place_behind_shell_taskbars(hwnd: HWND) {
    let Some(taskbar) = shell_taskbar_insert_after(hwnd) else {
        return;
    };
    let _ = SetWindowPos(
        hwnd,
        Some(taskbar),
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
    );
}

unsafe fn shell_taskbar_insert_after(hwnd: HWND) -> Option<HWND> {
    let target_monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST).0 as isize;
    let mut context = TopLevelWindows::default();
    let _ = EnumWindows(
        Some(collect_top_level_window),
        LPARAM(&mut context as *mut TopLevelWindows as isize),
    );
    let classes = context
        .windows
        .iter()
        .map(|window| (window.class_name.as_str(), window.topmost, window.monitor))
        .collect::<Vec<_>>();
    taskbar_insert_after_index(&classes, target_monitor).map(|index| context.windows[index].hwnd)
}

#[derive(Default)]
struct TopLevelWindows {
    windows: Vec<TopLevelWindow>,
}

struct TopLevelWindow {
    hwnd: HWND,
    class_name: String,
    topmost: bool,
    monitor: isize,
}

unsafe extern "system" fn collect_top_level_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let context = &mut *(lparam.0 as *mut TopLevelWindows);
    let mut class_name = [0u16; 256];
    let len = GetClassNameW(hwnd, &mut class_name);
    if len > 0 {
        context.windows.push(TopLevelWindow {
            hwnd,
            class_name: String::from_utf16_lossy(&class_name[..len as usize]),
            topmost: GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0,
            monitor: MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST).0 as isize,
        });
    }
    true.into()
}

fn taskbar_insert_after_index(
    windows: &[(&str, bool, isize)],
    target_monitor: isize,
) -> Option<usize> {
    windows.iter().rposition(|(class_name, topmost, monitor)| {
        *topmost && *monitor == target_monitor && TASKBAR_WINDOW_CLASSES.contains(class_name)
    })
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
        // Tauri's repeated `always_on_top`, showing, and focus operations can
        // request a new Z position. Rewrite every such request before Windows
        // applies it, so no later stage emission or focus change can put this
        // window ahead of an auto-hidden taskbar when it is revealed.
        WM_WINDOWPOSCHANGING if lparam.0 != 0 => {
            let window_pos = &mut *(lparam.0 as *mut WINDOWPOS);
            if !window_pos.flags.contains(SWP_NOZORDER) {
                if let Some(taskbar) = shell_taskbar_insert_after(hwnd) {
                    window_pos.hwndInsertAfter = taskbar;
                }
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

#[cfg(test)]
mod tests {
    use super::taskbar_insert_after_index;

    #[test]
    fn taskbar_target_is_the_last_shell_taskbar_in_z_order() {
        let classes = [
            ("ApplicationWindow", false, 1),
            ("Shell_TrayWnd", true, 1),
            ("Chrome_WidgetWin_1", true, 1),
            ("Shell_SecondaryTrayWnd", true, 1),
            ("ApplicationFrameWindow", false, 1),
        ];

        assert_eq!(taskbar_insert_after_index(&classes, 1), Some(3));
    }

    #[test]
    fn taskbar_target_is_absent_without_explorer_taskbars() {
        let classes = [
            ("ApplicationWindow", false, 1),
            ("Chrome_WidgetWin_1", true, 1),
        ];

        assert_eq!(taskbar_insert_after_index(&classes, 1), None);
    }

    #[test]
    fn non_topmost_shell_window_is_not_used_as_the_insertion_target() {
        let classes = [
            ("Shell_TrayWnd", true, 1),
            ("Shell_SecondaryTrayWnd", false, 1),
        ];

        assert_eq!(taskbar_insert_after_index(&classes, 1), Some(0));
    }

    #[test]
    fn taskbar_on_another_monitor_is_not_used_as_the_insertion_target() {
        let classes = [
            ("Shell_TrayWnd", true, 1),
            ("ApplicationWindow", true, 1),
            ("Shell_SecondaryTrayWnd", true, 2),
        ];

        assert_eq!(taskbar_insert_after_index(&classes, 1), Some(0));
        assert_eq!(taskbar_insert_after_index(&classes, 2), Some(2));
    }
}
