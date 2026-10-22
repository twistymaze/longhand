//! The plain mix of a stereo segment (KTD9 of the app audio plan).
//!
//! A stereo segment has the microphone on the left and the app audio on the
//! right. The live archive reader and Assemble give both channels to [`mix`],
//! so the live transcript and the processing after Stop read the same audio.
//! The mix does not remove echo and does not change the level of a channel.
//! The recorder writes the raw channels.

/// The plain mix of one stretch of the two channels: the sum of each pair of
/// samples, clamped to [-1.0, 1.0]. `right` is as long as `left`; extra
/// samples of either are not read.
pub fn mix(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter()
        .zip(right)
        .map(|(l, r)| (l + r).clamp(-1.0, 1.0))
        .collect()
}

// The microphone copy check (`mic_copy`) uses the items below. They go with
// that module.
pub(crate) const MIN_SAMPLES: usize = 512;
pub(crate) const LAG_STEP: usize = 80;

#[derive(Debug, Clone, Copy)]
pub(crate) struct EchoScore {
    pub(crate) correlation: f32,
}

/// anarlog's `echo_score_at_lag` for two aligned slices of equal length,
/// reduced to the correlation.
pub(crate) fn score(mic: &[f32], speaker: &[f32]) -> Option<EchoScore> {
    let overlap = mic.len().min(speaker.len());
    if overlap < MIN_SAMPLES {
        return None;
    }
    let mut mic_energy = 0.0f32;
    let mut speaker_energy = 0.0f32;
    let mut cross_energy = 0.0f32;
    for (&m, &s) in mic[..overlap].iter().zip(&speaker[..overlap]) {
        mic_energy += m * m;
        speaker_energy += s * s;
        cross_energy += m * s;
    }
    if mic_energy <= f32::EPSILON || speaker_energy <= f32::EPSILON {
        return None;
    }
    Some(EchoScore {
        correlation: cross_energy.abs() / (mic_energy * speaker_energy).sqrt().max(1e-6),
    })
}
