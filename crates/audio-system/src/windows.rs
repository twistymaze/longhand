//! The Windows backend: WASAPI loopback on the output device that
//! [`EndpointFollower`] picks (KTD4, KD12).
//!
//! From anarlog's `speaker/windows.rs` (see PROVENANCE.md), with these
//! changes:
//!
//! - The capture thread downmixes every channel to mono. anarlog keeps only
//!   the first channel.
//! - The thread resamples with [`Converter`] and sends fixed-size frames. On
//!   a switch to another output device, it takes the new format and starts
//!   the resampler again, with no segment boundary.
//! - At each follower poll, the thread reads the peak meter of each active
//!   output and applies the other-output rule ([`OtherOutputWatch`], U4
//!   step 6). The result goes to [`SystemAudioStatus`].
//! - A read error on the followed device opens the device that the rule
//!   picks now. When no device opens, the stream ends with
//!   [`Error::SpeakerStreamEnded`].

use std::collections::VecDeque;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use ::windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use ::windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, IMMDeviceEnumerator, MMDeviceEnumerator, eRender,
};
use ::windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
use tokio::sync::mpsc;
use wasapi::{
    AudioCaptureClient, AudioClient, Device, DeviceEnumerator, Direction, Handle, SampleType,
    SessionState, StreamMode, WaveFormat, initialize_mta,
};

use crate::convert::{Converter, PcmFormat, SampleKind};
use crate::follow::{ENDPOINT_POLL_INTERVAL, EndpointFollower, OtherOutputWatch};
use crate::{CaptureFrame, Error, SystemAudioStatus, speaker_frame};

/// Every endpoint opens at this rate with WASAPI's conversion, so the rate
/// stays the same across endpoint switches.
const DEFAULT_SAMPLE_RATE: u32 = 44_100;

/// How long the thread waits for the first packet event, in milliseconds.
const EVENT_WAIT_MS: u32 = 250;

/// How long [`open`] waits for the capture thread to start.
const INIT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
struct WasapiCaptureFormat {
    sample_rate: u32,
    channels: usize,
    sample_type: SampleType,
    bits_per_sample: u16,
}

impl WasapiCaptureFormat {
    fn pcm(self) -> Result<PcmFormat, Error> {
        let kind = match (self.sample_type, self.bits_per_sample) {
            (SampleType::Float, 32) => SampleKind::F32,
            (SampleType::Int, 16) => SampleKind::I16,
            (SampleType::Int, 32) => SampleKind::I32,
            (sample_type, bits) => {
                return Err(Error::SpeakerStreamInitializationFailed(format!(
                    "unsupported WASAPI capture format: {sample_type:?} {bits}-bit"
                )));
            }
        };
        Ok(PcmFormat {
            channels: self.channels,
            kind,
            sample_rate: self.sample_rate,
        })
    }
}

pub(crate) struct LoopbackCapture {
    pub(crate) audio_client: AudioClient,
    event: Handle,
    pub(crate) capture_client: AudioCaptureClient,
    pub(crate) format: PcmFormat,
    device_id: String,
    pub(crate) device_name: Option<String>,
}

pub(crate) fn failed(what: &str, e: impl std::fmt::Display) -> Error {
    Error::SpeakerStreamInitializationFailed(format!("{what}: {e}"))
}

pub(crate) fn probe() -> Result<(), Error> {
    initialize_mta()
        .ok()
        .map_err(|e| failed("COM apartment", e))?;
    let enumerator = DeviceEnumerator::new().map_err(|e| failed("device enumerator", e))?;
    enumerator
        .get_default_device(&Direction::Render)
        .map_err(|e| failed("default output device", e))?;
    Ok(())
}

