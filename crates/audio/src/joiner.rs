//! Pair each microphone frame with system audio (KTD5 of the system audio
//! plan).
//!
//! anarlog's `Joiner` reshaped. anarlog queues frames on both sides, waits
//! up to four frames for the late side, and drops frames on either side.
//! Here the microphone is the clock: segment time and the live tap index
//! count microphone samples, so the joiner never pads, delays or drops the
//! microphone.
//!
//! - For each microphone frame, the joiner takes the same number of speaker
//!   samples from its queue.
//! - When the queue holds too few, it pads the speaker with silence at once
//!   and does not wait. Windows sends no packets while nothing plays, so an
//!   empty queue is normal.
//! - The queue holds at most [`MAX_SPEAKER_QUEUE_MS`] of audio. Above that
//!   bound, the joiner drops the oldest speaker samples until the queue is
//!   at the bound.
//!
//! So the joiner adds at most 60 ms of offset between the two channels, also
//! after a stall and over hours of drift between the two device clocks. The
//! speaker stream must already be at the microphone's rate. See
//! PROVENANCE.md.

use std::collections::VecDeque;

/// The most speaker audio that the queue holds.
pub const MAX_SPEAKER_QUEUE_MS: u32 = 60;

/// The joiner of one segment. `T` is the sample type: `f32` for audio. The
/// tests use tags to follow each sample. `T::default()` is silence.
#[derive(Debug, Clone)]
pub struct Joiner<T = f32> {
    queue: VecDeque<T>,
    bound: usize,
    dropped: u64,
    padded: u64,
}

impl<T: Copy + Default> Joiner<T> {
    /// A joiner for streams at `rate` Hz, the microphone's native rate.
    pub fn new(rate: u32) -> Self {
        let bound = (u64::from(rate) * u64::from(MAX_SPEAKER_QUEUE_MS) / 1000).max(1) as usize;
        Self {
            queue: VecDeque::with_capacity(bound * 2),
            bound,
            dropped: 0,
            padded: 0,
        }
    }

    /// The queue bound in samples.
    pub fn bound(&self) -> usize {
        self.bound
    }

    /// Speaker samples in the queue.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// Add speaker samples. Returns how many of the oldest samples it
    /// dropped to keep the queue at the bound.
    pub fn push_speaker(&mut self, samples: &[T]) -> usize {
        self.queue.extend(samples);
        let excess = self.queue.len().saturating_sub(self.bound);
        self.queue.drain(..excess);
        self.dropped += excess as u64;
        excess
    }

    /// The right channel for one microphone frame of `mic_len` samples:
    /// `out` gets exactly `mic_len` samples. Returns how many of them are
    /// silence padding.
    pub fn pair(&mut self, mic_len: usize, out: &mut Vec<T>) -> usize {
        out.clear();
        let take = mic_len.min(self.queue.len());
        out.extend(self.queue.drain(..take));
        let pad = mic_len - take;
        out.resize(mic_len, T::default());
        self.padded += pad as u64;
        pad
    }

    /// Forget the queued samples: a new speaker stream starts, or the
    /// stream stops.
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// Speaker samples dropped since the start.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Silence samples padded since the start.
    pub fn padded(&self) -> u64 {
        self.padded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_speaker_queue_pairs_each_mic_frame_with_silence_at_once() {
        let mut joiner: Joiner<f32> = Joiner::new(16_000);
        let mut right = Vec::new();
        let mut mic_out = 0;
        for _ in 0..50 {
            assert_eq!(joiner.pair(320, &mut right), 320);
            assert!(right.iter().all(|&s| s == 0.0));
            mic_out += right.len();
        }
        assert_eq!(mic_out, 50 * 320, "the mic count equals its input count");
        assert_eq!(joiner.padded(), 50 * 320);
    }

    #[test]
    fn a_short_queue_is_used_and_the_rest_is_padded() {
        let mut joiner: Joiner<f32> = Joiner::new(16_000);
        joiner.push_speaker(&[0.5; 100]);
        let mut right = Vec::new();
        assert_eq!(joiner.pair(320, &mut right), 220);
        assert_eq!(right.len(), 320);
        assert!(right[..100].iter().all(|&s| s == 0.5));
        assert!(right[100..].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn a_full_speaker_queue_drops_the_oldest_samples_down_to_60_ms() {
        let mut joiner: Joiner<u32> = Joiner::new(16_000);
        assert_eq!(joiner.bound(), 960);
        let samples: Vec<u32> = (1..=2_000).collect();
        assert_eq!(joiner.push_speaker(&samples), 1_040);
        assert_eq!(joiner.queued(), 960);
        assert_eq!(joiner.dropped(), 1_040);
        // The newest samples stay, in order.
        let mut right = Vec::new();
        let mut mic_out = 0;
        for frame in 0..5 {
            joiner.pair(320, &mut right);
            mic_out += right.len();
            if frame == 0 {
                assert_eq!(right[0], 1_041);
            }
        }
        // No mic frame is dropped: five frames in, five frames out.
        assert_eq!(mic_out, 5 * 320);
        assert_eq!(joiner.padded(), 5 * 320 - 960);
    }

    #[test]
    fn a_clear_forgets_the_queue() {
        let mut joiner: Joiner<f32> = Joiner::new(48_000);
        joiner.push_speaker(&[0.5; 960]);
        joiner.clear();
        let mut right = Vec::new();
        assert_eq!(joiner.pair(960, &mut right), 960);
    }
}
