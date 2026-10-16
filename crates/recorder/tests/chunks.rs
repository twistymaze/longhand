use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use std::io::Write;

use audio::{AudioProvider, CaptureFrame, CaptureStream};
use audio_mock::MockAudio;
use chrono::{FixedOffset, TimeZone};
use recorder::storage::DISK_RESERVE_BYTES;
use recorder::wav::to_i16;
use recorder::{
    ChunkedWriter, Frame, RecorderError, SegmentArgs, SegmentEnd, SegmentInput, SegmentOutcome,
    SpeakerCommand, list_chunks, recover_partial_chunks, remove_segment, run_segment,
};
use session_core::timeline::{Closed, Timeline};
use tokio_util::sync::CancellationToken;

const GB: u64 = 1 << 30;

fn args(dir: &Path, segment: u32, offset_ms: u64, sample_rate: u32) -> SegmentArgs {
    SegmentArgs {
        dir: dir.to_path_buf(),
        segment,
        capture_started_at: 1_700_000_000_000,
        offset_ms,
        sample_rate,
        channels: 1,
        floor_bytes: DISK_RESERVE_BYTES,
    }
}

fn plenty() -> recorder::storage::SpaceProbe {
    Box::new(|_| Some(100 * GB))
}

fn writer(dir: &Path, segment: u32, offset_ms: u64, sample_rate: u32) -> ChunkedWriter {
    ChunkedWriter::with_probe(args(dir, segment, offset_ms, sample_rate), plenty()).unwrap()
}

fn decode(path: &Path) -> Vec<i16> {
    hound::WavReader::open(path)
        .unwrap()
        .into_samples()
        .map(Result::unwrap)
        .collect()
}

/// A signal where every sample differs from its neighbours, so a duplicated
/// or dropped sample at a boundary shows up.
fn ramp(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| ((i % 20_000) as f32 / 20_000.0) - 0.5)
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_130_s_recording_closes_into_three_overlapping_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 48_000;
    let mock = MockAudio::tone(440.0, rate, Duration::from_secs(130));
    let stream = mock.open_mic_capture(None, rate, 960).unwrap();

    let outcome = run_segment(
        stream,
        writer(dir.path(), 0, 0, rate),
        CancellationToken::new(),
        |_| {},
    )
    .await;
    // The mock's stream ends after its samples, which reads as a lost source.
    assert!(matches!(
        outcome.end,
        SegmentEnd::SourceLost(audio::Error::MicStreamEnded)
    ));
    let span = outcome.span.unwrap();
    assert_eq!((span.start_ms, span.end_ms), (0, 130_000));

    let chunks = list_chunks(dir.path()).unwrap();
    let stems: Vec<_> = chunks
        .iter()
        .map(|c| (c.start_ms, c.end_ms, c.audio_start_ms))
        .collect();
    assert_eq!(
        stems,
        [
            (0, 60_000, 0),
            (60_000, 120_000, 58_000),
            (120_000, 130_000, 118_000)
        ]
    );
    let lengths: Vec<_> = chunks.iter().map(|c| decode(&c.path).len()).collect();
    assert_eq!(lengths, [60 * 48_000, 62 * 48_000, 12 * 48_000]);
    // Nothing is left open.
    assert!(
        std::fs::read_dir(dir.path())
            .unwrap()
            .all(|e| { e.unwrap().file_name().to_string_lossy().ends_with(".wav") })
    );
}

#[test]
fn consecutive_chunks_rejoin_without_a_missing_or_doubled_sample() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 16_000;
    let source = ramp(70 * rate as usize);
    let mut w = writer(dir.path(), 0, 0, rate);
    for frame in source.chunks(333) {
        w.write(frame).unwrap();
    }
    w.finish().unwrap();

    let chunks = list_chunks(dir.path()).unwrap();
    assert_eq!(chunks.len(), 2);
    let mut joined = Vec::new();
    for c in &chunks {
        let samples = decode(&c.path);
        let lead = ((c.start_ms - c.audio_start_ms) * rate as u64 / 1000) as usize;
        joined.extend_from_slice(&samples[lead..]);
    }
    let expected: Vec<i16> = source.iter().map(|&s| to_i16(s)).collect();
    assert_eq!(joined.len(), expected.len());
    assert!(joined == expected);
}

