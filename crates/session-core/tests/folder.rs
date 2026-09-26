use chrono::{DateTime, FixedOffset, TimeDelta};
use session_core::folder::{
    SessionDoc, Summary, SummaryMeta, Transcript, TranscriptLine, fingerprint, format_position,
    render_notes,
};
use session_core::{Segment, SpeakerCount, Timeline};

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
            channels: 1,
        },
        Segment {
            index: 1,
            offset_ms: 5_640_000,
            duration_ms: 1_800_123,
            wall_start: wall(94),
            wall_end: wall(124),
            audio_file: "audio/segment-02.opus".into(),
            channels: 1,
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
fn session_md_round_trips_with_a_title() {
    let segments = vec![Segment {
        index: 0,
        offset_ms: 0,
        duration_ms: 60_000,
        wall_start: wall(0),
        wall_end: wall(1),
        audio_file: "audio/segment-01.opus".into(),
        channels: 1,
    }];
    let mut doc = SessionDoc::new("01J8Z", None, &segments).unwrap();
    assert_eq!(doc.title, None);
    let untitled = doc.render();
    assert!(!untitled.contains("title:"), "{untitled}");
    assert_eq!(SessionDoc::parse(&untitled).unwrap(), doc);

    doc.title = Some("Budget review: Q3 \"plan\"".into());
    let text = doc.render();
    assert!(text.contains("# Budget review: Q3 \"plan\""), "{text}");
    assert_eq!(SessionDoc::parse(&text).unwrap(), doc);
}

#[test]
fn notes_md_is_the_notes_document_byte_for_byte() {
    let document = "# Agenda\n\n## Budget\n\n- [ ] ask about Q3\n- [x] confirm date\n\n\
                    ```rust\nfn main() {}\n```\n\n- [00:12:40] an earlier note\n\ttabbed  text \n";
    assert_eq!(render_notes(document), document);
    assert_eq!(render_notes(""), "");
}

#[test]
fn summary_md_round_trips_both_forms_and_never_holds_a_credential() {
    for summary in [
        Summary {
            meta: SummaryMeta::Complete {
                endpoint_host: "litellm.example.com".into(),
                model: "gpt-5".into(),
                prompt_name: Some("Team sync".into()),
            },
            body: "## Decisions\n\n- Ship Friday.".into(),
        },
        Summary {
            meta: SummaryMeta::Complete {
                endpoint_host: "litellm.example.com".into(),
                model: "gpt-5".into(),
                prompt_name: None,
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
fn a_summary_md_with_no_prompt_name_parses_and_renders_again_without_one() {
    let older = "---\nstatus: complete\nendpoint_host: litellm.example.com\nmodel: gpt-5\n---\n\n## Decisions\n";
    let summary = Summary::parse(older).unwrap();
    assert_eq!(
        summary.meta,
        SummaryMeta::Complete {
            endpoint_host: "litellm.example.com".into(),
            model: "gpt-5".into(),
            prompt_name: None,
        }
    );
    assert_eq!(summary.render(), older);
}

#[test]
fn a_summary_md_records_the_prompt_name_in_its_frontmatter() {
    let summary = Summary {
        meta: SummaryMeta::Complete {
            endpoint_host: "litellm.example.com".into(),
            model: "gpt-5".into(),
            prompt_name: Some("Team sync".into()),
        },
        body: "## Decisions".into(),
    };
    let text = summary.render();
    assert!(text.contains("\nprompt_name: Team sync\n"), "{text}");
}

#[test]
fn a_stub_renders_with_no_prompt_name() {
    let stub = Summary {
        meta: SummaryMeta::Stub {
            failed_stage: "summarize".into(),
        },
        body: "No summary was produced.".into(),
    };
    assert!(!stub.render().contains("prompt_name"));
}

#[test]
fn positions_render_past_ninety_nine_minutes() {
    assert_eq!(format_position(4 * 3_600_000 + 61_000), "04:01:01");
}

fn one_segment(channels: u16) -> Vec<Segment> {
    vec![Segment {
        index: 0,
        offset_ms: 0,
        duration_ms: 60_000,
        wall_start: wall(0),
        wall_end: wall(1),
        audio_file: "audio/segment-01.opus".into(),
        channels,
    }]
}

#[test]
fn session_md_round_trips_the_complete_record() {
    use session_core::{Title, TitleSource};
    let mut doc = SessionDoc::new("01J8Z", None, &one_segment(1)).unwrap();
    doc.title = Some("Budget review".into());
    doc.title_source = Some(TitleSource::Proposed);
    doc.asr_model = Some("parakeet-tdt-0.6b-v3".into());
    doc.found_speakers = Some(3);
    doc.transcript_fingerprint = Some(fingerprint(b"transcript"));
    doc.summary_fingerprint = Some(fingerprint(b"summary"));
    let text = doc.render();
    assert!(text.contains("title_source: proposed"), "{text}");
    let parsed = SessionDoc::parse(&text).unwrap();
    assert_eq!(parsed, doc);
    assert_eq!(
        parsed.title_and_source(),
        Some(Title {
            text: "Budget review".into(),
            source: TitleSource::Proposed
        })
    );
}

#[test]
fn a_session_md_from_before_the_new_fields_parses_and_its_title_is_the_users() {
    use session_core::{Title, TitleSource};
    let text = "---\nsession_id: 01J8Z\nstarted: 2026-09-22T09:00:00-05:00\ntitle: Old note\n\
                segments:\n- index: 0\n  wall_start: 2026-09-22T09:00:00-05:00\n  \
                wall_end: 2026-09-22T09:01:00-05:00\n  offset_ms: 0\n  duration_ms: 60000\n  \
                audio: audio/segment-01.opus\n---\n\n# Old note\n";
    let doc = SessionDoc::parse(text).unwrap();
    assert_eq!(doc.title_source, None);
    assert_eq!(doc.asr_model, None);
    assert_eq!(doc.found_speakers, None);
    assert_eq!(doc.transcript_fingerprint, None);
    assert_eq!(doc.summary_fingerprint, None);
    assert_eq!(
        doc.title_and_source(),
        Some(Title {
            text: "Old note".into(),
            source: TitleSource::User
        })
    );
    let untitled = SessionDoc::new("01J8Z", None, &one_segment(1)).unwrap();
    assert_eq!(untitled.title_and_source(), None);
}

#[test]
fn a_session_md_with_crlf_line_ends_parses() {
    let mut doc = SessionDoc::new("01J8Z", None, &one_segment(1)).unwrap();
    doc.title = Some("Windows".into());
    doc.found_speakers = Some(2);
    let text = doc.render().replace('\n', "\r\n");
    assert_eq!(SessionDoc::parse(&text).unwrap(), doc);

    let summary = Summary {
        meta: SummaryMeta::Stub {
            failed_stage: "summarize".into(),
        },
        body: "# Summary".into(),
    };
    let text = summary.render().replace('\n', "\r\n");
    assert_eq!(Summary::parse(&text).unwrap(), summary);
}

#[test]
fn a_stereo_segment_keeps_two_channels_and_gives_stereo_from() {
    let mut segments = one_segment(1);
    segments.push(Segment {
        index: 1,
        offset_ms: 120_000,
        duration_ms: 60_000,
        wall_start: wall(2),
        wall_end: wall(3),
        audio_file: "audio/segment-02.opus".into(),
        channels: 2,
    });
    let doc = SessionDoc::new("01J8Z", None, &segments).unwrap();
    let parsed = SessionDoc::parse(&doc.render()).unwrap();
    assert_eq!(parsed.segments[1].channels, 2);
    assert_eq!(parsed.stereo_from(), Some(1));
    let mono = SessionDoc::new("01J8Z", None, &one_segment(1)).unwrap();
    assert_eq!(mono.stereo_from(), None);
}

#[test]
fn a_fingerprint_is_the_sha256_of_the_bytes() {
    assert_eq!(
        fingerprint(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
