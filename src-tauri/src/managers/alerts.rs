//! The failed-capture alert (R5, R5c, R5d, KD9, KTD14): the cue first,
//! since it carries across a room and nothing suppresses it; the
//! notification second; the tray and the in-app state, which is the store's
//! Interrupted state itself, always correct.
//!
//! One alert is held at a time, for the session whose capture stopped. It
//! ends when that session leaves unacknowledged Interrupted by any path —
//! resume, the device's return, Stop, the auto-queue — which a watch over
//! the store notices, so no path has to remember to clear it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use session_core::{InterruptReason, SessionState};
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

use crate::managers::capture::CaptureManager;

/// When the cue plays after the alert is raised: once, then at a widening
/// interval, three times in all (R5d).
pub const CUE_AT: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_secs(30),
    Duration::from_secs(90),
];

/// When the cue plays for one alert. `cue` is false when nothing was in
/// progress to hear. `alert_sound` is the user's setting (R9, KTD2). The
/// tray and the notification do not depend on either.
fn cue_times(cue: bool, alert_sound: bool) -> &'static [Duration] {
    if cue && alert_sound {
        &CUE_AT[..]
    } else {
        &[][..]
    }
}

/// How often a raised alert checks whether its session has moved on.
const WATCH: Duration = Duration::from_secs(1);

struct Raised {
    session_id: String,
    generation: u64,
    cancel: CancellationToken,
}

#[derive(Default)]
pub struct AlertsManager {
    raised: Mutex<Option<Raised>>,
    generations: AtomicU64,
}

/// What the user is told, by the reason capture stopped.
pub fn headline(reason: InterruptReason) -> &'static str {
    match reason {
        InterruptReason::Device => "The microphone stopped and could not be reopened",
        InterruptReason::Disk => "Recording stopped: the disk is nearly full",
        InterruptReason::Sleep => "Recording paused while the computer slept",
        InterruptReason::Crash => "Longhand quit unexpectedly while recording",
    }
}

/// Whether a session still holds the failed-capture state.
fn failed(state: &SessionState) -> bool {
    matches!(
        state,
        SessionState::Interrupted {
            acknowledged: false,
            ..
        }
    )
}

/// Whether the alert of a note holds, from its stored `state` and `open`,
/// which is true while the capture manager holds the note open. The alert
/// holds while the note is failed. It also holds while the store says
/// Capturing and nothing records the note: the interrupt write failed, and
/// a Stop settles the note (KTD6 of the capture recovery plan).
pub(crate) fn alert_holds(state: Option<&SessionState>, open: bool) -> bool {
    match state {
        Some(SessionState::Capturing) => !open,
        Some(state) => failed(state),
        None => false,
    }
}

/// The notification text of an alert that the launch restores. A note
/// whose launch recovery failed (`recovery_failed`) says so; the log has
/// the error.
fn restore_detail(recovery_failed: bool) -> Option<&'static str> {
    recovery_failed.then_some("Some recorded audio could not be recovered")
}

impl AlertsManager {
    /// Raise the alert for a session whose capture stopped. `cue` is false
    /// when nothing was in progress to hear, as after a relaunch. The cue
    /// also plays only when the alert sound setting is on (R9); the alert
    /// reads it once, here.
    pub fn raise(
        self: &Arc<Self>,
        app: &AppHandle,
        capture: &Arc<CaptureManager>,
        session_id: &str,
        reason: InterruptReason,
        detail: Option<&str>,
        cue: bool,
    ) {
        let cues = cue_times(
            cue,
            crate::managers::settings::get_settings(app).alert_sound,
        );
        let cancel = CancellationToken::new();
        let generation = self.generations.fetch_add(1, Ordering::Relaxed);
        {
            let mut raised = self.raised.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(previous) = raised.replace(Raised {
                session_id: session_id.to_string(),
                generation,
                cancel: cancel.clone(),
            }) {
                previous.cancel.cancel();
            }
        }
        crate::tray::set_alert(app, Some(headline(reason)));
        notify(app, reason, detail);
        log::warn!("failed-capture alert raised for {session_id}");

        let alerts = Arc::clone(self);
        let app = app.clone();
        let capture = Arc::clone(capture);
        let id = session_id.to_string();
        let spawned = std::thread::Builder::new()
            .name("longhand-alert".into())
            .spawn(move || {
                let raised_at = Instant::now();
                let mut cues = cues;
                while !cancel.is_cancelled() {
                    if !capture.alert_holds(&id) {
                        break;
                    }
                    if let Some((next, rest)) = cues.split_first()
                        && raised_at.elapsed() >= *next
                    {
                        cues = rest;
                        if let Err(e) = sfx::play_cue() {
                            log::warn!("the alert cue could not play: {e}");
                        }
                        continue;
                    }
                    std::thread::sleep(WATCH);
                }
                alerts.cleared(&app, &id, generation);
            });
        if let Err(e) = spawned {
            log::error!("could not start the alert: {e}");
        }
    }