#[test]
fn a_segment_under_a_second_is_discarded_with_its_audio() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 16_000;
    let at = FixedOffset::east_opt(0)
        .unwrap()
        .timestamp_opt(1_700_000_000, 0)
        .unwrap();
    let mut timeline = Timeline::new();
    let index = timeline.open(0, at).unwrap();

    let mut w = writer(dir.path(), index, 0, rate);
    w.write(&vec![0.1; 900 * rate as usize / 1000]).unwrap();
    let span = w.finish().unwrap();
    assert_eq!(span.duration_ms(), 900);

    let closed = timeline.close(span.end_ms, at, String::new()).unwrap();
    assert_eq!(closed, Closed::Discarded);
    remove_segment(dir.path(), span.segment).unwrap();
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn a_resumed_segment_continues_the_timeline_after_its_gap() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 8_000;
    let at = FixedOffset::east_opt(0)
        .unwrap()
        .timestamp_opt(1_700_000_000, 0)
        .unwrap();
    let mut timeline = Timeline::new();

    let first = timeline.open(0, at).unwrap();
    let mut w = writer(dir.path(), first, 0, rate);
    w.write(&vec![0.1; 300 * rate as usize]).unwrap();
    let span = w.finish().unwrap();
    timeline.close(span.end_ms, at, "a".into()).unwrap();

    // Resumed two minutes later.
    let second = timeline.open(420_000, at).unwrap();
    let mut w = writer(dir.path(), second, 420_000, rate);
    w.write(&vec![0.1; 60 * rate as usize]).unwrap();
    let span = w.finish().unwrap();
    timeline.close(span.end_ms, at, "b".into()).unwrap();

    let resumed: Vec<_> = list_chunks(dir.path())
        .unwrap()
        .into_iter()
        .filter(|c| c.segment == second)
        .map(|c| (c.start_ms, c.end_ms, c.audio_start_ms))
        .collect();
    // A new segment starts with no lookback: its first sample is its start.
    assert_eq!(resumed, [(420_000, 480_000, 420_000)]);
    assert_eq!(timeline.gaps(), [(first, 120_000)]);
}

#[test]
fn free_space_falling_below_the_floor_stops_at_the_next_check() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 16_000;
    let free = Arc::new(AtomicU64::new(300 * 1024 * 1024));
    let probe_free = Arc::clone(&free);
    let mut w = ChunkedWriter::with_probe(
        args(dir.path(), 0, 0, rate),
        Box::new(move |_| Some(probe_free.load(Ordering::Relaxed))),
    )
    .unwrap();

    let second = vec![0.1; rate as usize];
    let mut failed_at = None;
    for s in 0..20 {
        if s == 2 {
            free.store(200 * 1024 * 1024, Ordering::Relaxed);
        }
        if let Err(e) = w.write(&second) {
            assert!(
                matches!(e, RecorderError::LowDisk { available, .. } if available == 200 * 1024 * 1024)
            );
            failed_at = Some(w.end_ms());
            break;
        }
    }
    // Checked every 5 s of audio: the drop at 2 s is caught at 5 s.
    assert_eq!(failed_at, Some(5_000));
    // What was written before the floor is kept.
    w.finish().unwrap();
    assert_eq!(list_chunks(dir.path()).unwrap()[0].end_ms, 5_000);
}

#[test]
fn a_disk_already_below_the_floor_refuses_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let result = ChunkedWriter::with_probe(
        args(dir.path(), 0, 0, 16_000),
        Box::new(|_| Some(10 * 1024 * 1024)),
    );
    assert!(matches!(result, Err(RecorderError::LowDisk { .. })));
}

#[tokio::test(start_paused = true)]
async fn stopping_closes_the_open_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 16_000;
    let mock = MockAudio::tone(440.0, rate, Duration::from_secs(600));
    let stream = mock.open_mic_capture(None, rate, 320).unwrap();
    let stop = CancellationToken::new();
    let task = tokio::spawn(run_segment(
        stream,
        writer(dir.path(), 0, 0, rate),
        stop.clone(),
        |_| {},
    ));
    tokio::time::sleep(Duration::from_secs(3)).await;
    stop.cancel();
    let outcome = task.await.unwrap();
    assert!(matches!(outcome.end, SegmentEnd::Stopped));
    let span = outcome.span.unwrap();
    assert!((2_900..=3_100).contains(&span.end_ms), "{span:?}");
    assert_eq!(list_chunks(dir.path()).unwrap().len(), 1);
}

