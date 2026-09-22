//! A deterministic [`AudioProvider`] for tests: plays a fixed buffer of
//! samples as capture frames paced in real (or paused tokio) time.
//!
//! Modified from anarlog's `crates/audio-mock`: the frame loop, chunking and
//! cancellation are upstream's; the MP3 fixtures, `rodio` decoding and audible
//! playback are replaced by in-memory buffers, a tone generator and WAV
//! loading through `hound`. See PROVENANCE.md.

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures_util::Stream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, MissedTickBehavior};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;

pub use audio::{AudioProvider, CaptureConfig, CaptureFrame, CaptureStream, Error};

const MOCK_MIC_DEVICE_NAME: &str = "mock-mic";

pub struct MockAudio {
    mic: MockAudioData,
}

impl MockAudio {
    /// Mono samples at `sample_rate`, delivered once and then the stream ends.
    pub fn from_samples(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            mic: MockAudioData {
                samples: Arc::from(samples.into_boxed_slice()),
                sample_rate,
            },
        }
    }

    /// A sine tone at `frequency_hz`, amplitude 0.5, lasting `duration`.
    pub fn tone(frequency_hz: f32, sample_rate: u32, duration: Duration) -> Self {
        let len = (duration.as_secs_f64() * sample_rate as f64).round() as usize;
        let step = std::f32::consts::TAU * frequency_hz / sample_rate as f32;
        let samples = (0..len).map(|i| 0.5 * (step * i as f32).sin()).collect();
        Self::from_samples(samples, sample_rate)
    }

    /// The first channel of a WAV file.
    pub fn from_wav(path: &Path) -> Result<Self, Error> {
        let reader = hound::WavReader::open(path).map_err(|_| Error::MicStreamSetupFailed)?;
        let spec = reader.spec();
        let channels = spec.channels.max(1) as usize;
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .into_samples::<f32>()
                .step_by(channels)
                .collect::<Result<_, _>>()
                .map_err(|_| Error::MicStreamSetupFailed)?,
            hound::SampleFormat::Int => {
                let scale = (1_i64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .into_samples::<i32>()
                    .step_by(channels)
                    .map(|s| s.map(|s| (s as f32 / scale).clamp(-1.0, 1.0)))
                    .collect::<Result<_, _>>()
                    .map_err(|_| Error::MicStreamSetupFailed)?
            }
        };
        Ok(Self::from_samples(samples, spec.sample_rate))
    }

    pub fn sample_rate(&self) -> u32 {
        self.mic.sample_rate
    }
}

impl AudioProvider for MockAudio {
    fn open_capture(&self, config: CaptureConfig) -> Result<CaptureStream, Error> {
        Ok(open_capture_stream(
            self.mic.clone(),
            MockAudioData::silence(),
            config,
        ))
    }

    // Longhand captures the microphone only (R2); the speaker half of the
    // trait stays so the borrowed interface keeps its upstream shape.
    fn open_speaker_capture(
        &self,
        _sample_rate: u32,
        _chunk_size: usize,
    ) -> Result<CaptureStream, Error> {
        Err(Error::SpeakerStreamSetupFailed)
    }

    fn open_mic_capture(
        &self,
        _device: Option<String>,
        sample_rate: u32,
        chunk_size: usize,
    ) -> Result<CaptureStream, Error> {
        Ok(open_capture_stream(
            self.mic.clone(),
            MockAudioData::silence(),
            CaptureConfig {
                sample_rate,
                chunk_size,
                mic_device: None,
                enable_aec: false,
            },
        ))
    }

    fn default_device_name(&self) -> String {
        MOCK_MIC_DEVICE_NAME.to_string()
    }

    fn list_mic_devices(&self) -> Vec<String> {
        vec![MOCK_MIC_DEVICE_NAME.to_string()]
    }

    fn play_silence(&self) -> std::sync::mpsc::Sender<()> {
        let (tx, _rx) = std::sync::mpsc::channel();
        tx
    }

    fn play_bytes(&self, _bytes: &'static [u8]) -> std::sync::mpsc::Sender<()> {
        let (tx, _rx) = std::sync::mpsc::channel();
        tx
    }

    fn probe_mic(&self, _device: Option<String>) -> Result<(), Error> {
        Ok(())
    }

    fn probe_speaker(&self) -> Result<(), Error> {
        Err(Error::SpeakerStreamSetupFailed)
    }
}

#[derive(Clone)]
struct MockAudioData {
    samples: Arc<[f32]>,
    sample_rate: u32,
}

impl MockAudioData {
    fn silence() -> Self {
        Self {
            samples: Arc::from([]),
            sample_rate: 0,
        }
    }

    fn has_data(&self) -> bool {
        !self.samples.is_empty()
    }
}

