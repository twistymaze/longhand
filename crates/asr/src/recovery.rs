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