#[test]
fn a_crash_leaves_a_partial_that_launch_recovery_closes() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 16_000;
    let mut w = writer(dir.path(), 0, 0, rate);
    for _ in 0..70 {
        w.write(&vec![0.1; rate as usize]).unwrap();
    }
    // The process dies: no finish, no header, whatever reached the OS stays.
    drop(w);
    assert_eq!(list_chunks(dir.path()).unwrap().len(), 1);

    let recovered = recover_partial_chunks(dir.path()).unwrap().recovered;
    assert_eq!(recovered.len(), 1);
    let chunks = list_chunks(dir.path()).unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(
        (
            chunks[1].start_ms,
            chunks[1].end_ms,
            chunks[1].audio_start_ms
        ),
        (60_000, 70_000, 58_000)
    );
    assert_eq!(decode(&chunks[1].path).len(), 12 * rate as usize);
}

#[test]
fn a_partial_holding_only_lookback_is_removed_at_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 8_000;
    let mut w = writer(dir.path(), 0, 0, rate);
    w.write(&vec![0.1; 60 * rate as usize]).unwrap();
    w.write(&[0.1; 1]).unwrap(); // opens the next chunk with 2 s of lookback
    drop(w);
    // Simulate a crash before the new sample reaches the disk.
    let part = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "part"))
        .unwrap();
    let len = std::fs::metadata(&part).unwrap().len();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&part)
        .unwrap()
        .set_len(len - 2)
        .unwrap();

    let report = recover_partial_chunks(dir.path()).unwrap();
    assert!(report.recovered.is_empty() && report.left.is_empty());
    assert_eq!(list_chunks(dir.path()).unwrap().len(), 1);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

fn long_run(minutes: u64) {
    let dir = tempfile::tempdir().unwrap();
    let rate = 48_000;
    let frame = vec![0.1_f32; 960];
    let mut w = writer(dir.path(), 0, 0, rate);
    let frames = minutes * 60 * rate as u64 / 960;
    for _ in 0..frames {
        w.write(&frame).unwrap();
    }
    let span = w.finish().unwrap();
    assert_eq!(span.end_ms, minutes * 60_000);
    let chunks = list_chunks(dir.path()).unwrap();
    assert_eq!(chunks.len() as u64, minutes);
    assert!(chunks.windows(2).all(|p| p[0].end_ms == p[1].start_ms));
}

/// Twenty minutes at 48 kHz as fast as the writer accepts: the writer's only
/// buffers are the 2 s lookback and one frame, so memory is flat by
/// construction; this checks the chunk sequence stays contiguous.
#[test]
fn twenty_minutes_of_native_rate_audio_stays_contiguous() {
    long_run(20);
}

/// The four-hour soak (about 1.4 GB of WAV). Run with `--ignored`.
#[test]
#[ignore = "writes 1.4 GB"]
fn four_hours_of_native_rate_audio() {
    long_run(240);
}

#[tokio::test(start_paused = true)]
async fn every_quarter_second_of_audio_reports_its_peak_and_position() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 16_000;
    let mock = MockAudio::tone(440.0, rate, Duration::from_secs(2));
    let stream = mock.open_mic_capture(None, rate, 320).unwrap();
    let mut ticks = Vec::new();
    run_segment(
        stream,
        writer(dir.path(), 0, 60_000, rate),
        CancellationToken::new(),
        |t| ticks.push(t),
    )
    .await;
    let positions: Vec<_> = ticks.iter().map(|t| t.position_ms).collect();
    assert_eq!(
        positions,
        [
            60_250, 60_500, 60_750, 61_000, 61_250, 61_500, 61_750, 62_000
        ]
    );
    assert!(ticks.iter().all(|t| (t.peak - 0.5).abs() < 0.01));
}

