//! The macOS backend: a global Core Audio process tap in a private
//! aggregate device (KTD4, KTD13).
//!
//! From anarlog's `speaker/macos.rs` (see PROVENANCE.md), with these
//! changes:
//!
//! - The tap excludes Longhand's own process, so the alert sound and other
//!   Longhand sounds are not in the note.
//! - A dedicated thread owns the tap and the device, like the microphone
//!   capture in `crates/audio-cpal`. The IO proc writes into an `rtrb` ring.
//!   The thread reads the ring, resamples with [`RateFollower`] and sends
//!   fixed-size frames.
//! - The thread reads the rate that the device measures on the first
//!   callback and every [`DEVICE_CHECK`] after it, not only the format that
//!   the tap reported when it was created.
//! - A device that is no longer alive ends the stream with
//!   [`Error::SpeakerStreamEnded`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use ca::aggregate_device_keys as agg_keys;
use cidre::{arc, av, cat, cf, core_audio as ca, ns, os};
use tokio::sync::mpsc;

use crate::rate::{RateFollower, snap_rate};
use crate::{CaptureFrame, Error, own_process_exclusion, speaker_frame};

/// The name of the private aggregate device that holds the tap.
const TAP_DEVICE_NAME: &str = "Longhand System Audio";

/// Seconds of audio that the ring holds between the IO proc and the thread.
const RING_SECONDS: usize = 10;

/// The ring's capacity when the tap reports no usable rate.
const FALLBACK_RATE: u32 = 48_000;

/// How long the thread sleeps when the ring is empty.
const IDLE_POLL: Duration = Duration::from_millis(4);

/// Time between two checks of the device: is it alive, and at which rate
/// does it run.
const DEVICE_CHECK: Duration = Duration::from_millis(500);

/// What the IO proc needs. The thread keeps it in a `Box` that outlives the
/// started device.
struct Ctx {
    common_format: av::audio::CommonFormat,
    producer: rtrb::Producer<f32>,
    dropped: Arc<AtomicU64>,
}

pub(crate) fn probe() -> Result<(), Error> {
    let device = ca::System::default_output_device()
        .map_err(|e| Error::SpeakerStreamInitializationFailed(format!("{e:?}")))?;
    if device.is_unknown() {
        return Err(Error::SpeakerStreamSetupFailed);
    }
    Ok(())
}

pub(crate) fn open(
    sample_rate: u32,
    chunk_size: usize,
    tx: mpsc::Sender<Result<CaptureFrame, Error>>,
) -> Result<(), Error> {
    let (init_tx, init_rx) = std_mpsc::sync_channel::<Result<(), Error>>(1);
    std::thread::Builder::new()
        .name("longhand-system-audio".into())
        .spawn(move || run(sample_rate, chunk_size, tx, init_tx))
        .map_err(|e| Error::SpeakerStreamInitializationFailed(e.to_string()))?;
    init_rx.recv().map_err(|_| {
        Error::SpeakerStreamInitializationFailed("system audio thread exited".into())
    })?
}

fn failed(what: &str, e: impl std::fmt::Debug) -> Error {
    Error::SpeakerStreamInitializationFailed(format!("{what}: {e:?}"))
}

/// Longhand's own process, as the tap description wants it.
fn exclusion() -> arc::R<ns::Array<ns::Number>> {
    let excluded = own_process_exclusion(std::process::id(), |pid| {
        let pid = i32::try_from(pid).map_err(|e| e.to_string())?;
        match ca::Process::with_pid(pid) {
            Ok(process) if process.0 == ca::Obj::UNKNOWN => Ok(None),
            Ok(process) => Ok(Some(ns::Number::with_u32(process.0.0))),
            Err(e) => Err(format!("{e:?}")),
        }
    });
    if !excluded.is_empty() {
        log::info!("the system audio tap excludes Longhand's own process");
    }
    ns::Array::from_slice_retained(&excluded)
}

