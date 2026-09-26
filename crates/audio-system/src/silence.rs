//! The silent Computer audio flag (U5 step 6).
//!
//! When the permission check reports [`Permission::Unknown`], Longhand
//! cannot tell a denied tap from a quiet computer. A denied tap gives
//! digital silence: every sample is exactly zero. So while Computer audio is
//! on and the status is unknown, the capture watcher compares two facts:
//!
//! - the output device plays audio for some process
//!   ([`crate::output_running`]);
//! - the right channel has held digital silence for [`SILENT_AFTER`]
//!   ([`SilenceRun`]).
//!
//! When both are true, [`SilentCheck`] sets the flag, and the recording page
//! warns that computer audio can be missing (U7). The flag clears when audio
//! appears on the right, or when the status is no longer unknown.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use crate::permission::Permission;

/// How long the right channel must hold digital silence, while the output
/// plays, before the flag sets.
pub const SILENT_AFTER: Duration = Duration::from_secs(10);

/// The length of the digital silence at the end of the right channel. The
/// segment thread pushes each right-channel frame. The capture watcher reads
/// the length at each poll. Clones share one counter.
#[derive(Debug, Clone, Default)]
pub struct SilenceRun(Arc<Run>);

#[derive(Debug, Default)]
struct Run {
    /// Samples of digital silence since the last sample that was not zero.
    samples: AtomicU64,
    /// The rate of the pushed samples.
    rate: AtomicU32,
}

impl SilenceRun {
    /// Add one frame of the right channel at `rate` Hz.
    pub fn push(&self, right: &[f32], rate: u32) {
        self.0.rate.store(rate, Ordering::Relaxed);
        match right.iter().rposition(|s| *s != 0.0) {
            None => {
                self.0
                    .samples
                    .fetch_add(right.len() as u64, Ordering::Relaxed);
            }
            Some(last) => {
                let zeros = right.len() - last - 1;
                self.0.samples.store(zeros as u64, Ordering::Relaxed);
            }
        }
    }

    /// How long the right channel has held digital silence. Zero before the
    /// first push.
    pub fn silent_for(&self) -> Duration {
        let rate = self.0.rate.load(Ordering::Relaxed);
        if rate == 0 {
            return Duration::ZERO;
        }
        let samples = self.0.samples.load(Ordering::Relaxed);
        Duration::from_micros(samples * 1_000_000 / u64::from(rate))
    }

    /// Start the count again, for example when a new speaker stream opens.
    pub fn reset(&self) {
        self.0.samples.store(0, Ordering::Relaxed);
    }
}

/// The flag state between two polls of the capture watcher.
#[derive(Debug, Clone, Copy, Default)]
pub struct SilentCheck {
    flagged: bool,
}

impl SilentCheck {
    /// One poll. `permission` is the latest check, `output_running` is
    /// whether the output device plays audio now, and `silent_for` is
    /// [`SilenceRun::silent_for`]. It returns the flag.
    ///
    /// The flag sets when the status is unknown, the output plays, and the
    /// right channel has been silent for [`SILENT_AFTER`]. After that, the
    /// output can stop and the flag stays: the right channel stayed silent
    /// while audio played. The flag clears when audio appears on the right,
    /// or when the status is no longer unknown.
    pub fn poll(
        &mut self,
        permission: Permission,
        output_running: bool,
        silent_for: Duration,
    ) -> bool {
        let flagged = permission == Permission::Unknown
            && silent_for >= SILENT_AFTER
            && (self.flagged || output_running);
        if flagged != self.flagged {
            if flagged {
                log::warn!(
                    "the output plays, but system audio has been digital silence for {} s \
                     and the permission is unknown; Computer audio can be missing",
                    silent_for.as_secs()
                );
            } else {
                log::info!("the silent Computer audio flag cleared");
            }
        }
        self.flagged = flagged;
        flagged
    }

