use chrono::{DateTime, FixedOffset, TimeDelta};
use session_core::folder::{
    AudioState, PendingType, REMOVAL_MARKER_TEXT, ResultFlag, SessionDoc, Summary, SummaryMeta,
    Transcript, TranscriptLine, audio_removed, audio_state, fingerprint, format_position,
    removal_marker, render_notes, session_fingerprint,
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
            ends_in_pause: false,
        },
        Segment {
            index: 1,
            offset_ms: 5_640_000,
            duration_ms: 1_800_123,
            wall_start: wall(94),
            wall_end: wall(124),
            audio_file: "audio/segment-02.opus".into(),
            channels: 1,
            ends_in_pause: false,
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
fn a_pause_line_round_trips_and_differs_from_a_gap_line() {
    let transcript = Transcript {
        has_speakers: true,
        lines: vec![
            TranscriptLine::Turn {
                position_ms: 0,
                speaker: Some(1),
                text: "Before the break.".into(),
            },
            TranscriptLine::Pause {
                duration_ms: 900_000,
            },
            TranscriptLine::Turn {
                position_ms: 3_900_000,
                speaker: Some(1),
                text: "After the break.".into(),
            },
            TranscriptLine::Gap {
                duration_ms: 60_000,
            },
        ],
    };
    let text = transcript.render();
    assert!(text.contains("\n--- pause 00:15:00 ---\n"), "{text}");
    assert!(text.contains("\n--- gap 00:01:00 ---\n"), "{text}");
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
        ends_in_pause: false,
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
                prompt_id: Some("b1".into()),
            },
            body: "## Decisions\n\n- Ship Friday.".into(),
        },
        Summary {
            meta: SummaryMeta::Complete {
                endpoint_host: "litellm.example.com".into(),
                model: "gpt-5".into(),
                prompt_name: None,
                prompt_id: None,
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
fn a_summary_md_with_no_prompt_name_or_id_parses_and_renders_again_without_one() {
    let older = "---\nstatus: complete\nendpoint_host: litellm.example.com\nmodel: gpt-5\n---\n\n## Decisions\n";
    let summary = Summary::parse(older).unwrap();
    assert_eq!(
        summary.meta,
        SummaryMeta::Complete {
            endpoint_host: "litellm.example.com".into(),
            model: "gpt-5".into(),
            prompt_name: None,
            prompt_id: None,
        }
    );
    assert_eq!(summary.render(), older);
}

#[test]
fn a_summary_md_records_the_prompt_name_and_id_in_its_frontmatter() {
    let summary = Summary {
        meta: SummaryMeta::Complete {
            endpoint_host: "litellm.example.com".into(),
            model: "gpt-5".into(),
            prompt_name: Some("Team sync".into()),
            prompt_id: Some("b1".into()),
        },
        body: "## Decisions".into(),
    };
    let text = summary.render();
    assert!(text.contains("\nprompt_name: Team sync\n"), "{text}");
    assert!(text.contains("\nprompt_id: b1\n"), "{text}");
}

#[test]
fn a_stub_renders_with_no_prompt_name_or_id() {
    let stub = Summary {
        meta: SummaryMeta::Stub {
            failed_stage: "summarize".into(),
        },
        body: "No summary was produced.".into(),
    };
    assert!(!stub.render().contains("prompt_"));
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
        ends_in_pause: false,
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
    assert!(!Segment::from(&doc.segments[0]).ends_in_pause);
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
        ends_in_pause: false,
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

#[test]
fn speaker_recognition_off_is_not_reported_as_failure() {
    let transcript = Transcript {
        has_speakers: false,
        lines: vec![TranscriptLine::Turn {
            position_ms: 0,
            speaker: None,
            text: "A selected mode is not an error.".into(),
        }],
    };
    let rendered = transcript.render_speaker_off();
    assert!(rendered.contains("Speaker recognition was off"));
    assert!(!rendered.contains("failed"));
    assert_eq!(Transcript::parse(&rendered).unwrap(), transcript);
    assert!(transcript.render().contains("failed"));
}

// ---------------------------------------------------------------- removal markers (U2)

/// A note folder with the audio files `present` and removal markers for
/// the audio files `marked`.
fn audio_folder(present: &[&str], marked: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("audio")).unwrap();
    for audio in present {
        std::fs::write(dir.path().join(audio), b"opus").unwrap();
    }
    for audio in marked {
        std::fs::write(dir.path().join(removal_marker(audio)), REMOVAL_MARKER_TEXT).unwrap();
    }
    dir
}

const ONE: &str = "audio/segment-01.opus";
const TWO: &str = "audio/segment-02.opus";

#[test]
fn the_removal_marker_is_beside_its_audio_file_and_has_no_leading_dot() {
    assert_eq!(removal_marker(ONE), "audio/segment-01.opus.removed");
    let marker = removal_marker(ONE);
    let name = std::path::Path::new(&marker)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(!name.starts_with('.'));
    assert!(!name.ends_with(".tmp"));
}

#[test]
fn a_segment_with_a_marker_is_removed_with_or_without_its_audio() {
    let gone = audio_folder(&[], &[ONE]);
    assert!(audio_removed(gone.path(), ONE));
    let kept = audio_folder(&[ONE], &[ONE]);
    assert!(audio_removed(kept.path(), ONE));
    let plain = audio_folder(&[ONE], &[]);
    assert!(!audio_removed(plain.path(), ONE));
}

#[test]
fn a_marker_with_any_content_counts() {
    let dir = audio_folder(&[], &[]);
    std::fs::write(dir.path().join(removal_marker(ONE)), b"").unwrap();
    assert!(audio_removed(dir.path(), ONE));
}

#[test]
fn every_audio_file_present_and_no_marker_is_present() {
    let dir = audio_folder(&[ONE, TWO], &[]);
    assert_eq!(audio_state(dir.path(), [ONE, TWO]), AudioState::Present);
}

#[test]
fn a_marker_with_the_audio_still_present_is_removed() {
    let dir = audio_folder(&[ONE, TWO], &[ONE]);
    assert_eq!(audio_state(dir.path(), [ONE, TWO]), AudioState::Removed);
}

#[test]
fn a_missing_audio_file_with_no_marker_is_waiting() {
    let dir = audio_folder(&[ONE], &[]);
    assert_eq!(audio_state(dir.path(), [ONE, TWO]), AudioState::Waiting);
}

#[test]
fn a_marked_segment_wins_over_a_missing_unmarked_segment() {
    let dir = audio_folder(&[], &[ONE]);
    assert_eq!(audio_state(dir.path(), [ONE, TWO]), AudioState::Removed);
    let dir = audio_folder(&[], &[TWO]);
    assert_eq!(audio_state(dir.path(), [ONE, TWO]), AudioState::Removed);
}

#[test]
fn a_folder_that_does_not_exist_is_waiting() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        audio_state(&dir.path().join("gone"), [ONE]),
        AudioState::Waiting
    );
}

