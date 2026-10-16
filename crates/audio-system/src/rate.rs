//! The rate reader: resample system audio whose native rate can change to
//! one fixed output rate and frame size (U4 step 3).
//!
//! The macOS tap reports a format when it is created, but the output device
//! can run at another rate, and the rate changes when the user changes the
//! output device. The backend reads the rate that the device measures and
//! gives it to [`RateFollower::set_in_hz`].

use std::time::Duration;

use audio::resampler::FrameResampler;

/// Sample rates that a device reports. A measured rate snaps to the nearest
/// one of these when it is within [`SNAP_TOLERANCE`].
const COMMON_RATES: [u32; 12] = [
    8_000, 11_025, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000,
];

/// The largest relative difference between a measured rate and a common
/// rate that still counts as that common rate.
const SNAP_TOLERANCE: f64 = 0.01;

/// Turn a measured sample rate into the rate to resample from. A measured
/// rate changes by small amounts from one read to the next, so it snaps to
/// the nearest common rate. Returns `None` for a rate that is not positive.
pub fn snap_rate(measured: f64) -> Option<u32> {
    if !measured.is_finite() || measured < 1.0 {
        return None;
    }
    let nearest = COMMON_RATES
        .iter()
        .copied()
        .min_by(|a, b| {
            let da = (f64::from(*a) - measured).abs();
            let db = (f64::from(*b) - measured).abs();
            da.total_cmp(&db)
        })
        .expect("the list of common rates is not empty");
    if (f64::from(nearest) - measured).abs() / f64::from(nearest) <= SNAP_TOLERANCE {
        Some(nearest)
    } else {
        Some(measured.round() as u32)
    }
}

/// Resamples mono audio at a native rate that can change to `out_hz`, in
/// frames of a fixed number of samples.
pub struct RateFollower {
    in_hz: u32,
    out_hz: u32,
    frame: Duration,
    resampler: FrameResampler,
}

impl RateFollower {
    /// `frame_samples` is the number of output samples in each frame.
    pub fn new(in_hz: u32, out_hz: u32, frame_samples: usize) -> Self {
        let frame = Duration::from_secs_f64(frame_samples.max(1) as f64 / f64::from(out_hz));
        Self {
            in_hz,
            out_hz,
            frame,
            resampler: FrameResampler::new(in_hz as usize, out_hz as usize, frame),
        }
    }

    /// The native rate that the resampler reads now.
    pub fn in_hz(&self) -> u32 {
        self.in_hz
    }

    /// Set the native rate of the next samples. When the rate changes, the
    /// audio at the old rate goes to `emit` first, and a new resampler
    /// starts. Returns true when the rate changed.
    pub fn set_in_hz(&mut self, in_hz: u32, emit: impl FnMut(&[f32])) -> bool {
        if in_hz == 0 || in_hz == self.in_hz {
            return false;
        }
        self.resampler.finish(emit);
        log::info!(
            "system audio rate changed from {} Hz to {in_hz} Hz",
            self.in_hz
        );
        self.in_hz = in_hz;
        self.resampler = FrameResampler::new(in_hz as usize, self.out_hz as usize, self.frame);
        true
    }

    pub fn push(&mut self, samples: &[f32], emit: impl FnMut(&[f32])) {
        self.resampler.push(samples, emit);
    }

    /// Send the audio that the resampler holds to `emit`, with the last
    /// frame padded to its full length. Call this at the end of the stream.
    pub fn finish(&mut self, emit: impl FnMut(&[f32])) {
        self.resampler.finish(emit);
    }

    /// Discard the audio that the resampler holds, and read `in_hz` from
    /// now on. The Windows backend calls this when it follows another
    /// output device.
    pub fn reset_at(&mut self, in_hz: u32) {
        if in_hz == 0 || in_hz == self.in_hz {
            self.resampler.reset();
            return;
        }
        self.in_hz = in_hz;
        self.resampler = FrameResampler::new(in_hz as usize, self.out_hz as usize, self.frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(follower: &mut RateFollower, in_hz: u32, seconds: f64) -> usize {
        let samples = vec![0.25_f32; (f64::from(in_hz) * seconds) as usize];
        let mut out = 0;
        follower.push(&samples, |frame| out += frame.len());
        out
    }

    #[test]
    fn a_rate_change_from_48_to_44_1_khz_changes_the_ratio() {
        let mut follower = RateFollower::new(48_000, 16_000, 320);
        let mut total = count(&mut follower, 48_000, 1.0);
        let changed = follower.set_in_hz(44_100, |frame| total += frame.len());
        assert!(changed);
        assert_eq!(follower.in_hz(), 44_100);
        total += count(&mut follower, 44_100, 1.0);
        let mut last = Vec::new();
        follower.resampler.finish(|frame| {
            total += frame.len();
            last = frame.to_vec();
        });
        // Two seconds at 16 kHz, plus the two resampler delays and the
        // padding of the last frame of each rate. At the old ratio, the
        // 44.1 kHz second gives 14 700 samples and the total is lower.
        assert!(
            (32_000..=33_280).contains(&total),
            "two seconds of output, got {total} samples"
        );
        assert_eq!(total % 320, 0, "every frame is whole");
    }

    /// C16: at the end of the stream, `finish` sends the audio that the
    /// resampler still holds.
    #[test]
    fn finish_emits_the_audio_that_the_resampler_holds() {
        let mut follower = RateFollower::new(48_000, 16_000, 320);
        let mut total = count(&mut follower, 48_000, 0.5);
        assert!(total < 8_000, "the resampler holds a tail, got {total}");
        follower.finish(|frame| {
            assert_eq!(frame.len(), 320, "every frame is whole");
            total += frame.len();
        });
        // Half a second at 16 kHz. The resampler delay and the padding of
        // the last frame can add less than two frames.
        assert!(
            (8_000..8_640).contains(&total),
            "half a second of output, got {total} samples"
        );
    }

    #[test]
    fn the_same_rate_or_zero_changes_nothing() {
        let mut follower = RateFollower::new(48_000, 16_000, 320);
        assert!(!follower.set_in_hz(48_000, |_| panic!("no flush")));
        assert!(!follower.set_in_hz(0, |_| panic!("no flush")));
        assert_eq!(follower.in_hz(), 48_000);
    }

    #[test]
    fn a_measured_rate_snaps_to_the_nearest_common_rate() {
        assert_eq!(snap_rate(47_999.6), Some(48_000));
        assert_eq!(snap_rate(44_101.2), Some(44_100));
        assert_eq!(snap_rate(96_000.0), Some(96_000));
        assert_eq!(snap_rate(37_000.4), Some(37_000));
        assert_eq!(snap_rate(0.0), None);
        assert_eq!(snap_rate(f64::NAN), None);
    }
}
