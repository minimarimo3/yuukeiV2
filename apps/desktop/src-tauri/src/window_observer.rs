use yuukei_device_host::{DesktopWindowObservation, ObservationSettingsState};

pub fn observation_loop_enabled(settings: &ObservationSettingsState) -> bool {
    settings.windows
}

pub fn collect_desktop_windows() -> Vec<DesktopWindowObservation> {
    platform::collect_desktop_windows()
}

#[cfg(target_os = "macos")]
mod platform {
    use core_foundation::{
        base::{CFType, TCFType},
        dictionary::{CFDictionary, CFDictionaryRef},
        number::CFNumber,
        string::CFString,
    };
    use core_graphics::{
        geometry::CGRect,
        window::{
            copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer,
            kCGWindowListOptionOnScreenOnly, kCGWindowNumber, kCGWindowOwnerName,
            kCGWindowOwnerPID,
        },
    };
    use objc2_app_kit::NSWorkspace;
    use yuukei_device_host::{DesktopWindowFrame, DesktopWindowObservation};

    pub fn collect_desktop_windows() -> Vec<DesktopWindowObservation> {
        let own_pid = std::process::id() as i64;
        let frontmost_pid = frontmost_pid();
        let Some(array) = copy_window_info(kCGWindowListOptionOnScreenOnly, kCGNullWindowID) else {
            return Vec::new();
        };
        let mut observations = Vec::new();
        let mut focused_assigned = false;
        for item in array.iter() {
            let dictionary = unsafe {
                CFDictionary::<CFString, CFType>::wrap_under_get_rule(*item as CFDictionaryRef)
            };
            let Some(layer) = number_value(&dictionary, unsafe { kCGWindowLayer }) else {
                continue;
            };
            if layer != 0 {
                continue;
            }
            let Some(pid) = number_value(&dictionary, unsafe { kCGWindowOwnerPID }) else {
                continue;
            };
            if pid == own_pid {
                continue;
            }
            let Some(window_number) = number_value(&dictionary, unsafe { kCGWindowNumber }) else {
                continue;
            };
            let Some(app) = string_value(&dictionary, unsafe { kCGWindowOwnerName }) else {
                continue;
            };
            if app.trim().is_empty() || app == "Yuukei" {
                continue;
            }
            let Some(frame) = frame_value(&dictionary) else {
                continue;
            };
            if frame.width <= 1.0 || frame.height <= 1.0 {
                continue;
            }
            let focused = !focused_assigned && frontmost_pid == Some(pid);
            focused_assigned |= focused;
            observations.push(DesktopWindowObservation {
                window_key: window_number.to_string(),
                app,
                frame,
                focused,
            });
        }
        observations
    }

    fn number_value(
        dictionary: &CFDictionary<CFString, CFType>,
        key: core_foundation::string::CFStringRef,
    ) -> Option<i64> {
        let key = unsafe { CFString::wrap_under_get_rule(key) };
        dictionary
            .find(&key)
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|number| number.to_i64())
    }

    fn string_value(
        dictionary: &CFDictionary<CFString, CFType>,
        key: core_foundation::string::CFStringRef,
    ) -> Option<String> {
        let key = unsafe { CFString::wrap_under_get_rule(key) };
        dictionary
            .find(&key)
            .and_then(|value| value.downcast::<CFString>())
            .map(|string| string.to_string())
    }

    fn frame_value(dictionary: &CFDictionary<CFString, CFType>) -> Option<DesktopWindowFrame> {
        let key = unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) };
        let bounds = dictionary
            .find(&key)
            .and_then(|value| value.downcast::<CFDictionary>())
            .and_then(|dictionary| CGRect::from_dict_representation(&dictionary))?;
        Some(DesktopWindowFrame {
            x: bounds.origin.x,
            y: bounds.origin.y,
            width: bounds.size.width,
            height: bounds.size.height,
        })
    }

    fn frontmost_pid() -> Option<i64> {
        let workspace = NSWorkspace::sharedWorkspace();
        let app = workspace.frontmostApplication()?;
        Some(app.processIdentifier() as i64)
    }
}

#[cfg(windows)]
mod platform {
    use std::{ffi::OsString, mem::size_of, os::windows::ffi::OsStringExt};

    use windows::Win32::{
        Foundation::{BOOL, HWND, LPARAM, RECT},
        Graphics::{
            Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
            Gdi::IsRectEmpty,
        },
        System::{
            ProcessStatus::K32GetModuleBaseNameW,
            Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ},
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId,
            IsWindowVisible,
        },
    };
    use yuukei_device_host::{DesktopWindowFrame, DesktopWindowObservation};

    pub fn collect_desktop_windows() -> Vec<DesktopWindowObservation> {
        let mut context = WindowsContext {
            own_pid: std::process::id(),
            foreground: unsafe { GetForegroundWindow() },
            observations: Vec::new(),
        };
        unsafe {
            let _ = EnumWindows(Some(enum_window), LPARAM(&mut context as *mut _ as isize));
        }
        context.observations
    }

    struct WindowsContext {
        own_pid: u32,
        foreground: HWND,
        observations: Vec<DesktopWindowObservation>,
    }

    unsafe extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let context = &mut *(lparam.0 as *mut WindowsContext);
        if !IsWindowVisible(hwnd).as_bool() || is_cloaked(hwnd) {
            return true.into();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == context.own_pid {
            return true.into();
        }
        let Some(app) = process_name(pid) else {
            return true.into();
        };
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() || IsRectEmpty(&rect).as_bool() {
            return true.into();
        }
        context.observations.push(DesktopWindowObservation {
            window_key: format!("{:?}", hwnd.0),
            app,
            frame: DesktopWindowFrame {
                x: rect.left as f64,
                y: rect.top as f64,
                width: (rect.right - rect.left) as f64,
                height: (rect.bottom - rect.top) as f64,
            },
            focused: hwnd == context.foreground,
        });
        true.into()
    }

    unsafe fn is_cloaked(hwnd: HWND) -> bool {
        let mut cloaked = 0u32;
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut _,
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
    }

    unsafe fn process_name(pid: u32) -> Option<String> {
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            false,
            pid,
        )
        .ok()?;
        let mut buffer = [0u16; 260];
        let len = K32GetModuleBaseNameW(handle, None, &mut buffer);
        if len == 0 {
            return None;
        }
        let mut name = OsString::from_wide(&buffer[..len as usize])
            .to_string_lossy()
            .to_string();
        if let Some(stripped) = name.strip_suffix(".exe") {
            name = stripped.to_string();
        }
        (!name.is_empty()).then_some(name)
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use yuukei_device_host::DesktopWindowObservation;

    pub fn collect_desktop_windows() -> Vec<DesktopWindowObservation> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn observation_loop_is_enabled_only_for_windows_setting() {
        let base = ObservationSettingsState {
            windows: false,
            folders: true,
            downloads: true,
            settings_path: PathBuf::from("observations.json"),
        };
        assert!(!observation_loop_enabled(&base));
        assert!(observation_loop_enabled(&ObservationSettingsState {
            windows: true,
            ..base
        }));
    }
}
