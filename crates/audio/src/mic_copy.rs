//! The microphone copy flag: the system audio holds a late copy of the
//! microphone.
//!
//! An audio mixer app (Wave Link, OBS monitoring, and others) can play the
//! microphone to an output, and the system audio then records the user's
//! own voice a second time, late. The mono mix then holds each word twice,
//! and recognition fails. The macOS tap excludes the known mixer apps
//! (`audio_system::is_mixer`). This check finds the copy that an unknown
//! mixer makes, on both platforms, so the recording page can warn.
//!
//! The test is the echo test of the live mix with the roles changed: the
//! system channel is the target, and the microphone is the reference. It
//! searches delays from [`MIN_LAG`] to [`MAX_LAG`]. The joiner can give the
//! system channel up to 60 ms late (KTD5), so a normal echo of the speakers
//! in the microphone can look like a copy with a delay below 60 ms. The
//! search starts above that.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::level::rms;
use crate::live_mix::{LAG_STEP, MIN_SAMPLES, score};

/// The rate of the copies that the test reads.
const RATE: usize = 16_000;
/// The test reads the last 120 ms of the system channel.
const WINDOW: usize = RATE * 120 / 1000;
/// The shortest delay of the copy that the test searches: 80 ms.
const MIN_LAG: usize = RATE * 80 / 1000;
/// The longest delay of the copy that the test searches: 400 ms.
const MAX_LAG: usize = RATE * 400 / 1000;
/// The microphone audio that the test keeps.
const HISTORY: usize = MAX_LAG + WINDOW;
/// The test runs after each 100 ms of audio.
const TEST_EVERY: usize = RATE / 10;
/// The flag counts the tests of the last 30 s.
const RECENT_TESTS: u64 = 300;
/// The flag is set while at least this many of the recent tests found a
/// copy: 1 s of copied speech.
const MIN_HITS: usize = 10;
/// The correlation that a copy needs. The live gate also limits the part
/// that the reference does not explain, but a mixer can play the
/// microphone to two outputs with two delays, and the sum of two copies
/// leaves a large part that one copy does not explain. Two unrelated
/// voices stay far below this correlation.
const MIN_COPY_CORRELATION: f32 = 0.6;
/// Both channels must have speech: the levels of the live echo gate.
const MIN_MIC_RMS: f32 = 0.0025;
const MIN_SYSTEM_RMS: f32 = 0.01;

/// The flag that the recording status reads. Clones share one value.
#[derive(Debug, Clone, Default)]
pub struct MicCopyFlag(Arc<AtomicBool>);

impl MicCopyFlag {
    /// True while the system audio of the last 30 s held a late copy of the
    /// microphone.
    pub fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// The check. The segment thread gives it each pair of frames.
pub struct MicCopyCheck {
    /// Native samples for each sample of the 16 kHz copies.
    step: f64,
    phase: f64,
    sum: (f32, f32),
    count: u32,
    mic16: Vec<f32>,
    system16: Vec<f32>,
    since_test: usize,
    test: u64,
    /// The numbers of the recent tests that found a copy.
    hits: VecDeque<u64>,
    flag: MicCopyFlag,
}

impl MicCopyCheck {
    /// A check for frames at `rate`, the microphone's native rate. It sets
    /// `flag`.
    pub fn new(rate: u32, flag: MicCopyFlag) -> Self {
        Self {
            step: f64::from(rate.max(1)) / RATE as f64,
            phase: 0.0,
            sum: (0.0, 0.0),
            count: 0,
            mic16: Vec::with_capacity(2 * HISTORY),
            system16: Vec::with_capacity(2 * WINDOW),
            since_test: 0,
            test: 0,
            hits: VecDeque::new(),
            flag,
        }
    }

    /// Add one pair of frames. `system` is as long as `mic`; extra samples
    /// of either are not read.
    pub fn push(&mut self, mic: &[f32], system: &[f32]) {
        for (&m, &s) in mic.iter().zip(system) {
            self.sum.0 += m;
            self.sum.1 += s;
            self.count += 1;
            self.phase += 1.0;
            while self.phase >= self.step {
                self.phase -= self.step;
                let n = self.count.max(1) as f32;
                push_bounded(&mut self.mic16, self.sum.0 / n, HISTORY);
                push_bounded(&mut self.system16, self.sum.1 / n, WINDOW);
                self.sum = (0.0, 0.0);
                self.count = 0;
                self.since_test += 1;
                if self.since_test >= TEST_EVERY {
                    self.since_test = 0;
                    self.run_test();
                }
            }
        }
    }

