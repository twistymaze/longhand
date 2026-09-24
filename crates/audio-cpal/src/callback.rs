//! The real-time half of capture: the device callback and the ring it feeds.
//!
//! Handy's `build_stream` and `write_input_to_ring` (see PROVENANCE.md) with
//! the pause handshake removed — a segment ends by closing the stream — and
//! error kinds recorded instead of a bare flag, so the worker can tell a lost
//! device from a transient overrun.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use cpal::traits::DeviceTrait;
use cpal::{Device, ErrorKind, FromSample, Sample, SizedSample, Stream, SupportedStreamConfig};
use rtrb::{Consumer, Producer, RingBuffer};

/// Seconds of audio the ring holds between the callback and the worker:
/// enough to ride out a disk stall while another session is saved.
pub const RING_SECONDS: usize = 10;

/// Drops the callback can report before the worker reads them. Consecutive
/// drops at one position are merged, so this is only reached by a worker
/// stalled across many separate overruns; beyond it, [`Shared::unplaced`]
/// keeps the count.
const GAP_QUEUE: usize = 256;

/// Frames the callback dropped because the ring was full, and where: `at`
/// is how many frames the callback had written into the ring before them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overrun {
    pub at: u64,
    pub frames: u64,
}

/// The callback's own bookkeeping: frames written so far and a drop not yet
/// reported, held until the next write fixes its extent.
#[derive(Default)]
pub struct Written {
    total: u64,
    pending: Option<Overrun>,
}

pub const NO_FAULT: u8 = 0;
/// The device went away, the stream was invalidated, or the route moved to
/// another device: capture from this stream is over.
pub const FAULT_DEVICE_LOST: u8 = 1;
/// Any other stream error. Also fatal: cpal reports no error it recovers from
/// itself except an xrun, which is counted rather than stored here.
pub const FAULT_OTHER: u8 = 2;

#[derive(Default)]
pub struct Shared {
    pub fault: AtomicU8,
    pub overrun_samples: AtomicU64,
    /// Dropped frames the gap queue had no room for; the worker fills them
    /// with silence where it notices them.
    pub unplaced: AtomicU64,
    pub xruns: AtomicU64,
}

pub fn build_stream<T>(
    device: &Device,
    config: &SupportedStreamConfig,
    shared: Arc<Shared>,
) -> Result<(Stream, Consumer<f32>, Consumer<Overrun>), cpal::Error>
where
    T: Sample + SizedSample + Copy + Send + 'static,
    f32: FromSample<T>,
{
    let channels = config.channels() as usize;
    let ring_capacity = config.sample_rate() as usize * RING_SECONDS;
    let (mut producer, mut consumer) = RingBuffer::new(ring_capacity);
    let (mut gaps, gaps_out) = RingBuffer::new(GAP_QUEUE);
    let mut written = Written::default();

    // Touch rtrb's uninitialized pages before the stream starts to reduce
    // callback page faults. This does not pin them.
    {
        let chunk = producer
            .write_chunk(ring_capacity)
            .expect("new audio ring has its full capacity available");
        chunk.commit_all();
    }
    {
        let chunk = consumer
            .read_chunk(ring_capacity)
            .expect("pre-filled audio ring is readable");
        chunk.commit_all();
    }

    let data_shared = Arc::clone(&shared);
    let stream = device.build_input_stream(
        config.config(),
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            write_input_to_ring(
                data,
                channels,
                &mut producer,
                &mut gaps,
                &mut written,
                &data_shared,
            );
        },
        move |error: cpal::Error| {
            // Error callbacks may share the platform audio thread. Record the
            // kind; the worker logs and reacts.
            match error.kind() {
                ErrorKind::Xrun => {
                    shared.xruns.fetch_add(1, Ordering::Relaxed);
                }
                ErrorKind::DeviceNotAvailable
                | ErrorKind::StreamInvalidated
                | ErrorKind::DeviceChanged => {
                    shared.fault.store(FAULT_DEVICE_LOST, Ordering::Release);
                }
                _ => {
                    let _ = shared.fault.compare_exchange(
                        NO_FAULT,
                        FAULT_OTHER,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                }
            }
        },
        None,
    )?;
    Ok((stream, consumer, gaps_out))
}

