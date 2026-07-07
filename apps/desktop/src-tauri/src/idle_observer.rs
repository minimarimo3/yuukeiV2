pub fn seconds_since_last_user_input() -> Option<f64> {
    platform::seconds_since_last_user_input()
}

#[cfg(target_os = "macos")]
mod platform {
    const K_CG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE: u32 = 0;
    const K_CG_ANY_INPUT_EVENT_TYPE: u32 = u32::MAX;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state_id: u32, event_type: u32) -> f64;
    }

    pub fn seconds_since_last_user_input() -> Option<f64> {
        let seconds = unsafe {
            CGEventSourceSecondsSinceLastEventType(
                K_CG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE,
                K_CG_ANY_INPUT_EVENT_TYPE,
            )
        };
        seconds.is_finite().then_some(seconds)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    #[repr(C)]
    struct LastInputInfo {
        cb_size: u32,
        dw_time: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetLastInputInfo(plii: *mut LastInputInfo) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetTickCount64() -> u64;
    }

    pub fn seconds_since_last_user_input() -> Option<f64> {
        let mut info = LastInputInfo {
            cb_size: std::mem::size_of::<LastInputInfo>() as u32,
            dw_time: 0,
        };
        let ok = unsafe { GetLastInputInfo(&mut info) };
        if ok == 0 {
            return None;
        }
        let now_ms = unsafe { GetTickCount64() };
        let idle_ms = now_ms.saturating_sub(u64::from(info.dw_time));
        Some(idle_ms as f64 / 1000.0)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    pub fn seconds_since_last_user_input() -> Option<f64> {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unsupported_platforms_return_none() {
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(super::seconds_since_last_user_input(), None);
    }
}