// ---------------------------------------------------------------- notes with no audio

#[test]
fn a_note_with_no_audio_round_trips_with_its_open_time_and_the_result_flag() {
    let doc = SessionDoc::without_audio("01J8Z", wall(0), None);
    let text = doc.render();
    assert!(
        text.contains("started: 2026-09-22T09:00:00-05:00"),
        "{text}"
    );
    assert!(text.contains("segments: []"), "{text}");
    assert!(text.contains("result: none"), "{text}");
    let parsed = SessionDoc::parse(&text).unwrap();
    assert_eq!(parsed, doc);
    assert_eq!(parsed.result, Some(ResultFlag::NoRecording));
    assert!(parsed.segments.is_empty());
}

#[test]
fn a_recording_not_saved_round_trips_as_pending() {
    let mut doc = SessionDoc::without_audio("01J8Z", wall(0), None);
    doc.result = Some(ResultFlag::NotSaved);
    let text = doc.render();
    assert!(text.contains("result: pending"), "{text}");
    assert_eq!(SessionDoc::parse(&text).unwrap(), doc);
}

#[test]
fn a_note_with_a_result_writes_no_result_field_and_an_older_document_parses() {
    let doc = SessionDoc::new("01J8Z", None, &one_segment(1)).unwrap();
    assert_eq!(doc.result, None);
    assert!(!doc.render().contains("result:"), "{}", doc.render());
    // An older build wrote no `result` field, and can write no `segments`.
    let text = "---\nsession_id: 01J8Z\nstarted: 2026-09-22T09:00:00-05:00\n---\n\n# Old note\n";
    let parsed = SessionDoc::parse(text).unwrap();
    assert_eq!(parsed.result, None);
    assert!(parsed.segments.is_empty());
}

