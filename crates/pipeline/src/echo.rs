//! Echo removal and the mono mix of a stereo segment (KTD8 steps 2 to 6).
//!
//! Assemble writes the segment as a temporary 16 kHz stereo file and gives
//! each pair of samples to an [`Analysis`] as it writes them (step 1). The
//! analysis keeps the level of each 20 ms frame and measures the delay of
//! the system channel in the microphone channel for each probe window, with
//! GCC-PHAT (step 2). [`finish`] then finds the echo spans (step 3), runs the
//! echo canceller on the microphone channel inside the spans only, with the
//! system channel aligned by the measured delay as its reference, in place
//! in the temporary file (step 4), sets the gain of each channel (step 5)
//! and gives the mono mix to the caller (step 6).
//!
//! The analysis holds 13 bytes for each 20 ms frame (about 10 MB for four
//! hours) and one record for each confident probe window; the audio stays in
//! the file.
//!
//! An echo span is present echo, not dominant echo: an utterance of the
//! system channel is an echo span when the microphone holds a delayed copy of
//! it, also when room speech is louder there. Only the live gate tests for
//! dominance (KTD11).

use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use asr::Span;
use audio_sync::{GccPhatLagEstimator, SyncProbeConfig};
use serde::{Deserialize, Serialize};
use session_core::Stage;

use crate::{Control, SharedStore, StageError, checkpoint};

/// The rate of the temporary file, the probe and the canceller.
const RATE: usize = aec::SAMPLE_RATE as usize;
/// One level frame: 20 ms.
const FRAME: usize = RATE / 50;
/// The largest delay of the system channel in the microphone channel that
/// the probe searches, in each direction: 400 ms. Bluetooth output adds 150
/// to 250 ms, and the joiner adds up to 60 ms (KTD5).
const MAX_LAG: usize = RATE * 400 / 1000;
/// The probe measures one window every 500 ms.
const PROBE_HOP: usize = RATE / 2;
/// A probe window needs this RMS on the system channel (-50 dBFS), as the
/// delay probe of `audio-sync` does.
const PROBE_MIN_SYSTEM_RMS: f32 = 0.003;
/// A probe window needs this RMS on the microphone channel (-60 dBFS).
const PROBE_MIN_MIC_RMS: f32 = 0.001;
/// A confident GCC-PHAT peak. The window searches 12,801 lags, so two
/// unrelated voices give a peak ratio near 10 and a distinctiveness near 1.1;
/// speaker echo gives more than 100 and 6, and more than 29 and 3.5 under
/// louder room speech (the fixtures of `tests/assemble.rs`). The live delay
/// probe of `audio-sync` holds a lock at 8 and 1.05, which is too low here.
const MIN_PEAK_RATIO: f32 = 20.0;
const MIN_DISTINCTIVENESS: f32 = 1.5;
/// Two lags within this many samples (3 ms) are the same delay.
const LAG_TOLERANCE: isize = 48;
/// A confident window is echo only when another confident window within
/// this many probe hops has the same delay: one window alone can be chance.
const CONFIRM_HOPS: u64 = 3;
/// The power of a 20 ms frame with speech: above -50 dBFS.
const SPEECH_FLOOR: f32 = 1e-5;
/// An utterance of the system channel continues over pauses shorter than
/// this many frames (500 ms).
const UTTERANCE_GAP_FRAMES: usize = 25;
/// An echo span continues this long after its utterance ends, for the room
/// reverberation: 300 ms.
const ECHO_TAIL: usize = RATE * 3 / 10;
/// The canceller starts this long before each span: 1 s.
const PREROLL: usize = RATE;
/// Before it cleans a span, the canceller also runs over this much of the
/// span start (1 s) and does not use that output. A span starts where its
/// echo starts, so the audio before a span has no echo to adapt to; without
/// this, the first 100 ms of each span lose about 1 dB of echo, not 25 dB.
const WARMUP: usize = RATE;
/// The canceller reads this many samples at a time: 1 s, a multiple of
/// `aec::BLOCK_SHIFT`. The linear step measures one gain for each chunk; with
/// chunks of 96 ms the fixtures lose 10 dB less echo.
const CANCEL_CHUNK: usize = aec::BLOCK_SHIFT * 125;
/// The system channel is off (the Computer audio switch was off) where it
/// holds exact silence for at least this many frames (30 s).
const OFF_MIN_FRAMES: usize = 1_500;
/// A microphone frame in an echo span counts as microphone speech only when
/// the canceller kept at least this part of its power (-10 dB): what is left
/// of an echo does not count.
const NEAR_SPEECH_RATIO: f32 = 0.1;
/// Each channel needs at least this many frames with speech (1 s) to get a
/// gain other than 1.
const MIN_SPEECH_FRAMES: usize = 50;
/// The speech level that the mix gives each channel: -26 dBFS RMS.
const TARGET_RMS: f32 = 0.05;
/// The gain of a channel stays between -12 dB and +24 dB.
const MIN_GAIN: f32 = 0.25;
const MAX_GAIN: f32 = 16.0;
/// Bytes of one pair of samples in the temporary file.
const PAIR_BYTES: u64 = 8;

