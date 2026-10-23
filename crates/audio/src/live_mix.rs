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
