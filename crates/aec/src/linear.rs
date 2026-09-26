//! The linear residual step. After the model, a part of the echo can stay in
//! the output as a scaled copy of the reference. This step measures that
//! scale and subtracts the scaled reference.

const AEC_MIN_REFERENCE_RMS: f32 = 1e-4;
const AEC_MIN_MIC_RMS: f32 = 1e-4;
const AEC_MIN_REFERENCE_CORRELATION: f32 = 0.12;
const AEC_MAX_LINEAR_GAIN: f32 = 1.25;
const AEC_LINEAR_GAIN_SMOOTHING: f32 = 0.12;
const AEC_DOUBLE_TALK_RESIDUAL_RATIO: f32 = 0.08;
const AEC_ROBUST_GAIN_SEGMENTS: usize = 8;
const AEC_ROBUST_GAIN_MIN_SEGMENTS: usize = 3;

/// Subtracts the part of `processed` that is a scaled copy of `speaker`.
///
/// `speaker` must be aligned with `processed`. `linear_echo_gain` keeps the
/// smoothed gain between chunks; set it to `None` when the alignment changes.
/// When the chunk is too quiet or the correlation is too weak, the function
/// returns `processed` unchanged.
pub fn cancel_linear_echo(
    speaker: &[f32],
    processed: Vec<f32>,
    linear_echo_gain: &mut Option<f32>,
) -> Vec<f32> {
    let len = speaker.len().min(processed.len());
    if len == 0 {
        return processed;
    }

    let mut processed_energy = 0.0;
    let mut speaker_energy = 0.0;
    let mut cross_energy = 0.0;
    for idx in 0..len {
        let processed_sample = processed[idx];
        let speaker_sample = speaker[idx];
        processed_energy += processed_sample * processed_sample;
        speaker_energy += speaker_sample * speaker_sample;
        cross_energy += processed_sample * speaker_sample;
    }

    let len_f32 = len as f32;
    let processed_rms = (processed_energy / len_f32).sqrt();
    let speaker_rms = (speaker_energy / len_f32).sqrt();
    if processed_rms < AEC_MIN_MIC_RMS || speaker_rms < AEC_MIN_REFERENCE_RMS {
        return processed;
    }

    let correlation = cross_energy.abs() / (processed_energy * speaker_energy).sqrt().max(1e-6);
    if correlation < AEC_MIN_REFERENCE_CORRELATION {
        return processed;
    }

    let instantaneous_gain =
        (cross_energy / speaker_energy.max(1e-6)).clamp(-AEC_MAX_LINEAR_GAIN, AEC_MAX_LINEAR_GAIN);
    let residual_energy =
        (processed_energy - (cross_energy * cross_energy / speaker_energy.max(1e-6))).max(0.0);
    let residual_ratio = (residual_energy / len_f32).sqrt() / processed_rms.max(1e-6);
    let measured_gain = if residual_ratio > AEC_DOUBLE_TALK_RESIDUAL_RATIO {
        segmented_trimmed_gain(&processed, speaker, len, instantaneous_gain)
    } else {
        instantaneous_gain
    };
    let smoothed_gain = linear_echo_gain
        .map(|gain| gain + (measured_gain - gain) * AEC_LINEAR_GAIN_SMOOTHING)
        .unwrap_or(measured_gain);
    *linear_echo_gain = Some(smoothed_gain);

    let gain = if residual_ratio > AEC_DOUBLE_TALK_RESIDUAL_RATIO {
        smoothed_gain
    } else {
        measured_gain
    };

    let mut output = Vec::with_capacity(processed.len());
    output.extend(processed.iter().zip(speaker).take(len).map(
        |(processed_sample, speaker_sample)| {
            (processed_sample - gain * speaker_sample).clamp(-1.0, 1.0)
        },
    ));
    output.extend_from_slice(&processed[len..]);
    output
}

