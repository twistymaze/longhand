//! The macOS backend: a Core Audio process tap in a private aggregate
//! device (KTD3, KTD4, KTD13).
//!
//! From anarlog's `speaker/macos.rs` (see PROVENANCE.md), with these
//! changes:
//!
//! - [`Target::Apps`]: the tap records only the processes of the apps in
//!   the note's app set. At each device check, the thread finds those
//!   processes again and gives the tap a new description when they
//!   changed, so an app that starts or quits during the note needs no
//!   restart (R8, R10). While no process of the set exists, the thread
//!   keeps no tap and sends no frames.
//! - [`Target::Global`]: the tap records all processes except Longhand's
//!   own, so the alert sound and other Longhand sounds are not in it. The
//!   permission request uses it (KTD11).
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
use crate::{AppSet, CaptureFrame, Error, own_process_exclusion, speaker_frame};

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

pub(crate) fn output_running() -> Option<bool> {
    let device = match ca::System::default_output_device() {
        Ok(device) if !device.is_unknown() => device,
        Ok(_) => return None,
        Err(e) => {
            log::warn!("could not read the default output device: {e:?}");
            return None;
        }
    };
    match device.prop::<u32>(&ca::PropSelector::DEVICE_IS_RUNNING_SOMEWHERE.global_addr()) {
        Ok(running) => Some(running != 0),
        Err(e) => {
            log::warn!("could not read whether the output device runs: {e:?}");
            None
        }
    }
}

/// The processes that the tap records.
pub(crate) enum Target {
    /// All processes except Longhand's own.
    Global,
    /// The processes of the apps in the set.
    Apps(AppSet),
}

pub(crate) fn open(
    sample_rate: u32,
    chunk_size: usize,
    target: Target,
    tx: mpsc::Sender<Result<CaptureFrame, Error>>,
) -> Result<(), Error> {
    let (init_tx, init_rx) = std_mpsc::sync_channel::<Result<(), Error>>(1);
    std::thread::Builder::new()
        .name("longhand-system-audio".into())
        .spawn(move || run(sample_rate, chunk_size, target, tx, init_tx))
        .map_err(|e| Error::SpeakerStreamInitializationFailed(e.to_string()))?;
    init_rx.recv().map_err(|_| {
        Error::SpeakerStreamInitializationFailed("system audio thread exited".into())
    })?
}

fn failed(what: &str, e: impl std::fmt::Debug) -> Error {
    Error::SpeakerStreamInitializationFailed(format!("{what}: {e:?}"))
}

