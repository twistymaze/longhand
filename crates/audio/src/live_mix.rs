//! The live mix of a stereo segment (KTD11 of the system audio plan).
//!
//! Capture gives each pair of frames (microphone and system audio, at the
//! microphone's native rate) to a [`LiveMix`] before it offers audio to the
//! live worker. The mix gives one mono frame of the same length:
//!
//! - Each channel gets a slow gain from a running speech level, so both sit
//!   near one target level. The gain stays between [`MIN_GAIN`] and
//!   [`MAX_GAIN`]. While a channel has no speech above the floor, both gains
//!   are 1, so a silent system channel adds nothing to the microphone.
//! - The echo gate zeroes the microphone part while the microphone is mostly
//!   a delayed copy of the system channel, so the live transcript does not
//!   hear the call twice. The gate is anarlog's `suppress_echo_dominant_mic`
//!   (see PROVENANCE.md). Its constants are sample counts at 16 kHz, so it
//!   runs on a 16 kHz copy of both channels. It searches delays up to 400 ms,
//!   because Bluetooth output adds 150 to 250 ms, and keeps that much system
//!   audio plus one window.
//!
//! The mix changes only what the live worker hears. The recorder writes the
//! raw channels.

/// The rate of the copy that the gate reads.
const GATE_RATE: usize = 16_000;
/// The gate tests the last 120 ms of the microphone.
const WINDOW: usize = GATE_RATE * 120 / 1000;
/// The gate tests again after each 20 ms of audio.
const HOP: usize = GATE_RATE / 50;
/// The largest delay of the echo in the microphone that the gate searches:
/// 400 ms, the same as the Assemble probe.
const MAX_LAG: usize = GATE_RATE * 400 / 1000;
/// The joiner can give the system channel up to 60 ms late (KTD5), so the
/// gate also searches an echo that is early by that much.
const MAX_EARLY: usize = GATE_RATE * 60 / 1000;
/// The system audio that the gate keeps.
const HISTORY: usize = MAX_LAG + WINDOW;
// anarlog's gate constants.
pub(crate) const MIN_SAMPLES: usize = 512;
pub(crate) const LAG_STEP: usize = 80;
const MIN_MIC_RMS: f32 = 0.0025;
const MIN_SPEAKER_RMS: f32 = 0.01;
pub(crate) const MIN_CORRELATION: f32 = 0.55;
pub(crate) const MAX_RESIDUAL_RATIO: f32 = 0.45;

// The balance and the mix in Assemble (KTD8 step 5) use the same constants
// from the `level` module.
pub use crate::level::{MAX_GAIN, MIN_GAIN};
use crate::level::{MIN_SPEECH_FRAMES, SPEECH_FLOOR, rms};
/// The running level follows the last 10 s of speech (500 frames).
const LEVEL_FRAMES: f32 = 500.0;
/// A gain moves to a new value over about 1 s (50 frames).
const GAIN_FRAMES: f32 = 50.0;

/// The live mix of one stereo segment.
pub struct LiveMix {
    /// Native samples for each sample of the 16 kHz copy.
    step: f64,
    /// Native samples in the 16 kHz sample that is being built.
    phase: f64,
    sum: (f32, f32),
    count: u32,
    mic16: Vec<f32>,
    speaker16: Vec<f32>,
    since_test: usize,
    gated: bool,
    /// One 20 ms level frame at the native rate.
    frame_len: usize,
    frame: LevelFrame,
    mic_level: Level,
    speaker_level: Level,
    mic_gain: f32,
    speaker_gain: f32,
}

#[derive(Default)]
struct LevelFrame {
    samples: usize,
    mic: f64,
    speaker: f64,
    /// The gate zeroed the microphone for part of the frame: that part is
    /// echo, not microphone speech.
    gated: bool,
}

/// The running speech level of one channel.
#[derive(Default)]
struct Level {
    frames: usize,
    sum: f64,
    power: f32,
}