/// Measures the gain on segments of the chunk and takes a trimmed, weighted
/// mean, so that near speech in some segments does not change the gain much.
fn segmented_trimmed_gain(
    processed: &[f32],
    speaker: &[f32],
    len: usize,
    initial_gain: f32,
) -> f32 {
    let segment_len = (len / AEC_ROBUST_GAIN_SEGMENTS).max(1);
    let mut gains = Vec::with_capacity(AEC_ROBUST_GAIN_SEGMENTS);
    let mut start = 0;
    while start < len {
        let end = (start + segment_len).min(len);
        let mut processed_energy = 0.0;
        let mut speaker_energy = 0.0;
        let mut cross_energy = 0.0;
        for idx in start..end {
            let processed_sample = processed[idx];
            let speaker_sample = speaker[idx];
            processed_energy += processed_sample * processed_sample;
            speaker_energy += speaker_sample * speaker_sample;
            cross_energy += processed_sample * speaker_sample;
        }

        if processed_energy > 1e-8 && speaker_energy > 1e-8 {
            let correlation =
                cross_energy.abs() / (processed_energy * speaker_energy).sqrt().max(1e-6);
            if correlation >= AEC_MIN_REFERENCE_CORRELATION {
                let gain = (cross_energy / speaker_energy.max(1e-6))
                    .clamp(-AEC_MAX_LINEAR_GAIN, AEC_MAX_LINEAR_GAIN);
                gains.push((gain, speaker_energy));
            }
        }

        start = end;
    }

    if gains.len() < AEC_ROBUST_GAIN_MIN_SEGMENTS {
        return initial_gain;
    }

    gains.sort_by(|left, right| left.0.total_cmp(&right.0));
    let trim = if gains.len() >= 6 { gains.len() / 6 } else { 0 };
    let kept = &gains[trim..gains.len() - trim];
    let weighted_gain = kept.iter().map(|(gain, energy)| gain * energy).sum::<f32>();
    let kept_energy = kept.iter().map(|(_, energy)| energy).sum::<f32>();
    if kept_energy <= 1e-8 {
        return initial_gain;
    }

    (weighted_gain / kept_energy).clamp(-AEC_MAX_LINEAR_GAIN, AEC_MAX_LINEAR_GAIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(len: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                ((state as f32 / u32::MAX as f32) * 2.0 - 1.0) * 0.2
            })
            .collect()
    }

    fn energy(signal: &[f32]) -> f32 {
        signal.iter().map(|x| x * x).sum()
    }

    #[test]
    fn removes_a_scaled_copy_of_the_speaker() {
        let speaker = noise(1_600, 0x1234_5678);
        let processed: Vec<f32> = speaker.iter().map(|x| 0.3 * x).collect();
        let mut gain = None;

        let output = cancel_linear_echo(&speaker, processed.clone(), &mut gain);

        assert!(energy(&output) < energy(&processed) * 1e-4);
        assert!((gain.unwrap() - 0.3).abs() < 1e-3);
    }

    #[test]
    fn keeps_near_speech_and_removes_the_copy() {
        let speaker = noise(1_600, 0x1234_5678);
        let near = noise(1_600, 0x0bad_f00d);
        let processed: Vec<f32> = near
            .iter()
            .zip(&speaker)
            .map(|(n, s)| n + 0.5 * s)
            .collect();
        let mut gain = None;

        let output = cancel_linear_echo(&speaker, processed, &mut gain);

        let residual: Vec<f32> = output.iter().zip(&near).map(|(o, n)| o - n).collect();
        assert!(energy(&residual) < energy(&near) * 0.05);
    }

    #[test]
    fn returns_the_input_when_the_speaker_is_silent() {
        let processed = noise(1_600, 0x0bad_f00d);
        let mut gain = None;

        let output = cancel_linear_echo(&vec![0.0; 1_600], processed.clone(), &mut gain);

        assert_eq!(output, processed);
        assert_eq!(gain, None);
    }

    #[test]
    fn returns_the_input_when_the_correlation_is_weak() {
        let speaker = noise(1_600, 0x1234_5678);
        let processed = noise(1_600, 0x0bad_f00d);
        let mut gain = None;

        let output = cancel_linear_echo(&speaker, processed.clone(), &mut gain);

        assert_eq!(output, processed);
    }
}