    fn run_test(&mut self) {
        self.test += 1;
        if self.mic16.len() >= HISTORY
            && self.system16.len() >= WINDOW
            && is_copy(last(&self.system16, WINDOW), last(&self.mic16, HISTORY))
        {
            self.hits.push_back(self.test);
        }
        while self
            .hits
            .front()
            .is_some_and(|&t| t + RECENT_TESTS <= self.test)
        {
            self.hits.pop_front();
        }
        let flagged = self.hits.len() >= MIN_HITS;
        if flagged != self.flag.get() {
            if flagged {
                log::warn!(
                    "system audio holds a late copy of the microphone: \
                     an audio mixer app can play the microphone to an output"
                );
            } else {
                log::info!("system audio no longer holds a copy of the microphone");
            }
            self.flag.0.store(flagged, Ordering::Relaxed);
        }
    }
}

/// True when `system` is mostly a copy of the microphone from
/// [`MIN_LAG`] to [`MAX_LAG`] before it. `history` is the microphone audio
/// that ends at the same time as `system`.
fn is_copy(system: &[f32], history: &[f32]) -> bool {
    let n = system.len();
    if n < MIN_SAMPLES || rms(system) < MIN_SYSTEM_RMS {
        return false;
    }
    let before = history.len().saturating_sub(n);
    let mut lag = MIN_LAG;
    while lag <= MAX_LAG.min(before) {
        let start = before - lag;
        let reference = &history[start..start + n];
        if rms(reference) >= MIN_MIC_RMS
            && let Some(s) = score(system, reference)
            && s.correlation >= MIN_COPY_CORRELATION
        {
            return true;
        }
        lag += LAG_STEP;
    }
    false
}

/// Push `value` and keep at least the last `keep` values, as the live mix
/// does.
fn push_bounded(buffer: &mut Vec<f32>, value: f32, keep: usize) {
    if buffer.len() >= 2 * keep {
        buffer.drain(..buffer.len() - keep);
    }
    buffer.push(value);
}

fn last(buffer: &[f32], n: usize) -> &[f32] {
    &buffer[buffer.len().saturating_sub(n)..]
}

#[cfg(test)]
mod tests {
    use super::*;

    const NATIVE: u32 = 48_000;
    const FRAME: usize = 960;

    /// Noise with a slow level change, like speech, from a fixed seed.
    fn voice(seconds: f32, seed: u64) -> Vec<f32> {
        let mut state = seed;
        let n = (seconds * NATIVE as f32) as usize;
        (0..n)
            .map(|i| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let noise = ((state >> 33) as f32 / (1u64 << 31) as f32) - 0.5;
                let t = i as f32 / NATIVE as f32;
                let envelope = 0.5 + 0.5 * (t * 3.0).sin().abs();
                0.3 * noise * envelope
            })
            .collect()
    }

    fn delayed(x: &[f32], ms: usize, gain: f32) -> Vec<f32> {
        let d = NATIVE as usize * ms / 1000;
        (0..x.len())
            .map(|i| if i >= d { gain * x[i - d] } else { 0.0 })
            .collect()
    }

    fn run(mic: &[f32], system: &[f32]) -> bool {
        let flag = MicCopyFlag::default();
        let mut check = MicCopyCheck::new(NATIVE, flag.clone());
        for (m, s) in mic.chunks(FRAME).zip(system.chunks(FRAME)) {
            check.push(m, s);
        }
        flag.get()
    }

    /// The case from the Wave Link test: the system audio holds the
    /// microphone 140 ms late, louder.
    #[test]
    fn a_late_copy_of_the_microphone_sets_the_flag() {
        let mic = voice(5.0, 1);
        assert!(run(&mic, &delayed(&mic, 140, 1.5)));
    }

    #[test]
    fn a_copy_with_other_audio_on_top_sets_the_flag() {
        let mic = voice(5.0, 1);
        let other = voice(5.0, 9);
        let system: Vec<f32> = delayed(&mic, 200, 1.0)
            .iter()
            .zip(&other)
            .map(|(a, b)| a + 0.2 * b)
            .collect();
        assert!(run(&mic, &system));
    }

    #[test]
    fn independent_audio_does_not_set_the_flag() {
        assert!(!run(&voice(5.0, 1), &voice(5.0, 2)));
    }

    /// Speakers: the microphone holds the call late. The joiner can give
    /// the system channel up to 60 ms late, so the call in the system
    /// channel can come 40 ms after its echo in the microphone.
    #[test]
    fn a_speaker_echo_does_not_set_the_flag() {
        let call = voice(5.0, 3);
        assert!(!run(&delayed(&call, 20, 0.3), &call));
        let echo = delayed(&call, 20, 0.3);
        assert!(!run(&echo, &delayed(&call, 60, 1.0)));
    }

    #[test]
    fn the_flag_clears_when_the_copy_stops() {
        let flag = MicCopyFlag::default();
        let mut check = MicCopyCheck::new(NATIVE, flag.clone());
        let mic = voice(5.0, 1);
        let copy = delayed(&mic, 140, 1.0);
        for (m, s) in mic.chunks(FRAME).zip(copy.chunks(FRAME)) {
            check.push(m, s);
        }
        assert!(flag.get());
        let mic = voice(31.0, 4);
        let other = voice(31.0, 5);
        for (m, s) in mic.chunks(FRAME).zip(other.chunks(FRAME)) {
            check.push(m, s);
        }
        assert!(!flag.get());
    }
}
