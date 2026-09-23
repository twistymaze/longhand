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
    /// The last output probe, by the output it was taken on.
    checked: Mutex<Option<(Option<String>, Option<String>)>>,
}

/// Whether an alert raised now would reach the user (R5a).
#[derive(serde::Serialize, specta::Type, Clone, Debug)]
pub struct AlertCheck {
    /// Why the cue would not be heard, when it would not.
    pub output_problem: Option<String>,
    /// The user has confirmed hearing the cue at setup.
    pub tone_heard: bool,
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

impl AlertsManager {
    /// Raise the alert for a session whose capture stopped. `cue` is false
    /// when nothing was in progress to hear, as after a relaunch.
    pub fn raise(
        self: &Arc<Self>,
        app: &AppHandle,
        capture: &Arc<CaptureManager>,
        session_id: &str,
        reason: InterruptReason,
        detail: Option<&str>,
        cue: bool,
    ) {
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
                let mut cues = if cue { &CUE_AT[..] } else { &[][..] };
                while !cancel.is_cancelled() {
                    if !capture.session_state(&id).is_some_and(|s| failed(&s)) {
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

    /// Check the alert path. The output is probed again only when `force`
    /// or when the default output has changed since the last probe, since
    /// the probe spawns a process on macOS.
    pub fn check(&self, tone_heard: bool, force: bool) -> AlertCheck {
        let output = sfx::default_output_name();
        let mut checked = self.checked.lock().unwrap_or_else(|e| e.into_inner());
        let cached = checked
            .as_ref()
            .filter(|(name, _)| !force && *name == output)
            .map(|(_, problem)| problem.clone());
        let output_problem = match cached {
            Some(problem) => problem,
            None => {
                let problem = sfx::output_state().problem().map(|p| p.to_string());
                if let Some(problem) = &problem {
                    log::info!("alert path: {problem}");
                }
                *checked = Some((output, problem.clone()));
                problem
            }
        };
        AlertCheck {
            output_problem,
            tone_heard,
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
        self.raise(app, capture, &id, reason, None, false);
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
}
