//! From capture frames to durable chunk files, one segment at a time
//! (KTD4, R7, R8, R35).
//!
//! A stereo segment records the microphone on the left and system audio on
//! the right (R11 of the system audio plan). The segment thread owns the
//! joined stream: it takes the microphone stream at the start, and attach
//! and detach commands for the speaker stream while it runs (KTD18). The
//! [`audio::joiner::Joiner`] pairs each microphone frame with speaker
//! samples, so the microphone is the clock (KTD5).

pub mod chunks;
pub mod storage;
pub mod wav;

use std::sync::Arc;

use audio::joiner::Joiner;
use audio::{CaptureStream, Error as AudioError};
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;

pub use chunks::{
    ChunkInfo, ChunkedWriter, PartialRecovery, RecorderError, SegmentArgs, SegmentSpan,
    chunk_channels, chunk_segment_of, list_chunks, recover_partial_chunks, remove_segment,
};

/// Why a segment stopped recording.
#[derive(Debug)]
pub enum SegmentEnd {
    /// The user (or a new session) stopped it.
    Stopped,
    /// The source failed or stopped delivering: the device went away, or the
    /// stream ended on its own. Consumed by interruption handling (U14).
    /// Only the microphone ends a segment: a speaker stream that ends leaves
    /// silence on the right.
    SourceLost(AudioError),
    /// Writing failed; the chunks written so far are kept (U15).
    Storage(RecorderError),
}

/// A quarter-second of capture, for the level history and input status (U7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick {
    /// Peak absolute microphone sample over the quarter-second, 0.0 to 1.0.
    pub peak: f32,
    /// Peak absolute system audio sample over the quarter-second, for a
    /// stereo segment. `None` for a mono segment.
    pub speaker_peak: Option<f32>,
    /// Where the segment has reached on the session timeline.
    pub position_ms: u64,
    /// Audio this segment has recorded so far.
    pub segment_ms: u64,
}

/// Ticks per second of audio.
pub const TICKS_PER_SECOND: u32 = 4;

/// Microphone audio between two log lines about dropped speaker samples.
const DROP_LOG_SECONDS: u64 = 60;

pub struct SegmentOutcome {
    pub end: SegmentEnd,
    /// The span the chunks on disk cover, or the error closing the last one.
    pub span: Result<SegmentSpan, RecorderError>,
}

/// A command to the segment thread of a stereo segment (KTD18).
pub enum SpeakerCommand {
    /// Pair the microphone with this speaker stream from now on. It replaces
    /// an attached stream. The stream must be at the microphone's rate.
    Attach(CaptureStream),
    /// Stop the speaker stream. The right channel gets silence.
    Detach,
}

/// One frame that the segment wrote, for the live worker and the silence
/// check.
pub struct Frame<'a> {
    /// The segment's microphone sample count before the frame.
    pub index: u64,
    pub mic: &'a Arc<[f32]>,
    /// The right channel of a stereo segment, as long as `mic`: system
    /// audio, or silence while no speaker stream gives audio. `None` for a
    /// mono segment.
    pub speaker: Option<&'a [f32]>,
}

/// What a segment records from.
pub struct SegmentInput {
    pub mic: CaptureStream,
    /// The commands for the speaker stream of a stereo segment. `None`, or
    /// a mono writer, records the microphone only.
    pub speaker: Option<UnboundedReceiver<SpeakerCommand>>,
}

impl SegmentInput {
    pub fn mono(mic: CaptureStream) -> Self {
        Self { mic, speaker: None }
    }
}

/// Write `stream` into `writer` until `stop` fires or the source or the disk
/// fails. `on_tick` is called for every quarter-second of audio written, so its
/// cadence is the frames' own: when frames stop arriving, ticks stop too. File writes block, so run this on a thread of its own (the capture
/// manager does), not on a shared async worker.
pub async fn run_segment(
    stream: CaptureStream,
    writer: ChunkedWriter,
    stop: CancellationToken,
    on_tick: impl FnMut(Tick),
) -> SegmentOutcome {
    run_segment_with(stream, writer, stop, on_tick, |_, _: &Arc<[f32]>| {}).await
}

/// [`run_segment`], and `on_frame` gets each frame after it is written, with
/// the segment's sample count before the frame (KTD3). The live worker
/// receives its copy of the audio this way. `on_frame` must never wait: a
/// full channel drops the frame, and the recording continues.
pub async fn run_segment_with(
    stream: CaptureStream,
    writer: ChunkedWriter,
    stop: CancellationToken,
    on_tick: impl FnMut(Tick),
    mut on_frame: impl FnMut(u64, &Arc<[f32]>),
) -> SegmentOutcome {
    run_joined_segment(
        SegmentInput::mono(stream),
        writer,
        stop,
        on_tick,
        |frame: Frame<'_>| on_frame(frame.index, frame.mic),
        |_| {},
    )
    .await
}