/// The echo spans checkpoint of one stereo segment (KTD8 step 7). U10's
/// gap fill reads it through [`saved`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EchoSpans {
    pub segment: u32,
    /// The spans of the segment where the microphone holds echo of the
    /// system audio, in ms from the segment start. The mix in the processing
    /// file has the echo removed there.
    pub spans: Vec<Span>,
    /// The gains that the mix gives the microphone and the system channel.
    pub mic_gain: f32,
    pub system_gain: f32,
}

/// The saved echo spans of a segment. `None` for a mono segment, and for a
/// stereo segment that Assemble has not finished.
pub fn saved(
    store: &SharedStore,
    session_id: &str,
    segment: u32,
) -> Result<Option<EchoSpans>, StageError> {
    checkpoint::get(
        store,
        session_id,
        Stage::Assemble,
        &checkpoint::echo_key(segment),
    )
}

/// Bytes of the temporary 16 kHz stereo file for each second.
pub const TEMPORARY_BYTES_PER_SECOND: u64 = RATE as u64 * PAIR_BYTES;

/// A probe window with a confident peak.
#[derive(Debug, Clone, Copy)]
struct Probe {
    /// The first microphone sample of the window.
    start: usize,
    /// The delay of the system channel in the microphone channel.
    lag: isize,
}

/// Step 1 and 2: writes the temporary file and measures levels and delays.
pub(crate) struct Analysis {
    temp: BufWriter<File>,
    samples: usize,
    frame_mic: f32,
    frame_system: f32,
    frame_silent: bool,
    frame_fill: usize,
    mic_power: Vec<f32>,
    system_power: Vec<f32>,
    /// Frames where the system channel is exact silence.
    system_silent: Vec<bool>,
    estimator: GccPhatLagEstimator,
    window: usize,
    mic_history: Vec<f32>,
    system_history: Vec<f32>,
    /// The sample that `mic_history[0]` holds.
    history_start: usize,
    /// The sample where the next probe window ends.
    next_probe_end: usize,
    probes: Vec<Probe>,
}

impl Analysis {
    pub(crate) fn create(temp: &Path) -> Result<Self, StageError> {
        let window = SyncProbeConfig::for_max_lag(RATE as u32, MAX_LAG).window_samples;
        Ok(Self {
            temp: BufWriter::new(crate::assemble::create_fresh(temp)?),
            samples: 0,
            frame_mic: 0.0,
            frame_system: 0.0,
            frame_silent: true,
            frame_fill: 0,
            mic_power: Vec::new(),
            system_power: Vec::new(),
            system_silent: Vec::new(),
            estimator: GccPhatLagEstimator::new(window, MAX_LAG),
            window,
            mic_history: Vec::with_capacity(window * 2),
            system_history: Vec::with_capacity(window * 2),
            history_start: 0,
            next_probe_end: window,
            probes: Vec::new(),
        })
    }

    /// Take the next samples of both channels at 16 kHz.
    pub(crate) fn push(&mut self, mic: &[f32], system: &[f32]) -> Result<(), StageError> {
        debug_assert_eq!(mic.len(), system.len());
        for (&m, &s) in mic.iter().zip(system) {
            self.temp.write_all(&m.to_le_bytes())?;
            self.temp.write_all(&s.to_le_bytes())?;
            self.frame_mic += m * m;
            self.frame_system += s * s;
            self.frame_silent &= s == 0.0;
            self.frame_fill += 1;
            if self.frame_fill == FRAME {
                self.end_frame();
            }
        }
        self.samples += mic.len();
        self.mic_history.extend_from_slice(mic);
        self.system_history.extend_from_slice(system);
        while self.next_probe_end <= self.samples {
            let start = self.next_probe_end - self.window;
            self.probe(start - self.history_start, start);
            self.next_probe_end += PROBE_HOP;
        }
        // Keep the history from the start of the next window.
        let keep_from = self.next_probe_end - self.window;
        if keep_from > self.history_start + self.window {
            let drop = keep_from - self.history_start;
            self.mic_history.drain(..drop);
            self.system_history.drain(..drop);
            self.history_start = keep_from;
        }
        Ok(())
    }