pub(crate) fn open(
    sample_rate: u32,
    chunk_size: usize,
    tx: mpsc::Sender<Result<CaptureFrame, Error>>,
    status: SystemAudioStatus,
) -> Result<(), Error> {
    let (init_tx, init_rx) = std_mpsc::sync_channel::<Result<(), Error>>(1);
    std::thread::Builder::new()
        .name("longhand-system-audio".into())
        .spawn(move || run(sample_rate, chunk_size, tx, status, init_tx))
        .map_err(|e| Error::SpeakerStreamInitializationFailed(e.to_string()))?;
    match init_rx.recv_timeout(INIT_TIMEOUT) {
        Ok(result) => result,
        Err(_) => Err(Error::SpeakerStreamInitializationFailed(
            "WASAPI loopback did not start in time".into(),
        )),
    }
}

fn run(
    sample_rate: u32,
    chunk_size: usize,
    tx: mpsc::Sender<Result<CaptureFrame, Error>>,
    status: SystemAudioStatus,
    init_tx: std_mpsc::SyncSender<Result<(), Error>>,
) {
    let opened = (|| {
        initialize_mta()
            .ok()
            .map_err(|e| failed("COM apartment", e))?;
        let enumerator = DeviceEnumerator::new().map_err(|e| failed("device enumerator", e))?;
        let device = open_render_device(&enumerator)?;
        let capture = open_loopback_capture(device)?;
        Ok((enumerator, capture))
    })();
    let (enumerator, capture) = match opened {
        Ok(opened) => {
            let _ = init_tx.send(Ok(()));
            opened
        }
        Err(e) => {
            let _ = init_tx.send(Err(e));
            return;
        }
    };
    log::info!(
        "WASAPI loopback started on {:?} at {} Hz, {} channel(s)",
        capture.device_name,
        capture.format.sample_rate,
        capture.format.channels
    );
    let out_rate = if sample_rate == 0 {
        capture.format.sample_rate
    } else {
        sample_rate
    };
    let end = capture_loop(&enumerator, capture, out_rate, chunk_size, &tx, &status);
    status.set_other_output(false);
    if let Some(error) = end {
        let _ = tx.blocking_send(Err(error));
    }
    log::info!("WASAPI loopback stopped");
}

/// Returns the error that ends the stream, or `None` when the consumer
/// closed it.
fn capture_loop(
    enumerator: &DeviceEnumerator,
    mut capture: LoopbackCapture,
    out_rate: u32,
    chunk_size: usize,
    tx: &mpsc::Sender<Result<CaptureFrame, Error>>,
    status: &SystemAudioStatus,
) -> Option<Error> {
    let mut converter = Converter::new(capture.format, out_rate, chunk_size);
    let mut follower = EndpointFollower::new(capture.device_id.clone());
    let mut watch = OtherOutputWatch::new();
    let mut next_poll = Instant::now() + ENDPOINT_POLL_INTERVAL;
    let mut queue = VecDeque::new();

    loop {
        if tx.is_closed() {
            let _ = capture.audio_client.stop_stream();
            return None;
        }
        if Instant::now() >= next_poll {
            next_poll = Instant::now() + ENDPOINT_POLL_INTERVAL;
            if let Some(device) = poll_render_endpoint_switch(enumerator, &mut follower) {
                match open_loopback_capture(device) {
                    Ok(next) => {
                        switch(
                            &mut capture,
                            next,
                            &mut converter,
                            &mut follower,
                            &mut watch,
                        );
                        status.set_other_output(false);
                    }
                    Err(e) => {
                        log::warn!(
                            "WASAPI loopback could not switch; it keeps the current output: {e}"
                        );
                    }
                }
            }
            let (followed, other) = output_peaks(&capture.device_id);
            status.set_other_output(watch.observe(followed, other, Instant::now()));
        }

        if capture.event.wait_for_event(EVENT_WAIT_MS).is_err() {
            continue;
        }

        queue.clear();
        if let Err(e) = drain_packets(&capture.capture_client, &mut queue) {
            log::warn!(
                "WASAPI loopback read failed on {:?}: {e}",
                capture.device_name
            );
            let reopened = open_render_device(enumerator).and_then(open_loopback_capture);
            match reopened {
                Ok(next) => {
                    switch(
                        &mut capture,
                        next,
                        &mut converter,
                        &mut follower,
                        &mut watch,
                    );
                    status.set_other_output(false);
                    continue;
                }
                Err(e) => {
                    log::warn!("no output device opens for WASAPI loopback: {e}");
                    let _ = capture.audio_client.stop_stream();
                    return Some(Error::SpeakerStreamEnded);
                }
            }
        }
        if queue.is_empty() {
            continue;
        }

        let mut closed = false;
        converter.push(queue.make_contiguous(), |frame| {
            if !closed {
                closed = tx.blocking_send(Ok(speaker_frame(frame))).is_err();
            }
        });
        if closed {
            let _ = capture.audio_client.stop_stream();
            return None;
        }
    }
}

