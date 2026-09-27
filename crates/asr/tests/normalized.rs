use asr::{
    Attribution, Finality, RecognitionRequest, SampleRange, SpeakerScope, TimingPrecision,
    normalize_transcript,
};
use transcribe_cpp::{Segment, SpeakerSegment, TimestampKind, Transcript};

fn request(attribution: Attribution) -> RecognitionRequest {
    RecognitionRequest {
        scope: SpeakerScope {
            generation: "generation-a".into(),
            epoch: 3,
        },
        start_sample: 160_003,
        revision: 7,
        attribution,
    }
}

fn native(start: i64, end: i64) -> Transcript {
    Transcript {
        text: "One turn".into(),
        timestamp_kind: TimestampKind::Segment,
        segments: vec![Segment {
            t0_ms: start,
            t1_ms: end,
            text: "One turn".into(),
            speaker_id: 2,
            ..Default::default()
        }],
        speaker_segments: vec![SpeakerSegment {
            t0_ms: start,
            t1_ms: end,
            speaker_id: 2,
            p: f32::NAN,
        }],
        ..Default::default()
    }
}

#[test]
fn native_labels_keep_scope_precision_and_the_full_input_range() {
    let result =
        normalize_transcript(native(100, 200), 16_007, &request(Attribution::Native)).unwrap();
    assert_eq!(
        result.coverage,
        SampleRange {
            start: 160_003,
            end: 176_010
        }
    );
    let row = &result.segments[0];
    assert_eq!(
        row.range,
        SampleRange {
            start: 161_603,
            end: 163_203
        }
    );
    assert_eq!(row.timing, TimingPrecision::NativeSegment);
    assert_eq!(row.revision, 7);
    assert_eq!(row.text_finality, Finality::Final);
    assert_eq!(row.speaker_finality, Finality::Final);
    assert_eq!(row.speaker.as_ref().unwrap().scope.epoch, 3);
    assert_eq!(row.speaker.as_ref().unwrap().local_id, 2);
    assert_eq!(
        result.speaker_turns[0].speaker,
        *row.speaker.as_ref().unwrap()
    );
}

#[test]
fn zero_times_retain_source_window_precision_without_inventing_turn_boundaries() {
    let mut transcript = native(0, 0);
    transcript.segments.push(Segment {
        text: "Next turn".into(),
        speaker_id: 1,
        ..Default::default()
    });
    let result = normalize_transcript(transcript, 480_000, &request(Attribution::Native)).unwrap();
    assert_eq!(result.segments.len(), 2);
    for row in &result.segments {
        assert_eq!(row.range, result.coverage);
        assert_eq!(row.timing, TimingPrecision::SourceWindow);
    }
    assert_ne!(result.segments[0].id, result.segments[1].id);
}

#[test]
fn off_removes_all_native_assignments_even_if_the_runtime_returns_them() {
    let result =
        normalize_transcript(native(100, 200), 16_000, &request(Attribution::Off)).unwrap();
    assert!(result.segments.iter().all(|row| row.speaker.is_none()));
    assert!(result.speaker_turns.is_empty());
}

#[test]
fn a_new_epoch_or_window_never_reuses_an_independent_speaker_key() {
    let first =
        normalize_transcript(native(100, 200), 16_000, &request(Attribution::Native)).unwrap();
    let mut next = request(Attribution::Native);
    next.scope.epoch += 1;
    let second = normalize_transcript(native(100, 200), 16_000, &next).unwrap();
    assert_ne!(first.segments[0].speaker, second.segments[0].speaker);
    next = request(Attribution::Native);
    next.start_sample += 16_000;
    let third = normalize_transcript(native(100, 200), 16_000, &next).unwrap();
    assert_ne!(first.segments[0].speaker, third.segments[0].speaker);
}

#[test]
fn negative_reversed_and_outside_times_fail_instead_of_claiming_exact_coverage() {
    for (start, end) in [(-1, 100), (200, 100), (1001, 1200)] {
        assert!(
            normalize_transcript(native(start, end), 16_000, &request(Attribution::Native))
                .is_err()
        );
    }
    let mut overflow = request(Attribution::Native);
    overflow.start_sample = u64::MAX;
    assert!(normalize_transcript(native(1, 2), 16_000, &overflow).is_err());
}

#[test]
fn padding_is_excluded_and_empty_audio_cannot_contain_text() {
    let result =
        normalize_transcript(native(600, 900), 11_200, &request(Attribution::Native)).unwrap();
    assert_eq!(result.segments[0].range.end, result.coverage.end);
    assert!(normalize_transcript(native(0, 0), 0, &request(Attribution::Native)).is_err());
    let empty = normalize_transcript(Transcript::default(), 0, &request(Attribution::Off)).unwrap();
    assert!(empty.segments.is_empty());
}

#[test]
fn revisions_keep_row_ids_and_other_generations_do_not() {
    let first =
        normalize_transcript(native(100, 200), 16_000, &request(Attribution::Native)).unwrap();
    let mut next = request(Attribution::Native);
    next.revision += 1;
    let updated = normalize_transcript(native(100, 210), 16_000, &next).unwrap();
    assert_eq!(first.segments[0].id, updated.segments[0].id);
    assert_eq!(updated.segments[0].revision, 8);
    next.scope.generation = "generation-b".into();
    let replacement = normalize_transcript(native(100, 200), 16_000, &next).unwrap();
    assert_ne!(first.segments[0].id, replacement.segments[0].id);
    assert_ne!(first.segments[0].speaker, replacement.segments[0].speaker);
}

#[test]
fn text_only_results_retain_text_with_explicit_window_timing() {
    let transcript = Transcript {
        text: "No alignment".into(),
        ..Default::default()
    };
    let result = normalize_transcript(transcript, 16_000, &request(Attribution::Off)).unwrap();
    assert_eq!(result.segments[0].text, "No alignment");
    assert_eq!(result.segments[0].timing, TimingPrecision::SourceWindow);
    assert_eq!(result.segments[0].range, result.coverage);
    assert!(result.segments[0].speaker.is_none());
}

#[test]
fn compatibility_recognizers_reject_native_and_invalid_audio_before_inference() {
    use asr::{AsrError, AsrSegment, Recognizer, WindowText};
    struct Legacy {
        calls: usize,
    }
    impl Recognizer for Legacy {
        fn model_id(&self) -> &str {
            "legacy"
        }
        fn transcribe(&mut self, _: &[f32]) -> Result<WindowText, AsrError> {
            self.calls += 1;
            Ok(WindowText {
                text: "Legacy text".into(),
                segments: vec![AsrSegment {
                    start_ms: 0,
                    end_ms: 1_000,
                    text: "Legacy text".into(),
                }],
            })
        }
    }
    let mut legacy = Legacy { calls: 0 };
    assert!(
        legacy
            .transcribe_normalized(&[0.0], &request(Attribution::Native))
            .is_err()
    );
    assert!(
        legacy
            .transcribe_normalized(&[f32::NAN], &request(Attribution::Off))
            .is_err()
    );
    assert!(
        legacy
            .transcribe_normalized(&[f32::INFINITY], &request(Attribution::Off))
            .is_err()
    );
    assert_eq!(legacy.calls, 0);
    let result = legacy
        .transcribe_normalized(&vec![0.0; 16_000], &request(Attribution::Off))
        .unwrap();
    assert_eq!(legacy.calls, 1);
    assert_eq!(result.text, "Legacy text");
    assert_eq!(result.segments[0].range.start, 160_003);
    assert_eq!(result.segments[0].range.end, 176_003);
}