    fn end_frame(&mut self) {
        let n = self.frame_fill.max(1) as f32;
        self.mic_power.push(self.frame_mic / n);
        self.system_power.push(self.frame_system / n);
        self.system_silent.push(self.frame_silent);
        self.frame_mic = 0.0;
        self.frame_system = 0.0;
        self.frame_silent = true;
        self.frame_fill = 0;
    }

    fn probe(&mut self, from: usize, start: usize) {
        let to = from + self.window;
        let mic = &self.mic_history[from..to];
        let system = &self.system_history[from..to];
        if rms(system) < PROBE_MIN_SYSTEM_RMS || rms(mic) < PROBE_MIN_MIC_RMS {
            return;
        }
        if let Some(estimate) = self.estimator.estimate(system, mic)
            && estimate.peak_ratio >= MIN_PEAK_RATIO
            && estimate.distinctiveness >= MIN_DISTINCTIVENESS
        {
            self.probes.push(Probe {
                start,
                lag: estimate.lag_samples,
            });
        }
    }

    /// Flush the temporary file and return what pass 1 measured.
    pub(crate) fn finish(mut self) -> Result<Measured, StageError> {
        if self.frame_fill > 0 {
            self.end_frame();
        }
        self.temp.flush()?;
        drop(self.temp);
        Ok(Measured {
            samples: self.samples,
            raw_mic_power: self.mic_power.clone(),
            mic_power: self.mic_power,
            system_power: self.system_power,
            system_silent: self.system_silent,
            probes: self.probes,
            window: self.window,
        })
    }
}

/// What pass 1 measured.
pub(crate) struct Measured {
    samples: usize,
    raw_mic_power: Vec<f32>,
    /// The microphone power of each frame after the canceller.
    mic_power: Vec<f32>,
    system_power: Vec<f32>,
    system_silent: Vec<bool>,
    probes: Vec<Probe>,
    window: usize,
}

/// A stretch of the microphone channel with echo at one delay, in samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    start: usize,
    end: usize,
    delay: isize,
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn median(values: &mut [isize]) -> isize {
    values.sort_unstable();
    values[values.len() / 2]
}

/// Steps 3 to 6. Removes the echo in the temporary file, gives the mono mix
/// to `emit` and returns the echo spans row for `segment`.
pub(crate) fn finish(
    mut measured: Measured,
    temp: &Path,
    segment: u32,
    control: &dyn Control,
    emit: &mut dyn FnMut(&[f32]) -> Result<(), StageError>,
) -> Result<EchoSpans, StageError> {
    let runs = find_runs(&measured);
    if !runs.is_empty() {
        cancel(&mut measured, temp, &runs, control)?;
    }
    let (mic_gain, system_gain) = gains(&measured);
    mix(temp, measured.samples, mic_gain, system_gain, control, emit)?;
    Ok(EchoSpans {
        segment,
        spans: spans_ms(&runs),
        mic_gain,
        system_gain,
    })
}

