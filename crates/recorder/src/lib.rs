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

pub struct SegmentOutcome {
    pub end: SegmentEnd,
    /// The span the chunks on disk cover, or the error closing the last one.
    pub span: Result<SegmentSpan, RecorderError>,
}

/// Write `stream` into `writer` until `stop` fires or the source or the disk
/// fails. File writes block, so run this on a thread of its own (the capture
/// manager does), not on a shared async worker.
pub async fn run_segment(
    mut stream: CaptureStream,
    mut writer: ChunkedWriter,
    stop: CancellationToken,
) -> SegmentOutcome {
    let end = loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break SegmentEnd::Stopped,
            next = stream.next() => match next {
                Some(Ok(frame)) => {
                    if let Err(e) = writer.write(&frame.raw_mic) {
                        break SegmentEnd::Storage(e);
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