/// Stop the old capture and continue on `next`, with no segment boundary.
fn switch(
    capture: &mut LoopbackCapture,
    next: LoopbackCapture,
    converter: &mut Converter,
    follower: &mut EndpointFollower,
    watch: &mut OtherOutputWatch,
) {
    let _ = capture.audio_client.stop_stream();
    *capture = next;
    converter.switch(capture.format);
    *follower = EndpointFollower::new(capture.device_id.clone());
    watch.reset();
    log::info!(
        "WASAPI loopback switched to {:?} at {} Hz, {} channel(s)",
        capture.device_name,
        capture.format.sample_rate,
        capture.format.channels
    );
}

fn poll_render_endpoint_switch(
    enumerator: &DeviceEnumerator,
    follower: &mut EndpointFollower,
) -> Option<Device> {
    let device = match open_render_device(enumerator) {
        Ok(device) => device,
        Err(_) => {
            follower.reset();
            return None;
        }
    };
    let device_id = match device.get_id() {
        Ok(device_id) => device_id,
        Err(_) => {
            follower.reset();
            return None;
        }
    };
    follower.observe(device_id).map(|_| device)
}

pub(crate) fn open_loopback_capture(device: Device) -> Result<LoopbackCapture, Error> {
    let device_id = device.get_id().map_err(|e| failed("render device id", e))?;
    let device_name = device.get_friendlyname().ok();
    let (mut audio_client, accepted_format, buffer_duration_hns) =
        open_endpoint_loopback_client(&device)?;

    let format = WasapiCaptureFormat {
        sample_rate: accepted_format.get_samplespersec(),
        channels: accepted_format.get_nchannels() as usize,
        sample_type: accepted_format
            .get_subformat()
            .map_err(|e| failed("WASAPI sample type", e))?,
        bits_per_sample: accepted_format.get_bitspersample(),
    };
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns,
    };

    audio_client
        .initialize_client(&accepted_format, &Direction::Capture, &mode)
        .map_err(|e| failed("WASAPI loopback client", e))?;
    if format.sample_rate != DEFAULT_SAMPLE_RATE {
        return Err(Error::SpeakerStreamInitializationFailed(format!(
            "WASAPI loopback did not accept {DEFAULT_SAMPLE_RATE} Hz (got {})",
            format.sample_rate
        )));
    }

    let event = audio_client
        .set_get_eventhandle()
        .map_err(|e| failed("WASAPI event handle", e))?;
    let capture_client = audio_client
        .get_audiocaptureclient()
        .map_err(|e| failed("WASAPI capture client", e))?;

    audio_client
        .start_stream()
        .map_err(|e| failed("WASAPI loopback start", e))?;

    Ok(LoopbackCapture {
        audio_client,
        event,
        capture_client,
        format: format.pcm()?,
        device_id,
        device_name,
    })
}