struct CaptureStreamInner {
    inner: ReceiverStream<Result<CaptureFrame, Error>>,
    cancel_token: CancellationToken,
    task: JoinHandle<()>,
}

impl Stream for CaptureStreamInner {
    type Item = Result<CaptureFrame, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl Drop for CaptureStreamInner {
    fn drop(&mut self) {
        self.cancel_token.cancel();
        self.task.abort();
    }
}

fn open_capture_stream(
    mic: MockAudioData,
    speaker: MockAudioData,
    config: CaptureConfig,
) -> CaptureStream {
    let cancel_token = CancellationToken::new();
    let (tx, rx) = mpsc::channel(32);
    let task = tokio::spawn(run_capture_loop(
        tx,
        cancel_token.clone(),
        mic,
        speaker,
        config,
    ));

    CaptureStream::new(CaptureStreamInner {
        inner: ReceiverStream::new(rx),
        cancel_token,
        task,
    })
}

async fn run_capture_loop(
    tx: mpsc::Sender<Result<CaptureFrame, Error>>,
    cancel_token: CancellationToken,
    mic: MockAudioData,
    speaker: MockAudioData,
    config: CaptureConfig,
) {
    let chunk_size = config.chunk_size.max(1);
    let mut interval = tokio::time::interval(duration_for_tick(config.sample_rate, chunk_size));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut mic_position = 0;
    let mut speaker_position = 0;

    loop {
        if is_all_exhausted(&mic, mic_position, &speaker, speaker_position) {
            return;
        }

        tokio::select! {
            _ = cancel_token.cancelled() => return,
            _ = interval.tick() => {
                let raw_mic = next_chunk(&mic.samples, &mut mic_position, chunk_size);
                let raw_speaker = next_chunk(&speaker.samples, &mut speaker_position, chunk_size);
                let raw_mic = Arc::<[f32]>::from(raw_mic);
                let raw_speaker = Arc::<[f32]>::from(raw_speaker);
                let aec_mic = config.enable_aec.then(|| Arc::clone(&raw_mic));

                if tx.send(Ok(CaptureFrame { raw_mic, raw_speaker, aec_mic })).await.is_err() {
                    return;
                }
            }
        }
    }
}

fn is_all_exhausted(
    mic: &MockAudioData,
    mic_pos: usize,
    speaker: &MockAudioData,
    speaker_pos: usize,
) -> bool {
    let mic_done = !mic.has_data() || mic_pos >= mic.samples.len();
    let spk_done = !speaker.has_data() || speaker_pos >= speaker.samples.len();
    mic_done && spk_done && (mic.has_data() || speaker.has_data())
}

fn next_chunk(samples: &[f32], position: &mut usize, chunk_size: usize) -> Vec<f32> {
    let mut chunk = vec![0.0; chunk_size];
    if *position >= samples.len() {
        return chunk;
    }

    let available = (samples.len() - *position).min(chunk_size);
    chunk[..available].copy_from_slice(&samples[*position..*position + available]);
    *position += available;
    chunk
}

fn duration_for_tick(sample_rate: u32, samples_per_tick: usize) -> Duration {
    Duration::from_secs_f64(samples_per_tick as f64 / sample_rate.max(1) as f64)
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn tone_arrives_in_configured_chunks_and_ends() {
        let mock = MockAudio::tone(1_000.0, 16_000, Duration::from_secs(1));
        let mut stream = mock.open_mic_capture(None, 16_000, 1_600).unwrap();

        let mut frames = 0;
        let mut peak = 0.0_f32;
        while let Some(frame) = stream.next().await {
            let frame = frame.unwrap();
            assert_eq!(frame.raw_mic.len(), 1_600);
            peak = frame.raw_mic.iter().fold(peak, |p, s| p.max(s.abs()));
            frames += 1;
        }
        assert_eq!(frames, 10);
        assert!((peak - 0.5).abs() < 0.01, "tone peak {peak}");
    }

    #[tokio::test(start_paused = true)]
    async fn frames_are_paced_at_the_sample_rate() {
        let mock = MockAudio::tone(1_000.0, 16_000, Duration::from_secs(2));
        let mut stream = mock.open_mic_capture(None, 16_000, 1_600).unwrap();
        let start = tokio::time::Instant::now();
        while stream.next().await.is_some() {}
        // 20 frames of 100 ms; the first tick fires immediately.
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(1_900), "{elapsed:?}");
    }

    #[test]
    fn speaker_capture_is_not_offered() {
        let mock = MockAudio::tone(1_000.0, 16_000, Duration::from_millis(10));
        assert!(matches!(
            mock.open_speaker_capture(16_000, 160),
            Err(Error::SpeakerStreamSetupFailed)
        ));
    }
}