/// Longhand's own process object, or `None` when the lookup fails.
fn own_process() -> Option<u32> {
    let own = own_process_exclusion(std::process::id(), |pid| {
        let pid = i32::try_from(pid).map_err(|e| e.to_string())?;
        match ca::Process::with_pid(pid) {
            Ok(process) if process.0 == ca::Obj::UNKNOWN => Ok(None),
            Ok(process) => Ok(Some(process.0.0)),
            Err(e) => Err(format!("{e:?}")),
        }
    });
    own.first().copied()
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

fn numbers(objects: &[u32]) -> arc::R<ns::Array<ns::Number>> {
    let numbers: Vec<arc::R<ns::Number>> =
        objects.iter().copied().map(ns::Number::with_u32).collect();
    ns::Array::from_slice_retained(&numbers)
}

/// A tap of all processes except Longhand's own process.
fn global_desc(own: Option<u32>) -> arc::R<ca::TapDesc> {
    let own: Vec<u32> = own.into_iter().collect();
    ca::TapDesc::with_mono_global_tap_excluding_processes(&numbers(&own))
}

/// A tap of only these processes.
fn apps_desc(objects: &[u32]) -> arc::R<ca::TapDesc> {
    ca::TapDesc::with_mono_mixdown_of_processes(&numbers(objects))
}

/// The process objects of the apps in the set now, sorted.
fn app_objects(apps: &AppSet) -> Vec<u32> {
    crate::apps::process_objects(
        &apps.get(),
        &crate::apps::audio_processes(),
        std::process::id(),
    )
}

/// A started tap: the process tap, its private aggregate device, and the
/// ring that the IO proc fills.
struct Tap {
    // Fields drop in this order: `started` stops the device before `_ctx`,
    // which the IO proc reads, is freed.
    started: ca::hardware::StartedDevice<ca::AggregateDevice>,
    _ctx: Box<Ctx>,
    tap: ca::TapGuard,
    consumer: rtrb::Consumer<f32>,
    /// The rate that the tap reported when it was created.
    rate: u32,
}

impl Tap {
    fn start(desc: arc::R<ca::TapDesc>, dropped: &Arc<AtomicU64>) -> Result<Self, Error> {
        let tap = desc
            .create_process_tap()
            .map_err(|e| failed("process tap", e))?;
        let asbd = tap.asbd().map_err(|e| failed("tap format", e))?;
        let format = av::AudioFormat::with_asbd(&asbd)
            .ok_or_else(|| failed("tap format", "unsupported system audio format"))?;
        let rate = snap_rate(asbd.sample_rate).unwrap_or(FALLBACK_RATE);
        let (producer, consumer) = rtrb::RingBuffer::new(rate as usize * RING_SECONDS);
        let mut ctx = Box::new(Ctx {
            common_format: format.common_format(),
            producer,
            dropped: Arc::clone(dropped),
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
            "system audio tap started at {rate} Hz, {} channel(s)",
            asbd.channels_per_frame
        );
        Ok(Self {
            started,
            _ctx: ctx,
            tap,
            consumer,
            rate,
        })
    }
}

/// What the thread does with the tap after it finds the processes of the
/// app set again.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Keep,
    /// No tap runs, and a process of the set exists.
    Start,
    /// The tap runs, and the processes of the set changed.
    Update,
    /// The tap runs, and no process of the set exists.
    Stop,
}

fn step(running: bool, current: &[u32], now: &[u32]) -> Step {
    match (running, now.is_empty()) {
        (false, true) => Step::Keep,
        (false, false) => Step::Start,
        (true, true) => Step::Stop,
        (true, false) if current == now => Step::Keep,
        (true, false) => Step::Update,
    }
}

fn run(
    sample_rate: u32,
    chunk_size: usize,
    target: Target,
    tx: mpsc::Sender<Result<CaptureFrame, Error>>,
    init_tx: std_mpsc::SyncSender<Result<(), Error>>,
) {
    let dropped = Arc::new(AtomicU64::new(0));
    // The global tap starts at once, so an error goes to the caller. The
    // app tap starts at the first check that finds a process of the set.
    let tap = match &target {
        Target::Global => {
            let own = own_process();
            if own.is_some() {
                log::info!("the system audio tap excludes Longhand's own process");
            }
            match Tap::start(global_desc(own), &dropped) {
                Ok(tap) => Some(tap),
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                    return;
                }
            }
        }
        Target::Apps(_) => None,
    };
    let _ = init_tx.send(Ok(()));
    let in_rate = tap.as_ref().map_or(FALLBACK_RATE, |t| t.rate);
    let out_rate = if sample_rate == 0 {
        in_rate
    } else {
        sample_rate
    };
    drain(tap, &target, &dropped, in_rate, out_rate, chunk_size, &tx);
    log::info!("system audio capture stopped");
}