/// Step 3: the echo runs, in microphone samples.
fn find_runs(m: &Measured) -> Vec<Run> {
    // A confident window with no neighbour at the same delay is chance. The
    // probes are in time order, one for each hop at most.
    let hop = PROBE_HOP as u64;
    let hops = CONFIRM_HOPS as usize;
    let probes = &m.probes;
    let echo: Vec<Probe> = (0..probes.len())
        .filter(|&i| {
            let p = probes[i];
            (i.saturating_sub(hops)..(i + hops + 1).min(probes.len())).any(|j| {
                let q = probes[j];
                j != i
                    && (p.start as u64).abs_diff(q.start as u64) <= CONFIRM_HOPS * hop
                    && (p.lag - q.lag).abs() <= LAG_TOLERANCE
            })
        })
        .map(|i| probes[i])
        .collect();
    if echo.is_empty() {
        return Vec::new();
    }

    let mut runs = Vec::new();
    for (u0, u1) in utterances(&m.system_power) {
        // The windows that hold the echo of this utterance. A window that
        // starts outside this range cannot hold it at any lag.
        let first = echo.partition_point(|p| p.start + m.window + MAX_LAG <= u0);
        let last = echo.partition_point(|p| p.start < u1 + MAX_LAG);
        let near: Vec<Probe> = echo[first..last.max(first)]
            .iter()
            .filter(|p| {
                let (from, to) = (p.start as isize, (p.start + m.window) as isize);
                let (a, b) = (u0 as isize + p.lag, u1 as isize + p.lag);
                a < to && b > from
            })
            .copied()
            .collect();
        if near.is_empty() {
            continue;
        }
        // Consecutive windows with the same delay make one group; a change
        // of delay starts a new run.
        let mut groups: Vec<Vec<Probe>> = vec![vec![near[0]]];
        for p in &near[1..] {
            let last = groups.last_mut().expect("one group at least");
            if (p.lag - last[last.len() - 1].lag).abs() <= LAG_TOLERANCE {
                last.push(*p);
            } else {
                groups.push(vec![*p]);
            }
        }
        let delays: Vec<isize> = groups
            .iter()
            .map(|g| median(&mut g.iter().map(|p| p.lag).collect::<Vec<_>>()))
            .collect();
        let centre = |p: &Probe| (p.start + m.window / 2) as isize;
        let mut start = u0 as isize + delays[0];
        for (k, group) in groups.iter().enumerate() {
            let end = match groups.get(k + 1) {
                Some(next) => (centre(&group[group.len() - 1]) + centre(&next[0])) / 2,
                None => u1 as isize + delays[k] + ECHO_TAIL as isize,
            };
            let (a, b) = (
                start.max(0) as usize,
                end.clamp(0, m.samples as isize) as usize,
            );
            if a < b {
                runs.push(Run {
                    start: a,
                    end: b,
                    delay: delays[k],
                });
            }
            start = end;
        }
    }
    runs.sort_by_key(|r| r.start);
    // Runs of two utterances can touch: the same delay merges, a new
    // delay starts where the next run starts.
    let mut out: Vec<Run> = Vec::with_capacity(runs.len());
    for run in runs {
        match out.last_mut() {
            Some(last) if run.start < last.end => {
                if (run.delay - last.delay).abs() <= LAG_TOLERANCE {
                    last.end = last.end.max(run.end);
                    continue;
                }
                last.end = run.start;
                out.push(run);
            }
            _ => out.push(run),
        }
    }
    out.retain(|r| r.start < r.end);
    out
}

/// The utterances of the system channel, in samples: frames with speech,
/// with pauses shorter than `UTTERANCE_GAP_FRAMES` inside.
fn utterances(system_power: &[f32]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (f, &p) in system_power.iter().enumerate() {
        if p < SPEECH_FLOOR {
            continue;
        }
        match out.last_mut() {
            Some(last) if f <= last.1 + UTTERANCE_GAP_FRAMES => last.1 = f + 1,
            _ => out.push((f, f + 1)),
        }
    }
    out.into_iter()
        .map(|(a, b)| (a * FRAME, b * FRAME))
        .collect()
}

/// The runs as spans in ms, with touching runs merged.
fn spans_ms(runs: &[Run]) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for run in runs {
        let span = Span {
            start_ms: (run.start * 1000 / RATE) as u64,
            end_ms: (run.end * 1000).div_ceil(RATE) as u64,
        };
        match out.last_mut() {
            Some(last) if span.start_ms <= last.end_ms => {
                last.end_ms = last.end_ms.max(span.end_ms)
            }
            _ => out.push(span),
        }
    }
    out
}

/// The temporary file, read and written as pairs of samples.
struct Pairs {
    file: File,
    samples: usize,
}

impl Pairs {
    /// One channel (0 the microphone, 1 the system) from `from` for `len`
    /// samples, with silence outside the file.
    fn read(&mut self, channel: usize, from: isize, len: usize) -> Result<Vec<f32>, StageError> {
        let mut out = vec![0.0f32; len];
        let a = from.max(0) as usize;
        let b = ((from + len as isize).max(0) as usize).min(self.samples);
        if a >= b {
            return Ok(out);
        }
        let pairs = self.read_pairs(a, b - a)?;
        let offset = (a as isize - from) as usize;
        for (o, pair) in out[offset..].iter_mut().zip(pairs.chunks_exact(2)) {
            *o = pair[channel];
        }
        Ok(out)
    }

