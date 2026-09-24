//! Against the machine's real microphone. Ignored by default: CI has no
//! input device, and a local run prompts for microphone access.
//!
//! `cargo test -p audio-cpal --test hardware -- --ignored --nocapture`

use std::time::{Duration, Instant};

use audio_cpal::{AudioProvider, CpalAudio};
use futures_util::StreamExt;

#[tokio::test]
#[ignore = "needs a real input device"]
async fn the_default_device_delivers_a_first_frame_within_500_ms() {
    let audio = CpalAudio::new();
    let rate = audio.prepare(None).expect("default input resolves");
    // The press: everything after `prepare`, as the shortcut will see it.
    let started = Instant::now();
    let mut stream = audio
        .open_mic_capture(None, 0, (rate / 50) as usize)
        .expect("default input opens");
    let frame = tokio::time::timeout(Duration::from_millis(500), stream.next())
        .await
        .expect("a frame within 500 ms")
        .expect("stream still open")
        .expect("frame, not error");
    println!(
        "native rate {rate} Hz; first frame of {} samples after {:?}",
        frame.raw_mic.len(),
        started.elapsed()
    );
    assert_eq!(frame.raw_mic.len(), (rate / 50) as usize);
}

#[tokio::test]
#[ignore = "needs a real input device"]
async fn three_seconds_of_the_real_microphone_arrive_as_a_playable_wav() {
    use recorder::{ChunkedWriter, SegmentArgs, SegmentEnd, list_chunks, run_segment};
    use tokio_util::sync::CancellationToken;

    let dir = tempfile::tempdir().unwrap();
    let audio = CpalAudio::new();
    let rate = audio.prepare(None).unwrap();
    let stream = audio
        .open_mic_capture(None, 0, (rate / 50) as usize)
        .unwrap();
    let writer = ChunkedWriter::create(SegmentArgs {
        dir: dir.path().to_path_buf(),
        segment: 0,
        capture_started_at: 0,
        offset_ms: 0,
        sample_rate: rate,
        floor_bytes: recorder::storage::DISK_RESERVE_BYTES,
    })
    .unwrap();
    let stop = CancellationToken::new();
    let task = tokio::spawn(run_segment(stream, writer, stop.clone(), |_| {}));
    tokio::time::sleep(Duration::from_secs(3)).await;
    stop.cancel();
    let outcome = task.await.unwrap();
    assert!(matches!(outcome.end, SegmentEnd::Stopped));
    let span = outcome.span.unwrap();

    let chunks = list_chunks(dir.path()).unwrap();
    assert_eq!(chunks.len(), 1);
    let reader = hound::WavReader::open(&chunks[0].path).unwrap();
    let spec = reader.spec();
    let samples: Vec<i16> = reader.into_samples().map(Result::unwrap).collect();
    let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    println!(
        "{} Hz, {} samples ({} ms on the timeline), peak {peak}",
        spec.sample_rate,
        samples.len(),
        span.end_ms
    );
    assert_eq!(spec.sample_rate, rate);
    // CoreAudio can hand over a short pre-roll when the stream starts.
    assert!((2_700..=3_500).contains(&span.end_ms));
    let keep = std::env::var_os("LONGHAND_KEEP_WAV");
    if let Some(target) = keep {
        std::fs::copy(&chunks[0].path, target).unwrap();
    }
}
