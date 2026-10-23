//! WASAPI loopback of one output device. The Teams mode of the Windows app
//! capture records the output device of the Teams session with it (KTD5).
//!
//! From anarlog's `speaker/windows.rs` (see PROVENANCE.md), with these
//! changes: the capture format holds every channel, so the caller can
//! downmix all channels to mono (anarlog keeps only the first channel).

use std::collections::VecDeque;

use wasapi::{
    AudioCaptureClient, AudioClient, Device, DeviceEnumerator, Direction, Handle, SampleType,
    StreamMode, WaveFormat, initialize_mta,
};

use crate::Error;
use crate::convert::{PcmFormat, SampleKind};

/// Every endpoint opens at this rate with WASAPI's conversion, so the rate
/// stays the same across endpoint switches.
const DEFAULT_SAMPLE_RATE: u32 = 44_100;

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
    /// The client signals this event. It must stay open while the client runs.
    _event: Handle,
    pub(crate) capture_client: AudioCaptureClient,
    pub(crate) format: PcmFormat,
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

pub(crate) fn open_loopback_capture(device: Device) -> Result<LoopbackCapture, Error> {
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
        _event: event,
        capture_client,
        format: format.pcm()?,
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
