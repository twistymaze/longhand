use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use audio::AudioProvider;
use audio_mock::MockAudio;
use chrono::{FixedOffset, TimeZone};
use recorder::storage::DISK_RESERVE_BYTES;
use recorder::wav::to_i16;
use recorder::{
    ChunkedWriter, RecorderError, SegmentArgs, SegmentEnd, list_chunks, recover_partial_chunks,
    remove_segment, run_segment,
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

    let recovered = recover_partial_chunks(dir.path()).unwrap();
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
    // Simulate the crash landing before the new sample reached the disk.
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

    assert!(recover_partial_chunks(dir.path()).unwrap().is_empty());
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
