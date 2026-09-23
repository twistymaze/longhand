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
        SessionState::Landed,
        SessionState::Discarded,
    ]
}

/// Resume inside the window, most recent, space re-checked.
fn resume_ok() -> Event {
    Event::UserResume {
        now: t(10),
        is_most_recent: true,
        space_ok: true,
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
        Event::LandingFailed("gone".into()),
        Event::Landed,
        Event::LandedEdit,
        Event::ClaimLost,
    ]
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
        (S::Processing { .. }, Event::LandingFailed(_)) => Some("awaiting_destination"),
        (S::Processing { .. }, Event::Landed) => Some("landed"),
        (S::Processing { .. } | S::Blocked { .. }, Event::ClaimLost) => Some("queued"),
        (S::Landed, Event::LandedEdit) => Some("landed"),
        (_, Event::UserResume { .. }) if post_stop => Some("capturing"),
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
fn landed_cannot_resume() {
    assert!(SessionState::Landed.transition(&resume_ok()).is_err());
}

#[test]
fn resume_after_the_window_is_illegal() {
    let state = SessionState::Processing { stopped_at: t(0) };
    let late = Event::UserResume {
        now: t(31),
        is_most_recent: true,
        space_ok: true,
    };
    assert!(state.transition(&late).is_err());
}

#[test]
fn resume_of_an_older_session_is_illegal() {
    let state = SessionState::Processing { stopped_at: t(0) };
    let older = Event::UserResume {
        now: t(10),
        is_most_recent: false,
        space_ok: true,
    };
    assert!(state.transition(&older).is_err());
}

#[test]
fn disk_interruption_ignores_device_return_and_needs_space_to_resume() {
    let state = interrupted(InterruptReason::Disk);
    assert!(state.transition(&Event::DeviceReturned).is_err());
    let no_space = Event::UserResume {
        now: t(5),
        is_most_recent: true,
        space_ok: false,
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
fn untouched_interruption_is_queued_when_the_window_elapses() {
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
fn a_clock_set_backwards_does_not_close_the_window() {
    let state = SessionState::Queued { stopped_at: t(0) };
    let before = Event::UserResume {
        now: t(-120),
        is_most_recent: true,
        space_ok: true,
    };
    assert_eq!(state.transition(&before).unwrap(), SessionState::Capturing);
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
