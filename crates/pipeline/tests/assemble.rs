//! The assemble stage over real chunk files.

mod common;

use std::f64::consts::TAU;
use std::fs::File;

use common::{at, store, write_segment};
use pipeline::assemble::{self, PROC_RATE};
use pipeline::{RECOVERY_DIR, Unattended, proc_file};
use session_core::Stage;

/// A tone whose phase does not repeat every lookback, so a mis-trimmed
/// overlap shows as a discontinuity.
const TONE_HZ: f64 = 437.3;

fn tone(rate: u32, secs: f64) -> Vec<f32> {
    (0..(rate as f64 * secs) as usize)
        .map(|i| (0.5 * (TAU * TONE_HZ * i as f64 / rate as f64).sin()) as f32)
        .collect()
}

fn read_proc(path: &std::path::Path) -> (hound::WavSpec, Vec<f32>) {
    let reader = hound::WavReader::open(path).unwrap();
    let spec = reader.spec();
    (
        spec,
        reader.into_samples::<f32>().map(|s| s.unwrap()).collect(),
    )
}

/// Decode an Ogg Opus file and return its sample count after pre-skip and
/// end trimming, as a player would present it.
fn opus_samples(path: &std::path::Path) -> (u32, u64) {
    let mut reader = ogg::PacketReader::new(File::open(path).unwrap());
    let head = reader.read_packet_expected().unwrap();
    assert_eq!(&head.data[..8], b"OpusHead");
    assert_eq!(head.data[9], 1, "mono");
    let pre_skip = u16::from_le_bytes([head.data[10], head.data[11]]) as u64;
    let input_rate = u32::from_le_bytes(head.data[12..16].try_into().unwrap());
    let tags = reader.read_packet_expected().unwrap();
    assert_eq!(&tags.data[..8], b"OpusTags");

    let mut decoder = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
    let mut out = vec![0f32; 5_760];
    let mut decoded = 0u64;
    let mut last_granule = 0;
    while let Some(packet) = reader.read_packet().unwrap() {
        decoded += decoder.decode_float(&packet.data, &mut out, false).unwrap() as u64;
        last_granule = packet.absgp_page();
    }
    assert!(decoded + pre_skip >= last_granule);
    (input_rate, last_granule - pre_skip)
}

#[test]
fn three_overlapping_chunks_and_a_part_tail_become_one_continuous_segment() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("s1");
    let audio = tone(48_000, 130.0);
    let segment = write_segment(&dir, 0, 0, 48_000, &audio, true);

    let names: Vec<String> = std::fs::read_dir(dir.join(RECOVERY_DIR))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.iter().filter(|n| n.ends_with(".wav")).count(), 2);
    assert_eq!(names.iter().filter(|n| n.ends_with(".part")).count(), 1);

    let store = store();
    {
        let mut s = store.lock().unwrap();
        s.create_session("s1", at(0), None).unwrap();
        s.put_segment("s1", &segment).unwrap();
    }
    let done = assemble::run(
        &store,
        "s1",
        &dir,
        std::slice::from_ref(&segment),
        None,
        &Unattended,
    )
    .unwrap();

    // The `.part` was repaired and named, and the lookback trimmed away.
    assert!(
        std::fs::read_dir(dir.join(RECOVERY_DIR))
            .unwrap()
            .all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".part"))
    );
    assert_eq!(done[0].native_samples, 130 * 48_000);

    let (spec, proc) = read_proc(&dir.join(proc_file(&segment.audio_file)));
    assert_eq!(spec.sample_rate, PROC_RATE);
    assert_eq!(spec.channels, 1);
    assert_eq!(spec.sample_format, hound::SampleFormat::Float);
    assert_eq!(spec.bits_per_sample, 32);
    let expected = 130 * PROC_RATE as i64;
    assert!(
        (proc.len() as i64 - expected).abs() <= 320,
        "{} samples",
        proc.len()
    );
    assert_eq!(done[0].proc_samples, proc.len() as u64);

    // Continuous across both chunk boundaries: the output is the same tone.
    for boundary_s in [60, 120] {
        let centre = boundary_s * PROC_RATE as usize;
        for (i, &got) in proc.iter().enumerate().skip(centre - 800).take(1_600) {
            let want = 0.5 * (TAU * TONE_HZ * i as f64 / PROC_RATE as f64).sin();
            let got = got as f64;
            assert!((got - want).abs() < 0.02, "sample {i}: {got} vs {want}");
        }
    }

    let (input_rate, samples) = opus_samples(&dir.join(&segment.audio_file));
    assert_eq!(input_rate, 48_000);
    assert!(
        (samples as i64 - 130 * 48_000).abs() <= 960,
        "{samples} opus samples"
    );

    // The checkpoint is written and a second run does not redo the work.
    let rows = store
        .lock()
        .unwrap()
        .checkpoints("s1", Stage::Assemble)
        .unwrap();
    assert!(rows.iter().any(|(k, _)| k == "seg-0000"));
    assert!(rows.iter().any(|(k, _)| k == "done"));
    let mtime = std::fs::metadata(dir.join(&segment.audio_file))
        .unwrap()
        .modified()
        .unwrap();
    assemble::run(
        &store,
        "s1",
        &dir,
        std::slice::from_ref(&segment),
        None,
        &Unattended,
    )
    .unwrap();
    assert_eq!(
        std::fs::metadata(dir.join(&segment.audio_file))
            .unwrap()
            .modified()
            .unwrap(),
        mtime
    );
}