fn open_endpoint_loopback_client(device: &Device) -> Result<(AudioClient, WaveFormat, i64), Error> {
    let audio_client = device
        .get_iaudioclient()
        .map_err(|e| failed("IAudioClient", e))?;

    let mix_format = audio_client
        .get_mixformat()
        .map_err(|e| failed("WASAPI mix format", e))?;
    let desired_format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        DEFAULT_SAMPLE_RATE as usize,
        mix_format.get_nchannels() as usize,
        Some(mix_format.get_dwchannelmask()),
    );

    let (_default_period, min_period) = audio_client
        .get_device_period()
        .map_err(|e| failed("WASAPI device period", e))?;

    Ok((audio_client, desired_format, min_period))
}

pub(crate) fn drain_packets(
    capture_client: &AudioCaptureClient,
    queue: &mut VecDeque<u8>,
) -> Result<(), Error> {
    loop {
        match capture_client.get_next_packet_size() {
            Ok(Some(frames)) if frames > 0 => {
                capture_client
                    .read_from_device_to_deque(queue)
                    .map_err(|e| failed("WASAPI read", e))?;
            }
            Ok(_) => return Ok(()),
            Err(e) => return Err(failed("WASAPI packet size", e)),
        }
    }
}

fn open_render_device(enumerator: &DeviceEnumerator) -> Result<Device, Error> {
    let default = enumerator
        .get_default_device(&Direction::Render)
        .map_err(|e| failed("default render device", e))?;

    Ok(render_device_in_use(enumerator, &default).unwrap_or(default))
}

// Meeting apps often play through an endpoint that is not the system default. When another process
// is actively rendering somewhere, follow it; the default wins ties so unrelated playback on a
// secondary device does not pull us away from a meeting on the default one.
fn render_device_in_use(enumerator: &DeviceEnumerator, default: &Device) -> Option<Device> {
    if has_foreign_active_session(default) {
        return None;
    }

    let collection = enumerator.get_device_collection(&Direction::Render).ok()?;
    collection
        .into_iter()
        .filter_map(|device| device.ok())
        .find(has_foreign_active_session)
}

fn has_foreign_active_session(device: &Device) -> bool {
    let self_pid = std::process::id();
    let Ok(manager) = device.get_iaudiosessionmanager() else {
        return false;
    };
    let Ok(sessions) = manager.get_audiosessionenumerator() else {
        return false;
    };
    let Ok(count) = sessions.get_count() else {
        return false;
    };

    (0..count).any(|index| {
        sessions.get_session(index).ok().is_some_and(|session| {
            session.get_state().ok() == Some(SessionState::Active)
                && session
                    .get_process_id()
                    .ok()
                    .is_some_and(|pid| pid != 0 && pid != self_pid)
        })
    })
}

/// The peak meter of the followed output, and the highest peak of the
/// other active outputs. A meter that cannot be read counts as silent.
fn output_peaks(followed_id: &str) -> (f32, f32) {
    let mut followed = 0.0_f32;
    let mut other = 0.0_f32;
    for (id, peak) in active_output_peaks() {
        if id == followed_id {
            followed = followed.max(peak);
        } else {
            other = other.max(peak);
        }
    }
    (followed, other)
}

/// The ID and the peak meter value of each active output device.
fn active_output_peaks() -> Vec<(String, f32)> {
    // SAFETY: the thread is in the COM multithreaded apartment
    // (`initialize_mta` in `run`), and every interface is used on it only.
    unsafe {
        let Ok(enumerator) =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return Vec::new();
        };
        let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
            return Vec::new();
        };
        let count = collection.GetCount().unwrap_or(0);
        let mut peaks = Vec::with_capacity(count as usize);
        for index in 0..count {
            let Ok(device) = collection.Item(index) else {
                continue;
            };
            let Ok(raw_id) = device.GetId() else {
                continue;
            };
            let id = raw_id.to_string();
            CoTaskMemFree(Some(raw_id.0 as *const _));
            let Ok(id) = id else {
                continue;
            };
            let peak = device
                .Activate::<IAudioMeterInformation>(CLSCTX_ALL, None)
                .and_then(|meter| meter.GetPeakValue())
                .unwrap_or(0.0);
            peaks.push((id, peak));
        }
        peaks
    }
}
