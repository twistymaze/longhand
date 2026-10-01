//! Every (state, event) pair against the lifecycle diagram: the drawn edges
//! succeed with the drawn target, and every other pair is rejected.

use chrono::{DateTime, FixedOffset, TimeDelta};
use session_core::{Event, InterruptReason, SessionState, Stage};

fn t(minutes: i64) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-22T10:00:00-05:00").unwrap() + TimeDelta::minutes(minutes)
}

fn interrupted(reason: InterruptReason) -> SessionState {
    SessionState::Interrupted {
        reason,
        since: t(0),
        acknowledged: false,
    }
}

fn states() -> Vec<SessionState> {
    vec![
        SessionState::Capturing,
        interrupted(InterruptReason::Device),
        interrupted(InterruptReason::Disk),
        interrupted(InterruptReason::Sleep),
        interrupted(InterruptReason::Crash),
        SessionState::Queued { stopped_at: t(0) },
        SessionState::Processing { stopped_at: t(0) },
        SessionState::Blocked { stopped_at: t(0) },
        SessionState::ProcessingFailed {
            stopped_at: t(0),
            stage: Stage::Transcribe,
        },
        SessionState::AwaitingDestination {
            stopped_at: t(0),
            problem: "unmounted".into(),
        },
        SessionState::Saved,
        SessionState::Discarded,
    ]
}

/// Resume with space re-checked and, for a completed session, its folder
/// holding every segment's audio.
fn resume_ok() -> Event {
    Event::UserResume {
        space_ok: true,
        audio_intact: true,
    }
}

fn events() -> Vec<Event> {
    vec![
        Event::AutoResumed,
        Event::CaptureLost {
            reason: InterruptReason::Device,
            at: t(0),
        },
        Event::DidWake,
        resume_ok(),
        Event::DeviceReturned,
        Event::UserStop {
            at: t(1),
            retained_audio: true,
        },
        Event::NewSessionStarted {
            at: t(1),
            retained_audio: true,
        },
        Event::Tick {
            now: t(31),
            retained_audio: true,
        },
        Event::Acknowledge,
        Event::Dequeued,
        Event::CredentialUnavailable,
        Event::StoreUnlocked,
        Event::StageFailed(Stage::Verify),
        Event::Retry,
        Event::SaveFailed("gone".into()),
        Event::Saved,
        Event::SavedEdit,
        Event::ClaimLost,
        transcribe_again_ok(),
        Event::TranscribeAgainFailed,
        record_ok(),
    ]
}

/// Record into a note that has no segments (KTD2).
fn record_ok() -> Event {
    Event::UserRecord {
        has_segments: false,
    }
}

/// Transcribe Again with the audio in the folder, no capture and no
/// summary regeneration (KTD14).
fn transcribe_again_ok() -> Event {
    Event::UserTranscribeAgain {
        at: t(5),
        audio_intact: true,
        capture_active: false,
        regenerating: false,
    }
}

/// The diagram, as (state name, event, target name). Anything absent is illegal.
fn expected(state: &SessionState, event: &Event) -> Option<&'static str> {
    use InterruptReason as R;
    use SessionState as S;
    let post_stop = matches!(
        state,
        S::Queued { .. }
            | S::Processing { .. }
            | S::Blocked { .. }
            | S::ProcessingFailed { .. }
            | S::AwaitingDestination { .. }
    );
    match (state, event) {
        (S::Capturing, Event::AutoResumed) => Some("capturing"),
        (S::Capturing, Event::CaptureLost { .. }) => Some("interrupted"),
        (S::Capturing, Event::UserStop { .. } | Event::NewSessionStarted { .. }) => Some("queued"),
        (
            S::Interrupted {
                reason: R::Sleep, ..
            },
            Event::DidWake,
        ) => Some("capturing"),
        (
            S::Interrupted {
                reason: R::Device, ..
            },
            Event::DeviceReturned,
        ) => Some("capturing"),
        (S::Interrupted { .. }, Event::UserResume { .. }) => Some("capturing"),
        (
            S::Interrupted { .. },
            Event::UserStop { .. } | Event::NewSessionStarted { .. } | Event::Tick { .. },
        ) => Some("queued"),
        (S::Interrupted { .. }, Event::Acknowledge) => Some("interrupted"),
        (S::Queued { .. }, Event::Dequeued) => Some("processing"),
        (S::Processing { .. }, Event::CredentialUnavailable) => Some("blocked"),
        (S::Blocked { .. }, Event::StoreUnlocked) => Some("processing"),
        (S::Processing { .. }, Event::StageFailed(_)) => Some("processing_failed"),
        (S::ProcessingFailed { .. } | S::AwaitingDestination { .. }, Event::Retry) => {
            Some("processing")
        }
        (S::Processing { .. }, Event::SaveFailed(_)) => Some("awaiting_destination"),
        (S::Processing { .. }, Event::Saved) => Some("saved"),
        (S::Processing { .. } | S::Blocked { .. }, Event::ClaimLost) => Some("queued"),
        (S::Saved, Event::SavedEdit) => Some("saved"),
        (S::Saved, Event::UserTranscribeAgain { .. }) => Some("queued"),
        (S::Processing { .. }, Event::TranscribeAgainFailed) => Some("saved"),
        (_, Event::UserResume { .. }) if post_stop => Some("capturing"),
        (S::Saved, Event::UserResume { .. }) => Some("capturing"),
        (S::Saved, Event::UserRecord { .. }) => Some("capturing"),
        _ => None,
    }
}