    fn read_pairs(&mut self, from: usize, len: usize) -> Result<Vec<f32>, StageError> {
        let mut bytes = vec![0u8; len * PAIR_BYTES as usize];
        self.file.seek(SeekFrom::Start(from as u64 * PAIR_BYTES))?;
        self.file.read_exact(&mut bytes)?;
        Ok(bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect())
    }

    /// Replace the microphone samples from `from`, and add the change of
    /// power of each frame to `mic_power`.
    fn write_mic(
        &mut self,
        from: usize,
        mic: &[f32],
        mic_power: &mut [f32],
    ) -> Result<(), StageError> {
        let mut pairs = self.read_pairs(from, mic.len())?;
        for (i, (pair, &m)) in pairs.chunks_exact_mut(2).zip(mic).enumerate() {
            let frame = (from + i) / FRAME;
            if let Some(power) = mic_power.get_mut(frame) {
                *power += (m * m - pair[0] * pair[0]) / FRAME as f32;
            }
            pair[0] = m;
        }
        let bytes: Vec<u8> = pairs.iter().flat_map(|s| s.to_le_bytes()).collect();
        self.file.seek(SeekFrom::Start(from as u64 * PAIR_BYTES))?;
        self.file.write_all(&bytes)?;
        Ok(())
    }
}

/// Step 4: the canceller on the microphone channel, inside the runs only.
fn cancel(
    measured: &mut Measured,
    temp: &Path,
    runs: &[Run],
    control: &dyn Control,
) -> Result<(), StageError> {
    let mut canceller = aec::EchoCanceller::new()
        .map_err(|e| StageError::Failed(format!("the echo canceller: {e}")))?;
    let mut pairs = Pairs {
        file: OpenOptions::new().read(true).write(true).open(temp)?,
        samples: measured.samples,
    };
    let latency = aec::STREAMING_LATENCY_SAMPLES;
    // Runs at one delay with less than the preroll between them share one
    // pass of the canceller.
    let mut passes: Vec<Vec<Run>> = Vec::new();
    for run in runs {
        match passes.last_mut() {
            Some(pass)
                if pass[pass.len() - 1].delay == run.delay
                    && run.start < pass[pass.len() - 1].end + PREROLL =>
            {
                pass.push(*run)
            }
            _ => passes.push(vec![*run]),
        }
    }
    for pass in &passes {
        canceller.reset();
        let delay = pass[0].delay;
        let (first, last) = (pass[0].start, pass[pass.len() - 1].end);
        let mut run_chunk = |pairs: &mut Pairs, t: usize| -> Result<Vec<f32>, StageError> {
            let mic = pairs.read(0, t as isize, CANCEL_CHUNK)?;
            // The reference is the system channel aligned with its echo.
            let reference = pairs.read(1, t as isize - delay, CANCEL_CHUNK)?;
            canceller
                .cancel_chunk(&mic, &reference)
                .map_err(|e| StageError::Failed(format!("the echo canceller: {e}")))
        };
        // The canceller starts 1 s before the span and runs to 1 s into it,
        // and its output is not used: a span starts where the echo starts,
        // and the model needs echo to adapt. It then starts again one model
        // block before the span with the model state that it has, so the
        // first syllable of the span loses its echo too.
        let mut t = first.saturating_sub(PREROLL);
        while t < (first + WARMUP).min(last) {
            control.proceed()?;
            run_chunk(&mut pairs, t)?;
            t += CANCEL_CHUNK;
        }
        let mut t = first.saturating_sub(aec::BLOCK_SIZE);
        while t < last + latency {
            control.proceed()?;
            let out = run_chunk(&mut pairs, t)?;
            // The output is late: out[i] is the sample at t + i - latency.
            let (from, to) = (
                t as isize - latency as isize,
                (t + CANCEL_CHUNK - latency) as isize,
            );
            for run in pass {
                let a = from.max(run.start as isize);
                let b = to.min(run.end as isize);
                if a < b {
                    let slice = &out[(a - from) as usize..(b - from) as usize];
                    pairs.write_mic(a as usize, slice, &mut measured.mic_power)?;
                }
            }
            t += CANCEL_CHUNK;
        }
    }
    pairs.file.flush()?;
    Ok(())
}