#[test]
fn a_missing_result_field_reads_as_no_recording_only_for_a_note_with_nothing() {
    // KTD1: an older build that rewrites `session.md` drops the field.
    let mut empty = SessionDoc::without_audio("01J8Z", wall(0), None);
    empty.result = None;
    assert_eq!(empty.result_flag(), Some(ResultFlag::NoRecording));

    let mut transcribed = empty.clone();
    transcribed.transcript_fingerprint = Some(fingerprint(b"transcript"));
    assert_eq!(transcribed.result_flag(), None);
    let mut summarized = empty.clone();
    summarized.summary_fingerprint = Some(fingerprint(b"summary"));
    assert_eq!(summarized.result_flag(), None);

    let recorded = SessionDoc::new("01J8Z", None, &one_segment(1)).unwrap();
    assert_eq!(recorded.result_flag(), None);

    let mut pending = SessionDoc::new("01J8Z", None, &one_segment(1)).unwrap();
    pending.result = Some(ResultFlag::NotSaved);
    assert_eq!(pending.result_flag(), Some(ResultFlag::NotSaved));
}

#[test]
fn a_note_opened_before_it_records_keeps_its_open_time() {
    // Covers AE12: opened at 09:00, first segment at 10:00.
    let mut segments = one_segment(1);
    segments[0].wall_start = wall(60);
    segments[0].wall_end = wall(61);
    let doc = SessionDoc::at("01J8Z", wall(0), None, &segments);
    let text = doc.render();
    assert!(
        text.contains("started: 2026-09-22T09:00:00-05:00"),
        "{text}"
    );
    assert!(text.contains("Started 2026-09-22 09:00."), "{text}");
    assert_eq!(doc.result, None);
    assert_eq!(SessionDoc::parse(&text).unwrap(), doc);
}

// ---------------------------------------------------------------- user fields (KTD2)

fn pending_type() -> PendingType {
    PendingType {
        to: Some("One on One".into()),
        from: None,
        by: "computer-b".into(),
        at: wall(5),
    }
}

#[test]
fn session_md_round_trips_the_tags_and_the_pending_type() {
    let mut doc = SessionDoc::new("01J8Z", None, &one_segment(1)).unwrap();
    doc.tags = vec!["Person::Nathan".into(), "follow up".into()];
    doc.pending_type = Some(pending_type());
    let text = doc.render();
    assert!(text.contains("longhand_tags:"), "{text}");
    assert!(text.contains("longhand_pending_type:"), "{text}");
    assert!(!text.contains("\ntags:"), "{text}");
    let parsed = SessionDoc::parse(&text).unwrap();
    assert_eq!(parsed, doc);
    assert_eq!(parsed.pending_type.unwrap().at, wall(5));
}

#[test]
fn a_note_with_no_tags_and_no_pending_type_renders_no_new_keys() {
    let doc = SessionDoc::new("01J8Z", None, &one_segment(1)).unwrap();
    let text = doc.render();
    assert!(!text.contains("longhand_"), "{text}");
    let parsed = SessionDoc::parse(&text).unwrap();
    assert!(parsed.tags.is_empty());
    assert_eq!(parsed.pending_type, None);
}

#[test]
fn a_pending_change_to_no_type_round_trips() {
    let mut doc = SessionDoc::new("01J8Z", None, &one_segment(1)).unwrap();
    doc.pending_type = Some(PendingType {
        to: None,
        from: Some("Pathfinder".into()),
        ..pending_type()
    });
    assert_eq!(SessionDoc::parse(&doc.render()).unwrap(), doc);
}

