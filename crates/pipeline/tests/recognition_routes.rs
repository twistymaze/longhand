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
fn off_and_integrated_have_no_separate_task() {
    for source in [SpeakerSource::Off, SpeakerSource::Integrated] {
        let spec = resolve(&RecognitionConfig {
            asr_model: "moss-transcribe-diarize-q80".into(),
            speaker_source: source,
            ..Default::default()
        })
        .unwrap();
        let graph = TaskGraph::resolve(&spec).unwrap();
        assert!(!graph.separate_analysis);
        assert_eq!(
            graph.route,
            if source == SpeakerSource::Off {
                Route::Off
            } else {
                Route::Integrated
            }
        );
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
        route: Route::Integrated,
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
        route: Route::Integrated,
        coverage: vec![SampleRange {
            start: 0,
            end: 65600,
        }],
        rows: vec![first, second, third, fourth],
        speakers: vec![],
        coverage_limitations: vec![],
    };
    let transcript = pipeline::recognition::transcript(&result);
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
        pipeline::recognition::transcript(&result)
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
        route: Route::Integrated,
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