/// Real-time callback body. Keep this allocation-free, wait-free, and free
/// of locks, logging, clocks, and system calls. Multichannel input is
/// averaged to mono (R2: one microphone). Frames that do not fit are
/// reported with their position, so the worker can put the same length of
/// silence in their place and the timeline stays true to the clock (U25).
pub fn write_input_to_ring<T>(
    data: &[T],
    channels: usize,
    producer: &mut Producer<f32>,
    gaps: &mut Producer<Overrun>,
    written_so_far: &mut Written,
    shared: &Shared,
) where
    T: Sample + SizedSample + Copy,
    f32: FromSample<T>,
{
    let frame_count = data.len() / channels;
    let writable_frames = producer.slots().min(frame_count);
    // An overrun is reported before the frames that follow it are committed, so
    // the worker always learns of it before reading past its position.
    if writable_frames > 0
        && let Some(overrun) = written_so_far.pending.take()
        && gaps.push(overrun).is_err()
    {
        shared.unplaced.fetch_add(overrun.frames, Ordering::Relaxed);
    }
    let written =
        if writable_frames == 0 {
            0
        } else {
            let chunk = producer
                .write_chunk_uninit(writable_frames)
                .expect("the producer just reported this many writable slots");
            if channels == 1 {
                chunk.fill_from_iter(
                    data.iter()
                        .take(writable_frames)
                        .map(|&sample| sample.to_sample::<f32>()),
                )
            } else {
                chunk.fill_from_iter(data.chunks_exact(channels).take(writable_frames).map(
                    |frame| {
                        frame
                            .iter()
                            .map(|&sample| sample.to_sample::<f32>())
                            .sum::<f32>()
                            / channels as f32
                    },
                ))
            }
        };
    debug_assert_eq!(written, writable_frames);
    written_so_far.total += written as u64;

    let dropped = (frame_count - written) as u64;
    if dropped > 0 {
        shared.overrun_samples.fetch_add(dropped, Ordering::Relaxed);
        let at = written_so_far.total;
        written_so_far.pending = Some(match written_so_far.pending {
            Some(earlier) if earlier.at == at => Overrun {
                at,
                frames: earlier.frames + dropped,
            },
            _ => Overrun {
                at,
                frames: dropped,
            },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write<T>(data: &[T], channels: usize, producer: &mut Producer<f32>, shared: &Shared)
    where
        T: Sample + SizedSample + Copy,
        f32: FromSample<T>,
    {
        let (mut gaps, _) = RingBuffer::new(4);
        write_input_to_ring(
            data,
            channels,
            producer,
            &mut gaps,
            &mut Written::default(),
            shared,
        );
    }

    #[test]
    fn stereo_is_averaged_to_mono() {
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(8);
        let shared = Shared::default();
        write(&[0.2_f32, 0.4, -1.0, 1.0], 2, &mut producer, &shared);
        let chunk = consumer.read_chunk(2).unwrap();
        let (a, _) = chunk.as_slices();
        assert!((a[0] - 0.3).abs() < 1e-6);
        assert!(a[1].abs() < 1e-6);
    }

    #[test]
    fn a_full_ring_counts_what_it_drops() {
        let (mut producer, _consumer) = RingBuffer::<f32>::new(3);
        let shared = Shared::default();
        write(&[0.1_f32; 5], 1, &mut producer, &shared);
        assert_eq!(shared.overrun_samples.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn a_drop_is_reported_with_its_position_once_writing_resumes() {
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(4);
        let (mut gaps, mut gaps_out) = RingBuffer::new(4);
        let mut written = Written::default();
        let shared = Shared::default();
        let mut call = |data: &[f32], producer: &mut Producer<f32>| {
            write_input_to_ring(data, 1, producer, &mut gaps, &mut written, &shared)
        };
        call(&[0.1; 3], &mut producer);
        // Two callbacks overrun at the same position; they merge.
        call(&[0.2; 3], &mut producer);
        call(&[0.3; 2], &mut producer);
        assert!(
            gaps_out.pop().is_err(),
            "not reported until its extent is known"
        );
        consumer.read_chunk(4).unwrap().commit_all();
        call(&[0.4; 2], &mut producer);
        assert_eq!(gaps_out.pop(), Ok(Overrun { at: 4, frames: 4 }));
        assert!(gaps_out.pop().is_err());
    }

    #[test]
    fn integer_samples_convert_to_float() {
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(2);
        let shared = Shared::default();
        write(&[i16::MAX, i16::MIN], 1, &mut producer, &shared);
        let chunk = consumer.read_chunk(2).unwrap();
        let (a, _) = chunk.as_slices();
        assert!(a[0] > 0.99 && a[1] <= -0.99);
    }
}