fn drain(
    mut tap: Option<Tap>,
    target: &Target,
    dropped: &Arc<AtomicU64>,
    in_rate: u32,
    out_rate: u32,
    chunk_size: usize,
    tx: &mpsc::Sender<Result<CaptureFrame, Error>>,
) {
    let mut rate = RateFollower::new(in_rate, out_rate, chunk_size);
    let mut objects: Vec<u32> = Vec::new();
    let mut failed_objects: Vec<u32> = Vec::new();
    let mut next_check = Instant::now();
    let mut first = true;
    let mut last_dropped = 0;
    let mut closed = false;
    let send = |frame: &[f32], closed: &mut bool| {
        if !*closed {
            *closed = tx.blocking_send(Ok(speaker_frame(frame))).is_err();
        }
    };
    // Send the audio that is in the ring and in the resampler.
    let flush = |tap: &mut Tap, rate: &mut RateFollower, closed: &mut bool| {
        let left = tap.consumer.slots();
        if left > 0
            && let Ok(chunk) = tap.consumer.read_chunk(left)
        {
            let (a, b) = chunk.as_slices();
            rate.push(a, |frame| send(frame, closed));
            rate.push(b, |frame| send(frame, closed));
            chunk.commit_all();
        }
        rate.finish(|frame| send(frame, closed));
    };

    loop {
        if closed || tx.is_closed() {
            return;
        }
        let available = tap.as_ref().map_or(0, |t| t.consumer.slots());
        let now = Instant::now();
        // The first callback, and every check after it: the device can run
        // at a rate other than the one the tap reported.
        if (first && available > 0) || now >= next_check {
            next_check = now + DEVICE_CHECK;
            if let Some(t) = tap.as_mut() {
                if !t.started.is_alive().unwrap_or(false) {
                    log::warn!("the system audio device is no longer alive; ending system audio");
                    flush(t, &mut rate, &mut closed);
                    if !closed {
                        let _ = tx.blocking_send(Err(Error::SpeakerStreamEnded));
                    }
                    return;
                }
                if let Some(hz) = device_rate(&t.started, &t.tap) {
                    rate.set_in_hz(hz, |frame| send(frame, &mut closed));
                }
            }
            if let Target::Apps(apps) = target {
                let found = app_objects(apps);
                match step(tap.is_some(), &objects, &found) {
                    Step::Keep => {}
                    Step::Start => match Tap::start(apps_desc(&found), dropped) {
                        Ok(t) => {
                            log::info!("the app audio tap records {} process(es)", found.len());
                            rate.reset_at(t.rate);
                            tap = Some(t);
                            first = true;
                            objects = found;
                            failed_objects.clear();
                        }
                        // A process can quit between the list and the tap.
                        // The next check tries again; log each list once.
                        Err(e) => {
                            if failed_objects != found {
                                log::warn!("the app audio tap did not start: {e}");
                                failed_objects = found;
                            }
                        }
                    },
                    Step::Update => {
                        if let Some(t) = tap.as_mut() {
                            match t.tap.set_desc(apps_desc(&found)) {
                                Ok(()) => {
                                    log::info!(
                                        "the app audio tap now records {} process(es)",
                                        found.len()
                                    );
                                    objects = found;
                                }
                                Err(e) => log::warn!(
                                    "the app audio tap did not take the new process list: {e:?}"
                                ),
                            }
                        }
                    }
                    Step::Stop => {
                        if let Some(mut t) = tap.take() {
                            flush(&mut t, &mut rate, &mut closed);
                        }
                        log::info!("no app in the app set plays; the app audio tap stopped");
                        objects.clear();
                    }
                }
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
        let Some(t) = tap.as_mut() else {
            continue;
        };
        if first {
            log::info!("first system audio callback at {} Hz", rate.in_hz());
            first = false;
        }
        let Ok(chunk) = t.consumer.read_chunk(available) else {
            continue;
        };
        let (a, b) = chunk.as_slices();
        rate.push(a, |frame| send(frame, &mut closed));
        rate.push(b, |frame| send(frame, &mut closed));
        chunk.commit_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_process_of_the_set_keeps_no_tap() {
        assert_eq!(step(false, &[], &[]), Step::Keep);
    }

    #[test]
    fn a_process_that_starts_starts_the_tap() {
        assert_eq!(step(false, &[], &[7]), Step::Start);
    }

    #[test]
    fn a_changed_process_list_updates_the_tap() {
        assert_eq!(step(true, &[7], &[7, 9]), Step::Update);
        assert_eq!(step(true, &[7, 9], &[9]), Step::Update);
        assert_eq!(step(true, &[7, 9], &[7, 9]), Step::Keep);
    }

    #[test]
    fn an_empty_set_stops_the_tap_with_no_error() {
        assert_eq!(step(true, &[7], &[]), Step::Stop);
    }
}
