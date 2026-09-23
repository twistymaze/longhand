//! Sleep and wake notices (U13): the lid, the menu, or a sleep the assertion
//! did not prevent. The capture manager closes the segment on the first and
//! opens the next on the second, where anarlog ends the session instead.

use std::sync::Arc;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    /// The system is about to sleep. Delivered synchronously: sleep waits,
    /// briefly, for the handler to return, so a segment closed here is closed
    /// on disk before the machine stops.
    WillSleep,
    DidWake,
}

pub type PowerCallback = Arc<dyn Fn(PowerEvent) + Send + Sync + 'static>;

/// Delivers [`PowerEvent`]s until dropped.
pub struct SleepWatch {
    #[cfg(target_os = "macos")]
    _platform: macos::Watch,
    #[cfg(windows)]
    _platform: windows::Watch,
}

impl SleepWatch {
    /// Start watching. `None` where the platform has no watcher or it could
    /// not register; the failure is logged, and a sleep then reads as a lost
    /// device (U14) instead.
    pub fn start(callback: impl Fn(PowerEvent) + Send + Sync + 'static) -> Option<Self> {
        let callback: PowerCallback = Arc::new(callback);
        #[cfg(target_os = "macos")]
        return macos::Watch::start(callback).map(|w| Self { _platform: w });
        #[cfg(windows)]
        return windows::Watch::start(callback).map(|w| Self { _platform: w });
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = callback;
            None
        }
    }
}
