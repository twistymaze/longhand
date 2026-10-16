//! Microphone capture on cpal 0.18 (KTD3). This crate captures only the
//! microphone. `crates/audio-system` captures system audio, and the
//! provider in `src-tauri/src/audio.rs` joins the two into one
//! `AudioProvider`.
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
mod startup;

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{HostTrait, StreamTrait};
use cpal::{SampleFormat, SupportedStreamConfig};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub use audio::{CaptureFrame, CaptureStream, Error};
pub use config::find_choice;

use audio::resampler::FrameResampler;
use callback::{FAULT_DEVICE_LOST, NO_FAULT, Overrun, Shared};
use startup::Starting;

/// Frames queued between the capture thread and the consumer. At 20 ms per
/// frame this is about 1.3 s, on top of the ring's ten seconds.
const FRAME_QUEUE: usize = 64;

/// How long the capture thread sleeps when the ring is empty.
const IDLE_POLL: Duration = Duration::from_millis(4);

/// One input device: cpal's stable ID and the device name (KTD1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub id: String,
    pub name: String,
}

/// A saved microphone choice: the device ID, with the device name as its
/// label. Lookup is by ID first, then by the exact label (KTD1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicChoice {
    pub id: String,
    pub label: String,
}

/// A snapshot of the system's inputs; see [`CpalAudio::inputs`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Inputs {
    pub default: Option<Input>,
    /// Every input, by name. Inputs with the same name keep the order of
    /// the device list.
    pub all: Vec<Input>,
}

#[derive(Default)]
pub struct CpalAudio {
    /// Configs the device accepted, by device ID, so an open after
    /// [`CpalAudio::prepare`] skips the HAL property queries (R1b).
    configs: Arc<Mutex<HashMap<String, SupportedStreamConfig>>>,
    /// Whether each default input seen so far is Bluetooth, by device ID.
    bluetooth: Arc<Mutex<HashMap<String, bool>>>,
}

impl CpalAudio {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget every device's config, so the next open queries the device
    /// again: after a loss the device may be another one, or the same one at
    /// a new rate.
    pub fn forget(&self) {
        self.configs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// The inputs the system offers now: the default and every device, with
    /// ID and name. Compared between polls, a change is what "the microphone
    /// came back" looks like without a platform device-event listener (U14).
    pub fn inputs() -> Inputs {
        let host = cpal::default_host();
        let default = host.default_input_device().map(|d| config::input_of(&d));
        let mut all: Vec<Input> = host
            .input_devices()
            .map(|devices| devices.map(|d| config::input_of(&d)).collect())
            .unwrap_or_default();
        // A stable sort, so inputs with the same name keep the device order.
        all.sort_by(|a, b| a.name.cmp(&b.name));
        Inputs { default, all }
    }

    /// Resolve the device and its stream config ahead of the press, and
    /// return the native sample rate capture will run at. `None` is the
    /// system default. Does not open the stream: an open stream lights the
    /// system's microphone indicator.
    pub fn prepare(&self, choice: Option<&MicChoice>) -> Result<u32, Error> {
        self.prepare_input(choice).map(|(_, rate)| rate)
    }

    /// [`CpalAudio::prepare`], and also the input that capture will open:
    /// the name that the recording page shows (R4).
    pub fn prepare_input(&self, choice: Option<&MicChoice>) -> Result<(Input, u32), Error> {
        let host = cpal::default_host();
        let device = config::resolve_input(&host, choice, &self.bluetooth)?;
        let config = self.config_for(&device)?;
        Ok((config::input_of(&device), config.sample_rate()))
    }

    fn config_for(&self, device: &cpal::Device) -> Result<SupportedStreamConfig, Error> {
        let key = config::cache_key(device);
        if let Some(cached) = self
            .configs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
        {
            return Ok(*cached);
        }
        let preferred = config::preferred_config(device)?;
        self.configs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, preferred);
        Ok(preferred)
    }

