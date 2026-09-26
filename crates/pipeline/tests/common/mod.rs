#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, FixedOffset, TimeZone};
use pipeline::{RECOVERY_DIR, SharedStore};
use recorder::{ChunkedWriter, SegmentArgs};
use session_core::{Event, Segment, Store};

pub fn at(secs: i64) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(0)
        .unwrap()
        .timestamp_opt(1_800_000_000 + secs, 0)
        .unwrap()
}

pub fn store() -> SharedStore {
    Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
}

/// Real far-field speech, mono 16 kHz (the AMI fixture of `crates/vad`).
pub fn speech() -> Vec<f32> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vad/tests/fixtures/ami-sdm-speech.wav"
    );
    hound::WavReader::open(path)
        .unwrap()
        .into_samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32_768.0)
        .collect()
}

/// Write `audio` as segment `index` of a session through the real chunk
/// writer, and return the segment row. With `crash`, the last chunk is left
/// as the `.part` a killed process leaves.
pub fn write_segment(
    session_dir: &Path,
    index: u32,
    offset_ms: u64,
    rate: u32,
    audio: &[f32],
    crash: bool,
) -> Segment {
    let mut writer = ChunkedWriter::with_probe(
        SegmentArgs {
            dir: session_dir.join(RECOVERY_DIR),
            segment: index,
            capture_started_at: 1_800_000_000_000,
            offset_ms,
            sample_rate: rate,
            channels: 1,
            floor_bytes: 0,
        },
        Box::new(|_| Some(u64::MAX)),
    )
    .unwrap();
    for block in audio.chunks(rate as usize / 50) {
        writer.write(block).unwrap();
    }
    let duration_ms = audio.len() as u64 * 1000 / rate as u64;
    if crash {
        drop(writer);
    } else {
        writer.finish().unwrap();
    }
    Segment {
        index,
        offset_ms,
        duration_ms,
        wall_start: at(offset_ms as i64 / 1000),
        wall_end: at((offset_ms + duration_ms) as i64 / 1000),
        audio_file: format!("audio/segment-{:02}.opus", index + 1),
        channels: 1,
    }
}

/// [`write_segment`] for a stereo segment: `left` is the microphone and
/// `right` is system audio, of the same length. It writes interleaved
/// frames through the real chunk writer.
#[allow(dead_code, reason = "not every test binary writes a stereo segment")]
pub fn write_stereo_segment(
    session_dir: &Path,
    index: u32,
    offset_ms: u64,
    rate: u32,
    left: &[f32],
    right: &[f32],
    crash: bool,
) -> Segment {
    assert_eq!(left.len(), right.len());
    let mut writer = ChunkedWriter::with_probe(
        SegmentArgs {
            dir: session_dir.join(RECOVERY_DIR),
            segment: index,
            capture_started_at: 1_800_000_000_000,
            offset_ms,
            sample_rate: rate,
            channels: 2,
            floor_bytes: 0,
        },
        Box::new(|_| Some(u64::MAX)),
    )
    .unwrap();
    let frame = rate as usize / 50;
    for (l, r) in left.chunks(frame).zip(right.chunks(frame)) {
        let block: Vec<f32> = l.iter().zip(r).flat_map(|(&l, &r)| [l, r]).collect();
        writer.write(&block).unwrap();
    }
    let duration_ms = left.len() as u64 * 1000 / rate as u64;
    if crash {
        drop(writer);
    } else {
        writer.finish().unwrap();
    }
    Segment {
        index,
        offset_ms,
        duration_ms,
        wall_start: at(offset_ms as i64 / 1000),
        wall_end: at((offset_ms + duration_ms) as i64 / 1000),
        audio_file: format!("audio/segment-{:02}.opus", index + 1),
        channels: 2,
    }
}

/// A stopped, queued session with the given segments already on disk.
pub fn queue_session(
    store: &SharedStore,
    root: &Path,
    id: &str,
    stopped_at: i64,
    model: Option<&str>,
    segments: &[(u32, Vec<f32>)],
) {
    record_session(store, root, id, stopped_at, model, segments);
    stop_session(store, id, stopped_at);
}

/// A session that is still recording, with the given segments on disk.
pub fn record_session(
    store: &SharedStore,
    root: &Path,
    id: &str,
    stopped_at: i64,
    model: Option<&str>,
    segments: &[(u32, Vec<f32>)],
) {
    let dir = root.join(id);
    let mut s = store.lock().unwrap();
    s.create_session(id, at(stopped_at - 100), None).unwrap();
    let mut offset = 0;
    for (rate, audio) in segments {
        let segment = write_segment(
            &dir,
            s.segments(id).unwrap().len() as u32,
            offset,
            *rate,
            audio,
            false,
        );
        offset = segment.end_ms() + 5_000;
        s.put_segment(id, &segment).unwrap();
    }
    if let Some(model) = model {
        s.set_asr_model(id, model).unwrap();
    }
}

/// Stop a recording session, which queues it.
pub fn stop_session(store: &SharedStore, id: &str, stopped_at: i64) {
    store
        .lock()
        .unwrap()
        .apply(
            id,
            &Event::UserStop {
                at: at(stopped_at),
                retained_audio: true,
            },
        )
        .unwrap();
}