fn aggregate_desc(
    tap: &ca::TapGuard,
) -> Result<arc::R<cf::DictionaryOf<cf::String, cf::Type>>, Error> {
    let tap_uid = tap.uid().map_err(|e| failed("tap UID", e))?;
    let sub_tap =
        cf::DictionaryOf::with_keys_values(&[ca::sub_device_keys::uid()], &[tap_uid.as_type_ref()]);
    Ok(cf::DictionaryOf::with_keys_values(
        &[
            agg_keys::is_private(),
            agg_keys::tap_auto_start(),
            agg_keys::name(),
            agg_keys::uid(),
            agg_keys::tap_list(),
        ],
        &[
            cf::Boolean::value_true().as_type_ref(),
            cf::Boolean::value_false(),
            cf::String::from_str(TAP_DEVICE_NAME).as_ref(),
            &cf::Uuid::new().to_cf_string(),
            &cf::ArrayOf::from_slice(&[sub_tap.as_ref()]),
        ],
    ))
}

extern "C" fn proc(
    _device: ca::Device,
    _now: &cat::AudioTimeStamp,
    input_data: &cat::AudioBufList<1>,
    _input_time: &cat::AudioTimeStamp,
    _output_data: &mut cat::AudioBufList<1>,
    _output_time: &cat::AudioTimeStamp,
    ctx: Option<&mut Ctx>,
) -> os::Status {
    let Some(ctx) = ctx else {
        return os::Status::NO_ERR;
    };
    let buffer = &input_data.buffers[0];
    match ctx.common_format {
        av::audio::CommonFormat::PcmF32 => push::<f32>(ctx, buffer, |s| s),
        av::audio::CommonFormat::PcmF64 => push::<f64>(ctx, buffer, |s| s as f32),
        av::audio::CommonFormat::PcmI32 => push::<i32>(ctx, buffer, |s| s as f32 / 2_147_483_648.0),
        av::audio::CommonFormat::PcmI16 => push::<i16>(ctx, buffer, |s| f32::from(s) / 32_768.0),
        _ => {}
    }
    os::Status::NO_ERR
}

fn read_samples<T: Copy>(buffer: &cat::AudioBuf) -> Option<&[T]> {
    let byte_count = buffer.data_bytes_size as usize;
    if byte_count == 0 || buffer.data.is_null() {
        return None;
    }
    let data = buffer.data as *const T;
    if !(data as usize).is_multiple_of(std::mem::align_of::<T>()) {
        return None;
    }
    let sample_count = byte_count / std::mem::size_of::<T>();
    if sample_count == 0 {
        return None;
    }
    // SAFETY: Core Audio owns the buffer for the length of the callback, the
    // pointer is aligned for `T`, and `sample_count` samples fit in it.
    Some(unsafe { std::slice::from_raw_parts(data, sample_count) })
}

/// Real-time safe: no lock and no allocation. Samples that do not fit in
/// the ring are counted, not written.
fn push<T: Copy>(ctx: &mut Ctx, buffer: &cat::AudioBuf, convert: impl Fn(T) -> f32) {
    let Some(samples) = read_samples::<T>(buffer) else {
        return;
    };
    let n = samples.len().min(ctx.producer.slots());
    if n > 0
        && let Ok(chunk) = ctx.producer.write_chunk_uninit(n)
    {
        chunk.fill_from_iter(samples[..n].iter().map(|s| convert(*s)));
    }
    let dropped = samples.len() - n;
    if dropped > 0 {
        ctx.dropped.fetch_add(dropped as u64, Ordering::Relaxed);
    }
}

/// The rate that the aggregate device runs at: the rate it measures, then
/// its nominal rate, then the tap's format.
fn device_rate(device: &ca::Device, tap: &ca::TapGuard) -> Option<u32> {
    device
        .actual_sample_rate()
        .ok()
        .and_then(snap_rate)
        .or_else(|| device.nominal_sample_rate().ok().and_then(snap_rate))
        .or_else(|| tap.asbd().ok().and_then(|asbd| snap_rate(asbd.sample_rate)))
}