#[test]
fn a_44_1_khz_device_writes_its_rate_to_the_archive_header_and_16_khz_to_proc() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("s2");
    let segment = write_segment(&dir, 0, 0, 44_100, &tone(44_100, 7.5), false);
    let store = store();
    store
        .lock()
        .unwrap()
        .create_session("s2", at(0), None)
        .unwrap();
    let done = assemble::run(
        &store,
        "s2",
        &dir,
        std::slice::from_ref(&segment),
        None,
        &Unattended,
    )
    .unwrap();
    assert_eq!(done[0].native_rate, 44_100);
    let (spec, proc) = read_proc(&dir.join(proc_file(&segment.audio_file)));
    assert_eq!(spec.sample_rate, PROC_RATE);
    assert!((proc.len() as i64 - 120_000).abs() <= 320);
    let (input_rate, samples) = opus_samples(&dir.join(&segment.audio_file));
    assert_eq!(input_rate, 44_100);
    assert!((samples as i64 - 360_000).abs() <= 960);
}

#[test]
fn a_resumed_sessions_segment_is_decoded_back_from_its_completed_folder() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("s3");
    let audio = tone(48_000, 20.0);
    let segment = write_segment(&dir, 0, 0, 48_000, &audio, false);
    let store = store();
    {
        let mut s = store.lock().unwrap();
        s.create_session("s3", at(0), None).unwrap();
        s.put_segment("s3", &segment).unwrap();
    }
    let first = assemble::run(
        &store,
        "s3",
        &dir,
        std::slice::from_ref(&segment),
        None,
        &Unattended,
    )
    .unwrap();
    let (_, original) = read_proc(&dir.join(proc_file(&segment.audio_file)));

    // The session completed: its archive is in the folder, and the working
    // directory, processing audio and chunks included, is gone.
    let home = root.path().join("folder");
    std::fs::create_dir_all(home.join("audio")).unwrap();
    std::fs::rename(
        dir.join(&segment.audio_file),
        home.join(&segment.audio_file),
    )
    .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let archive = std::fs::read(home.join(&segment.audio_file)).unwrap();

    let done = assemble::run(
        &store,
        "s3",
        &dir,
        std::slice::from_ref(&segment),
        Some(&home),
        &Unattended,
    )
    .unwrap();
    let (spec, decoded) = read_proc(&dir.join(proc_file(&segment.audio_file)));
    assert_eq!(spec.sample_rate, 16_000);
    assert_eq!(done[0].proc_samples, decoded.len() as u64);
    let drift = (decoded.len() as i64 - first[0].proc_samples as i64).abs();
    assert!(
        drift <= 16,
        "{} decoded against {} originally",
        decoded.len(),
        first[0].proc_samples
    );
    let rms = |v: &[f32]| (v.iter().map(|s| s * s).sum::<f32>() / v.len() as f32).sqrt();
    let (a, b) = (rms(&original), rms(&decoded));
    assert!((a - b).abs() < a * 0.1, "level {b} against {a}");
    assert_eq!(
        std::fs::read(home.join(&segment.audio_file)).unwrap(),
        archive,
        "the folder's audio is never rewritten"
    );
    assert!(!dir.join(&segment.audio_file).exists(), "no second archive");
}