/// Step 5: the gain of each channel. Levels count only the frames where
/// the Computer audio switch was on. When only one channel has speech, both
/// gains are 1: a stereo segment with a silent system channel is mixed as
/// the microphone alone.
fn gains(m: &Measured) -> (f32, f32) {
    let on = on_frames(&m.system_silent);
    let mut mic = (0.0f64, 0usize);
    let mut system = (0.0f64, 0usize);
    for (f, &on) in on.iter().enumerate() {
        if !on {
            continue;
        }
        let power = m.mic_power[f];
        if power >= SPEECH_FLOOR && power >= m.raw_mic_power[f] * NEAR_SPEECH_RATIO {
            mic.0 += f64::from(power);
            mic.1 += 1;
        }
        if m.system_power[f] >= SPEECH_FLOOR {
            system.0 += f64::from(m.system_power[f]);
            system.1 += 1;
        }
    }
    if mic.1 < MIN_SPEECH_FRAMES || system.1 < MIN_SPEECH_FRAMES {
        return (1.0, 1.0);
    }
    let gain = |(sum, n): (f64, usize)| {
        let level = (sum / n as f64).sqrt() as f32;
        (TARGET_RMS / level).clamp(MIN_GAIN, MAX_GAIN)
    };
    (gain(mic), gain(system))
}

/// Frames where the system channel was on: all but runs of exact silence of
/// at least `OFF_MIN_FRAMES`.
fn on_frames(silent: &[bool]) -> Vec<bool> {
    let mut on = vec![true; silent.len()];
    let mut f = 0;
    while f < silent.len() {
        if !silent[f] {
            f += 1;
            continue;
        }
        let start = f;
        while f < silent.len() && silent[f] {
            f += 1;
        }
        if f - start >= OFF_MIN_FRAMES {
            on[start..f].fill(false);
        }
    }
    on
}

/// Step 6: the mono mix of the temporary file.
fn mix(
    temp: &Path,
    samples: usize,
    mic_gain: f32,
    system_gain: f32,
    control: &dyn Control,
    emit: &mut dyn FnMut(&[f32]) -> Result<(), StageError>,
) -> Result<(), StageError> {
    let mut reader = BufReader::new(File::open(temp)?);
    let mut bytes = vec![0u8; RATE * PAIR_BYTES as usize];
    let mut out = Vec::with_capacity(RATE);
    let mut left = samples;
    while left > 0 {
        control.proceed()?;
        let n = left.min(RATE);
        let buf = &mut bytes[..n * PAIR_BYTES as usize];
        reader.read_exact(buf)?;
        out.clear();
        out.extend(buf.chunks_exact(PAIR_BYTES as usize).map(|b| {
            let mic = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            let system = f32::from_le_bytes([b[4], b[5], b[6], b[7]]);
            (mic_gain * mic + system_gain * system).clamp(-1.0, 1.0)
        }));
        emit(&out)?;
        left -= n;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utterances_continue_over_short_pauses() {
        let mut power = vec![0.0; 200];
        power[10..20].fill(1e-3);
        power[30..40].fill(1e-3);
        power[100..110].fill(1e-3);
        assert_eq!(
            utterances(&power),
            [(10 * FRAME, 40 * FRAME), (100 * FRAME, 110 * FRAME)]
        );
    }

    #[test]
    fn long_exact_silence_is_off_and_short_silence_is_on() {
        let mut silent = vec![false; 4_000];
        silent[100..200].fill(true);
        silent[1_000..2_600].fill(true);
        let on = on_frames(&silent);
        assert!(on[150]);
        assert!(!on[1_500]);
        assert!(on[2_700]);
    }

    #[test]
    fn touching_runs_are_one_span() {
        let runs = [
            Run {
                start: 16_000,
                end: 32_000,
                delay: 960,
            },
            Run {
                start: 32_000,
                end: 48_000,
                delay: 4_000,
            },
            Run {
                start: 80_000,
                end: 96_000,
                delay: 4_000,
            },
        ];
        assert_eq!(
            spans_ms(&runs),
            [
                Span {
                    start_ms: 1_000,
                    end_ms: 3_000
                },
                Span {
                    start_ms: 5_000,
                    end_ms: 6_000
                }
            ]
        );
    }
}
