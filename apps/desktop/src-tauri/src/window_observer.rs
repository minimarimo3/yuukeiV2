use std::{
    env,
    path::{Path, PathBuf},
};

use yuukei_device_host::{
    DesktopFolderObservation, DesktopWindowObservation, KnownDesktopFolders,
    ObservationSettingsState,
};

pub fn observation_loop_enabled(settings: &ObservationSettingsState) -> bool {
    settings.windows || settings.folders
}

pub fn collect_desktop_windows() -> Vec<DesktopWindowObservation> {
    platform::collect_desktop_windows()
}

pub fn collect_desktop_folders() -> Vec<DesktopFolderObservation> {
    platform::collect_desktop_folders()
}

pub fn downloads_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join("Downloads"))
}

fn known_desktop_folders() -> KnownDesktopFolders {
    let Some(home) = home_dir() else {
        return KnownDesktopFolders::default();
    };
    KnownDesktopFolders {
        downloads: path_string(home.join("Downloads")),
        desktop: path_string(home.join("Desktop")),
        documents: path_string(home.join("Documents")),
        pictures: path_string(home.join("Pictures")),
        trash: macos_trash_path(&home),
    }
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
}

fn path_string(path: PathBuf) -> Option<String> {
    Some(path.to_string_lossy().to_string()).filter(|value| !value.trim().is_empty())
}

#[cfg(target_os = "macos")]
fn macos_trash_path(home: &Path) -> Option<String> {
    path_string(home.join(".Trash"))
}

#[cfg(not(target_os = "macos"))]
fn macos_trash_path(_home: &Path) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
mod platform {
    use std::process::Command;

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
    use yuukei_device_host::{
        categorize_desktop_folder_path, DesktopFolderObservation, DesktopWindowFrame,
        DesktopWindowObservation,
    };

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

    pub fn collect_desktop_folders() -> Vec<DesktopFolderObservation> {
        if frontmost_app_name()
            .as_deref()
            .map(|name| name.eq_ignore_ascii_case("Finder"))
            != Some(true)
        {
            return Vec::new();
        }
        let Ok(output) = Command::new("osascript")
            .args([
                "-e",
                "tell application \"Finder\" to get POSIX path of (target of front window as alias)",
            ])
            .output()
        else {
            return Vec::new();
        };
        if !output.status.success() {
            return Vec::new();
        }
        let path = String::from_utf8_lossy(&output.stdout);
        let path = path.trim();
        if path.is_empty() {
            return Vec::new();
        }
        let category = categorize_desktop_folder_path(path, &super::known_desktop_folders());
        vec![DesktopFolderObservation {
            folder_key: "finder-front".to_string(),
            category,
            app: "finder".to_string(),
        }]
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

    fn frontmost_app_name() -> Option<String> {
        let workspace = NSWorkspace::sharedWorkspace();
        let app = workspace.frontmostApplication()?;
        Some(app.localizedName()?.to_string())
    }
}

#[cfg(windows)]
mod platform {
    use std::{ffi::OsString, mem::size_of, os::windows::ffi::OsStringExt};

    use windows::core::Interface;
    use windows::Win32::{
        Foundation::{BOOL, HWND, LPARAM, RECT},
        Graphics::{
            Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
            Gdi::IsRectEmpty,
        },
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL,
                COINIT_APARTMENTTHREADED,
            },
            ProcessStatus::K32GetModuleBaseNameW,
            Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ},
            Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4},
        },
        UI::{
            Shell::{Folder2, IShellFolderViewDual, IShellWindows, IWebBrowserApp, ShellWindows},
            WindowsAndMessaging::{
                EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId,
                IsWindowVisible,
            },
        },
    };
    use yuukei_device_host::{
        categorize_desktop_folder_path, DesktopFolderObservation, DesktopWindowFrame,
        DesktopWindowObservation,
    };

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

    pub fn collect_desktop_folders() -> Vec<DesktopFolderObservation> {
        unsafe { collect_desktop_folders_com().unwrap_or_default() }
    }

    unsafe fn collect_desktop_folders_com() -> windows::core::Result<Vec<DesktopFolderObservation>>
    {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = collect_desktop_folders_after_com_init();
        if initialized {
            CoUninitialize();
        }
        result
    }

    unsafe fn collect_desktop_folders_after_com_init(
    ) -> windows::core::Result<Vec<DesktopFolderObservation>> {
        let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        let count = windows.Count()?;
        let known = super::known_desktop_folders();
        let mut observations = Vec::new();
        for index in 0..count {
            let index = variant_i4(index);
            let Ok(dispatch) = windows.Item(&index) else {
                continue;
            };
            let Ok(browser) = dispatch.cast::<IWebBrowserApp>() else {
                continue;
            };
            let Ok(document) = browser.Document() else {
                continue;
            };
            let Ok(view) = document.cast::<IShellFolderViewDual>() else {
                continue;
            };
            let Ok(folder) = view.Folder() else {
                continue;
            };
            let Ok(folder2) = folder.cast::<Folder2>() else {
                continue;
            };
            let Ok(item) = folder2.Self_() else {
                continue;
            };
            let Ok(path) = item.Path() else {
                continue;
            };
            let path = path.to_string();
            if path.trim().is_empty() {
                continue;
            }
            let hwnd = browser.HWND().map(|handle| handle.0).unwrap_or_default();
            observations.push(DesktopFolderObservation {
                folder_key: hwnd.to_string(),
                category: categorize_desktop_folder_path(&path, &known),
                app: "explorer".to_string(),
            });
        }
        Ok(observations)
    }

    fn variant_i4(value: i32) -> VARIANT {
        VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: std::mem::ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_I4,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: VARIANT_0_0_0 { lVal: value },
                }),
            },
        }
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
    use yuukei_device_host::{DesktopFolderObservation, DesktopWindowObservation};

    pub fn collect_desktop_windows() -> Vec<DesktopWindowObservation> {
        Vec::new()
    }

    pub fn collect_desktop_folders() -> Vec<DesktopFolderObservation> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn observation_loop_is_enabled_for_windows_or_folders_setting() {
        let base = ObservationSettingsState {
            windows: false,
            folders: false,
            downloads: true,
            settings_path: PathBuf::from("observations.json"),
        };
        assert!(!observation_loop_enabled(&base));
        assert!(observation_loop_enabled(&ObservationSettingsState {
            windows: true,
            ..base.clone()
        }));
        assert!(observation_loop_enabled(&ObservationSettingsState {
            folders: true,
            ..base
        }));
    }
}
