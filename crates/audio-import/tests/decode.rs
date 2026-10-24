//! Tests of the copy, the decode and the recording time (U2), with the
//! synthetic files in `testdata/audio-import/`.

use std::path::{Path, PathBuf};

use audio_import::{DecodeError, copy_with_hash, decode_mono, hash_file, recording_time};
use chrono::{Local, TimeZone, Utc};
use sha2::{Digest, Sha256};

fn sample(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/audio-import")
        .join(name)
}

fn sha256_hex(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap();
    Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Decodes `path` and returns the sample rate and all the mono samples.
fn decode_all(path: &Path) -> (u32, Vec<f32>) {
    let mut samples = Vec::new();
    let info = decode_mono(path, |block| samples.extend_from_slice(block)).unwrap();
    assert_eq!(info.frames, samples.len() as u64);
    (info.sample_rate, samples)
}

#[test]
fn a_copy_has_the_hash_of_the_source_and_decodes_to_its_length() {
    let dir = tempfile::tempdir().unwrap();
    let source = sample("tone-mono.m4a");

    let copied = copy_with_hash(&source, &dir.path().join("copy.m4a")).unwrap();

    assert_eq!(copied.sha256, sha256_hex(&source));
    assert_eq!(copied.bytes, std::fs::metadata(&source).unwrap().len());
    assert_eq!(
        std::fs::read(&copied.path).unwrap(),
        std::fs::read(&source).unwrap()
    );
    let (rate, samples) = decode_all(&copied.path);
    assert_eq!(rate, 22050);
    let seconds = samples.len() as f64 / f64::from(rate);
    assert!((seconds - 5.0).abs() < 0.1, "{seconds} s");
}

#[test]
fn an_m4a_with_moov_after_mdat_has_the_hash_of_the_whole_file() {
    let dir = tempfile::tempdir().unwrap();
    let source = sample("tone-moov-last.m4a");

    let copied = copy_with_hash(&source, &dir.path().join("copy.m4a")).unwrap();

    assert_eq!(copied.sha256, sha256_hex(&source));
    let (_, samples) = decode_all(&copied.path);
    assert!(!samples.is_empty());
}

#[test]
fn a_stereo_file_decodes_to_the_average_of_its_channels() {
    // The left channel is a 440 Hz tone at ffmpeg's default amplitude of
    // 1/8, and the right channel is silent, so the mono peak is 1/16.
    let (rate, samples) = decode_all(&sample("tone-stereo.mp3"));
    assert_eq!(rate, 22050);
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!((peak - 0.0625).abs() < 0.01, "peak {peak}");
}

#[test]
fn a_wav_decodes_through_the_same_function() {
    let (rate, samples) = decode_all(&sample("tone.wav"));
    assert_eq!(rate, 22050);
    assert_eq!(samples.len(), 5 * 22050);
}

#[test]
fn the_recording_time_comes_from_mvhd_or_tdrc() {
    // mvhd is UTC; TDRC has no time zone, so it is local time.
    let utc = Utc.with_ymd_and_hms(2026, 10, 20, 14, 30, 0).unwrap();
    let local = Local
        .with_ymd_and_hms(2026, 10, 20, 14, 30, 0)
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(recording_time(&sample("tone-mono.m4a")), Some(utc));
    assert_eq!(recording_time(&sample("tone-moov-last.m4a")), Some(utc));
    assert_eq!(recording_time(&sample("tone-stereo.mp3")), Some(local));
}

#[test]
fn a_file_with_no_recording_time_gives_none() {
    assert_eq!(recording_time(&sample("tone-no-time.m4a")), None);
    assert_eq!(recording_time(&sample("tone.wav")), None);
}

#[test]
fn a_truncated_m4a_is_a_decode_error() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = std::fs::read(sample("tone-moov-last.m4a")).unwrap();
    let truncated = dir.path().join("truncated.m4a");
    std::fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();

    let result = decode_mono(&truncated, |_| {});

    assert!(matches!(result, Err(DecodeError::Format(_))), "{result:?}");
}

#[test]
fn text_with_an_m4a_extension_is_a_decode_error() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("notes.m4a");
    std::fs::write(&fake, "These are notes, not audio.\n").unwrap();

    let result = decode_mono(&fake, |_| {});

    assert!(matches!(result, Err(DecodeError::Format(_))), "{result:?}");
}

#[test]
fn the_hash_with_no_copy_equals_the_hash_of_the_copy() {
    let dir = tempfile::tempdir().unwrap();
    let source = sample("tone-moov-last.m4a");
    let copied = copy_with_hash(&source, &dir.path().join("copy.m4a")).unwrap();

    assert_eq!(hash_file(&source).unwrap(), copied.sha256);
}

// The watched folder tells a slow download from a stalled one by the bytes
// that the hash has read (KTD8).
#[test]
fn the_counted_hash_equals_the_hash_and_counts_every_byte() {
    let source = sample("tone.wav");
    let read = std::sync::atomic::AtomicU64::new(0);
    let hash = audio_import::hash_file_counted(&source, &read).unwrap();
    assert_eq!(hash, sha256_hex(&source));
    assert_eq!(
        read.load(std::sync::atomic::Ordering::SeqCst),
        std::fs::metadata(&source).unwrap().len()
    );
}

#[test]
fn a_decode_that_the_caller_stops_gives_stopped_after_the_first_block() {
    let mut blocks = Vec::new();
    let result = audio_import::decode_mono_until(&sample("tone-mono.m4a"), |rate, block| {
        blocks.push((rate, block.len()));
        false
    });
    assert!(matches!(result, Err(DecodeError::Stopped)), "{result:?}");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, 22050);
}
