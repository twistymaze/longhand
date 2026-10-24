use asr::{
    Finality, RecognitionId, RecognitionSegment, SampleRange, SpeakerKey, SpeakerScope,
    SpeakerTurn, TimingPrecision,
};
use models::recognition::{
    AsrRuntime, RecognitionConfig, SpeakerSource, resolve, resolve_for_runtime,
};
use pipeline::recognition::{Route, RouteResult, TaskGraph, join_separate, validate_result};

fn row() -> RecognitionSegment {
    RecognitionSegment {
        id: RecognitionId {
            scope: SpeakerScope {
                generation: "g".into(),
                epoch: 1,
            },
            input_start_sample: 0,
            ordinal: 0,
        },
        range: SampleRange {
            start: 0,
            end: 16000,
        },
        timing: TimingPrecision::NativeSegment,
        text: "Hello".into(),
        speaker: None,
        revision: 1,
        text_finality: Finality::Final,
        speaker_finality: Finality::Final,
    }
}
fn segment(
    index: u32,
    offset_ms: u64,
    duration_ms: u64,
    ends_in_pause: bool,
) -> session_core::Segment {
    let at = chrono::DateTime::parse_from_rfc3339("2026-10-10T07:00:00+00:00").unwrap();
    session_core::Segment {
        index,
        offset_ms,
        duration_ms,
        wall_start: at,
        wall_end: at,
        audio_file: format!("audio/{index}.wav"),
        channels: 1,
        ends_in_pause,
    }
}
fn row_at(start_ms: u64, text: &str) -> RecognitionSegment {
    let mut r = row();
    r.range = SampleRange {
        start: start_ms * 16,
        end: (start_ms + 1000) * 16,
    };
    r.text = text.into();
    r
}
#[test]
fn a_recognition_transcript_has_a_line_between_segments() {
    use session_core::folder::TranscriptLine;
    let result = RouteResult {
        route: Route::Off,
        coverage: vec![SampleRange {
            start: 0,
            end: 80_000 * 16,
        }],
        rows: vec![
            row_at(1_000, "before the pause"),
            row_at(41_000, "after the pause"),
            row_at(71_000, "after the resume"),
        ],
        speakers: vec![],
        coverage_limitations: vec![],
    };
    // A pause after segment 0, a Stop and Resume after segment 1, and a
    // pause after segment 2 with no text after it.
    let segments = [
        segment(0, 0, 24_000, true),
        segment(1, 40_000, 20_000, false),
        segment(2, 70_000, 5_000, true),
        segment(3, 78_000, 1_000, false),
    ];
    let lines = pipeline::recognition::transcript(&result, &segments).lines;
    let got: Vec<String> = lines
        .iter()
        .map(|l| match l {
            TranscriptLine::Turn { text, .. } => text.clone(),
            TranscriptLine::Pause { duration_ms } => format!("pause {duration_ms}"),
            TranscriptLine::Gap { duration_ms } => format!("gap {duration_ms}"),
            TranscriptLine::Silence { .. } => "silence".into(),
            TranscriptLine::Text { text } => format!("text {text}"),
        })
        .collect();
    assert_eq!(
        got,
        [
            "before the pause",
            "pause 16000",
            "after the pause",
            "gap 10000",
            "after the resume",
            "pause 3000",
        ]
    );
}
// A note seeded from its transcript.md keeps the paragraphs of its old part
// when the recognition route writes the transcript again. The rows come
// from the seeded windows, as in the transcribe step of the route.
#[test]
fn a_seeded_recognition_transcript_keeps_the_old_paragraphs() {
    use session_core::folder::TranscriptLine;
    let mut store = session_core::Store::open_in_memory().unwrap();
    let started = chrono::DateTime::parse_from_rfc3339("2026-10-10T07:00:00+00:00").unwrap();
    store.create_session("n", started, None).unwrap();
    store
        .put_segment("n", &segment(0, 0, 120_000, false))
        .unwrap();
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("transcript.md"),
        "# Transcript\n\n- [00:00:01] A.\n- [00:00:05] B.\n",
    )
    .unwrap();
    assert!(pipeline::transcribe::seed_from_transcript(&store, "n", folder.path()).unwrap());
    let store = std::sync::Arc::new(std::sync::Mutex::new(store));
    let rows = pipeline::transcribe::results(&store, "n")
        .unwrap()
        .into_iter()
        .flat_map(|window| {
            window
                .segments
                .into_iter()
                .map(move |s| {
                    let mut r = row();
                    r.range = SampleRange {
                        start: (window.start_ms + s.start_ms) * 16,
                        end: (window.start_ms + s.end_ms) * 16,
                    };
                    r.text = s.text;
                    r
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let result = RouteResult {
        route: Route::Off,
        coverage: vec![SampleRange {
            start: 0,
            end: 120_000 * 16,
        }],
        rows,
        speakers: vec![],
        coverage_limitations: vec![],
    };
    let lines = pipeline::recognition::transcript(&result, &[segment(0, 0, 120_000, false)]).lines;
    let got: Vec<(u64, String)> = lines
        .into_iter()
        .filter_map(|l| match l {
            TranscriptLine::Turn {
                position_ms, text, ..
            } => Some((position_ms, text)),
            _ => None,
        })
        .collect();
    assert_eq!(got, [(1_000, "A.".into()), (5_000, "B.".into())]);
}
fn turn(start: u64, end: u64, id: u32) -> SpeakerTurn {
    SpeakerTurn {
        range: SampleRange { start, end },
        timing: TimingPrecision::NativeSegment,
        speaker: SpeakerKey {
            scope: SpeakerScope {
                generation: "g".into(),
                epoch: 2,
            },
            input_start_sample: 0,
            local_id: id,
        },
        finality: Finality::Final,
    }
}
#[test]
fn only_off_has_no_separate_task() {
    for (source, route) in [
        (SpeakerSource::Off, Route::Off),
        (SpeakerSource::Pyannote, Route::SeparatePyannote),
        (SpeakerSource::Nemotron, Route::SeparateNemo),
    ] {
        let spec = resolve(&RecognitionConfig {
            asr_model: "whisper-large-v3-turbo-q5km".into(),
            speaker_source: source,
            ..Default::default()
        })
        .unwrap();
        let graph = TaskGraph::resolve(&spec).unwrap();
        assert_eq!(graph.separate_analysis, source != SpeakerSource::Off);
        assert_eq!(graph.route, route);
    }
}
#[test]
fn both_nemo_routes_are_scheduled() {
    let config = RecognitionConfig {
        asr_model: "nemotron-speech-streaming-en-0.6b-q80".into(),
        speaker_source: SpeakerSource::Nemotron,
        ..Default::default()
    };
    assert_eq!(
        TaskGraph::resolve(&resolve(&config).unwrap())
            .unwrap()
            .route,
        Route::SeparateNemo
    );
    assert_eq!(
        TaskGraph::resolve(&resolve_for_runtime(&config, AsrRuntime::Nemo).unwrap())
            .unwrap()
            .route,
        Route::CombinedNemo
    );
}
#[test]
fn source_window_timing_and_overlap_ambiguity_remain_explicit() {
    let mut rows = vec![row()];
    rows[0].timing = TimingPrecision::SourceWindow;
    join_separate(&mut rows, &[turn(0, 16000, 1)]);
    assert!(rows[0].speaker.is_none());
    rows[0].timing = TimingPrecision::NativeSegment;
    join_separate(&mut rows, &[turn(0, 16000, 1), turn(0, 16000, 2)]);
    assert!(rows[0].speaker.is_none());
    join_separate(&mut rows, &[turn(0, 12000, 1), turn(12000, 16000, 2)]);
    assert_eq!(rows[0].speaker.as_ref().unwrap().local_id, 1);
}
#[test]
fn completion_requires_processed_coverage_and_final_requested_labels() {
    let expected = SampleRange {
        start: 0,
        end: 16000,
    };
    let mut result = RouteResult {
        route: Route::SeparatePyannote,
        coverage: vec![expected],
        rows: vec![row()],
        speakers: vec![],
        coverage_limitations: vec![],
    };
    assert!(validate_result(&result, &[expected]).is_err());
    result.rows[0].speaker = Some(turn(0, 16000, 1).speaker);
    assert!(validate_result(&result, &[expected]).is_ok());
    result.coverage[0].end = 8000;
    assert!(validate_result(&result, &[expected]).is_err());
    result.coverage = vec![expected];
    result.rows[0].speaker_finality = Finality::Provisional;
    assert!(validate_result(&result, &[expected]).is_err());
    result.rows.clear(); // Complete silence is valid.
    assert!(validate_result(&result, &[expected]).is_ok());
}
#[test]
fn multiple_epochs_cover_one_recording_without_replay() {
    use pipeline::recognition::covers;
    let whole = SampleRange {
        start: 32000,
        end: 32000 + 40 * 60 * 16000,
    };
    let middle = whole.start + 20 * 60 * 16000;
    assert!(covers(
        &[
            SampleRange {
                start: whole.start,
                end: middle
            },
            SampleRange {
                start: middle,
                end: whole.end
            }
        ],
        whole
    ));
    assert!(!covers(
        &[
            SampleRange {
                start: whole.start,
                end: middle - 1
            },
            SampleRange {
                start: middle,
                end: whole.end
            }
        ],
        whole
    ));
}
#[test]
fn transcript_groups_words_but_preserves_scope_and_pause_boundaries() {
    let mut first = row();
    first.range = SampleRange {
        start: 0,
        end: 1600,
    };
    first.speaker = Some(turn(0, 1600, 1).speaker);
    let mut second = first.clone();
    second.range = SampleRange {
        start: 1600,
        end: 3200,
    };
    second.text = "there.".into();
    let mut third = second.clone();
    third.range = SampleRange {
        start: 3200,
        end: 4800,
    };
    third.speaker.as_mut().unwrap().scope.epoch = 3;
    third.text = "Another speaker".into();
    let mut fourth = third.clone();
    fourth.range = SampleRange {
        start: 64000,
        end: 65600,
    };
    fourth.text = "Later".into();
    let result = RouteResult {
        route: Route::SeparatePyannote,
        coverage: vec![SampleRange {
            start: 0,
            end: 65600,
        }],
        rows: vec![first, second, third, fourth],
        speakers: vec![],
        coverage_limitations: vec![],
    };
    let transcript = pipeline::recognition::transcript(&result, &[]);
    assert_eq!(transcript.lines.len(), 3);
    assert!(
        matches!(&transcript.lines[0],session_core::folder::TranscriptLine::Turn{text,..} if text=="Hello there.")
    );
    assert!(matches!(
        &transcript.lines[1],
        session_core::folder::TranscriptLine::Turn {
            speaker: Some(2),
            ..
        }
    ));
}
#[test]
fn terminal_native_time_point_keeps_text_without_inventing_duration() {
    let mut last = row();
    last.range = SampleRange {
        start: 16000,
        end: 16000,
    };
    last.timing = TimingPrecision::NativeTimePoint;
    last.text = "tail".into();
    let result = RouteResult {
        route: Route::Off,
        coverage: vec![SampleRange {
            start: 0,
            end: 16000,
        }],
        rows: vec![last.clone()],
        speakers: vec![],
        coverage_limitations: vec![],
    };
    assert!(validate_result(&result, &result.coverage).is_ok());
    assert!(
        pipeline::recognition::transcript(&result, &[])
            .render()
            .contains("tail")
    );
    last.timing = TimingPrecision::NativeSegment;
    let invalid = RouteResult {
        rows: vec![last],
        ..result
    };
    assert!(validate_result(&invalid, &invalid.coverage).is_err());
}
#[test]
fn a_note_from_the_previous_runtimes_still_resolves() {
    let mut spec = resolve(&RecognitionConfig {
        speaker_source: SpeakerSource::Pyannote,
        ..Default::default()
    })
    .unwrap();
    for runtime in &mut spec.runtimes {
        runtime.revision = match runtime.name.as_str() {
            "transcribe.cpp" => "4807edaf210d0d7e8a6f7fb2a44b65966a2797f0".into(),
            "onnx-runtime" => "1.22.0".into(),
            _ => unreachable!(),
        };
    }
    assert!(TaskGraph::resolve(&spec).is_ok());
}
#[test]
fn historic_runtime_and_unimplemented_internal_combination_fail_explicitly() {
    let mut spec = resolve(&RecognitionConfig {
        speaker_source: SpeakerSource::Off,
        ..Default::default()
    })
    .unwrap();
    spec.runtimes[0].revision = "historic-runtime".into();
    assert!(
        TaskGraph::resolve(&spec)
            .unwrap_err()
            .to_string()
            .contains("different runtime revision")
    );
    let config = RecognitionConfig {
        asr_model: "nemotron-speech-streaming-en-0.6b-q80".into(),
        speaker_source: SpeakerSource::Off,
        ..Default::default()
    };
    let spec = resolve_for_runtime(&config, AsrRuntime::Nemo).unwrap();
    assert!(
        TaskGraph::resolve(&spec)
            .unwrap_err()
            .to_string()
            .contains("requires a compatible")
    );
}
#[test]
fn scoped_labels_do_not_claim_a_count_of_unique_people() {
    let mut first = row();
    first.speaker = Some(turn(0, 16000, 1).speaker);
    let mut second = first.clone();
    second.speaker.as_mut().unwrap().local_id = 2;
    let mut result = RouteResult {
        route: Route::SeparatePyannote,
        coverage: vec![first.range],
        rows: vec![first, second],
        speakers: vec![],
        coverage_limitations: vec![],
    };
    assert_eq!(pipeline::recognition::found_speakers(&result), Some(2));
    result.rows[1].speaker.as_mut().unwrap().input_start_sample = 16000;
    assert_eq!(pipeline::recognition::found_speakers(&result), None);
}

#[test]
fn nemo_mixed_timing_schedules_independent_analysis() {
    use models::recognition::ProcessingMode::{AfterStop, DuringRecording};
    for (text, speakers) in [(AfterStop, DuringRecording), (DuringRecording, AfterStop)] {
        let spec = resolve_for_runtime(
            &RecognitionConfig {
                asr_model: "nemotron-speech-streaming-en-0.6b-q80".into(),
                speaker_source: SpeakerSource::Nemotron,
                processing_mode: text,
                speaker_processing_mode: speakers,
                ..Default::default()
            },
            AsrRuntime::Nemo,
        )
        .unwrap();
        let graph = TaskGraph::resolve(&spec).unwrap();
        assert!(graph.separate_analysis);
        assert!(graph.stages.contains(&session_core::Stage::Diarize));
    }
}
