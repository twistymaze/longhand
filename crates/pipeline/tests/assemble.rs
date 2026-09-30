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

/// A saved note "s4" with one assembled segment, resumed: its archive is in
/// the home folder, its working directory is gone, and the Resume listed
/// the segment for its copy (KTD8 of the audio removal plan).
fn resumed_with_list(
    root: &std::path::Path,
) -> (
    pipeline::SharedStore,
    std::path::PathBuf,
    std::path::PathBuf,
    session_core::Segment,
) {
    let dir = root.join("s4");
    let segment = write_segment(&dir, 0, 0, 48_000, &tone(48_000, 5.0), false);
    let store = store();
    {
        let mut s = store.lock().unwrap();
        s.create_session("s4", at(0), None).unwrap();
        s.put_segment("s4", &segment).unwrap();
    }
    let slice = std::slice::from_ref(&segment);
    assemble::run(&store, "s4", &dir, slice, None, &Unattended).unwrap();
    let home = root.join("folder");
    std::fs::create_dir_all(home.join("audio")).unwrap();
    std::fs::rename(
        dir.join(&segment.audio_file),
        home.join(&segment.audio_file),
    )
    .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    pipeline::old_audio::begin(&dir, slice).unwrap();
    (store, dir, home, segment)
}

#[test]
fn an_old_segment_is_decoded_from_its_copy_after_its_home_file_goes() {
    let root = tempfile::tempdir().unwrap();
    let (store, dir, home, segment) = resumed_with_list(root.path());
    let archive = std::fs::read(home.join(&segment.audio_file)).unwrap();
    let copy_home = home.clone();
    pipeline::old_audio::copy(&dir, &move || Some(copy_home.clone())).unwrap();
    // Another computer removes the audio: the marker, then the delete.
    std::fs::write(
        home.join(session_core::folder::removal_marker(&segment.audio_file)),
        session_core::folder::REMOVAL_MARKER_TEXT,
    )
    .unwrap();
    std::fs::remove_file(home.join(&segment.audio_file)).unwrap();

    let slice = std::slice::from_ref(&segment);
    let done = assemble::run(&store, "s4", &dir, slice, Some(&home), &Unattended).unwrap();
    let (_, decoded) = read_proc(&dir.join(proc_file(&segment.audio_file)));
    assert_eq!(done[0].proc_samples, decoded.len() as u64);
    assert_eq!(
        std::fs::read(dir.join(&segment.audio_file)).unwrap(),
        archive
    );
    assert!(!home.join(&segment.audio_file).exists());
}

#[test]
fn assembly_makes_the_copy_that_the_resume_did_not_complete() {
    let root = tempfile::tempdir().unwrap();
    let (store, dir, home, segment) = resumed_with_list(root.path());
    let archive = std::fs::read(home.join(&segment.audio_file)).unwrap();
    let slice = std::slice::from_ref(&segment);
    assemble::run(&store, "s4", &dir, slice, Some(&home), &Unattended).unwrap();
    assert_eq!(
        std::fs::read(dir.join(&segment.audio_file)).unwrap(),
        archive
    );
    assert_eq!(
        pipeline::old_audio::old_segments(&dir).unwrap().unwrap()[0].state,
        pipeline::old_audio::CopyState::Copied
    );
}

#[test]
fn an_old_segment_removed_before_its_copy_fails_assembly_with_the_removal() {
    let root = tempfile::tempdir().unwrap();
    let (store, dir, home, segment) = resumed_with_list(root.path());
    std::fs::write(
        home.join(session_core::folder::removal_marker(&segment.audio_file)),
        session_core::folder::REMOVAL_MARKER_TEXT,
    )
    .unwrap();
    std::fs::remove_file(home.join(&segment.audio_file)).unwrap();
    let slice = std::slice::from_ref(&segment);
    let error = assemble::run(&store, "s4", &dir, slice, Some(&home), &Unattended).unwrap_err();
    assert!(error.to_string().contains("was removed"), "{error}");
    assert!(!dir.join(&segment.audio_file).exists(), "no copy is kept");
}

