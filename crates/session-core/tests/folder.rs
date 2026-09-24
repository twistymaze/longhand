use chrono::{DateTime, FixedOffset, TimeDelta};
use session_core::folder::{
    NoteLine, SessionDoc, Summary, SummaryMeta, Transcript, TranscriptLine, format_position,
    parse_notes, render_notes,
};
use session_core::{Anchor, Segment, SpeakerCount, Timeline};

fn wall(minutes: i64) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-22T09:00:00-05:00").unwrap() + TimeDelta::minutes(minutes)
}

#[test]
fn session_md_round_trips_the_timeline() {
    let segments = vec![
        Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 5_400_000,
            wall_start: wall(0),
            wall_end: wall(90),
            audio_file: "audio/segment-01.opus".into(),
        },
        Segment {
            index: 1,
            offset_ms: 5_640_000,
            duration_ms: 1_800_123,
            wall_start: wall(94),
            wall_end: wall(124),
            audio_file: "audio/segment-02.opus".into(),
        },
    ];
    let doc = SessionDoc::new(
        "01J8Z",
        Some(SpeakerCount::Range { min: 4, max: 6 }),
        &segments,
    )
    .unwrap();
    let text = doc.render();
    assert!(
        text.contains("Gap of 00:04:00 between segments 1 and 2."),
        "{text}"
    );
    let parsed = SessionDoc::parse(&text).unwrap();
    assert_eq!(parsed, doc);
    assert_eq!(
        Timeline::from_segments(parsed.timeline_segments()).unwrap(),
        Timeline::from_segments(segments).unwrap()
    );
}

#[test]
fn transcript_md_round_trips() {
    let transcript = Transcript {
        has_speakers: true,
        lines: vec![
            TranscriptLine::Turn {
                position_ms: 0,
                speaker: Some(1),
                text: "Shall we start?".into(),
            },
            TranscriptLine::Silence {
                duration_ms: 90_000,
            },
            TranscriptLine::Turn {
                position_ms: 95_000,
                speaker: Some(2),
                text: "Speaker 3: quoting someone".into(),
            },
            TranscriptLine::Gap {
                duration_ms: 252_000,
            },
            TranscriptLine::Turn {
                position_ms: 5_652_000,
                speaker: Some(1),
                text: "We're back.".into(),
            },
        ],
    };
    let text = transcript.render();
    assert!(
        text.contains("- [01:34:12] Speaker 1: We're back."),
        "{text}"
    );
    assert!(text.contains("--- gap 00:04:12 ---"));
    assert_eq!(Transcript::parse(&text).unwrap(), transcript);
}

#[test]
fn transcript_without_speakers_says_so_and_round_trips() {
    let transcript = Transcript {
        has_speakers: false,
        lines: vec![TranscriptLine::Turn {
            position_ms: 3_000,
            speaker: None,
            text: "Speaker 2: is literally what was said".into(),
        }],
    };
    let text = transcript.render();
    assert!(text.contains("no speaker marks"));
    assert_eq!(Transcript::parse(&text).unwrap(), transcript);
}

#[test]
fn notes_md_round_trips() {
    let notes = vec![
        NoteLine {
            anchor: Anchor::Unanchored,
            text: "agenda: budget".into(),
        },
        NoteLine {
            anchor: Anchor::At {
                position_ms: 95_000,
                gap_anchored: false,
            },
            text: "decision: ship Friday".into(),
        },
        NoteLine {
            anchor: Anchor::At {
                position_ms: 5_400_000,
                gap_anchored: true,
            },
            text: "[bracketed] thought".into(),
        },
    ];
    let text = render_notes(&notes);
    assert!(
        text.contains("- [01:30:00 ~gap] [bracketed] thought"),
        "{text}"
    );
    assert_eq!(parse_notes(&text).unwrap(), notes);
}

#[test]
fn summary_md_round_trips_both_forms_and_never_holds_a_credential() {
    for summary in [
        Summary {
            meta: SummaryMeta::Complete {
                endpoint_host: "litellm.example.com".into(),
                model: "gpt-5".into(),
            },
            body: "## Decisions\n\n- Ship Friday.".into(),
        },
        Summary {
            meta: SummaryMeta::Stub {
                failed_stage: "summarize".into(),
            },
            body: "Summarization failed. Regenerate to try again.".into(),
        },
    ] {
        let text = summary.render();
        assert!(!text.to_lowercase().contains("key"));
        assert_eq!(Summary::parse(&text).unwrap(), summary);
    }
}

#[test]
fn positions_render_past_ninety_nine_minutes() {
    assert_eq!(format_position(4 * 3_600_000 + 61_000), "04:01:01");
}
