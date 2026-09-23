//! The session lifecycle (High-Level Technical Design, "Session lifecycle").
//!
//! Every drawn transition is legal and anything else is rejected with a typed
//! error. Guards that depend on the rest of the system — whether the session
//! is the most recent one, whether free space was re-checked — are passed in
//! on the event, so this module stays pure and exhaustively testable.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

/// How long after stopping or being interrupted a session may be resumed (R6a).
pub const RESUME_WINDOW: chrono::TimeDelta = chrono::TimeDelta::minutes(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptReason {
    /// The input device failed and the restart budget ran out (R5b).
    Device,
    /// The free-space floor was reached (R7a). Never resumes on its own.
    Disk,
    /// The machine went to sleep (KTD12). Resumes at wake without a cue.
    Sleep,
    /// The process died while capturing; found by the launch sweep (R8).
    Crash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Assemble,
    Diarize,
    Transcribe,
    Join,
    Verify,
    Summarize,
    Land,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionState {
    Capturing,
    Interrupted {
        reason: InterruptReason,
        /// Wall-clock time the interruption began. The resume window spans
        /// process restarts, which a monotonic clock cannot.
        since: DateTime<FixedOffset>,
        /// The failed-capture state persists until acknowledged (R5c).
        acknowledged: bool,
    },
    Queued {
        stopped_at: DateTime<FixedOffset>,
    },
    Processing {
        stopped_at: DateTime<FixedOffset>,
    },
    Blocked {
        stopped_at: DateTime<FixedOffset>,
    },
    ProcessingFailed {
        stopped_at: DateTime<FixedOffset>,
        stage: Stage,
    },
    AwaitingDestination {
        stopped_at: DateTime<FixedOffset>,
        problem: String,
    },
    Landed,
    Discarded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Automatic resumption succeeded within the budget; a new segment opens.
    AutoResumed,
    /// Capture stopped and cannot continue on its own.
    CaptureLost {
        reason: InterruptReason,
        at: DateTime<FixedOffset>,
    },
    DidWake,
    /// The user resumed. `now` checks the window; `is_most_recent` is the
    /// caller's answer to R6a; `space_ok` is the disk re-check (R7a).
    UserResume {
        now: DateTime<FixedOffset>,
        is_most_recent: bool,
        space_ok: bool,
    },
    DeviceReturned,
    /// The user stopped. `retained_audio` is false when every segment was
    /// under a second and was dropped (R35).
    UserStop {
        at: DateTime<FixedOffset>,
        retained_audio: bool,
    },
    /// The shortcut started another session (R6c, R5e).
    NewSessionStarted {
        at: DateTime<FixedOffset>,
        retained_audio: bool,
    },
    /// Periodic check for an untouched interruption (R5e).
    Tick {
        now: DateTime<FixedOffset>,
        retained_audio: bool,
    },
    Acknowledge,
    Dequeued,
    CredentialUnavailable,
    StoreUnlocked,
    StageFailed(Stage),
    Retry,
    LandingFailed(String),
    Landed,
    /// A note edit or a regenerated summary on a landed session.
    LandedEdit,
    /// Launch sweep: this session was processing in a process that is gone.
    ClaimLost,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{event} is not legal from {state}")]
pub struct IllegalTransition {
    pub state: String,
    pub event: String,
}

impl SessionState {
    pub fn name(&self) -> &'static str {
        match self {
            SessionState::Capturing => "capturing",
            SessionState::Interrupted { .. } => "interrupted",
            SessionState::Queued { .. } => "queued",
            SessionState::Processing { .. } => "processing",
            SessionState::Blocked { .. } => "blocked",
            SessionState::ProcessingFailed { .. } => "processing_failed",
            SessionState::AwaitingDestination { .. } => "awaiting_destination",
            SessionState::Landed => "landed",
            SessionState::Discarded => "discarded",
        }
    }

    /// The wall-clock time from which the resume window runs, if the session
    /// is in a state that can be resumed at all.
    pub fn resume_window_start(&self) -> Option<DateTime<FixedOffset>> {
        match self {
            SessionState::Interrupted { since, .. } => Some(*since),
            SessionState::Queued { stopped_at }
            | SessionState::Processing { stopped_at }
            | SessionState::Blocked { stopped_at }
            | SessionState::ProcessingFailed { stopped_at, .. }
            | SessionState::AwaitingDestination { stopped_at, .. } => Some(*stopped_at),
            _ => None,
        }
    }

    /// Whether `now` is still inside the resume window. A clock set backwards
    /// reads as no time elapsed rather than as a negative interval.
    pub fn within_resume_window(&self, now: DateTime<FixedOffset>) -> bool {
        self.resume_window_start().is_some_and(|start| {
            now.signed_duration_since(start)
                .max(chrono::TimeDelta::zero())
                <= RESUME_WINDOW
        })
    }

    pub fn is_final(&self) -> bool {
        matches!(self, SessionState::Landed | SessionState::Discarded)
    }

    pub fn transition(&self, event: &Event) -> Result<SessionState, IllegalTransition> {
        use InterruptReason as R;
        use SessionState as S;

        let stopped = |at: DateTime<FixedOffset>, retained: bool| {
            if retained {
                S::Queued { stopped_at: at }
            } else {
                S::Discarded
            }
        };

        let next = match (self, event) {
            (S::Capturing, Event::AutoResumed) => Some(S::Capturing),
            (S::Capturing, Event::CaptureLost { reason, at }) => Some(S::Interrupted {
                reason: *reason,
                since: *at,
                acknowledged: false,
            }),
            (S::Capturing, Event::UserStop { at, retained_audio })
            | (S::Capturing, Event::NewSessionStarted { at, retained_audio }) => {
                Some(stopped(*at, *retained_audio))
            }

            (
                S::Interrupted {
                    reason: R::Sleep, ..
                },
                Event::DidWake,
            ) => Some(S::Capturing),
            (
                S::Interrupted {
                    reason: R::Device, ..
                },
                Event::DeviceReturned,
            ) => Some(S::Capturing),
            (
                S::Interrupted { reason, .. },
                Event::UserResume {
                    now,
                    is_most_recent,
                    space_ok,
                },
            ) if *is_most_recent
                && self.within_resume_window(*now)
                && (*reason != R::Disk || *space_ok) =>
            {
                Some(S::Capturing)
            }
            (S::Interrupted { .. }, Event::UserStop { at, retained_audio })
            | (S::Interrupted { .. }, Event::NewSessionStarted { at, retained_audio }) => {
                Some(stopped(*at, *retained_audio))
            }
            (
                S::Interrupted { since, .. },
                Event::Tick {
                    now,
                    retained_audio,
                },
            ) if !self.within_resume_window(*now) => {
                // Queued as of the moment the window closed, so FIFO order
                // reflects when the session became final, not when it was noticed.
                Some(stopped(*since + RESUME_WINDOW, *retained_audio))
            }
            (S::Interrupted { reason, since, .. }, Event::Acknowledge) => Some(S::Interrupted {
                reason: *reason,
                since: *since,
                acknowledged: true,
            }),

            (S::Queued { stopped_at }, Event::Dequeued) => Some(S::Processing {
                stopped_at: *stopped_at,
            }),
            (S::Processing { stopped_at }, Event::CredentialUnavailable) => Some(S::Blocked {
                stopped_at: *stopped_at,
            }),
            (S::Blocked { stopped_at }, Event::StoreUnlocked) => Some(S::Processing {
                stopped_at: *stopped_at,
            }),
            (S::Processing { stopped_at }, Event::StageFailed(stage)) => {
                Some(S::ProcessingFailed {
                    stopped_at: *stopped_at,
                    stage: *stage,
                })
            }
            (S::ProcessingFailed { stopped_at, .. }, Event::Retry)
            | (S::AwaitingDestination { stopped_at, .. }, Event::Retry) => Some(S::Processing {
                stopped_at: *stopped_at,
            }),
            (S::Processing { stopped_at }, Event::LandingFailed(problem)) => {
                Some(S::AwaitingDestination {
                    stopped_at: *stopped_at,
                    problem: problem.clone(),
                })
            }
            (S::Processing { .. }, Event::Landed) => Some(S::Landed),
            (S::Processing { stopped_at }, Event::ClaimLost)
            | (S::Blocked { stopped_at }, Event::ClaimLost) => Some(S::Queued {
                stopped_at: *stopped_at,
            }),
            (S::Landed, Event::LandedEdit) => Some(S::Landed),

            (
                S::Queued { .. }
                | S::Processing { .. }
                | S::ProcessingFailed { .. }
                | S::Blocked { .. }
                | S::AwaitingDestination { .. },
                Event::UserResume {
                    now,
                    is_most_recent,
                    ..
                },
            ) if *is_most_recent && self.within_resume_window(*now) => Some(S::Capturing),

            _ => None,
        };

        next.ok_or_else(|| IllegalTransition {
            state: self.name().to_string(),
            event: format!("{event:?}"),
        })
    }
}