fn run(
    sample_rate: u32,
    chunk_size: usize,
    tx: mpsc::Sender<Result<CaptureFrame, Error>>,
    init_tx: std_mpsc::SyncSender<Result<(), Error>>,
) {
    let dropped = Arc::new(AtomicU64::new(0));
    let opened = (|| {
        let excluded = exclusion();
        let desc = ca::TapDesc::with_mono_global_tap_excluding_processes(&excluded);
        let tap = desc
            .create_process_tap()
            .map_err(|e| failed("process tap", e))?;
        let asbd = tap.asbd().map_err(|e| failed("tap format", e))?;
        let format = av::AudioFormat::with_asbd(&asbd)
            .ok_or_else(|| failed("tap format", "unsupported system audio format"))?;
        let tap_rate = snap_rate(asbd.sample_rate).unwrap_or(FALLBACK_RATE);
        let (producer, consumer) = rtrb::RingBuffer::new(tap_rate as usize * RING_SECONDS);
        let mut ctx = Box::new(Ctx {
            common_format: format.common_format(),
            producer,
            dropped: Arc::clone(&dropped),
        });
        let agg_desc = aggregate_desc(&tap)?;
        let device =
            ca::AggregateDevice::with_desc(&agg_desc).map_err(|e| failed("aggregate device", e))?;
        let proc_id = device
            .create_io_proc_id(proc, Some(&mut *ctx))
            .map_err(|e| failed("IO proc", e))?;
        let started =
            ca::device_start(device, Some(proc_id)).map_err(|e| failed("device start", e))?;
        log::info!(
            "system audio tap started at {tap_rate} Hz, {} channel(s)",
            asbd.channels_per_frame
        );
        Ok((tap, ctx, started, consumer, tap_rate))
    })();
    // Drop order matters: `started` stops the device before `ctx`, which the
    // IO proc reads, is freed.
    let (tap, ctx, started, consumer, tap_rate) = match opened {
        Ok(opened) => {
            let _ = init_tx.send(Ok(()));
            opened
        }
        Err(e) => {
            let _ = init_tx.send(Err(e));
            return;
        }
    };
    let out_rate = if sample_rate == 0 {
        tap_rate
    } else {
        sample_rate
    };
    drain(
        consumer, &started, &tap, &dropped, tap_rate, out_rate, chunk_size, &tx,
    );
    drop(started);
    drop(ctx);
    drop(tap);
    log::info!("system audio tap stopped");
}

#[allow(clippy::too_many_arguments)]
fn drain(
    mut consumer: rtrb::Consumer<f32>,
    device: &ca::Device,
    tap: &ca::TapGuard,
    dropped: &AtomicU64,
    tap_rate: u32,
    out_rate: u32,
    chunk_size: usize,
    tx: &mpsc::Sender<Result<CaptureFrame, Error>>,
) {
    let mut rate = RateFollower::new(tap_rate, out_rate, chunk_size);
    let mut next_check = Instant::now();
    let mut first = true;
    let mut last_dropped = 0;
    let mut closed = false;
    let send = |frame: &[f32], closed: &mut bool| {
        if !*closed {
            *closed = tx.blocking_send(Ok(speaker_frame(frame))).is_err();
        }
    };

    loop {
        if closed || tx.is_closed() {
            return;
        }
        let available = consumer.slots();
        let now = Instant::now();
        // The first callback, and every check after it: the device can run
        // at a rate other than the one the tap reported.
        if (first && available > 0) || (!first && now >= next_check) {
            next_check = now + DEVICE_CHECK;
            if !device.is_alive().unwrap_or(false) {
                log::warn!("the system audio device is no longer alive; ending system audio");
                let _ = tx.blocking_send(Err(Error::SpeakerStreamEnded));
                return;
            }
            if let Some(hz) = device_rate(device, tap) {
                rate.set_in_hz(hz, |frame| send(frame, &mut closed));
            }
            let total = dropped.load(Ordering::Relaxed);
            if total != last_dropped {
                log::warn!(
                    "system audio ring overran: {} samples dropped",
                    total - last_dropped
                );
                last_dropped = total;
            }
        }
        if available == 0 {
            std::thread::sleep(IDLE_POLL);
            continue;
        }
        if first {
            log::info!("first system audio callback at {} Hz", rate.in_hz());
            first = false;
        }
        let Ok(chunk) = consumer.read_chunk(available) else {
            continue;
        };
        let (a, b) = chunk.as_slices();
        rate.push(a, |frame| send(frame, &mut closed));
        rate.push(b, |frame| send(frame, &mut closed));
        chunk.commit_all();
    }
}