    /// Stop the cue at once for a session the user has acted on.
    pub fn clear(&self, session_id: &str) {
        let raised = self.raised.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(raised) = raised.as_ref().filter(|r| r.session_id == session_id) {
            raised.cancel.cancel();
        }
    }

    fn cleared(&self, app: &AppHandle, session_id: &str, generation: u64) {
        let mut raised = self.raised.lock().unwrap_or_else(|e| e.into_inner());
        // A newer alert may have replaced this one; it owns the tray now.
        if raised.as_ref().is_some_and(|r| r.generation == generation) {
            *raised = None;
            crate::tray::set_alert(app, None);
            log::info!("failed-capture alert for {session_id} ended");
        }
    }

    /// At launch: a session still in failed-capture gets its alert back,
    /// without the cue (R5c).
    pub fn restore(self: &Arc<Self>, app: &AppHandle, capture: &Arc<CaptureManager>) {
        let Some((id, reason)) = capture.failed_session() else {
            return;
        };
        let detail = restore_detail(capture.recovery_failed(&id));
        self.raise(app, capture, &id, reason, detail, false);
    }
}

fn notify(app: &AppHandle, reason: InterruptReason, detail: Option<&str>) {
    use tauri_plugin_notification::NotificationExt;
    let body = match detail {
        Some(detail) => format!("{detail}. Open Longhand to resume or stop."),
        None => "Open Longhand to resume or stop.".to_string(),
    };
    if let Err(e) = app
        .notification()
        .builder()
        .title(headline(reason))
        .body(body)
        .show()
    {
        log::warn!("the alert notification could not be posted: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cue_plays_three_times_at_a_widening_interval() {
        assert_eq!(CUE_AT.len(), 3);
        assert_eq!(CUE_AT[0], Duration::ZERO);
        assert!(CUE_AT[2] - CUE_AT[1] > CUE_AT[1] - CUE_AT[0]);
    }

    // Covers AE5 (R9): with the alert sound off, no cue plays.
    #[test]
    fn the_alert_sound_setting_off_gives_no_cues() {
        assert!(cue_times(true, false).is_empty());
        assert!(cue_times(false, false).is_empty());
    }

    #[test]
    fn the_alert_sound_setting_on_gives_the_cues_only_when_a_cue_is_asked() {
        assert_eq!(cue_times(true, true), &CUE_AT[..]);
        assert!(cue_times(false, true).is_empty());
    }

    #[test]
    fn only_an_unacknowledged_interruption_holds_the_alert() {
        let since = chrono::Local::now().fixed_offset();
        let interrupted = |acknowledged| SessionState::Interrupted {
            reason: InterruptReason::Device,
            since,
            acknowledged,
        };
        assert!(failed(&interrupted(false)));
        assert!(!failed(&interrupted(true)));
        assert!(!failed(&SessionState::Capturing));
        assert!(!failed(&SessionState::Queued { stopped_at: since }));
    }

    // A failed interrupt write leaves the note Capturing with nothing that
    // records it: the alert holds until a Stop or a Resume.
    #[test]
    fn the_alert_holds_for_a_capturing_note_that_nothing_records() {
        let since = chrono::Local::now().fixed_offset();
        assert!(alert_holds(Some(&SessionState::Capturing), false));
        assert!(!alert_holds(Some(&SessionState::Capturing), true));
        let interrupted = SessionState::Interrupted {
            reason: InterruptReason::Disk,
            since,
            acknowledged: false,
        };
        assert!(alert_holds(Some(&interrupted), false));
        assert!(!alert_holds(
            Some(&SessionState::Queued { stopped_at: since }),
            false
        ));
        assert!(!alert_holds(None, false));
    }

    // Covers R5: a restored alert says when the launch recovery failed.
    #[test]
    fn a_restored_alert_says_that_the_recovery_failed() {
        let detail = restore_detail(true).unwrap();
        assert!(!detail.contains("chunk"));
        assert!(restore_detail(false).is_none());
    }
}
