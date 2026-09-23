use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

#[derive(Debug, thiserror::Error)]
pub enum CueError {
    #[error("no audio output device")]
    NoOutput,
    #[error("the audio output could not be opened: {0}")]
    Open(String),
}

/// A falling pair of tones, played twice: about 0.9 s, distinct from the
/// system's own alert sounds and from speech, and loud enough to carry across
/// a table at the system volume. Each tone carries its octave for small
/// laptop speakers, which roll off the fundamental.
const TONES_HZ: [f32; 4] = [988.0, 659.0, 988.0, 659.0];
const TONE: Duration = Duration::from_millis(180);
const GAP: Duration = Duration::from_millis(60);
const FADE: Duration = Duration::from_millis(10);
const AMPLITUDE: f32 = 0.7;

/// The cue as mono samples at `rate`.
pub fn cue_samples(rate: u32) -> Vec<f32> {
    let per = |d: Duration| (d.as_secs_f32() * rate as f32) as usize;
    let (tone, gap, fade) = (per(TONE), per(GAP), per(FADE).max(1));
    let mut out = Vec::with_capacity(TONES_HZ.len() * (tone + gap));
    for hz in TONES_HZ {
        for n in 0..tone {
            let t = n as f32 / rate as f32;
            let phase = std::f32::consts::TAU * hz * t;
            let wave = 0.75 * phase.sin() + 0.25 * (2.0 * phase).sin();
            // Raised-cosine edges, so the tones start and stop without clicks.
            let edge = n.min(tone - 1 - n);
            let envelope = if edge < fade {
                0.5 - 0.5 * (std::f32::consts::PI * edge as f32 / fade as f32).cos()
            } else {
                1.0
            };
            out.push(AMPLITUDE * envelope * wave);
        }
        out.extend(std::iter::repeat_n(0.0, gap));
    }
    out
}

/// Play the cue on the default output. Returns once the output is open, or
/// with the reason it could not be; the cue plays out on a thread of its own.
pub fn play_cue() -> Result<(), CueError> {
    let (opened_tx, opened) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("longhand-cue".into())
        .spawn(move || {
            let done = Arc::new(AtomicBool::new(false));
            let stream = match open(Arc::clone(&done)) {
                Ok(stream) => {
                    let _ = opened_tx.send(Ok(()));
                    stream
                }
                Err(e) => {
                    let _ = opened_tx.send(Err(e));
                    return;
                }
            };
            // Bounded, in case the device stops calling back mid-cue.
            for _ in 0..100 {
                if done.load(Ordering::Acquire) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            // Let the device drain its last buffer before the stream closes.
            std::thread::sleep(Duration::from_millis(100));
            drop(stream);
        })
        .map_err(|e| CueError::Open(e.to_string()))?;
    opened
        .recv()
        .map_err(|_| CueError::Open("the cue thread exited".into()))?
}

fn open(done: Arc<AtomicBool>) -> Result<cpal::Stream, CueError> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or(CueError::NoOutput)?;
    let config = device
        .default_output_config()
        .map_err(|e| CueError::Open(e.to_string()))?;
    let samples = Arc::new(cue_samples(config.sample_rate()));
    let stream = match config.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, samples, done),
        SampleFormat::I16 => build::<i16>(&device, &config, samples, done),
        SampleFormat::I32 => build::<i32>(&device, &config, samples, done),
        SampleFormat::U16 => build::<u16>(&device, &config, samples, done),
        other => {
            return Err(CueError::Open(format!(
                "unsupported sample format {other:?}"
            )));
        }
    }?;
    stream.play().map_err(|e| CueError::Open(e.to_string()))?;
    Ok(stream)
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    samples: Arc<Vec<f32>>,
    done: Arc<AtomicBool>,
) -> Result<cpal::Stream, CueError>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = config.channels() as usize;
    let position = AtomicUsize::new(0);
    device
        .build_output_stream(
            config.config(),
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let mut at = position.load(Ordering::Relaxed);
                for frame in data.chunks_mut(channels.max(1)) {
                    let value = samples.get(at).copied().unwrap_or(0.0);
                    for sample in frame {
                        *sample = T::from_sample(value);
                    }
                    at += 1;
                }
                position.store(at, Ordering::Relaxed);
                if at >= samples.len() {
                    done.store(true, Ordering::Release);
                }
            },
            |error: cpal::Error| log::warn!("cue playback: {error}"),
            None,
        )
        .map_err(|e| CueError::Open(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cue_is_under_a_second_and_never_clips() {
        let samples = cue_samples(48_000);
        let seconds = samples.len() as f32 / 48_000.0;
        assert!((0.9..1.0).contains(&seconds), "{seconds} s");
        assert!(samples.iter().all(|s| s.abs() <= AMPLITUDE));
        assert!(samples.iter().any(|s| s.abs() > AMPLITUDE / 2.0));
    }

    #[test]
    fn each_tone_starts_and_ends_silent() {
        let samples = cue_samples(44_100);
        let tone = (TONE.as_secs_f32() * 44_100.0) as usize;
        let gap = (GAP.as_secs_f32() * 44_100.0) as usize;
        for i in 0..TONES_HZ.len() {
            let start = i * (tone + gap);
            assert_eq!(samples[start], 0.0);
            assert!(samples[start + tone - 1].abs() < 1e-3);
        }
    }
}