#[test]
fn the_session_fingerprint_ignores_the_user_fields() {
    // Keys after the user fields must keep their order.
    let mut doc = SessionDoc::new("01J8Z", Some(SpeakerCount::Exact(2)), &one_segment(1)).unwrap();
    doc.asr_model = Some("parakeet-tdt-0.6b-v3".into());
    doc.found_speakers = Some(2);
    doc.transcript_fingerprint = Some(fingerprint(b"transcript"));
    doc.summary_fingerprint = Some(fingerprint(b"summary"));
    let plain = doc.render();
    let mut tagged = doc.clone();
    tagged.tags = vec!["Topic::Security".into()];
    tagged.pending_type = Some(pending_type());
    let tagged = tagged.render();
    assert_ne!(
        fingerprint(plain.as_bytes()),
        fingerprint(tagged.as_bytes())
    );
    assert_eq!(
        session_fingerprint(plain.as_bytes()),
        session_fingerprint(tagged.as_bytes())
    );
}

#[test]
fn the_session_fingerprint_sees_other_changes() {
    let doc = SessionDoc::new("01J8Z", None, &one_segment(1)).unwrap();
    let plain = doc.render();
    let mut titled = doc.clone();
    titled.title = Some("Budget".into());
    assert_ne!(
        session_fingerprint(plain.as_bytes()),
        session_fingerprint(titled.render().as_bytes())
    );
    let edited = format!("{plain}\nA line that a person added.\n");
    assert_ne!(
        session_fingerprint(plain.as_bytes()),
        session_fingerprint(edited.as_bytes())
    );
}

#[test]
fn the_session_fingerprint_of_bytes_with_no_frontmatter_is_their_fingerprint() {
    for bytes in [
        &b"no frontmatter"[..],
        &[0xff, 0xfe, 0x00][..],
        b"---\n- a\n---\n",
    ] {
        assert_eq!(session_fingerprint(bytes), fingerprint(bytes));
    }
}

#[test]
fn a_session_md_with_no_pause_renders_as_before() {
    // KTD4 of the pause plan: a note with no pause writes no new key.
    let doc = SessionDoc::new("01J8Z", None, &one_segment(1)).unwrap();
    assert_eq!(
        doc.render(),
        "---\nsession_id: 01J8Z\nstarted: 2026-09-22T09:00:00-05:00\nsegments:\n- index: 0\n  \
         wall_start: 2026-09-22T09:00:00-05:00\n  wall_end: 2026-09-22T09:01:00-05:00\n  \
         offset_ms: 0\n  duration_ms: 60000\n  audio: audio/segment-01.opus\n---\n\n\
         # Longhand session\n\nStarted 2026-09-22 09:00.\n\n\
         | Segment | Starts | Ends | At | Length | Audio |\n|---|---|---|---|---|---|\n\
         | 1 | 09:00:00 | 09:01:00 | 00:00:00 | 00:01:00 | \
         [audio/segment-01.opus](audio/segment-01.opus) |\n"
    );
}

#[test]
fn a_paused_segment_round_trips_through_session_md() {
    let mut segments = one_segment(1);
    segments[0].ends_in_pause = true;
    segments.push(Segment {
        index: 1,
        offset_ms: 120_000,
        duration_ms: 60_000,
        wall_start: wall(17),
        wall_end: wall(18),
        audio_file: "audio/segment-02.opus".into(),
        channels: 1,
        ends_in_pause: false,
    });
    let doc = SessionDoc::new("01J8Z", None, &segments).unwrap();
    let text = doc.render();
    assert_eq!(text.matches("ends_in_pause: true").count(), 1, "{text}");
    let parsed = SessionDoc::parse(&text).unwrap();
    assert_eq!(parsed, doc);
    let flags: Vec<bool> = parsed
        .segments
        .iter()
        .map(|s| Segment::from(s).ends_in_pause)
        .collect();
    assert_eq!(flags, [true, false]);
}
