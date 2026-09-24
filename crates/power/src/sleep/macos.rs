//! `NSWorkspace` sleep and wake notifications, after anarlog's
//! `SleepDetector`. See PROVENANCE.md.

use std::sync::mpsc;
use std::thread::JoinHandle;

use block2::RcBlock;
use objc2::{msg_send, rc::Retained};
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject, NSString};

use super::{PowerCallback, PowerEvent};

struct Observers {
    center: Retained<NSNotificationCenter>,
    will_sleep: Retained<NSObject>,
    did_wake: Retained<NSObject>,
}

impl Drop for Observers {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.center, removeObserver: &*self.will_sleep];
            let _: () = msg_send![&*self.center, removeObserver: &*self.did_wake];
        }
    }
}

/// The observers live on a thread of their own, which holds them until the
/// watch is dropped. With no queue given, a notification is delivered on the
/// thread that posts it (the main thread), so `WillSleep` runs before the
/// system goes on to sleep.
pub struct Watch {
    shutdown: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Watch {
    pub fn start(callback: PowerCallback) -> Option<Self> {
        let (shutdown, shutdown_rx) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("longhand-sleep-watch".into())
            .spawn(move || {
                let will_sleep_callback = callback.clone();
                let will_sleep_block = RcBlock::new(move |_: *const NSNotification| {
                    will_sleep_callback(PowerEvent::WillSleep);
                });
                let did_wake_block = RcBlock::new(move |_: *const NSNotification| {
                    callback(PowerEvent::DidWake);
                });
                let _observers = unsafe {
                    let center = NSWorkspace::sharedWorkspace().notificationCenter();
                    let will_sleep_name = NSString::from_str("NSWorkspaceWillSleepNotification");
                    let did_wake_name = NSString::from_str("NSWorkspaceDidWakeNotification");
                    let will_sleep: Retained<NSObject> = msg_send![
                        &*center,
                        addObserverForName: &*will_sleep_name,
                        object: std::ptr::null::<NSObject>(),
                        queue: std::ptr::null::<NSObject>(),
                        usingBlock: &*will_sleep_block
                    ];
                    let did_wake: Retained<NSObject> = msg_send![
                        &*center,
                        addObserverForName: &*did_wake_name,
                        object: std::ptr::null::<NSObject>(),
                        queue: std::ptr::null::<NSObject>(),
                        usingBlock: &*did_wake_block
                    ];
                    Observers {
                        center,
                        will_sleep,
                        did_wake,
                    }
                };
                let _ = shutdown_rx.recv();
            });
        match thread {
            Ok(thread) => Some(Self {
                shutdown: Some(shutdown),
                thread: Some(thread),
            }),
            Err(e) => {
                log::warn!("sleep notifications unavailable: {e}");
                None
            }
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        drop(self.shutdown.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
