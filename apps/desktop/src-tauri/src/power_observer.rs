#[cfg(target_os = "macos")]
use yuukei_device_host::LocalYuukeiRuntime;

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerEvent {
    SleepBefore,
    Wake,
}

#[cfg(target_os = "macos")]
pub async fn emit_power_event(runtime: LocalYuukeiRuntime, event: PowerEvent) {
    let result = match event {
        PowerEvent::SleepBefore => runtime.emit_device_sleep_before().await,
        PowerEvent::Wake => runtime.emit_device_wake().await,
    };
    if let Err(error) = result {
        let _ = runtime.logger().record(
            "power.event.error",
            "device-host",
            serde_json::json!({
                "event": power_event_name(event),
                "message": error.to_string()
            })
            .as_object()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect(),
        );
    }
}

#[cfg(any(target_os = "macos", test))]
pub fn power_event_name(event: PowerEvent) -> &'static str {
    match event {
        PowerEvent::SleepBefore => "device.sleep.before",
        PowerEvent::Wake => "device.wake",
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::{NSNotification, NSNotificationCenter};
    use yuukei_device_host::LocalYuukeiRuntime;

    use super::{emit_power_event, PowerEvent};

    pub struct PowerObserver {
        center: Retained<NSNotificationCenter>,
        observers: Vec<Retained<AnyObject>>,
        _blocks: Vec<RcBlock<dyn Fn(NonNull<NSNotification>)>>,
    }

    // SAFETY: The observer is an owning RAII handle for NSWorkspace notification
    // tokens and retained blocks. It does not expose those Objective-C objects to
    // other threads; notifications only clone the Send runtime handle and hop
    // into Tauri's async runtime. Drop only unregisters the retained tokens from
    // NSNotificationCenter.
    unsafe impl Send for PowerObserver {}
    unsafe impl Sync for PowerObserver {}

    impl PowerObserver {
        pub fn new(runtime: LocalYuukeiRuntime) -> Self {
            let center = NSWorkspace::sharedWorkspace().notificationCenter();
            let sleep_block = power_block(runtime.clone(), PowerEvent::SleepBefore);
            let wake_block = power_block(runtime, PowerEvent::Wake);
            let sleep_observer = unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(NSWorkspaceWillSleepNotification),
                    None,
                    None,
                    &sleep_block,
                )
            };
            let wake_observer = unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(NSWorkspaceDidWakeNotification),
                    None,
                    None,
                    &wake_block,
                )
            };
            Self {
                center,
                observers: vec![sleep_observer.into(), wake_observer.into()],
                _blocks: vec![sleep_block, wake_block],
            }
        }
    }

    impl Drop for PowerObserver {
        fn drop(&mut self) {
            for observer in &self.observers {
                unsafe {
                    self.center.removeObserver(observer);
                }
            }
        }
    }

    fn power_block(
        runtime: LocalYuukeiRuntime,
        event: PowerEvent,
    ) -> RcBlock<dyn Fn(NonNull<NSNotification>)> {
        RcBlock::new(move |_| {
            let runtime = runtime.clone();
            tauri::async_runtime::spawn(async move {
                emit_power_event(runtime, event).await;
            });
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use yuukei_device_host::LocalYuukeiRuntime;

    pub struct PowerObserver;

    impl PowerObserver {
        pub fn new(_runtime: LocalYuukeiRuntime) -> Self {
            Self
        }
    }
}

pub use platform::PowerObserver;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_events_map_to_runtime_event_names() {
        assert_eq!(
            power_event_name(PowerEvent::SleepBefore),
            "device.sleep.before"
        );
        assert_eq!(power_event_name(PowerEvent::Wake), "device.wake");
    }
}