impl Level {
    fn add(&mut self, power: f32) {
        if power < SPEECH_FLOOR {
            return;
        }
        if self.frames < MIN_SPEECH_FRAMES {
            self.sum += f64::from(power);
            self.frames += 1;
            if self.frames == MIN_SPEECH_FRAMES {
                self.power = (self.sum / MIN_SPEECH_FRAMES as f64) as f32;
            }
        } else {
            self.power += (power - self.power) / LEVEL_FRAMES;
        }
    }

    /// The gain that gives this channel the target level. `None` while the
    /// channel has too little speech.
    fn gain(&self) -> Option<f32> {
        (self.frames >= MIN_SPEECH_FRAMES).then(|| crate::level::gain(self.power.sqrt()))
    }
}

impl LiveMix {
    /// A live mix for frames at `rate`, the microphone's native rate.
    pub fn new(rate: u32) -> Self {
        let rate = rate.max(1);
        Self {
            step: f64::from(rate) / GATE_RATE as f64,
            phase: 0.0,
            sum: (0.0, 0.0),
            count: 0,
            mic16: Vec::with_capacity(2 * WINDOW),
            speaker16: Vec::with_capacity(2 * HISTORY),
            since_test: 0,
            gated: false,
            frame_len: (rate as usize / 50).max(1),
            frame: LevelFrame::default(),
            mic_level: Level::default(),
            speaker_level: Level::default(),
            mic_gain: 1.0,
            speaker_gain: 1.0,
        }
    }

    /// Mix one pair of frames to mono. `speaker` is as long as `mic`; extra
    /// samples of either are not read.
    pub fn mix(&mut self, mic: &[f32], speaker: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(mic.len());
        for (&m, &s) in mic.iter().zip(speaker) {
            self.copy(m, s);
            self.measure(m, s);
            let m = if self.gated { 0.0 } else { m };
            out.push((self.mic_gain * m + self.speaker_gain * s).clamp(-1.0, 1.0));
        }
        out
    }

    /// True while the gate zeroes the microphone.
    pub fn gated(&self) -> bool {
        self.gated
    }

    /// The gains of the microphone and the system channel.
    pub fn gains(&self) -> (f32, f32) {
        (self.mic_gain, self.speaker_gain)
    }

    /// Build the 16 kHz copy: each copy sample is the mean of the native
    /// samples in its time. The mean also removes part of the audio above
    /// 8 kHz.
    fn copy(&mut self, mic: f32, speaker: f32) {
        self.sum.0 += mic;
        self.sum.1 += speaker;
        self.count += 1;
        self.phase += 1.0;
        while self.phase >= self.step {
            self.phase -= self.step;
            let n = self.count.max(1) as f32;
            push_bounded(&mut self.mic16, self.sum.0 / n, WINDOW);
            push_bounded(&mut self.speaker16, self.sum.1 / n, HISTORY);
            self.sum = (0.0, 0.0);
            self.count = 0;
            self.since_test += 1;
            if self.since_test >= HOP {
                self.since_test = 0;
                self.gated = self.mic16.len() >= WINDOW
                    && echo_dominant(last(&self.mic16, WINDOW), last(&self.speaker16, HISTORY));
            }
        }
    }

    /// Add one pair to the level frame, and update the gains at its end.
    fn measure(&mut self, mic: f32, speaker: f32) {
        let f = &mut self.frame;
        f.samples += 1;
        f.mic += f64::from(mic * mic);
        f.speaker += f64::from(speaker * speaker);
        f.gated |= self.gated;
        if f.samples < self.frame_len {
            return;
        }
        let n = f.samples as f64;
        if !f.gated {
            self.mic_level.add((f.mic / n) as f32);
        }
        self.speaker_level.add((f.speaker / n) as f32);
        self.frame = LevelFrame::default();
        let (mic_target, speaker_target) = match (self.mic_level.gain(), self.speaker_level.gain())
        {
            (Some(m), Some(s)) => (m, s),
            _ => (1.0, 1.0),
        };
        self.mic_gain = approach(self.mic_gain, mic_target);
        self.speaker_gain = approach(self.speaker_gain, speaker_target);
    }
}