    /// Open the chosen microphone, or the system default for `None`, at the
    /// device's native rate, in frames of `1 / frames_per_second` s. The
    /// device lookup, the config query, the stream build and `play` all run
    /// on the capture thread within [`startup::START_LIMIT`] (KTD3). The open
    /// fails at once with [`Error::MicStartCancelled`] when `cancelled`
    /// returns true, and with [`Error::MicStreamInitializationFailed`] while
    /// an earlier start has not returned (KTD4). A choice that matches no
    /// input fails with [`Error::NoInputDevice`].
    pub fn open_native(
        &self,
        choice: Option<MicChoice>,
        frames_per_second: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Opened, Error> {
        self.open_with(choice, Frames::PerSecond(frames_per_second), cancelled)
    }

    /// Open the chosen microphone, or the system default for `None`.
    /// `sample_rate` of 0 means the device's native rate; any other value is
    /// resampled to. A choice that matches no input fails with
    /// [`Error::NoInputDevice`].
    pub fn open_input(
        &self,
        choice: Option<MicChoice>,
        sample_rate: u32,
        chunk_size: usize,
    ) -> Result<CaptureStream, Error> {
        let frames = Frames::Fixed {
            sample_rate,
            chunk_size,
        };
        self.open_with(choice, frames, &|| false)
            .map(|opened| opened.stream)
    }

    fn open_with(
        &self,
        choice: Option<MicChoice>,
        frames: Frames,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Opened, Error> {
        let (tx, rx) = mpsc::channel(FRAME_QUEUE);
        let configs = Arc::clone(&self.configs);
        let bluetooth = Arc::clone(&self.bluetooth);

        let started = startup::start(
            &startup::MIC_STARTING,
            startup::START_LIMIT,
            cancelled,
            move |starting: Starting<(Input, u32)>| {
                let provider = CpalAudio { configs, bluetooth };
                let shared = Arc::new(Shared::default());
                let open_device = |device: &cpal::Device| {
                    let config = provider.config_for(device)?;
                    let input = config::input_of(device);
                    log::info!(
                        "opening '{}' at {} Hz, {} channel(s), {:?}",
                        input.name,
                        config.sample_rate(),
                        config.channels(),
                        config.sample_format()
                    );
                    let (stream, consumer, gaps) = build(device, &config, Arc::clone(&shared))?;
                    stream
                        .play()
                        .map_err(|e| Error::MicStreamInitializationFailed(e.to_string()))?;
                    Ok((stream, consumer, gaps, config.sample_rate(), input))
                };
                let opened = (|| {
                    let host = cpal::default_host();
                    let (device, default_handle) =
                        config::resolve_input_from(&host, choice.as_ref(), &provider.bluetooth)?;
                    let first = if forced_default_failure(default_handle, false) {
                        Err(Error::MicStreamInitializationFailed(
                            "forced failure of the default microphone".into(),
                        ))
                    } else {
                        open_device(&device)
                    };
                    if !config::retry_by_id(default_handle, first.is_err()) {
                        return first;
                    }
                    retry_default_by_id(&host, &provider, first, &open_device)
                })();
                let (stream, consumer, gaps, native_rate) = match opened {
                    Ok((stream, consumer, gaps, native_rate, input)) => {
                        starting.report(Ok((input, native_rate)));
                        (stream, consumer, gaps, native_rate)
                    }
                    Err(e) => {
                        // A config that no longer opens is forgotten, so the
                        // next attempt queries the device afresh.
                        provider.forget();
                        starting.report(Err(e));
                        return;
                    }
                };
                let (out_rate, chunk_size) = frames.at(native_rate);
                drain(
                    consumer,
                    gaps,
                    &shared,
                    native_rate,
                    out_rate,
                    chunk_size,
                    &tx,
                );
                drop(stream);
            },
        );
        match started {
            Ok((input, rate)) => Ok(Opened {
                input,
                rate,
                stream: CaptureStream::new(ReceiverStream::new(rx)),
            }),
            Err(e) => {
                // The next start queries the device again. After a timeout,
                // the cached config can be the cause.
                self.forget();
                Err(e)
            }
        }
    }
}

/// A microphone that started: the input, its native rate, and its frames.
pub struct Opened {
    pub input: Input,
    pub rate: u32,
    pub stream: CaptureStream,
}

/// The rate and the frame size of the frames that capture sends.
#[derive(Debug, Clone, Copy)]
enum Frames {
    /// `sample_rate` of 0 is the native rate.
    Fixed { sample_rate: u32, chunk_size: usize },
    /// The native rate, in this many frames per second.
    PerSecond(u32),
}

impl Frames {
    /// The output rate and the frame size for a device at `native_rate`.
    fn at(self, native_rate: u32) -> (u32, usize) {
        match self {
            Frames::Fixed {
                sample_rate,
                chunk_size,
            } => {
                let rate = if sample_rate == 0 {
                    native_rate
                } else {
                    sample_rate
                };
                (rate, chunk_size.max(1))
            }
            Frames::PerSecond(per_second) => (
                native_rate,
                (native_rate / per_second.max(1)).max(1) as usize,
            ),
        }
    }
}

/// On Windows, open the current default input again, found by its ID, after
/// the default handle failed (KTD5, [`config::retry_by_id`]). Other
/// platforms return `first`.
#[cfg(windows)]
fn retry_default_by_id<T>(
    host: &cpal::Host,
    provider: &CpalAudio,
    first: Result<T, Error>,
    open_device: &dyn Fn(&cpal::Device) -> Result<T, Error>,
) -> Result<T, Error> {
    let Err(error) = first else { return first };
    let Some(device) = config::default_by_id(host) else {
        log::warn!("the default microphone did not open ({error}), and no device has its ID");
        return Err(error);
    };
    log::warn!("the default microphone did not open ({error}); opening it again by its ID");
    provider.forget();
    if forced_default_failure(true, true) {
        return Err(Error::MicStreamInitializationFailed(
            "forced failure of the default microphone by its ID".into(),
        ));
    }
    open_device(&device)
}

#[cfg(not(windows))]
fn retry_default_by_id<T>(
    _host: &cpal::Host,
    _provider: &CpalAudio,
    first: Result<T, Error>,
    _open_device: &dyn Fn(&cpal::Device) -> Result<T, Error>,
) -> Result<T, Error> {
    first
}

/// A Windows debug build fails the open of the default microphone when
/// `LONGHAND_DEBUG_FAIL_DEFAULT_MIC` is set, so that the retry by ID can be
/// tested (KTD6). The value `all` also fails the retry. A release build
/// never reads the variable.
#[cfg(all(windows, debug_assertions))]
fn forced_default_failure(default_handle: bool, by_id: bool) -> bool {
    match std::env::var("LONGHAND_DEBUG_FAIL_DEFAULT_MIC") {
        Ok(value) if by_id => value == "all",
        Ok(_) => default_handle,
        Err(_) => false,
    }
}

#[cfg(not(all(windows, debug_assertions)))]
fn forced_default_failure(_default_handle: bool, _by_id: bool) -> bool {
    false
}

fn build(
    device: &cpal::Device,
    config: &SupportedStreamConfig,
    shared: Arc<Shared>,
) -> Result<(cpal::Stream, rtrb::Consumer<f32>, rtrb::Consumer<Overrun>), Error> {
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

const SILENCE: [f32; 4_096] = [0.0; 4_096];

/// Pass `parts` (the next frames read from the ring, `consumed` frames in) to
/// `feed`, with silence the length of every overrun at its position, so what
/// leaves this thread is as long as the time that passed (U25). Overruns the
/// callback could not place are filled here, where they were noticed.
fn feed_with_silence(
    parts: [&[f32]; 2],
    consumed: &mut u64,
    gaps: &mut rtrb::Consumer<Overrun>,
    unplaced: u64,
    mut feed: impl FnMut(&[f32]),
) {
    let silence = |mut frames: u64, feed: &mut dyn FnMut(&[f32])| {
        while frames > 0 {
            let n = frames.min(SILENCE.len() as u64) as usize;
            feed(&SILENCE[..n]);
            frames -= n as u64;
        }
    };
    silence(unplaced, &mut feed);
    for mut part in parts {
        while !part.is_empty() {
            let next = gaps.peek().ok().copied();
            match next {
                Some(gap) if gap.at <= *consumed => {
                    let _ = gaps.pop();
                    silence(gap.frames, &mut feed);
                }
                Some(gap) if gap.at < *consumed + part.len() as u64 => {
                    let (before, after) = part.split_at((gap.at - *consumed) as usize);
                    feed(before);
                    *consumed += before.len() as u64;
                    part = after;
                }
                _ => {
                    feed(part);
                    *consumed += part.len() as u64;
                    part = &[];
                }
            }
        }
    }
    // An overrun right after the last frame read is filled now: the frames
    // after it were committed, or it would not have been reported.
    while let Ok(gap) = gaps.peek().copied() {
        if gap.at > *consumed {
            break;
        }
        let _ = gaps.pop();
        silence(gap.frames, &mut feed);
    }
}

/// Move samples from the ring to the consumer until the consumer goes away
/// or the stream faults. On a fault, the drain first sends the audio that is
/// in the ring and the audio that the resampler holds, then the error.
fn drain(
    mut consumer: rtrb::Consumer<f32>,
    mut gaps: rtrb::Consumer<Overrun>,
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
    let mut consumed = 0u64;
    let mut closed = false;
    let mut send = |frame: &[f32], closed: &mut bool| {
        if *closed {
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
        *closed = tx.blocking_send(Ok(frame)).is_err();
    };

    loop {
        if tx.is_closed() {
            return;
        }
        // Read the fault before the ring: all the frames that the callback
        // wrote before the fault are then in `available`.
        let fault = shared.fault.load(Ordering::Acquire);
        let overrun = shared.overrun_samples.load(Ordering::Relaxed);
        if overrun != last_overrun {
            log::warn!(
                "capture ring overran: {} samples dropped and replaced with silence",
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
        if available == 0 && fault == NO_FAULT {
            std::thread::sleep(IDLE_POLL);
            continue;
        }
        if available > 0
            && let Ok(chunk) = consumer.read_chunk(available)
        {
            let (a, b) = chunk.as_slices();
            let unplaced = shared.unplaced.swap(0, Ordering::Relaxed);
            feed_with_silence([a, b], &mut consumed, &mut gaps, unplaced, |part| {
                resampler.push(part, |frame| send(frame, &mut closed));
            });
            chunk.commit_all();
            if closed {
                return;
            }
        }
        if fault != NO_FAULT {
            if fault == FAULT_DEVICE_LOST {
                log::warn!("microphone lost; ending capture");
            } else {
                log::warn!("microphone stream failed; ending capture");
            }
            resampler.finish(|frame| send(frame, &mut closed));
            if !closed {
                let _ = tx.blocking_send(Err(Error::MicStreamEnded));
            }
            return;
        }
    }
}

/// The trait names a device by its name only: a choice with no ID, which
/// matches by the exact name.
fn by_name(device: Option<String>) -> Option<MicChoice> {
    device.map(|label| MicChoice {
        id: String::new(),
        label,
    })
}

/// The microphone half of `AudioProvider`. The provider in `src-tauri`
/// calls these methods.
impl CpalAudio {
    /// Open the microphone `device`, or the system default for `None`.
    /// `sample_rate` of 0 means the device's native rate.
    pub fn open_mic_capture(
        &self,
        device: Option<String>,
        sample_rate: u32,
        chunk_size: usize,
    ) -> Result<CaptureStream, Error> {
        self.open_input(by_name(device), sample_rate, chunk_size)
    }

    pub fn default_device_name(&self) -> String {
        cpal::default_host()
            .default_input_device()
            .map(|d| config::device_name(&d))
            .unwrap_or_default()
    }

    pub fn list_mic_devices(&self) -> Vec<String> {
        cpal::default_host()
            .input_devices()
            .map(|devices| devices.map(|d| config::device_name(&d)).collect())
            .unwrap_or_default()
    }

    pub fn probe_mic(&self, device: Option<String>) -> Result<(), Error> {
        self.prepare(by_name(device).as_ref()).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fed(parts: [&[f32]; 2], consumed: &mut u64, gaps: &[Overrun], unplaced: u64) -> Vec<f32> {
        let (mut tx, mut rx) = rtrb::RingBuffer::new(8);
        for gap in gaps {
            tx.push(*gap).unwrap();
        }
        let mut out = Vec::new();
        feed_with_silence(parts, consumed, &mut rx, unplaced, |p| {
            out.extend_from_slice(p)
        });
        out
    }

    #[test]
    fn an_overrun_becomes_silence_at_its_position() {
        let mut consumed = 0;
        let out = fed(
            [&[1.0, 2.0, 3.0], &[4.0]],
            &mut consumed,
            &[Overrun { at: 2, frames: 3 }],
            0,
        );
        assert_eq!(out, [1.0, 2.0, 0.0, 0.0, 0.0, 3.0, 4.0]);
        assert_eq!(consumed, 4, "silence is not ring frames");
    }

    #[test]
    fn an_overrun_between_the_two_slices_or_at_the_end_is_filled() {
        let mut consumed = 10;
        let gaps = [Overrun { at: 12, frames: 1 }, Overrun { at: 13, frames: 2 }];
        let out = fed([&[1.0, 2.0], &[3.0]], &mut consumed, &gaps, 0);
        assert_eq!(out, [1.0, 2.0, 0.0, 3.0, 0.0, 0.0]);
    }

    #[test]
    fn a_later_overrun_waits_and_an_unplaced_one_leads() {
        let mut consumed = 0;
        let out = fed(
            [&[1.0], &[]],
            &mut consumed,
            &[Overrun { at: 5, frames: 2 }],
            2,
        );
        assert_eq!(out, [0.0, 0.0, 1.0]);
    }

    /// C16: a fault does not drop the audio that is already in the ring.
    /// The drain sends all of it, with the resampler tail, before the error.
    #[test]
    fn a_fault_sends_the_ring_audio_and_the_resampler_tail_before_the_error() {
        const NATIVE: u32 = 48_000;
        const OUT: u32 = 16_000;
        const FRAME: usize = 480;
        const INPUT: usize = 24_000;
        let (mut producer, consumer) = rtrb::RingBuffer::new(INPUT);
        for _ in 0..INPUT {
            producer.push(0.25).unwrap();
        }
        let (_gaps_in, gaps) = rtrb::RingBuffer::new(1);
        let shared = Shared::default();
        shared.fault.store(FAULT_DEVICE_LOST, Ordering::Release);
        let (tx, mut rx) = mpsc::channel(1024);

        drain(consumer, gaps, &shared, NATIVE, OUT, FRAME, &tx);
        drop(tx);

        let mut total = 0;
        let mut ended = false;
        while let Ok(item) = rx.try_recv() {
            assert!(!ended, "nothing comes after the error");
            match item {
                Ok(frame) => {
                    assert_eq!(frame.raw_mic.len(), FRAME, "every frame is whole");
                    total += frame.raw_mic.len();
                }
                Err(Error::MicStreamEnded) => ended = true,
                Err(e) => panic!("unexpected error {e:?}"),
            }
        }
        assert!(ended, "the drain reports the end of the stream");
        // All input at the output rate. The resampler delay and the padding
        // of the last frame can add less than two frames.
        let expected = INPUT * OUT as usize / NATIVE as usize;
        assert!(
            (expected..expected + 2 * FRAME).contains(&total),
            "{expected} samples of output expected, got {total}"
        );
    }

    #[test]
    fn a_long_overrun_is_fed_in_bounded_pieces() {
        let (mut tx, mut rx) = rtrb::RingBuffer::new(1);
        tx.push(Overrun {
            at: 0,
            frames: 10_000,
        })
        .unwrap();
        let (mut total, mut largest) = (0, 0);
        feed_with_silence([&[], &[]], &mut 0, &mut rx, 0, |p| {
            total += p.len();
            largest = largest.max(p.len());
        });
        assert_eq!((total, largest), (10_000, SILENCE.len()));
    }
}
