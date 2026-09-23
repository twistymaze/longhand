//! [`AudioProvider`] on cpal 0.18 (KTD3).
//!
//! The device opens at its native rate. A dedicated thread owns the cpal
//! stream (which is not `Send` on every platform), starts it explicitly —
//! cpal 0.18 no longer starts a stream on build, and a stream that is never
//! started delivers nothing and reports nothing — and drains the ring into
//! fixed-size [`CaptureFrame`]s. A lost device ends the stream with
//! [`Error::MicStreamEnded`] instead of going quiet, which is the failure
//! Handy's recorder has and this crate exists to avoid.

mod callback;
mod config;
pub mod resampler;

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::time::Duration;

use cpal::traits::{HostTrait, StreamTrait};
use cpal::{SampleFormat, SupportedStreamConfig};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub use audio::{AudioProvider, CaptureConfig, CaptureFrame, CaptureStream, Error};

use callback::{FAULT_DEVICE_LOST, NO_FAULT, Shared};
use resampler::FrameResampler;

/// Frames queued between the capture thread and the consumer. At 20 ms per
/// frame this is about 1.3 s, on top of the ring's two seconds.
const FRAME_QUEUE: usize = 64;

/// How long the capture thread sleeps when the ring is empty.
const IDLE_POLL: Duration = Duration::from_millis(4);

#[derive(Default)]
pub struct CpalAudio {
    /// Configs the device accepted, by device name, so an open after
    /// [`CpalAudio::prepare`] skips the HAL property queries (R1b).
    configs: Arc<Mutex<HashMap<String, SupportedStreamConfig>>>,
    /// Whether each default input seen so far is Bluetooth.
    bluetooth: Arc<Mutex<HashMap<String, bool>>>,
}

impl CpalAudio {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve the device and its stream config ahead of the press, and
    /// return the native sample rate capture will run at. Does not open the
    /// stream: an open stream lights the system's microphone indicator.
    pub fn prepare(&self, device: Option<&str>) -> Result<u32, Error> {
        let host = cpal::default_host();
        let device = config::resolve_input(&host, device, &self.bluetooth)?;
        let config = self.config_for(&device)?;
        Ok(config.sample_rate())
    }

    fn config_for(&self, device: &cpal::Device) -> Result<SupportedStreamConfig, Error> {
        let name = config::device_name(device);
        if let Some(cached) = self
            .configs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&name)
        {
            return Ok(*cached);
        }
        let preferred = config::preferred_config(device)?;
        self.configs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name, preferred);
        Ok(preferred)
    }

    /// Open the microphone. `sample_rate` of 0 means the device's native
    /// rate; any other value is resampled to.
    fn open(
        &self,
        device: Option<String>,
        sample_rate: u32,
        chunk_size: usize,
    ) -> Result<CaptureStream, Error> {
        let (tx, rx) = mpsc::channel(FRAME_QUEUE);
        let (init_tx, init_rx) = std_mpsc::sync_channel::<Result<(), Error>>(1);
        let configs = Arc::clone(&self.configs);
        let bluetooth = Arc::clone(&self.bluetooth);
        let chunk_size = chunk_size.max(1);

        std::thread::Builder::new()
            .name("longhand-capture".into())
            .spawn(move || {
                let provider = CpalAudio { configs, bluetooth };
                let shared = Arc::new(Shared::default());
                let opened = (|| {
                    let host = cpal::default_host();
                    let device =
                        config::resolve_input(&host, device.as_deref(), &provider.bluetooth)?;
                    let config = provider.config_for(&device)?;
                    log::info!(
                        "opening '{}' at {} Hz, {} channel(s), {:?}",
                        config::device_name(&device),
                        config.sample_rate(),
                        config.channels(),
                        config.sample_format()
                    );
                    let (stream, consumer) = build(&device, &config, Arc::clone(&shared))?;
                    stream
                        .play()
                        .map_err(|e| Error::MicStreamInitializationFailed(e.to_string()))?;
                    Ok((stream, consumer, config.sample_rate()))
                })();
                let (stream, consumer, native_rate) = match opened {
                    Ok(opened) => {
                        let _ = init_tx.send(Ok(()));
                        opened
                    }
                    Err(e) => {
                        // A config that no longer opens is forgotten, so the
                        // next attempt queries the device afresh.
                        provider
                            .configs
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .clear();
                        let _ = init_tx.send(Err(e));
                        return;
                    }
                };
                let out_rate = if sample_rate == 0 {
                    native_rate
                } else {
                    sample_rate
                };
                drain(consumer, &shared, native_rate, out_rate, chunk_size, &tx);
                drop(stream);
            })
            .map_err(|e| Error::MicStreamInitializationFailed(e.to_string()))?;

        init_rx
            .recv()
            .map_err(|_| Error::MicStreamInitializationFailed("capture thread exited".into()))??;
        Ok(CaptureStream::new(ReceiverStream::new(rx)))
    }
}

