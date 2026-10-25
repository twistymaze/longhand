//! The session lifecycle (High-Level Technical Design, "Session lifecycle").
//!
//! Every drawn transition is legal and anything else is rejected with a typed
//! error. Guards that depend on the rest of the system — whether the session
//! is the most recent one, whether free space was re-checked — are passed in
//! on the event, so this module stays pure and exhaustively testable.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

/// How long an interrupted session waits untouched before it is stopped and
/// queued (R5e). Resume has no window (R6a).
pub const AUTO_QUEUE_AFTER: chrono::TimeDelta = chrono::TimeDelta::minutes(30);

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
    Save,
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
    Saved,
    Discarded,
    /// The import worker copies and converts an audio file into the note
    /// (KTD5 of the audio import plan). Only the worker and a user delete
    /// change the note. A stop that keeps the converted audio queues it.
    /// An older build cannot read this state, so the store version is
    /// above the version of each older build.
    Importing,
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
    /// The user resumed (R6a). `space_ok` is the disk re-check (R7a);
    /// `audio_intact` is the caller's answer, for a completed session, to
    /// whether its folder still holds every segment's audio (R6d).
    UserResume {
        space_ok: bool,
        audio_intact: bool,
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
    SaveFailed(String),
    Saved,
    /// A note edit or a regenerated summary on a saved session.
    SavedEdit,
    /// Launch sweep: this session was processing in a process that is gone.
    ClaimLost,
    /// The user's Transcribe Again on a saved session (R15, KTD14). The
    /// caller answers the guards: whether the folder still holds every
    /// segment's audio, whether a capture is active, and whether the
    /// session's summary is regenerated.
    UserTranscribeAgain {
        at: DateTime<FixedOffset>,
        audio_intact: bool,
        capture_active: bool,
        regenerating: bool,
    },
    /// A Transcribe Again run failed before its save. The session is saved
    /// again with its previous files (R19).
    TranscribeAgainFailed,
    /// The user's Record on a saved note with no audio (R11, KTD2).
    /// `has_segments` is the caller's answer to whether the note has
    /// segments. A note with segments uses [`Event::UserResume`].
    UserRecord {
        has_segments: bool,
    },
    /// The user stopped a recording that kept no segment: every segment
    /// was under a second and was dropped (R35). The note stays as a note
    /// with no audio (R19). The caller sends it only for a note that has
    /// no segments.
    RecordingDropped,
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
            SessionState::Saved => "saved",
            SessionState::Discarded => "discarded",
            SessionState::Importing => "importing",
        }
    }

    /// Whether the session has stopped and not completed: it holds a place
    /// in the processing lifecycle a resume takes it out of (R6b).
    pub fn is_stopped(&self) -> bool {
        matches!(
            self,
            SessionState::Queued { .. }
                | SessionState::Processing { .. }
                | SessionState::Blocked { .. }
                | SessionState::ProcessingFailed { .. }
                | SessionState::AwaitingDestination { .. }
        )
    }

    /// Whether an interrupted session has waited untouched long enough to be
    /// queued (R5e). A clock set backwards reads as no time elapsed rather
    /// than as a negative interval.
    pub fn auto_queue_due(&self, now: DateTime<FixedOffset>) -> bool {
        match self {
            SessionState::Interrupted { since, .. } => {
                now.signed_duration_since(*since)
                    .max(chrono::TimeDelta::zero())
                    > AUTO_QUEUE_AFTER
            }
            _ => false,
        }
    }

    pub fn is_final(&self) -> bool {
        matches!(self, SessionState::Saved | SessionState::Discarded)
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
            (S::Interrupted { reason, .. }, Event::UserResume { space_ok, .. })
                if *reason != R::Disk || *space_ok =>
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
            ) if self.auto_queue_due(*now) => {
                // Queued as of the moment the wait ran out, so FIFO order
                // reflects when the session stopped, not when it was noticed.
                Some(stopped(*since + AUTO_QUEUE_AFTER, *retained_audio))
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
            (S::Processing { stopped_at }, Event::SaveFailed(problem)) => {
                Some(S::AwaitingDestination {
                    stopped_at: *stopped_at,
                    problem: problem.clone(),
                })
            }
            (S::Processing { .. }, Event::Saved) => Some(S::Saved),
            (S::Processing { stopped_at }, Event::ClaimLost)
            | (S::Blocked { stopped_at }, Event::ClaimLost) => Some(S::Queued {
                stopped_at: *stopped_at,
            }),
            (S::Saved, Event::SavedEdit) => Some(S::Saved),
            (
                S::Saved,
                Event::UserTranscribeAgain {
                    at,
                    audio_intact,
                    capture_active,
                    regenerating,
                },
            ) if *audio_intact && !*capture_active && !*regenerating => {
                Some(S::Queued { stopped_at: *at })
            }
            (S::Processing { .. }, Event::TranscribeAgainFailed) => Some(S::Saved),
            (S::Saved, Event::UserRecord { has_segments }) if !*has_segments => Some(S::Capturing),
            (S::Capturing | S::Interrupted { .. }, Event::RecordingDropped) => Some(S::Saved),
            // The import worker stops the note with its converted segment.
            (
                S::Importing,
                Event::UserStop {
                    at,
                    retained_audio: true,
                },
            ) => Some(S::Queued { stopped_at: *at }),

            (_, Event::UserResume { .. }) if self.is_stopped() => Some(S::Capturing),
            (S::Saved, Event::UserResume { audio_intact, .. }) if *audio_intact => {
                Some(S::Capturing)
            }

            _ => None,
        };

        next.ok_or_else(|| IllegalTransition {
            state: self.name().to_string(),
            event: format!("{event:?}"),
        })
    }
}
