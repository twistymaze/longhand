//! Retry incomplete decodes without accepting partial model output.

use transcribe_cpp::{Error, Transcript};

pub(crate) struct DecodedPart {
    pub offset: usize,
    pub samples: usize,
    pub transcript: Transcript,
}

pub(crate) fn decode(
    pcm: &[f32],
    run: &mut impl FnMut(&[f32]) -> Result<Transcript, Error>,
) -> Result<Vec<DecodedPart>, Error> {
    split(pcm, 0, 0, run)
}

fn split(
    pcm: &[f32],
    offset: usize,
    depth: usize,
    run: &mut impl FnMut(&[f32]) -> Result<Transcript, Error>,
) -> Result<Vec<DecodedPart>, Error> {
    match run(pcm) {
        Ok(transcript) => Ok(vec![DecodedPart {
            offset,
            samples: pcm.len(),
            transcript,
        }]),
        Err(Error::OutputTruncated { .. })
            if depth < 4 && pcm.len() >= 2 * crate::SAMPLE_RATE as usize =>
        {
            // Keep split points on millisecond boundaries. All samples remain
            // in exactly one child. Each child gets at least one second.
            let midpoint =
                (pcm.len() / 2 / crate::SAMPLES_PER_MS as usize) * crate::SAMPLES_PER_MS as usize;
            let (left_pcm, right_pcm) = pcm.split_at(midpoint);
            let mut left = split(left_pcm, offset, depth + 1, run)?;
            left.extend(split(right_pcm, offset + midpoint, depth + 1, run)?);
            Ok(left)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn normalize_parts(
    parts: Vec<DecodedPart>,
    request: &crate::RecognitionRequest,
    samples: usize,
    normalize: impl Fn(
        Transcript,
        usize,
        &crate::RecognitionRequest,
    ) -> Result<crate::RecognitionOutput, crate::AsrError>,
) -> Result<crate::RecognitionOutput, crate::AsrError> {
    let mut result = crate::RecognitionOutput {
        scope: request.scope.clone(),
        revision: request.revision,
        coverage: request.coverage(samples)?,
        text: String::new(),
        segments: Vec::new(),
        speaker_turns: Vec::new(),
    };
    for part in parts {
        let mut child = request.clone();
        child.start_sample += part.offset as u64;
        // Speaker keys and row IDs include input_start_sample. Separate
        // decodes must not imply that equal local labels name the same person.
        let output = normalize(part.transcript, part.samples, &child)?;
        append_text(&mut result.text, &output.text);
        result.segments.extend(output.segments);
        result.speaker_turns.extend(output.speaker_turns);
    }
    Ok(result)
}

pub(crate) fn append_text(target: &mut String, text: &str) {
    let text = text.trim();
    if !text.is_empty() {
        if !target.is_empty() {
            target.push(' ');
        }
        target.push_str(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn truncated() -> Error {
        Error::OutputTruncated {
            message: "generation limit".into(),
            partial: Some(Box::new(Transcript {
                text: "incomplete output must not survive".into(),
                ..Default::default()
            })),
        }
    }

    #[test]
    fn truncated_decode_recovers_all_samples_in_order_without_partial_text() {
        let pcm: Vec<_> = (0..82_240).map(|i| i as f32).collect();
        let mut calls = 0;
        let parts = decode(&pcm, &mut |audio| {
            calls += 1;
            if audio.len() > 41_120 {
                return Err(truncated());
            }
            Ok(Transcript {
                text: audio[0].to_string(),
                ..Default::default()
            })
        })
        .unwrap();
        assert_eq!(calls, 3);
        assert_eq!(parts.len(), 2);
        assert_eq!((parts[0].offset, parts[0].samples), (0, 41_120));
        assert_eq!((parts[1].offset, parts[1].samples), (41_120, 41_120));
        assert_eq!(parts[0].transcript.text, "0");
        assert_eq!(parts[1].transcript.text, "41120");
    }

    #[test]
    fn persistent_truncation_is_bounded_and_does_not_return_partial_success() {
        let mut calls = 0;
        let result = decode(&vec![0.; 16_000 * 300], &mut |_| {
            calls += 1;
            Err(truncated())
        });
        assert!(matches!(result, Err(Error::OutputTruncated { .. })));
        assert_eq!(calls, 5);
    }

    #[test]
    fn recovered_native_labels_keep_independent_identity_and_absolute_times() {
        let request = crate::RecognitionRequest {
            scope: crate::SpeakerScope {
                generation: "test".into(),
                epoch: 2,
            },
            start_sample: 80_000,
            revision: 3,
            attribution: crate::Attribution::Native,
        };
        let transcript = || Transcript {
            text: "speech".into(),
            speaker_segments: vec![transcribe_cpp::SpeakerSegment {
                t0_ms: 0,
                t1_ms: 1_000,
                speaker_id: 1,
                ..Default::default()
            }],
            segments: vec![transcribe_cpp::Segment {
                t0_ms: 0,
                t1_ms: 1_000,
                text: "speech".into(),
                speaker_id: 1,
                ..Default::default()
            }],
            ..Default::default()
        };
        let parts = vec![
            DecodedPart {
                offset: 0,
                samples: 16_000,
                transcript: transcript(),
            },
            DecodedPart {
                offset: 16_000,
                samples: 16_000,
                transcript: transcript(),
            },
        ];
        let result = normalize_parts(parts, &request, 32_000, crate::normalize_transcript).unwrap();
        assert_eq!(
            result.coverage,
            crate::SampleRange {
                start: 80_000,
                end: 112_000
            }
        );
        assert_eq!(result.text, "speech speech");
        assert_eq!(
            result.segments[0].range,
            crate::SampleRange {
                start: 80_000,
                end: 96_000
            }
        );
        assert_eq!(
            result.segments[1].range,
            crate::SampleRange {
                start: 96_000,
                end: 112_000
            }
        );
        assert_ne!(result.segments[0].id, result.segments[1].id);
        assert_ne!(result.segments[0].speaker, result.segments[1].speaker);
        assert_eq!(result.speaker_turns.len(), 2);
        for (turn, segment) in result.speaker_turns.iter().zip(&result.segments) {
            assert_eq!(turn.range, segment.range);
            assert_eq!(Some(&turn.speaker), segment.speaker.as_ref());
        }
        assert_ne!(
            result.speaker_turns[0].speaker,
            result.speaker_turns[1].speaker
        );
        assert!(result.segments.iter().all(|row| row.revision == 3));
    }

    #[test]
    fn recovery_keeps_an_odd_sample_tail_and_does_not_split_short_audio() {
        let pcm = vec![0.; 32_003];
        let parts = decode(&pcm, &mut |audio| {
            if audio.len() > 16_003 {
                Err(truncated())
            } else {
                Ok(Transcript::default())
            }
        })
        .unwrap();
        assert_eq!(
            parts.iter().map(|part| part.samples).sum::<usize>(),
            pcm.len()
        );
        assert_eq!(parts[1].offset + parts[1].samples, pcm.len());
        let mut calls = 0;
        let result = decode(&pcm[..31_999], &mut |_| {
            calls += 1;
            Err(truncated())
        });
        assert!(matches!(result, Err(Error::OutputTruncated { .. })));
        assert_eq!(calls, 1);
    }

    #[test]
    fn later_child_error_discards_earlier_success() {
        let pcm: Vec<_> = (0..64_000).map(|i| i as f32).collect();
        let result = decode(&pcm, &mut |audio| {
            if audio.len() > 32_000 {
                return Err(truncated());
            }
            if audio[0] > 0. {
                return Err(Error::InputTooLong("child failure".into()));
            }
            Ok(Transcript {
                text: "earlier success".into(),
                ..Default::default()
            })
        });
        assert!(matches!(result, Err(Error::InputTooLong(message)) if message == "child failure"));
    }

    #[test]
    fn unrelated_errors_are_not_retried() {
        let mut calls = 0;
        let result = decode(&vec![0.; 82_240], &mut |_| {
            calls += 1;
            Err(Error::InputTooLong("different failure".into()))
        });
        assert!(matches!(result, Err(Error::InputTooLong(_))));
        assert_eq!(calls, 1);
    }
}