fn build(
    device: &cpal::Device,
    config: &SupportedStreamConfig,
    shared: Arc<Shared>,
) -> Result<(cpal::Stream, rtrb::Consumer<f32>), Error> {
    let built = match config.sample_format() {
        SampleFormat::U8 => callback::build_stream::<u8>(device, config, shared),
        SampleFormat::I8 => callback::build_stream::<i8>(device, config, shared),
        SampleFormat::I16 => callback::build_stream::<i16>(device, config, shared),
        SampleFormat::I32 => callback::build_stream::<i32>(device, config, shared),
        SampleFormat::F32 => callback::build_stream::<f32>(device, config, shared),
        other => {
            return Err(Error::MicStreamInitializationFailed(format!(
                "unsupported sample format {other:?}"
            )));
        }
    };
    built.map_err(|e| Error::MicStreamInitializationFailed(e.to_string()))
}

/// Move samples from the ring to the consumer until the consumer goes away
/// or the stream faults.
fn drain(
    mut consumer: rtrb::Consumer<f32>,
    shared: &Shared,
    native_rate: u32,
    out_rate: u32,
    chunk_size: usize,
    tx: &mpsc::Sender<Result<CaptureFrame, Error>>,
) {
    let frame_duration = Duration::from_secs_f64(chunk_size as f64 / out_rate as f64);
    let mut resampler =
        FrameResampler::new(native_rate as usize, out_rate as usize, frame_duration);
    let mut last_overrun = 0;
    let mut last_xruns = 0;
    let mut first = true;

    loop {
        if tx.is_closed() {
            return;
        }
        let fault = shared.fault.load(Ordering::Acquire);
        if fault != NO_FAULT {
            if fault == FAULT_DEVICE_LOST {
                log::warn!("microphone lost; ending capture");
            } else {
                log::warn!("microphone stream failed; ending capture");
            }
            let _ = tx.blocking_send(Err(Error::MicStreamEnded));
            return;
        }
        let overrun = shared.overrun_samples.load(Ordering::Relaxed);
        if overrun != last_overrun {
            log::warn!(
                "capture ring overran: {} samples dropped",
                overrun - last_overrun
            );
            last_overrun = overrun;
        }
        let xruns = shared.xruns.load(Ordering::Relaxed);
        if xruns != last_xruns {
            log::warn!("device reported {} xrun(s)", xruns - last_xruns);
            last_xruns = xruns;
        }

        let available = consumer.slots();
        if available == 0 {
            std::thread::sleep(IDLE_POLL);
            continue;
        }
        let Ok(chunk) = consumer.read_chunk(available) else {
            continue;
        };
        let (a, b) = chunk.as_slices();
        let mut closed = false;
        for part in [a, b] {
            resampler.push(part, |frame| {
                if closed {
                    return;
                }
                if first {
                    log::info!("first capture frame");
                    first = false;
                }
                let frame = CaptureFrame {
                    raw_mic: Arc::from(frame),
                    raw_speaker: Arc::from([] as [f32; 0]),
                    aec_mic: None,
                };
                closed = tx.blocking_send(Ok(frame)).is_err();
            });
        }
        chunk.commit_all();
        if closed {
            return;
        }
    }
}

impl AudioProvider for CpalAudio {
    /// Microphone only (R2): the speaker half of the config is ignored.
    fn open_capture(&self, config: CaptureConfig) -> Result<CaptureStream, Error> {
        self.open(config.mic_device, config.sample_rate, config.chunk_size)
    }

    fn open_speaker_capture(
        &self,
        _sample_rate: u32,
        _chunk_size: usize,
    ) -> Result<CaptureStream, Error> {
        Err(Error::SpeakerStreamSetupFailed)
    }

    fn open_mic_capture(
        &self,
        device: Option<String>,
        sample_rate: u32,
        chunk_size: usize,
    ) -> Result<CaptureStream, Error> {
        self.open(device, sample_rate, chunk_size)
    }

    fn default_device_name(&self) -> String {
        cpal::default_host()
            .default_input_device()
            .map(|d| config::device_name(&d))
            .unwrap_or_default()
    }

    fn list_mic_devices(&self) -> Vec<String> {
        cpal::default_host()
            .input_devices()
            .map(|devices| devices.map(|d| config::device_name(&d)).collect())
            .unwrap_or_default()
    }

    /// Cues are played elsewhere (rodio, U7); nothing listens on this sender.
    fn play_silence(&self) -> std_mpsc::Sender<()> {
        std_mpsc::channel().0
    }

    fn play_bytes(&self, _bytes: &'static [u8]) -> std_mpsc::Sender<()> {
        std_mpsc::channel().0
    }

    fn probe_mic(&self, device: Option<String>) -> Result<(), Error> {
        self.prepare(device.as_deref()).map(|_| ())
    }

    fn probe_speaker(&self) -> Result<(), Error> {
        Err(Error::SpeakerStreamSetupFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speaker_is_never_captured() {
        let audio = CpalAudio::new();
        assert!(matches!(
            audio.open_speaker_capture(16_000, 320),
            Err(Error::SpeakerStreamSetupFailed)
        ));
        assert!(matches!(
            audio.probe_speaker(),
            Err(Error::SpeakerStreamSetupFailed)
        ));
    }
}