// ------------------------------------------------------------ stereo (U9)

mod stereo {
    use super::*;
    use common::{Recorded, add, delayed, place, voice, write_stereo_segment};
    use pipeline::assemble::ArchiveReader;
    use pipeline::echo::{self, EchoSpans};
    use pipeline::{SharedStore, checkpoint};
    use session_core::Segment;

    const RATE: usize = 16_000;
    /// The speaker echo is late by 250 ms, as with Bluetooth output.
    const ECHO_DELAY: usize = RATE / 4;
    const ECHO_GAIN: f32 = 0.4;

    fn secs(s: f64) -> usize {
        (s * RATE as f64) as usize
    }

    /// The value that a sample has after the chunk writer stores it.
    fn stored(x: &[f32]) -> Vec<f32> {
        x.iter()
            .map(|&s| recorder::wav::to_i16(s) as f32 / 32_768.0)
            .collect()
    }

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|&s| f64::from(s) * f64::from(s)).sum()
    }

    fn db(ratio: f64) -> f64 {
        10.0 * ratio.max(1e-20).log10()
    }

    /// The RMS over the 20 ms frames above -50 dBFS, in dBFS.
    fn speech_level(x: &[f32]) -> f64 {
        let frames: Vec<f64> = x
            .chunks(320)
            .map(|f| energy(f) / f.len() as f64)
            .filter(|&p| p > 1e-5)
            .collect();
        assert!(!frames.is_empty(), "no speech");
        db(frames.iter().sum::<f64>() / frames.len() as f64)
    }

    /// The energy of the part of `signal` that is a scaled copy of `reference`.
    fn projected(signal: &[f32], reference: &[f32]) -> f64 {
        let cross: f64 = signal
            .iter()
            .zip(reference)
            .map(|(&a, &b)| f64::from(a) * f64::from(b))
            .sum();
        cross * cross / energy(reference).max(1e-20)
    }

    fn noise(len: usize, level: f32) -> Vec<f32> {
        let mut state = 0x2545_f491u32;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                level * ((state as f32 / u32::MAX as f32) * 2.0 - 1.0)
            })
            .collect()
    }

    struct Assembled {
        store: SharedStore,
        dir: std::path::PathBuf,
        _root: tempfile::TempDir,
        segment: Segment,
        proc: Vec<f32>,
        echo: EchoSpans,
    }

    fn assemble_stereo(rate: u32, left: &[f32], right: &[f32]) -> Assembled {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("s");
        let segment = write_stereo_segment(&dir, 0, 0, rate, left, right, false);
        let store = store();
        {
            let mut s = store.lock().unwrap();
            s.create_session("s", at(0), None).unwrap();
            s.put_segment("s", &segment).unwrap();
        }
        let done = assemble::run(
            &store,
            "s",
            &dir,
            std::slice::from_ref(&segment),
            None,
            &Unattended,
        )
        .unwrap();
        let (spec, proc) = read_proc(&dir.join(proc_file(&segment.audio_file)));
        assert_eq!(spec.channels, 1, "the processing file is mono");
        assert_eq!(done[0].proc_samples, proc.len() as u64);
        assert_eq!(
            done[0].native_samples,
            left.len() as u64,
            "samples per channel"
        );
        let echo = echo::saved(&store, "s", 0)
            .unwrap()
            .expect("an echo spans row");
        assert!(
            std::fs::read_dir(dir.join(pipeline::PROC_DIR))
                .unwrap()
                .all(|e| e.unwrap().file_name().to_string_lossy().ends_with(".wav")),
            "the temporary stereo file is gone"
        );
        Assembled {
            store,
            dir,
            _root: root,
            segment,
            proc,
            echo,
        }
    }

    /// Cancels once the processing file's `.tmp` exists: the stereo arm
    /// creates it just before echo removal and the mix.
    struct CancelInEcho(std::path::PathBuf);

    impl pipeline::Control for CancelInEcho {
        fn proceed(&self) -> Result<(), pipeline::StageError> {
            if self.0.exists() {
                Err(pipeline::StageError::Cancelled)
            } else {
                Ok(())
            }
        }
    }

    /// The files under `dir` whose names end with `.tmp`.
    fn temp_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.extend(temp_files(&path));
            } else if path.to_string_lossy().ends_with(".tmp") {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn a_cancel_during_echo_removal_removes_the_temporary_files() {
        let (system, _) = remote(secs(20.0));
        let mic = delayed(&system, ECHO_DELAY, ECHO_GAIN);
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("s");
        let segment = write_stereo_segment(&dir, 0, 0, RATE as u32, &mic, &system, false);
        let store = store();
        {
            let mut s = store.lock().unwrap();
            s.create_session("s", at(0), None).unwrap();
            s.put_segment("s", &segment).unwrap();
        }
        let processing = dir.join(proc_file(&segment.audio_file));
        let mut name = processing.file_name().unwrap().to_os_string();
        name.push(".tmp");
        let control = CancelInEcho(processing.with_file_name(name));

        let result = assemble::run(
            &store,
            "s",
            &dir,
            std::slice::from_ref(&segment),
            None,
            &control,
        );
        assert!(
            matches!(result, Err(pipeline::StageError::Cancelled)),
            "the stage stops at the cancel"
        );
        assert_eq!(
            temp_files(&dir),
            Vec::<std::path::PathBuf>::new(),
            "no temporary file stays after the cancel"
        );
    }

    /// Remote speech on the system channel: three sentences of 3 s.
    fn remote(len: usize) -> (Vec<f32>, Vec<(usize, usize)>) {
        let sentence = voice(secs(3.0), 200.0, 7, 0.05);
        let starts = [secs(2.0), secs(9.0), secs(16.0)];
        let parts: Vec<(usize, &[f32])> = starts.iter().map(|&s| (s, &sentence[..])).collect();
        let sentences = starts.iter().map(|&s| (s, s + sentence.len())).collect();
        (place(len, &parts), sentences)
    }

    fn ms(samples: usize) -> u64 {
        samples as u64 * 1000 / RATE as u64
    }

    #[test]
    fn a_stereo_archive_has_two_uncoupled_streams_and_no_leak_between_channels() {
        let rate = 48_000;
        let right = tone(rate, 6.0);
        let left = vec![0.0; right.len()];
        let a = assemble_stereo(rate, &left, &right);

        let archive = a.dir.join(&a.segment.audio_file);
        let mut packets = ogg::PacketReader::new(File::open(&archive).unwrap());
        let head = packets.read_packet_expected().unwrap().data;
        assert_eq!(&head[..8], b"OpusHead");
        assert_eq!(head[9], 2, "two channels");
        assert_eq!(u32::from_le_bytes(head[12..16].try_into().unwrap()), rate);
        assert_eq!(head[18], 1, "mapping family 1");
        assert_eq!(head[19], 2, "two streams");
        assert_eq!(head[20], 0, "no coupled streams");
        assert_eq!(
            &head[21..23],
            &[0, 1],
            "left is stream 0, right is stream 1"
        );
        assert_eq!(head.len(), 23);

        let mut reader = ArchiveReader::open(&archive).unwrap();
        assert_eq!(reader.channels(), 2);
        let mut pcm = Vec::new();
        while reader.read(&mut pcm).unwrap() {}
        let frames = pcm.len() / 2;
        assert!((frames as i64 - 6 * 48_000).abs() <= 960, "{frames} frames");
        let decoded_left: Vec<f32> = pcm.iter().step_by(2).copied().collect();
        let decoded_right: Vec<f32> = pcm.iter().skip(1).step_by(2).copied().collect();
        let leak = db(energy(&decoded_left) / energy(&decoded_right));
        assert!(leak < -40.0, "the right channel leaks at {leak:.1} dB");
        // The right channel keeps the tone at mono quality.
        let level = db(energy(&decoded_right) / energy(&right));
        assert!(level.abs() < 0.5, "the tone changed by {level:.2} dB");
    }

    #[test]
    fn with_headphones_no_echo_spans_and_the_mix_is_the_balanced_sum() {
        // Covers AE1.
        let len = secs(20.0);
        let room = voice(secs(8.0), 120.0, 3, 0.05);
        let call = voice(secs(10.0), 210.0, 9, 0.02);
        let mic = add(
            &place(len, &[(secs(1.0), &room), (secs(11.0), &room[..secs(5.0)])]),
            &noise(len, 1e-4),
        );
        let system = place(len, &[(secs(4.0), &call)]);
        let a = assemble_stereo(RATE as u32, &mic, &system);

        assert_eq!(a.echo.spans, [], "no echo spans");
        let (mic, system) = (stored(&mic), stored(&system));
        let (g_mic, g_system) = (a.echo.mic_gain, a.echo.system_gain);
        assert!(
            g_system > g_mic,
            "the quieter call is raised: {g_mic} {g_system}"
        );
        for (i, &got) in a.proc.iter().enumerate() {
            let want = g_mic * mic[i] + g_system * system[i];
            assert!(
                (got - want).abs() < 1e-5,
                "sample {i}: {got} against {want}"
            );
        }
        let scaled = |x: &[f32], g: f32| x.iter().map(|s| s * g).collect::<Vec<_>>();
        let diff = speech_level(&scaled(&mic, g_mic)) - speech_level(&scaled(&system, g_system));
        assert!(diff.abs() < 1.0, "levels differ by {diff:.2} dB");
    }

    type Call = (Vec<f32>, Vec<f32>, Vec<f32>, Vec<(usize, usize)>);

    /// The microphone hears the call from the speakers, late and quieter:
    /// the microphone, the system channel, the echo and the sentences.
    fn speaker_call(room: Option<Vec<f32>>) -> Call {
        let len = secs(24.0);
        let (system, sentences) = remote(len);
        let echo = delayed(&system, ECHO_DELAY, ECHO_GAIN);
        let mut mic = add(&echo, &noise(len, 1e-4));
        if let Some(room) = room {
            mic = add(&mic, &room);
        }
        (mic, system, echo, sentences)
    }

    /// The microphone part of the mix, from the mix and the system channel.
    fn mic_part(a: &Assembled, system: &[f32]) -> Vec<f32> {
        let system = stored(system);
        a.proc
            .iter()
            .zip(&system)
            .map(|(p, s)| (p - a.echo.system_gain * s) / a.echo.mic_gain)
            .collect()
    }

    #[test]
    fn speaker_echo_gives_echo_spans_over_the_echoed_speech_and_removes_the_echo() {
        // Covers AE2.
        let (mic, system, echo, sentences) = speaker_call(None);
        let a = assemble_stereo(RATE as u32, &mic, &system);

        assert_eq!(a.echo.spans.len(), sentences.len(), "{:?}", a.echo.spans);
        for (span, (start, end)) in a.echo.spans.iter().zip(&sentences) {
            let (start, end) = (ms(start + ECHO_DELAY), ms(end + ECHO_DELAY));
            assert!(
                span.start_ms.abs_diff(start) <= 100,
                "span {span:?} for the echo at {start}..{end} ms"
            );
            assert!(
                span.end_ms >= end - 100 && span.end_ms <= end + 600,
                "span {span:?} for the echo at {start}..{end} ms"
            );
        }
        let cleaned = mic_part(&a, &system);
        for (start, end) in &sentences {
            let (start, end) = (start + ECHO_DELAY, end + ECHO_DELAY);
            let loss = db(energy(&echo[start..end]) / energy(&cleaned[start..end]));
            assert!(loss >= 15.0, "the echo at {start} lost {loss:.1} dB");
        }
    }

    #[test]
    fn a_sentence_at_a_span_start_loses_its_echo_from_its_first_syllable() {
        let (mic, system, echo, sentences) = speaker_call(None);
        let a = assemble_stereo(RATE as u32, &mic, &system);
        let cleaned = mic_part(&a, &system);
        for (start, _) in &sentences {
            let start = start + ECHO_DELAY;
            let first = start..start + secs(0.3);
            let loss = db(energy(&echo[first.clone()]) / energy(&cleaned[first]));
            assert!(
                loss >= 15.0,
                "the first 300 ms at {start} lost {loss:.1} dB"
            );
        }
    }

    #[test]
    fn a_delay_that_changes_from_60_to_250_ms_is_followed() {
        // The output device changes after the first sentence.
        let len = secs(24.0);
        let (system, sentences) = remote(len);
        let early = delayed(&system[..secs(6.0)], secs(0.06), ECHO_GAIN);
        let mut echo = delayed(&system, ECHO_DELAY, ECHO_GAIN);
        echo[..secs(6.0)].copy_from_slice(&early);
        let mic = add(&echo, &noise(len, 1e-4));
        let a = assemble_stereo(RATE as u32, &mic, &system);

        assert_eq!(a.echo.spans.len(), 3, "{:?}", a.echo.spans);
        assert!(a.echo.spans[0].start_ms.abs_diff(ms(sentences[0].0) + 60) <= 100);
        let cleaned = mic_part(&a, &system);
        for (k, (start, end)) in sentences.iter().enumerate() {
            let delay = if k == 0 { secs(0.06) } else { ECHO_DELAY };
            let (start, end) = (start + delay, end + delay);
            let loss = db(energy(&echo[start..end]) / energy(&cleaned[start..end]));
            assert!(loss >= 15.0, "the echo at {start} lost {loss:.1} dB");
        }
    }

    /// The Assemble time of 30 minutes of a call on the speakers at 48 kHz,
    /// with echo in 80% of it. Run with `--release --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn thirty_minutes_of_a_call_on_the_speakers() {
        let rate = 48_000usize;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("s");
        // A 10 s pattern at 48 kHz: 8 s of remote speech, and its echo.
        let sentence = voice(rate * 8, 70.0, 7, 0.05);
        let system = place(rate * 10, &[(rate / 2, &sentence)]);
        let mic = add(
            &delayed(&system, rate / 4, ECHO_GAIN),
            &noise(rate * 10, 1e-4),
        );
        let block: Vec<f32> = mic
            .iter()
            .zip(&system)
            .flat_map(|(&m, &s)| [m, s])
            .collect();
        let mut writer = recorder::ChunkedWriter::with_probe(
            recorder::SegmentArgs {
                dir: dir.join(pipeline::RECOVERY_DIR),
                segment: 0,
                capture_started_at: 1_800_000_000_000,
                offset_ms: 0,
                sample_rate: rate as u32,
                channels: 2,
                floor_bytes: 0,
            },
            Box::new(|_| Some(u64::MAX)),
        )
        .unwrap();
        for _ in 0..180 {
            for frame in block.chunks(rate / 50 * 2) {
                writer.write(frame).unwrap();
            }
        }
        writer.finish().unwrap();
        let segment = Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 1_800_000,
            wall_start: at(0),
            wall_end: at(1_800),
            audio_file: "audio/segment-01.opus".into(),
            channels: 2,
        };
        let store = store();
        store
            .lock()
            .unwrap()
            .create_session("s", at(0), None)
            .unwrap();
        let started = std::time::Instant::now();
        assemble::run(
            &store,
            "s",
            &dir,
            std::slice::from_ref(&segment),
            None,
            &Unattended,
        )
        .unwrap();
        let echo = echo::saved(&store, "s", 0).unwrap().unwrap();
        let covered: u64 = echo.spans.iter().map(|s| s.end_ms - s.start_ms).sum();
        println!(
            "30 minutes assembled in {:.1} s; echo spans cover {:.0}%",
            started.elapsed().as_secs_f64(),
            covered as f64 / 18_000.0
        );
    }

    #[test]
    fn room_speech_over_speaker_echo_is_an_echo_span_and_keeps_the_room_speech() {
        let len = secs(24.0);
        // The room talks over the second remote sentence.
        let room = place(len, &[(secs(8.5), &voice(secs(4.0), 120.0, 3, 0.05))]);
        let (mic, system, echo, sentences) = speaker_call(Some(room.clone()));
        let a = assemble_stereo(RATE as u32, &mic, &system);

        let (start, end) = sentences[1];
        let (start, end) = (start + ECHO_DELAY, end + ECHO_DELAY);
        assert!(
            a.echo
                .spans
                .iter()
                .any(|s| s.start_ms <= ms(start) + 100 && s.end_ms >= ms(end)),
            "no echo span over the double talk: {:?}",
            a.echo.spans
        );
        let cleaned = mic_part(&a, &system);
        let overlap = start + secs(0.25)..end - secs(0.25);
        let loss = db(energy(&echo[overlap.clone()])
            / projected(&cleaned[overlap.clone()], &echo[overlap.clone()]));
        assert!(
            loss >= 10.0,
            "the echo under the room speech lost {loss:.1} dB"
        );
        let change = db(energy(&cleaned[overlap.clone()]) / energy(&room[overlap]));
        assert!(
            change.abs() < 3.0,
            "the room speech changed by {change:.2} dB"
        );
    }

    #[test]
    fn a_quiet_system_channel_is_raised_to_within_3_db_of_the_microphone() {
        let len = secs(18.0);
        let mic = place(len, &[(secs(2.0), &voice(secs(6.0), 120.0, 3, 0.05))]);
        // -20 dB against the microphone speech.
        let system = place(len, &[(secs(10.0), &voice(secs(6.0), 210.0, 9, 0.005))]);
        let a = assemble_stereo(RATE as u32, &mic, &system);

        let room = speech_level(&a.proc[secs(2.0)..secs(8.0)]);
        let call = speech_level(&a.proc[secs(10.0)..secs(16.0)]);
        assert!(
            (room - call).abs() <= 3.0,
            "the room is at {room:.1} dBFS and the call at {call:.1} dBFS"
        );
    }

    #[test]
    fn a_silent_system_channel_gives_a_mix_equal_to_the_microphone() {
        let len = secs(12.0);
        let mic = add(
            &place(len, &[(secs(1.0), &voice(secs(9.0), 120.0, 3, 0.05))]),
            &noise(len, 1e-4),
        );
        let a = assemble_stereo(RATE as u32, &mic, &vec![0.0; len]);
        assert_eq!(a.echo.spans, []);
        assert_eq!((a.echo.mic_gain, a.echo.system_gain), (1.0, 1.0));
        assert_eq!(a.proc, stored(&mic));
    }

    #[test]
    fn a_crash_after_the_echo_row_and_before_done_recomputes_and_keeps_one_row() {
        let (mic, system, _, _) = speaker_call(None);
        let a = assemble_stereo(RATE as u32, &mic, &system);
        // The echo row is saved; the record of the segment is not.
        a.store
            .lock()
            .unwrap()
            .delete_checkpoint("s", Stage::Assemble, &checkpoint::segment_key(0))
            .unwrap();
        assemble::run(
            &a.store,
            "s",
            &a.dir,
            std::slice::from_ref(&a.segment),
            None,
            &Unattended,
        )
        .unwrap();
        let rows = checkpoint::all(&a.store, "s", Stage::Assemble).unwrap();
        let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            [
                "audio-provenance-00000000",
                "done",
                "seg-0000",
                "seg-0000-echo"
            ]
        );
        assert!(
            assemble::provenance_valid(&a.store, "s", &a.dir, std::slice::from_ref(&a.segment))
                .unwrap()
        );
        let provenance: serde_json::Value =
            checkpoint::get(&a.store, "s", Stage::Assemble, "audio-provenance-00000000")
                .unwrap()
                .unwrap();
        assert_eq!(
            provenance["archive"],
            session_core::recognition::file_digest(&a.dir.join(&a.segment.audio_file)).unwrap()
        );
        assert_eq!(
            provenance["processing"],
            session_core::recognition::file_digest(&a.dir.join(proc_file(&a.segment.audio_file)))
                .unwrap()
        );
        assert_eq!(echo::saved(&a.store, "s", 0).unwrap().unwrap(), a.echo);
    }

    #[test]
    fn a_stereo_segment_without_its_echo_row_is_not_skipped_on_resume() {
        let (mic, system, _, _) = speaker_call(None);
        let a = assemble_stereo(RATE as u32, &mic, &system);
        a.store
            .lock()
            .unwrap()
            .delete_checkpoint("s", Stage::Assemble, &checkpoint::echo_key(0))
            .unwrap();
        let processing = a.dir.join(proc_file(&a.segment.audio_file));
        let before = std::fs::metadata(&processing).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assemble::run(
            &a.store,
            "s",
            &a.dir,
            std::slice::from_ref(&a.segment),
            None,
            &Unattended,
        )
        .unwrap();
        let after = std::fs::metadata(&processing).unwrap().modified().unwrap();
        assert_ne!(before, after, "the segment is assembled again");
        assert_eq!(echo::saved(&a.store, "s", 0).unwrap().unwrap(), a.echo);
    }

    #[test]
    fn a_mono_segment_writes_a_family_0_archive_and_no_echo_row() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("m");
        let segment = write_segment(&dir, 0, 0, 48_000, &tone(48_000, 3.0), false);
        let store = store();
        store
            .lock()
            .unwrap()
            .create_session("m", at(0), None)
            .unwrap();
        assemble::run(
            &store,
            "m",
            &dir,
            std::slice::from_ref(&segment),
            None,
            &Unattended,
        )
        .unwrap();
        let mut packets =
            ogg::PacketReader::new(File::open(dir.join(&segment.audio_file)).unwrap());
        let head = packets.read_packet_expected().unwrap().data;
        assert_eq!((head[9], head[18], head.len()), (1, 0, 19));
        assert_eq!(echo::saved(&store, "m", 0).unwrap(), None);
        let keys: Vec<String> = checkpoint::all(&store, "m", Stage::Assemble)
            .unwrap()
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(keys, ["audio-provenance-00000000", "done", "seg-0000"]);
        assert!(
            assemble::provenance_valid(&store, "m", &dir, std::slice::from_ref(&segment)).unwrap()
        );
        let provenance: serde_json::Value =
            checkpoint::get(&store, "m", Stage::Assemble, "audio-provenance-00000000")
                .unwrap()
                .unwrap();
        assert_eq!(
            provenance["archive"],
            session_core::recognition::file_digest(&dir.join(&segment.audio_file)).unwrap()
        );
        assert_eq!(
            provenance["processing"],
            session_core::recognition::file_digest(&dir.join(proc_file(&segment.audio_file)))
                .unwrap()
        );
        assert_eq!(
            provenance["echo"],
            session_core::recognition::digest(b"null")
        );
    }

    /// Move a completed session's archives to its folder, remove the working
    /// directory and set Assemble's checkpoints aside, as Transcribe Again
    /// does.
    fn complete(
        store: &SharedStore,
        dir: &std::path::Path,
        segments: &[Segment],
    ) -> std::path::PathBuf {
        let home = dir.with_file_name("folder");
        std::fs::create_dir_all(home.join("audio")).unwrap();
        for segment in segments {
            std::fs::rename(
                dir.join(&segment.audio_file),
                home.join(&segment.audio_file),
            )
            .unwrap();
        }
        std::fs::remove_dir_all(dir).unwrap();
        store
            .lock()
            .unwrap()
            .clear_checkpoints("s", Stage::Assemble)
            .unwrap();
        home
    }

    #[test]
    fn a_stereo_archive_decoded_again_gives_the_same_echo_spans_within_100_ms() {
        let (mic, system, _, _) = speaker_call(None);
        let a = assemble_stereo(RATE as u32, &mic, &system);
        let home = complete(&a.store, &a.dir, std::slice::from_ref(&a.segment));
        a.store
            .lock()
            .unwrap()
            .set_home_path("s", home.to_str().unwrap())
            .unwrap();
        let archive = std::fs::read(home.join(&a.segment.audio_file)).unwrap();

        let done = assemble::run(
            &a.store,
            "s",
            &a.dir,
            std::slice::from_ref(&a.segment),
            Some(&home),
            &Unattended,
        )
        .unwrap();
        assert!(
            assemble::provenance_valid(&a.store, "s", &a.dir, std::slice::from_ref(&a.segment))
                .unwrap()
        );
        assert!(
            !assemble::recognition_audio_identity(
                &a.store,
                "s",
                &a.dir,
                std::slice::from_ref(&a.segment)
            )
            .unwrap()
            .is_empty()
        );
        let again = echo::saved(&a.store, "s", 0)
            .unwrap()
            .expect("an echo spans row");
        assert_eq!(again.spans.len(), a.echo.spans.len(), "{:?}", again.spans);
        for (first, again) in a.echo.spans.iter().zip(&again.spans) {
            assert!(
                first.start_ms.abs_diff(again.start_ms) <= 100
                    && first.end_ms.abs_diff(again.end_ms) <= 100,
                "{first:?} against {again:?}"
            );
        }
        let (_, decoded) = read_proc(&a.dir.join(proc_file(&a.segment.audio_file)));
        assert_eq!(done[0].proc_samples, decoded.len() as u64);
        assert!((decoded.len() as i64 - a.proc.len() as i64).abs() <= 16);
        assert_eq!(
            std::fs::read(home.join(&a.segment.audio_file)).unwrap(),
            archive
        );
        assert!(
            !a.dir.join(&a.segment.audio_file).exists(),
            "no second archive"
        );
    }

    #[test]
    fn a_note_with_a_mono_and_a_stereo_segment_decodes_each_in_its_own_format() {
        let root = tempfile::tempdir().unwrap();
        let store = store();
        let (mic, system, _, _) = speaker_call(None);
        common::queue_recorded_session(
            &store,
            root.path(),
            "s",
            100,
            None,
            &[
                Recorded::Mono(48_000, tone(48_000, 5.0)),
                Recorded::Stereo(RATE as u32, mic, system),
            ],
        );
        let dir = root.path().join("s");
        let segments = store.lock().unwrap().segments("s").unwrap();
        assemble::run(&store, "s", &dir, &segments, None, &Unattended).unwrap();
        let first = echo::saved(&store, "s", 1).unwrap().unwrap();
        let home = complete(&store, &dir, &segments);

        let done = assemble::run(&store, "s", &dir, &segments, Some(&home), &Unattended).unwrap();
        assert_eq!(done.len(), 2);
        assert_eq!(
            echo::saved(&store, "s", 0).unwrap(),
            None,
            "mono has no echo row"
        );
        let again = echo::saved(&store, "s", 1).unwrap().unwrap();
        assert_eq!(again.spans.len(), first.spans.len());
        for segment in &segments {
            let (spec, pcm) = read_proc(&dir.join(proc_file(&segment.audio_file)));
            assert_eq!((spec.channels, spec.sample_rate), (1, PROC_RATE));
            let want = segment.duration_ms as i64 * 16;
            assert!(
                (pcm.len() as i64 - want).abs() <= 320,
                "{} samples",
                pcm.len()
            );
        }
    }

    #[test]
    fn the_disk_check_counts_the_temporary_stereo_file() {
        let store = store();
        let hour = 3_600u64;
        let segment = |channels| Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: hour * 1000,
            wall_start: at(0),
            wall_end: at(hour as i64),
            audio_file: "audio/segment-01.opus".into(),
            channels,
        };
        store
            .lock()
            .unwrap()
            .create_session("s", at(0), None)
            .unwrap();
        let reserve = recorder::storage::DISK_RESERVE_BYTES;
        let archive_and_mix = recorder::storage::processing_bytes_per_second(2) * hour;
        let temporary = 16_000 * 4 * 2 * hour;

        let check =
            |channels, free| assemble::check_space(&store, "s", &[segment(channels)], Some(free));
        let short = archive_and_mix + reserve + temporary / 2;
        let err = check(2, short).unwrap_err().to_string();
        assert!(err.contains("free some space"), "{err}");
        assert!(
            check(1, short).is_ok(),
            "a mono segment needs no temporary file"
        );
        assert!(check(2, archive_and_mix + temporary + reserve).is_ok());
    }
}