    /// The flag, as the last poll set it.
    pub fn flagged(&self) -> bool {
        self.flagged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;
    /// 20 ms at 48 kHz.
    const FRAME: usize = 960;

    /// Push `seconds` of frames of `value` into `run`.
    fn push_for(run: &SilenceRun, seconds: f32, value: f32) {
        let frames = (seconds * RATE as f32 / FRAME as f32).round() as usize;
        let frame = vec![value; FRAME];
        for _ in 0..frames {
            run.push(&frame, RATE);
        }
    }

    #[test]
    fn the_run_counts_digital_silence_and_resets_on_audio() {
        let run = SilenceRun::default();
        assert_eq!(run.silent_for(), Duration::ZERO);
        push_for(&run, 10.0, 0.0);
        assert_eq!(run.silent_for(), Duration::from_secs(10));
        push_for(&run, 0.02, 0.001);
        assert_eq!(run.silent_for(), Duration::ZERO);
        push_for(&run, 2.0, 0.0);
        assert_eq!(run.silent_for(), Duration::from_secs(2));
    }

    #[test]
    fn the_run_counts_the_zeros_after_the_last_sound_in_a_frame() {
        let run = SilenceRun::default();
        let mut frame = vec![0.0; 480];
        frame[0] = 0.5;
        run.push(&frame, 48_000);
        assert_eq!(run.silent_for(), Duration::from_micros(9_979));
    }

    #[test]
    fn a_reset_starts_the_count_again_and_clones_share_it() {
        let run = SilenceRun::default();
        let reader = run.clone();
        push_for(&run, 3.0, 0.0);
        assert_eq!(reader.silent_for(), Duration::from_secs(3));
        run.reset();
        assert_eq!(reader.silent_for(), Duration::ZERO);
    }

    #[test]
    fn unknown_status_running_output_and_ten_silent_seconds_set_the_flag() {
        let run = SilenceRun::default();
        let mut check = SilentCheck::default();
        push_for(&run, 9.98, 0.0);
        assert!(!check.poll(Permission::Unknown, true, run.silent_for()));
        push_for(&run, 0.02, 0.0);
        assert!(check.poll(Permission::Unknown, true, run.silent_for()));
        assert!(check.flagged());
    }

    #[test]
    fn an_output_that_does_not_run_sets_no_flag() {
        let run = SilenceRun::default();
        let mut check = SilentCheck::default();
        push_for(&run, 30.0, 0.0);
        assert!(!check.poll(Permission::Unknown, false, run.silent_for()));
    }

    #[test]
    fn a_known_status_sets_no_flag() {
        let run = SilenceRun::default();
        push_for(&run, 30.0, 0.0);
        for permission in [
            Permission::Allowed,
            Permission::Denied,
            Permission::NotAsked,
        ] {
            let mut check = SilentCheck::default();
            assert!(
                !check.poll(permission, true, run.silent_for()),
                "{permission:?}"
            );
        }
    }

    #[test]
    fn the_flag_clears_when_audio_appears_on_the_right() {
        let run = SilenceRun::default();
        let mut check = SilentCheck::default();
        push_for(&run, 10.0, 0.0);
        assert!(check.poll(Permission::Unknown, true, run.silent_for()));
        // The output stops, and the right channel stays silent: the flag stays.
        push_for(&run, 2.0, 0.0);
        assert!(check.poll(Permission::Unknown, false, run.silent_for()));
        push_for(&run, 0.02, 0.2);
        assert!(!check.poll(Permission::Unknown, true, run.silent_for()));
    }

    #[test]
    fn the_flag_clears_when_the_status_is_known_again() {
        let run = SilenceRun::default();
        let mut check = SilentCheck::default();
        push_for(&run, 12.0, 0.0);
        assert!(check.poll(Permission::Unknown, true, run.silent_for()));
        assert!(!check.poll(Permission::Allowed, true, run.silent_for()));
    }
}
