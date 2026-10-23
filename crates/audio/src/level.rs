//! The level of a stretch of audio.

/// The RMS of `x`. An empty slice gives 0.
pub fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}
