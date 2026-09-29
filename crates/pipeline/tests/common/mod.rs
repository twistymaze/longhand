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
    record_session_with_prompt(store, root, id, stopped_at, model, segments, None);
}

/// [`queue_session`] for a note that recorded `prompt` as its active
/// summary prompt at the start (KTD15).
pub fn queue_session_with_prompt(
    store: &SharedStore,
    root: &Path,
    id: &str,
    stopped_at: i64,
    model: Option<&str>,
    segments: &[(u32, Vec<f32>)],
    prompt: &session_core::PromptSnapshot,
) {
    record_session_with_prompt(store, root, id, stopped_at, model, segments, Some(prompt));
    stop_session(store, id, stopped_at);
}

fn record_session_with_prompt(
    store: &SharedStore,
    root: &Path,
    id: &str,
    stopped_at: i64,
    model: Option<&str>,
    segments: &[(u32, Vec<f32>)],
    prompt: Option<&session_core::PromptSnapshot>,
) {
    let dir = root.join(id);
    let mut s = store.lock().unwrap();
    match prompt {
        None => s.create_session(id, at(stopped_at - 100), None).unwrap(),
        Some(prompt) => {
            let model = session_core::RecordingModel {
                asr_model: model.unwrap_or("m").into(),
                live_transcript: false,
            };
            s.start_session_with_recognition(
                id,
                at(stopped_at - 100),
                None,
                &model,
                &session_core::CaptureChoice::default(),
                None,
                Some(prompt),
            )
            .unwrap();
        }
    }
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

/// The audio of one segment for [`queue_recorded_session`].
pub enum Recorded {
    Mono(u32, Vec<f32>),
    /// The microphone on the left and system audio on the right.
    Stereo(u32, Vec<f32>, Vec<f32>),
}

/// [`queue_session`] for segments that can be stereo. The first stereo
/// segment sets the note's stereo mark, as capture does.
pub fn queue_recorded_session(
    store: &SharedStore,
    root: &Path,
    id: &str,
    stopped_at: i64,
    model: Option<&str>,
    segments: &[Recorded],
) {
    let dir = root.join(id);
    {
        let mut s = store.lock().unwrap();
        s.create_session(id, at(stopped_at - 100), None).unwrap();
        let mut offset = 0;
        for recorded in segments {
            let index = s.segments(id).unwrap().len() as u32;
            let segment = match recorded {
                Recorded::Mono(rate, audio) => {
                    write_segment(&dir, index, offset, *rate, audio, false)
                }
                Recorded::Stereo(rate, left, right) => {
                    write_stereo_segment(&dir, index, offset, *rate, left, right, false)
                }
            };
            offset = segment.end_ms() + 5_000;
            s.put_segment(id, &segment).unwrap();
            if segment.channels == 2 {
                s.mark_stereo_from(id, index).unwrap();
            }
        }
        if let Some(model) = model {
            s.set_asr_model(id, model).unwrap();
        }
    }
    stop_session(store, id, stopped_at);
}

/// A synthetic voice at 16 kHz: a harmonic tone with a moving pitch, cut
/// into syllables of about 4 Hz, with a small noise part. Its RMS is near
/// `level` over the syllables.
pub fn voice(len: usize, base_pitch_hz: f32, seed: u32, level: f32) -> Vec<f32> {
    let mut state = seed.max(1);
    let mut phase = 0.0f32;
    let rate = 16_000.0f32;
    let tau = std::f32::consts::TAU;
    (0..len)
        .map(|idx| {
            let t = idx as f32 / rate;
            let pitch = base_pitch_hz * (1.0 + 0.08 * (tau * 0.7 * t + seed as f32).sin());
            phase = (phase + tau * pitch / rate) % (tau * 1_000.0);
            let mut tone = 0.0;
            for harmonic in 1..=12 {
                tone += (phase * harmonic as f32).sin() / harmonic as f32;
            }
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let noise = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let syllable = (tau * 4.0 * t + seed as f32).sin();
            let envelope = syllable.max(0.0).powi(2);
            // The syllable envelope and the tone have an RMS near 0.61 x 0.9.
            level / 0.55 * envelope * (tone + 0.2 * noise)
        })
        .collect()
}

/// `len` samples of silence with each part added at its start sample.
pub fn place(len: usize, parts: &[(usize, &[f32])]) -> Vec<f32> {
    let mut out = vec![0.0; len];
    for (start, part) in parts {
        for (o, s) in out[*start..].iter_mut().zip(part.iter()) {
            *o += s;
        }
    }
    out
}

/// `input` late by `delay` samples and scaled by `gain`, as a speaker echo.
pub fn delayed(input: &[f32], delay: usize, gain: f32) -> Vec<f32> {
    let mut out = vec![0.0; input.len()];
    for i in delay..input.len() {
        out[i] = gain * input[i - delay];
    }
    out
}

pub fn add(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b).map(|(a, b)| a + b).collect()
}