/// One step of a gain to its target, inside the bounds.
fn approach(gain: f32, target: f32) -> f32 {
    if gain == target {
        return gain;
    }
    (gain + (target - gain) / GAIN_FRAMES).clamp(MIN_GAIN, MAX_GAIN)
}

/// Push `value` and keep at least the last `keep` values. The buffer drops
/// its oldest values only when it holds twice `keep`, so a push costs little.
fn push_bounded(buffer: &mut Vec<f32>, value: f32, keep: usize) {
    if buffer.len() >= 2 * keep {
        buffer.drain(..buffer.len() - keep);
    }
    buffer.push(value);
}

fn last(buffer: &[f32], n: usize) -> &[f32] {
    &buffer[buffer.len().saturating_sub(n)..]
}

/// True when `mic` is mostly a delayed copy of the system channel. `history`
/// is the system audio that ends at the same time as `mic`, at 16 kHz. The
/// test follows anarlog's `suppress_echo_dominant_mic`: the best correlation
/// over the searched delays must be high, the part of the microphone that
/// the system channel does not explain must be small, and both channels
/// must be above their levels.
fn echo_dominant(mic: &[f32], history: &[f32]) -> bool {
    let n = mic.len();
    if n < MIN_SAMPLES || rms(mic) < MIN_MIC_RMS {
        return false;
    }
    // No part of the history of at least `MIN_SAMPLES` can reach the
    // speaker level: the search cannot pass.
    let history_energy: f32 = history.iter().map(|s| s * s).sum();
    if (history_energy / MIN_SAMPLES as f32).sqrt() < MIN_SPEAKER_RMS {
        return false;
    }
    let before = history.len().saturating_sub(n);
    let mut best: Option<EchoScore> = None;
    let mut consider = |score: Option<EchoScore>| {
        if let Some(score) = score
            && best.is_none_or(|b| score.correlation > b.correlation)
        {
            best = Some(score);
        }
    };
    // The echo is late by `lag`: the microphone sample i is the system
    // sample `lag` before it.
    let mut lag = 0;
    while lag <= MAX_LAG.min(before) {
        let start = before - lag;
        let end = (start + n).min(history.len());
        consider(score(&mic[..end - start], &history[start..end]));
        lag += LAG_STEP;
    }
    // The echo is early by `lag`: the system channel is late in the pair.
    let mut lag = LAG_STEP;
    while lag <= MAX_EARLY && n - lag >= MIN_SAMPLES {
        let start = before + lag;
        if start >= history.len() {
            break;
        }
        let end = (start + n - lag).min(history.len());
        consider(score(&mic[..end - start], &history[start..end]));
        lag += LAG_STEP;
    }
    best.is_some_and(|s| {
        s.correlation >= MIN_CORRELATION
            && s.residual_ratio <= MAX_RESIDUAL_RATIO
            && s.mic_rms >= MIN_MIC_RMS
            && s.speaker_rms >= MIN_SPEAKER_RMS
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EchoScore {
    pub(crate) correlation: f32,
    pub(crate) residual_ratio: f32,
    mic_rms: f32,
    speaker_rms: f32,
}

/// anarlog's `echo_score_at_lag` for two aligned slices of equal length.
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
    let overlap = overlap as f32;
    let residual_energy =
        (mic_energy - (cross_energy * cross_energy / speaker_energy.max(1e-6))).max(0.0);
    Some(EchoScore {
        correlation: cross_energy.abs() / (mic_energy * speaker_energy).sqrt().max(1e-6),
        residual_ratio: residual_energy.sqrt() / mic_energy.sqrt().max(1e-6),
        mic_rms: (mic_energy / overlap).sqrt(),
        speaker_rms: (speaker_energy / overlap).sqrt(),
    })
}