/// A minute of audio at `rate`, in the 20 ms blocks capture delivers.
fn write_minute(w: &mut ChunkedWriter, rate: u32) -> Result<(), RecorderError> {
    let block = vec![0.1; rate as usize / 50];
    for _ in 0..(50 * 60) {
        w.write(&block)?;
    }
    Ok(())
}

#[test]
fn a_slow_chunk_sync_neither_blocks_the_writer_nor_publishes_early() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 16_000;
    // The sync is held until the test lets it go, up to a bound that fails
    // the test loudly if the writer waited on it.
    static RELEASED: AtomicBool = AtomicBool::new(false);
    const HELD: Duration = Duration::from_secs(10);
    let mut w = writer(dir.path(), 0, 0, rate).with_sync(|_| {
        let started = std::time::Instant::now();
        while !RELEASED.load(Ordering::SeqCst) && started.elapsed() < HELD {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    });
    let started = std::time::Instant::now();
    write_minute(&mut w, rate).unwrap();
    assert!(started.elapsed() < HELD / 2, "{:?}", started.elapsed());
    assert!(
        list_chunks(dir.path()).unwrap().is_empty(),
        "published before it was synced"
    );
    RELEASED.store(true, Ordering::SeqCst);
    w.write(&[0.1; 320]).unwrap();
    w.finish().unwrap();
    let chunks = list_chunks(dir.path()).unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!((chunks[0].start_ms, chunks[0].end_ms), (0, 60_000));
    assert!(
        std::fs::read_dir(dir.path())
            .unwrap()
            .all(|e| e.unwrap().path().extension().unwrap() == "wav")
    );
}

#[test]
fn a_failed_chunk_sync_stops_the_writer_and_leaves_the_chunk_recoverable() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 16_000;
    let mut w =
        writer(dir.path(), 0, 0, rate).with_sync(|_| Err(std::io::Error::other("disk gone")));
    write_minute(&mut w, rate).unwrap();
    // The failure surfaces at the next rotation.
    assert!(write_minute(&mut w, rate).is_err());
    drop(w);
    assert!(list_chunks(dir.path()).unwrap().is_empty());
    recover_partial_chunks(dir.path()).unwrap();
    let chunks = list_chunks(dir.path()).unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].start_ms, 0);
    assert!(chunks[0].end_ms >= 60_000);
    assert_eq!(chunks[1].start_ms, 60_000);
}

#[tokio::test(start_paused = true)]
async fn each_frame_goes_to_the_frame_callback_with_its_sample_index() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 48_000;
    let mock = MockAudio::tone(440.0, rate, Duration::from_secs(3));
    let stream = mock.open_mic_capture(None, rate, 960).unwrap();
    let mut frames = Vec::new();
    let outcome = recorder::run_segment_with(
        stream,
        writer(dir.path(), 0, 0, rate),
        CancellationToken::new(),
        |_| {},
        |index, pcm: &Arc<[f32]>| frames.push((index, pcm.len())),
    )
    .await;
    assert_eq!(outcome.span.unwrap().end_ms, 3_000);
    assert_eq!(frames.len(), 150);
    // The index is the segment's sample count before the frame.
    for (n, &(index, len)) in frames.iter().enumerate() {
        assert_eq!((index, len), (n as u64 * 960, 960));
    }
}