/// The next item of an attached stream. With no stream, it never completes.
async fn next_of(
    stream: &mut Option<CaptureStream>,
) -> Option<Result<audio::CaptureFrame, AudioError>> {
    match stream {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

/// The next command. With no command channel, it never completes.
async fn next_command(
    commands: &mut Option<UnboundedReceiver<SpeakerCommand>>,
) -> Option<SpeakerCommand> {
    match commands {
        Some(commands) => commands.recv().await,
        None => std::future::pending().await,
    }
}

/// Peaks over a quarter-second.
struct TickMeter {
    len: usize,
    samples: usize,
    mic: f32,
    speaker: f32,
}

/// The segment runner (KTD18). A writer with two channels records a stereo
/// segment: each microphone frame gets its right channel from the joiner,
/// and the interleaved frame goes to the writer. Otherwise the segment is
/// mono, as before.
///
/// - `on_frame` gets each frame after it is written. It must never wait.
/// - `on_speaker_end` is called when an attached speaker stream fails or
///   ends. The segment continues with silence on the right, and the caller
///   can attach a new stream.
///
/// The sample index and the ticks count microphone frames.
pub async fn run_joined_segment(
    input: SegmentInput,
    mut writer: ChunkedWriter,
    stop: CancellationToken,
    mut on_tick: impl FnMut(Tick),
    mut on_frame: impl FnMut(Frame<'_>),
    mut on_speaker_end: impl FnMut(AudioError),
) -> SegmentOutcome {
    let SegmentInput {
        mic: mut stream,
        speaker: mut commands,
    } = input;
    let stereo = writer.channels() == 2;
    if !stereo {
        commands = None;
    }
    let rate = writer.sample_rate().max(1);
    let mut joiner: Joiner<f32> = Joiner::new(rate);
    let mut speaker: Option<CaptureStream> = None;
    let mut right: Vec<f32> = Vec::new();
    let mut interleaved: Vec<f32> = Vec::new();
    let mut meter = TickMeter {
        len: (rate / TICKS_PER_SECOND).max(1) as usize,
        samples: 0,
        mic: 0.0,
        speaker: 0.0,
    };
    let rate = u64::from(rate);
    let offset_ms = writer.offset_ms();
    let mut seen: u64 = 0;
    let mut dropped_logged: u64 = 0;
    let mut last_drop_log: Option<u64> = None;
    let end = loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break SegmentEnd::Stopped,
            command = next_command(&mut commands) => match command {
                Some(SpeakerCommand::Attach(stream)) => {
                    joiner.clear();
                    speaker = Some(stream);
                }
                Some(SpeakerCommand::Detach) => {
                    joiner.clear();
                    speaker = None;
                }
                // The owner went away: keep the speaker state as it is.
                None => commands = None,
            },
            next = next_of(&mut speaker) => match next {
                Some(Ok(frame)) => {
                    joiner.push_speaker(&frame.raw_speaker);
                }
                Some(Err(e)) => {
                    speaker = None;
                    joiner.clear();
                    on_speaker_end(e);
                }
                None => {
                    speaker = None;
                    joiner.clear();
                    on_speaker_end(AudioError::SpeakerStreamEnded);
                }
            },
            next = stream.next() => match next {
                Some(Ok(frame)) => {
                    let mic = &frame.raw_mic;
                    let written = if stereo {
                        joiner.pair(mic.len(), &mut right);
                        interleaved.clear();
                        interleaved.reserve(mic.len() * 2);
                        for (&l, &r) in mic.iter().zip(right.iter()) {
                            interleaved.push(l);
                            interleaved.push(r);
                        }
                        writer.write(&interleaved)
                    } else {
                        writer.write(mic)
                    };
                    if let Err(e) = written {
                        break SegmentEnd::Storage(e);
                    }
                    on_frame(Frame {
                        index: seen,
                        mic,
                        speaker: stereo.then_some(right.as_slice()),
                    });
                    for (i, &s) in mic.iter().enumerate() {
                        meter.mic = meter.mic.max(s.abs());
                        if stereo {
                            meter.speaker = meter.speaker.max(right[i].abs());
                        }
                        meter.samples += 1;
                        seen += 1;
                        if meter.samples == meter.len {
                            on_tick(Tick {
                                peak: meter.mic.min(1.0),
                                speaker_peak: stereo.then_some(meter.speaker.min(1.0)),
                                position_ms: offset_ms + seen * 1000 / rate,
                                segment_ms: seen * 1000 / rate,
                            });
                            meter.samples = 0;
                            meter.mic = 0.0;
                            meter.speaker = 0.0;
                        }
                    }
                    if joiner.dropped() > dropped_logged
                        && last_drop_log.is_none_or(|at| seen - at >= DROP_LOG_SECONDS * rate)
                    {
                        log::info!(
                            "the joiner dropped {} speaker samples to keep the channels within {} ms",
                            joiner.dropped() - dropped_logged,
                            audio::joiner::MAX_SPEAKER_QUEUE_MS
                        );
                        dropped_logged = joiner.dropped();
                        last_drop_log = Some(seen);
                    }
                }
                Some(Err(e)) => break SegmentEnd::SourceLost(e),
                None => break SegmentEnd::SourceLost(AudioError::MicStreamEnded),
            },
        }
    };
    if stereo {
        log::info!(
            "stereo segment ended: {} speaker samples dropped, {} padded with silence",
            joiner.dropped(),
            joiner.padded()
        );
    }
    SegmentOutcome {
        end,
        span: writer.finish(),
    }
}
