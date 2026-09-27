//! The idle-sleep assertion held for the life of active capture.
//!
//! `idle(true)` prevents sleep from inactivity; `display(false)` lets the
//! screen sleep, since nobody needs to watch it; explicit sleep (the lid,
//! the menu) is not prevented and is detected instead (U13). On Windows the
//! assertion is `SetThreadExecutionState`, which belongs to the calling
//! thread and clears silently when that thread ends — so the guard is
//! created, held, and dropped by the thread that owns capture, never moved
//! to a helper that might finish first.

/// Shown by `pmset -g assertions` on macOS.
pub const REASON: &str = "Longhand is recording";

/// Holds the assertion until dropped. Deliberately `!Send`: moving it to
/// another thread would break the Windows ownership rule above.
pub struct IdleGuard {
    _awake: keepawake::KeepAwake,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl IdleGuard {
    /// Take the assertion. A refusal is logged and returns `None`: recording
    /// matters more than the assertion, and sleep detection still covers a
    /// machine that sleeps anyway.
    pub fn acquire() -> Option<Self> {
        Self::acquire_for(REASON)
    }

    pub(crate) fn acquire_for(reason: &str) -> Option<Self> {
        match keepawake::Builder::default()
            .idle(true)
            .display(false)
            .reason(reason)
            .app_name("Longhand")
            .app_reverse_domain("net.ewal.longhand")
            .create()
        {
            Ok(awake) => {
                log::info!("idle-sleep assertion held");
                Some(Self {
                    _awake: awake,
                    _not_send: std::marker::PhantomData,
                })
            }
            Err(e) => {
                log::warn!("idle-sleep assertion refused: {e}");
                None
            }
        }
    }
}

impl Drop for IdleGuard {
    fn drop(&mut self) {
        log::info!("idle-sleep assertion released");
    }
}
