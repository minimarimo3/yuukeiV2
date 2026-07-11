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