#[test]
fn every_state_event_pair_matches_the_diagram() {
    let mut legal = 0;
    for state in states() {
        for event in events() {
            let result = state.transition(&event);
            match expected(&state, &event) {
                Some(target) => {
                    let next =
                        result.unwrap_or_else(|e| panic!("{state:?} + {event:?} rejected: {e}"));
                    assert_eq!(next.name(), target, "{state:?} + {event:?}");
                    legal += 1;
                }
                None => assert!(
                    result.is_err(),
                    "{state:?} + {event:?} should be illegal, got {result:?}"
                ),
            }
        }
    }
    assert!(legal > 20, "only {legal} legal pairs exercised");
}

#[test]
fn a_completed_session_resumes_only_while_its_folder_holds_its_audio() {
    assert_eq!(
        SessionState::Saved.transition(&resume_ok()).unwrap(),
        SessionState::Capturing
    );
    let missing = Event::UserResume {
        space_ok: true,
        audio_intact: false,
    };
    assert!(SessionState::Saved.transition(&missing).is_err());
    assert!(SessionState::Discarded.transition(&resume_ok()).is_err());
}

#[test]
fn record_starts_capture_only_in_a_saved_note_with_no_segments() {
    assert_eq!(
        SessionState::Saved.transition(&record_ok()).unwrap(),
        SessionState::Capturing
    );
    let with_segments = Event::UserRecord { has_segments: true };
    assert!(SessionState::Saved.transition(&with_segments).is_err());
    // A note with no segments gets no Resume: the caller's audio check
    // gives `audio_intact: false` for it, and Resume refuses that (KTD2).
    let resume_no_audio = Event::UserResume {
        space_ok: true,
        audio_intact: false,
    };
    assert!(SessionState::Saved.transition(&resume_no_audio).is_err());
}

#[test]
fn transcribe_again_queues_a_saved_session_only_when_nothing_refuses_it() {
    assert_eq!(
        SessionState::Saved
            .transition(&transcribe_again_ok())
            .unwrap(),
        SessionState::Queued { stopped_at: t(5) }
    );
    for refused in [
        Event::UserTranscribeAgain {
            at: t(5),
            audio_intact: false,
            capture_active: false,
            regenerating: false,
        },
        Event::UserTranscribeAgain {
            at: t(5),
            audio_intact: true,
            capture_active: true,
            regenerating: false,
        },
        Event::UserTranscribeAgain {
            at: t(5),
            audio_intact: true,
            capture_active: false,
            regenerating: true,
        },
    ] {
        assert!(
            SessionState::Saved.transition(&refused).is_err(),
            "{refused:?}"
        );
    }
}

#[test]
fn disk_interruption_ignores_device_return_and_needs_space_to_resume() {
    let state = interrupted(InterruptReason::Disk);
    assert!(state.transition(&Event::DeviceReturned).is_err());
    let no_space = Event::UserResume {
        space_ok: false,
        audio_intact: true,
    };
    assert!(state.transition(&no_space).is_err());
    assert_eq!(
        state.transition(&resume_ok()).unwrap(),
        SessionState::Capturing
    );
}

#[test]
fn device_interruption_resumes_on_device_return() {
    let state = interrupted(InterruptReason::Device);
    assert_eq!(
        state.transition(&Event::DeviceReturned).unwrap(),
        SessionState::Capturing
    );
}

#[test]
fn untouched_interruption_is_queued_after_thirty_minutes() {
    let state = interrupted(InterruptReason::Device);
    let early = Event::Tick {
        now: t(29),
        retained_audio: true,
    };
    assert!(state.transition(&early).is_err());
    let late = Event::Tick {
        now: t(30) + TimeDelta::seconds(1),
        retained_audio: true,
    };
    assert_eq!(
        state.transition(&late).unwrap(),
        SessionState::Queued { stopped_at: t(30) }
    );
}

#[test]
fn stopping_with_no_retained_audio_discards() {
    let stop = Event::UserStop {
        at: t(1),
        retained_audio: false,
    };
    assert_eq!(
        SessionState::Capturing.transition(&stop).unwrap(),
        SessionState::Discarded
    );
}

#[test]
fn a_clock_set_backwards_does_not_queue_an_interruption() {
    let state = interrupted(InterruptReason::Device);
    let before = Event::Tick {
        now: t(-120),
        retained_audio: true,
    };
    assert!(state.transition(&before).is_err());
}

#[test]
fn acknowledging_keeps_the_interruption() {
    let next = interrupted(InterruptReason::Device)
        .transition(&Event::Acknowledge)
        .unwrap();
    assert!(matches!(
        next,
        SessionState::Interrupted {
            acknowledged: true,
            ..
        }
    ));
}