#[tokio::test(start_paused = true)]
async fn a_slow_frame_consumer_never_blocks_the_recorder() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 48_000;
    let seconds = 130;
    let mock = MockAudio::tone(440.0, rate, Duration::from_secs(seconds));
    let stream = mock.open_mic_capture(None, rate, 960).unwrap();
    // A live worker that takes 5 ms for each frame, behind a bounded
    // channel that the callback never waits on.
    let (tx, rx) = std::sync::mpsc::sync_channel::<u64>(250);
    let received = Arc::new(AtomicU64::new(0));
    let counted = Arc::clone(&received);
    let worker = std::thread::spawn(move || {
        while rx.recv().is_ok() {
            std::thread::sleep(Duration::from_millis(5));
            counted.fetch_add(1, Ordering::SeqCst);
        }
    });
    let dropped = Arc::new(AtomicU64::new(0));
    let drops = Arc::clone(&dropped);
    let started = std::time::Instant::now();
    let outcome = recorder::run_segment_with(
        stream,
        writer(dir.path(), 0, 0, rate),
        CancellationToken::new(),
        |_| {},
        move |index, _: &Arc<[f32]>| {
            if tx.try_send(index).is_err() {
                drops.fetch_add(1, Ordering::SeqCst);
            }
        },
    )
    .await;
    let elapsed = started.elapsed();
    // 6,500 frames at 5 ms each would take more than 30 s.
    assert!(elapsed < Duration::from_secs(10), "took {elapsed:?}");
    assert_eq!(outcome.span.unwrap().end_ms, seconds * 1_000);
    let chunks = list_chunks(dir.path()).unwrap();
    assert_eq!(chunks.last().unwrap().end_ms, seconds * 1_000);
    assert!(dropped.load(Ordering::SeqCst) > 0, "the channel filled");
    worker.join().unwrap();
    assert_eq!(
        received.load(Ordering::SeqCst) + dropped.load(Ordering::SeqCst),
        seconds * 50
    );
}

// ------------------------------------------------------------ stereo (U6)

type FrameSender = tokio::sync::mpsc::UnboundedSender<Result<CaptureFrame, audio::Error>>;

/// A stream that the test feeds frame by frame.
fn fed_stream() -> (FrameSender, CaptureStream) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });
    (tx, CaptureStream::new(stream))
}

fn mic_frame(value: f32, len: usize) -> Result<CaptureFrame, audio::Error> {
    Ok(CaptureFrame {
        raw_mic: Arc::from(vec![value; len]),
        raw_speaker: Arc::from([] as [f32; 0]),
        aec_mic: None,
    })
}

fn speaker_frame(value: f32, len: usize) -> Result<CaptureFrame, audio::Error> {
    Ok(CaptureFrame {
        raw_mic: Arc::from([] as [f32; 0]),
        raw_speaker: Arc::from(vec![value; len]),
        aec_mic: None,
    })
}

fn stereo_writer(dir: &Path, segment: u32, offset_ms: u64, sample_rate: u32) -> ChunkedWriter {
    ChunkedWriter::with_probe(
        SegmentArgs {
            channels: 2,
            ..args(dir, segment, offset_ms, sample_rate)
        },
        plenty(),
    )
    .unwrap()
}

/// Left and right of every frame in a stereo chunk.
fn decode_stereo(path: &Path) -> Vec<(i16, i16)> {
    let reader = hound::WavReader::open(path).unwrap();
    assert_eq!(reader.spec().channels, 2);
    let samples: Vec<i16> = reader.into_samples().map(Result::unwrap).collect();
    samples.chunks(2).map(|f| (f[0], f[1])).collect()
}

/// What a scripted stereo segment gave back.
struct Script {
    outcome: SegmentOutcome,
    ticks: Vec<recorder::Tick>,
    /// (index, mic length, right channel) of each frame.
    frames: Vec<(u64, usize, Option<Vec<f32>>)>,
    speaker_ends: Vec<audio::Error>,
}

/// One step of a script: what the test sends before the segment runs
/// again.
enum Step {
    Mic(f32),
    Speaker(f32),
    SpeakerError,
    /// The speaker stream ends with no error.
    SpeakerEnd,
    Command(SpeakerCommand),
}

