//! The level balance of a stereo segment.
//!
//! The live mix (KTD11) and the mix in Assemble (KTD8 step 5) give each
//! channel a gain that brings its speech level to one target. Both use the
//! constants and the gain in this module.

/// The power of a 20 ms frame with speech: above -50 dBFS.
pub const SPEECH_FLOOR: f32 = 1e-5;
/// Each channel needs at least this many frames with speech (1 s) to get a
/// gain other than 1.
pub const MIN_SPEECH_FRAMES: usize = 50;
/// The speech level that the mix gives each channel: -26 dBFS RMS.
pub const TARGET_RMS: f32 = 0.05;
/// The gain of a channel stays between -12 dB and +24 dB.
pub const MIN_GAIN: f32 = 0.25;
pub const MAX_GAIN: f32 = 16.0;

/// The gain that changes a speech level of `rms` to [`TARGET_RMS`], inside
/// the bounds.
pub fn gain(rms: f32) -> f32 {
    (TARGET_RMS / rms).clamp(MIN_GAIN, MAX_GAIN)
}

/// The RMS of `x`. An empty slice gives 0.
pub fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}
