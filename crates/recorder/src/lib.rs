//! From capture frames to durable chunk files, one segment at a time
//! (KTD4, R7, R8, R35).

pub mod chunks;
pub mod storage;
pub mod wav;

use audio::{CaptureStream, Error as AudioError};
use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

pub use chunks::{
    ChunkInfo, ChunkedWriter, RecorderError, SegmentArgs, SegmentSpan, list_chunks,
    recover_partial_chunks, remove_segment,
};

/// Why a segment stopped recording.
#[derive(Debug)]
pub enum SegmentEnd {
    /// The user (or a new session) stopped it.
    Stopped,
    /// The source failed or stopped delivering: the device went away, or the
    /// stream ended on its own. Consumed by interruption handling (U14).
    SourceLost(AudioError),
    /// Writing failed; the chunks written so far are kept (U15).
    Storage(RecorderError),
}

/// A quarter-second of capture, for the level history and input status (U7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick {
    /// Peak absolute sample over the quarter-second, 0.0 to 1.0.
    pub peak: f32,
    /// Where the segment has reached on the session timeline.
    pub position_ms: u64,
}

/// Ticks per second of audio.
pub const TICKS_PER_SECOND: u32 = 4;

pub struct SegmentOutcome {
    pub end: SegmentEnd,
    /// The span the chunks on disk cover, or the error closing the last one.
    pub span: Result<SegmentSpan, RecorderError>,
}

/// Write `stream` into `writer` until `stop` fires or the source or the disk
/// fails. `on_tick` is called for every quarter-second of audio written, so its
/// cadence is the frames' own: when frames stop arriving, ticks stop too. File writes block, so run this on a thread of its own (the capture
/// manager does), not on a shared async worker.
pub async fn run_segment(
    mut stream: CaptureStream,
    mut writer: ChunkedWriter,
    stop: CancellationToken,
    mut on_tick: impl FnMut(Tick),
) -> SegmentOutcome {
    let tick_len = (writer.sample_rate() / TICKS_PER_SECOND).max(1) as usize;
    let mut tick_samples = 0usize;
    let mut tick_peak = 0f32;
    let rate = writer.sample_rate().max(1) as u64;
    let offset_ms = writer.offset_ms();
    let mut seen: u64 = 0;
    let end = loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break SegmentEnd::Stopped,
            next = stream.next() => match next {
                Some(Ok(frame)) => {
                    if let Err(e) = writer.write(&frame.raw_mic) {
                        break SegmentEnd::Storage(e);
                    }
                    for &s in frame.raw_mic.iter() {
                        tick_peak = tick_peak.max(s.abs());
                        tick_samples += 1;
                        seen += 1;
                        if tick_samples == tick_len {
                            on_tick(Tick {
                                peak: tick_peak.min(1.0),
                                position_ms: offset_ms + seen * 1000 / rate,
                            });
                            tick_samples = 0;
                            tick_peak = 0.0;
                        }
                    }
                }
                Some(Err(e)) => break SegmentEnd::SourceLost(e),
                None => break SegmentEnd::SourceLost(AudioError::MicStreamEnded),
            },
        }
    };
    SegmentOutcome {
        end,
        span: writer.finish(),
    }
}