/// Run a stereo segment at 16 kHz with 20 ms frames, and feed it `steps`
/// in order. The segment processes each step before the next one is sent.
async fn run_script(dir: &Path, steps: Vec<Step>) -> (Script, Vec<FrameSender>) {
    const LEN: usize = 320;
    let (mic_tx, mic) = fed_stream();
    let (command_tx, commands) = tokio::sync::mpsc::unbounded_channel();
    let stop = CancellationToken::new();
    let mut ticks = Vec::new();
    let mut frames = Vec::new();
    let mut speaker_ends = Vec::new();
    let mut speakers: Vec<FrameSender> = Vec::new();
    let segment = recorder::run_joined_segment(
        SegmentInput {
            mic,
            speaker: Some(commands),
        },
        stereo_writer(dir, 0, 0, 16_000),
        stop.clone(),
        |tick| ticks.push(tick),
        |frame: Frame<'_>| {
            frames.push((
                frame.index,
                frame.mic.len(),
                frame.speaker.map(<[f32]>::to_vec),
            ))
        },
        |e| speaker_ends.push(e),
    );
    let driver = async {
        for step in steps {
            match step {
                Step::Mic(v) => mic_tx.send(mic_frame(v, LEN)).unwrap(),
                Step::Speaker(v) => {
                    let _ = speakers.last().unwrap().send(speaker_frame(v, LEN));
                }
                Step::SpeakerError => {
                    let _ = speakers
                        .last()
                        .unwrap()
                        .send(Err(audio::Error::SpeakerStreamEnded));
                }
                Step::SpeakerEnd => drop(speakers.pop()),
                Step::Command(SpeakerCommand::Attach(_)) => {
                    let (tx, stream) = fed_stream();
                    speakers.push(tx);
                    command_tx.send(SpeakerCommand::Attach(stream)).unwrap();
                }
                Step::Command(command) => command_tx.send(command).unwrap(),
            }
            // Let the segment take what was sent.
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
        }
        stop.cancel();
    };
    let (outcome, ()) = tokio::join!(segment, driver);
    (
        Script {
            outcome,
            ticks,
            frames,
            speaker_ends,
        },
        speakers,
    )
}

fn attach() -> Step {
    let (_, stream) = fed_stream();
    Step::Command(SpeakerCommand::Attach(stream))
}

/// `n` frames of paired audio: a speaker frame, then a mic frame.
fn paired(n: usize, mic: f32, speaker: f32) -> Vec<Step> {
    (0..n)
        .flat_map(|_| [Step::Speaker(speaker), Step::Mic(mic)])
        .collect()
}

#[tokio::test]
async fn a_stereo_segment_records_the_mic_left_and_system_audio_right() {
    let dir = tempfile::tempdir().unwrap();
    let mut steps = vec![attach()];
    steps.extend(paired(100, 0.5, 0.25));
    let (script, _) = run_script(dir.path(), steps).await;
    assert!(matches!(script.outcome.end, SegmentEnd::Stopped));
    assert_eq!(script.outcome.span.unwrap().end_ms, 2_000);

    let chunks = list_chunks(dir.path()).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(recorder::chunk_channels(&chunks[0]).unwrap(), 2);
    let frames = decode_stereo(&chunks[0].path);
    assert_eq!(frames.len(), 100 * 320, "one frame for each mic sample");
    assert!(frames.iter().all(|&f| f == (to_i16(0.5), to_i16(0.25))));
    // The sample index counts mic frames, and the tick has both peaks.
    for (n, (index, len, right)) in script.frames.iter().enumerate() {
        assert_eq!((*index, *len), (n as u64 * 320, 320));
        assert_eq!(right.as_ref().unwrap().len(), 320);
    }
    assert_eq!(script.ticks.len(), 8);
    assert!(script.ticks.iter().all(|t| t.speaker_peak == Some(0.25)));
}

#[tokio::test]
async fn a_stereo_segment_with_no_speaker_packets_pads_the_right_with_silence() {
    // Windows sends no packets while nothing plays.
    let dir = tempfile::tempdir().unwrap();
    let mut steps = vec![attach()];
    steps.extend((0..50).map(|_| Step::Mic(0.5)));
    let (script, _) = run_script(dir.path(), steps).await;
    assert_eq!(script.outcome.span.unwrap().end_ms, 1_000);
    let chunks = list_chunks(dir.path()).unwrap();
    let frames = decode_stereo(&chunks[0].path);
    assert_eq!(frames.len(), 50 * 320);
    assert!(frames.iter().all(|&f| f == (to_i16(0.5), 0)));
}

#[tokio::test]
async fn system_audio_off_in_a_stereo_segment_writes_silence_right_in_one_segment() {
    let dir = tempfile::tempdir().unwrap();
    let mut steps = vec![attach()];
    steps.extend(paired(50, 0.5, 0.25));
    steps.push(Step::Command(SpeakerCommand::Detach));
    // The detached stream goes on sending: none of it is recorded.
    steps.extend(paired(50, 0.5, 0.9));
    // Turned on again: a new speaker stream attaches in the same segment.
    steps.push(attach());
    steps.extend(paired(50, 0.5, 0.75));
    let (script, _) = run_script(dir.path(), steps).await;
    let span = script.outcome.span.unwrap();
    assert_eq!((span.segment, span.end_ms), (0, 3_000));
    let chunks = list_chunks(dir.path()).unwrap();
    assert_eq!(chunks.len(), 1, "one segment, one chunk");
    let frames = decode_stereo(&chunks[0].path);
    assert_eq!(frames.len(), 150 * 320);
    let right: Vec<i16> = frames.iter().map(|f| f.1).collect();
    assert!(right[..50 * 320].iter().all(|&r| r == to_i16(0.25)));
    assert!(right[50 * 320..100 * 320].iter().all(|&r| r == 0));
    assert!(right[100 * 320..].iter().all(|&r| r == to_i16(0.75)));
    assert!(frames.iter().all(|f| f.0 == to_i16(0.5)));
}

#[tokio::test]
async fn a_speaker_stream_error_keeps_the_mic_recording() {
    let dir = tempfile::tempdir().unwrap();
    let mut steps = vec![attach()];
    steps.extend(paired(25, 0.5, 0.25));
    steps.push(Step::SpeakerError);
    steps.extend((0..25).map(|_| Step::Mic(0.5)));
    // The capture watcher opens a new speaker stream when the device is
    // back, and attaches it.
    steps.push(attach());
    steps.extend(paired(25, 0.5, 0.25));
    let (script, _) = run_script(dir.path(), steps).await;
    assert!(matches!(script.outcome.end, SegmentEnd::Stopped));
    assert_eq!(script.speaker_ends, [audio::Error::SpeakerStreamEnded]);
    assert_eq!(script.outcome.span.unwrap().end_ms, 1_500);
    let frames = decode_stereo(&list_chunks(dir.path()).unwrap()[0].path);
    assert_eq!(frames.len(), 75 * 320, "no mic frame is lost");
    let right: Vec<i16> = frames.iter().map(|f| f.1).collect();
    assert!(right[..25 * 320].iter().all(|&r| r == to_i16(0.25)));
    assert!(right[25 * 320..50 * 320].iter().all(|&r| r == 0));
    assert!(right[50 * 320..].iter().all(|&r| r == to_i16(0.25)));
}

#[tokio::test]
async fn a_speaker_stream_that_ends_is_a_speaker_end_not_a_segment_end() {
    let dir = tempfile::tempdir().unwrap();
    let mut steps = vec![attach()];
    steps.extend(paired(10, 0.5, 0.25));
    steps.push(Step::SpeakerEnd);
    steps.extend((0..10).map(|_| Step::Mic(0.5)));
    let (script, _) = run_script(dir.path(), steps).await;
    assert_eq!(script.speaker_ends, [audio::Error::SpeakerStreamEnded]);
    assert!(matches!(script.outcome.end, SegmentEnd::Stopped));
    assert_eq!(script.outcome.span.unwrap().end_ms, 400);
    let frames = decode_stereo(&list_chunks(dir.path()).unwrap()[0].path);
    assert!(frames[10 * 320..].iter().all(|&f| f == (to_i16(0.5), 0)));
}

#[tokio::test(start_paused = true)]
async fn a_mono_segment_writes_mono_chunks_as_before() {
    // AE7: system audio off from start to Stop.
    let dir = tempfile::tempdir().unwrap();
    let rate = 48_000;
    let mock = MockAudio::tone(440.0, rate, Duration::from_secs(2));
    let stream = mock.open_mic_capture(None, rate, 960).unwrap();
    let mut ticks = Vec::new();
    run_segment(
        stream,
        writer(dir.path(), 0, 0, rate),
        CancellationToken::new(),
        |tick| ticks.push(tick),
    )
    .await;
    let chunks = list_chunks(dir.path()).unwrap();
    assert_eq!(recorder::chunk_channels(&chunks[0]).unwrap(), 1);
    assert_eq!(decode(&chunks[0].path).len(), 2 * rate as usize);
    assert!(ticks.iter().all(|t| t.speaker_peak.is_none()));
}

#[test]
fn a_crash_in_a_stereo_segment_leaves_a_partial_that_recovery_closes_on_whole_frames() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 16_000;
    let mut w = stereo_writer(dir.path(), 3, 5_000, rate);
    let second: Vec<f32> = (0..rate as usize).flat_map(|_| [0.5f32, -0.25]).collect();
    for _ in 0..70 {
        w.write(&second).unwrap();
    }
    drop(w);
    // Half a frame reached the disk before the crash.
    let part = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "part"))
        .unwrap();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&part)
        .unwrap()
        .write_all(&[9, 9])
        .unwrap();

    let recovered = recover_partial_chunks(dir.path()).unwrap().recovered;
    assert_eq!(recovered.len(), 1);
    let chunks = list_chunks(dir.path()).unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(
        (
            chunks[1].start_ms,
            chunks[1].end_ms,
            chunks[1].audio_start_ms
        ),
        (65_000, 75_000, 63_000),
        "the millisecond math counts frames, not samples"
    );
    assert_eq!(recorder::chunk_channels(&chunks[1]).unwrap(), 2);
    let frames = decode_stereo(&chunks[1].path);
    assert_eq!(frames.len(), 12 * rate as usize);
    assert!(frames.iter().all(|&f| f == (to_i16(0.5), to_i16(-0.25))));
}

#[test]
fn a_stereo_writer_refuses_a_part_frame() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = stereo_writer(dir.path(), 0, 0, 16_000);
    assert!(w.write(&[0.1, 0.2, 0.3]).is_err());
    w.write(&[0.1, 0.2]).unwrap();
    assert_eq!(w.end_ms(), 0);
}

/// The `.part` files in `dir`.
fn parts(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut parts: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "part"))
        .collect();
    parts.sort();
    parts
}

#[cfg(unix)]
#[test]
fn a_read_only_recovery_folder_leaves_each_partial_and_reports_it() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let rate = 16_000;
    for segment in 0..2 {
        let mut w = writer(dir.path(), segment, segment as u64 * 10_000, rate);
        w.write(&vec![0.1; 3 * rate as usize]).unwrap();
        drop(w);
    }
    let before = parts(dir.path());
    assert_eq!(before.len(), 2);
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    // A process that ignores permissions (root) cannot test this.
    let ignores_permissions = std::fs::write(dir.path().join("probe"), b"").is_ok();
    let result = recover_partial_chunks(dir.path());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    if ignores_permissions {
        return;
    }

    let report = result.unwrap();
    assert!(report.recovered.is_empty());
    let mut left = report.left;
    left.sort();
    assert_eq!(left, before);
    assert_eq!(parts(dir.path()), before, "each partial stays in place");
}

#[test]
fn a_partial_that_cannot_be_repaired_is_reported_and_the_others_are_recovered() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 16_000;
    let mut w = writer(dir.path(), 0, 0, rate);
    w.write(&vec![0.1; 3 * rate as usize]).unwrap();
    drop(w);
    let bad = dir.path().join("5-1700000000000-0-0.part");
    // Longer than a header, so recovery cannot tell that it holds no samples.
    std::fs::write(&bad, [b'x'; 100]).unwrap();

    let report = recover_partial_chunks(dir.path()).unwrap();

    assert_eq!(report.recovered.len(), 1);
    assert_eq!(report.recovered[0].segment, 0);
    assert_eq!(report.left, vec![bad.clone()]);
    assert!(bad.is_file());
    assert_eq!(list_chunks(dir.path()).unwrap().len(), 1);
}

#[test]
fn an_empty_partial_holds_no_samples_and_recovery_removes_it() {
    // WavFile::create buffers the header, so a crash before the first flush
    // leaves a 0-byte `.part`. A `.part` of header length holds no samples.
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("0-1700000000000-0-0.part");
    std::fs::write(&empty, b"").unwrap();
    let header_only = dir.path().join("1-1700000000000-0-0.part");
    std::fs::write(&header_only, [0u8; recorder::wav::HEADER_BYTES as usize]).unwrap();

    let report = recover_partial_chunks(dir.path()).unwrap();

    assert!(report.recovered.is_empty());
    assert!(report.left.is_empty(), "{:?}", report.left);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
