//! Calibration of the speech and quiet values of the long-silence cut (U13
//! of the forgotten-recording plan), on real recordings.
//!
//! The tests are ignored: they read private recordings that the repository
//! does not hold. Give the paths in environment variables, and run with
//! `--ignored --nocapture`. The output names each file by its position in
//! the list, never by its name.
//!
//! - `LONGHAND_CALIBRATION_WAV`: WAV files (mono, or stereo with the
//!   microphone on the left and the app audio on the right), separated by
//!   `:`. A file with an RTTM file of the same stem beside it gets a check
//!   against the reference speech of that file.
//! - `LONGHAND_TIMING_WAV`: one WAV file of a meeting. The timing test adds
//!   `LONGHAND_TIMING_QUIET_MINUTES` (default 160) of low room tone after it,
//!   and measures the cut and Preparing the audio.

mod common;

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use models::recognition::{RecognitionConfig, SpeakerSource, resolve};
use pipeline::cut::{ChunkInput, CutInput, CutSettings, SegmentInput, detect};
use pipeline::{RECOVERY_DIR, Unattended};
use recorder::speech::{
    Cell, ChannelCell, EarshotDetector, GuardFeed, GuardMeter, SPEECH_WINDOW_CELLS, SpeechWindow,
    chunk_cells,
};
use recorder::{ChunkedWriter, SegmentArgs};
use session_core::Segment;

const CELL_MS: u64 = recorder::speech::CELL_MS;

/// Write `wav` as segment 0 in `dir` through the real chunk writer with the
/// real speech detector, as capture does. Returns the channel count and the
/// duration in milliseconds.
fn record(wav: &Path, dir: &Path, extra_quiet_minutes: u64) -> (u16, u32, u64) {
    let mut reader = hound::WavReader::open(wav).unwrap();
    let spec = reader.spec();
    let mut writer = ChunkedWriter::with_probe(
        SegmentArgs {
            dir: dir.to_path_buf(),
            segment: 0,
            capture_started_at: 1_800_000_000_000,
            offset_ms: 0,
            sample_rate: spec.sample_rate,
            channels: spec.channels,
            floor_bytes: 0,
        },
        Box::new(|_| Some(u64::MAX)),
    )
    .unwrap()
    .with_speech(|rate| Box::new(EarshotDetector::new(rate)), |_| {});
    let block = spec.sample_rate as usize / 50 * usize::from(spec.channels);
    let mut buffer = Vec::with_capacity(block);
    let mut frames = 0u64;
    let scale = match spec.sample_format {
        hound::SampleFormat::Int => 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32,
        hound::SampleFormat::Float => 1.0,
    };
    let samples: Box<dyn Iterator<Item = f32>> = match spec.sample_format {
        hound::SampleFormat::Int => Box::new(
            reader
                .samples::<i32>()
                .map(move |s| s.unwrap() as f32 * scale),
        ),
        hound::SampleFormat::Float => Box::new(reader.samples::<f32>().map(Result::unwrap)),
    };
    for sample in samples {
        buffer.push(sample);
        if buffer.len() == block {
            writer.write(&buffer).unwrap();
            frames += (block / usize::from(spec.channels)) as u64;
            buffer.clear();
        }
    }
    let whole = buffer.len() - buffer.len() % usize::from(spec.channels);
    if whole > 0 {
        writer.write(&buffer[..whole]).unwrap();
        frames += (whole / usize::from(spec.channels)) as u64;
    }
    // Low room tone: noise near -62 dBFS RMS on the microphone, near -90
    // dBFS on the app audio.
    let mut state = 0x1234_5678u32;
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let quiet_frames = extra_quiet_minutes * 60 * u64::from(spec.sample_rate);
    let mut written = 0;
    let mut low = 0.0f32;
    while written < quiet_frames {
        buffer.clear();
        for _ in 0..block / usize::from(spec.channels) {
            // A one-pole low-pass filter makes the noise less bright.
            low = 0.9 * low + 0.1 * noise();
            buffer.push(low * 0.0055);
            if spec.channels == 2 {
                buffer.push(noise() * 0.00005);
            }
        }
        writer.write(&buffer).unwrap();
        written += (block / usize::from(spec.channels)) as u64;
    }
    frames += written;
    writer.finish().unwrap();
    (
        spec.channels,
        spec.sample_rate,
        frames * 1000 / u64::from(spec.sample_rate),
    )
}

/// The reference speech of an RTTM file, merged, in milliseconds.
fn reference(wav: &Path) -> Option<Vec<(u64, u64)>> {
    let text = std::fs::read_to_string(wav.with_extension("rttm")).ok()?;
    let mut turns: Vec<(u64, u64)> = text
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let start: f64 = fields.get(3)?.parse().ok()?;
            let length: f64 = fields.get(4)?.parse().ok()?;
            Some(((start * 1000.0) as u64, ((start + length) * 1000.0) as u64))
        })
        .collect();
    turns.sort();
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (start, end) in turns {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    Some(merged)
}

fn overlap(turns: &[(u64, u64)], start: u64, end: u64) -> u64 {
    turns
        .iter()
        .map(|&(s, e)| e.min(end).saturating_sub(s.max(start)))
        .sum()
}

fn segment_row(
    index: u32,
    offset_ms: u64,
    duration_ms: u64,
    channels: u16,
    pause: bool,
) -> Segment {
    Segment {
        index,
        offset_ms,
        duration_ms,
        wall_start: common::at(offset_ms as i64 / 1000),
        wall_end: common::at((offset_ms + duration_ms) as i64 / 1000),
        audio_file: format!("audio/segment-{:02}.opus", index + 1),
        channels,
        ends_in_pause: pause,
        cut_before_ms: 0,
        cut_after_ms: 0,
    }
}

const SETTINGS: CutSettings = CutSettings {
    cut_minutes: Some(10),
    silence_check_minutes: Some(30),
    stopped_by_silence_check: false,
};

/// Whether the cut can remove `chunk` by itself. The chunk is a segment
/// that ends in a Pause, so the stretch keeps no margin, and a later speech
/// segment stops the rule that keeps the first minute. The speech test then
/// sees only the cells of the chunk, which is stricter than in a note.
fn removable(chunk: &ChunkInput, channels: u16) -> bool {
    let speech = Cell {
        mic: ChannelCell {
            voiced: 255,
            level: 20,
        },
        app: (channels == 2).then_some(ChannelCell {
            voiced: 255,
            level: 20,
        }),
    };
    let length = chunk.end_ms - chunk.start_ms;
    let input = CutInput {
        segments: vec![
            SegmentInput {
                row: segment_row(0, chunk.start_ms, length, channels, true),
                cut_checked: false,
                chunks: vec![chunk.clone()],
            },
            SegmentInput {
                row: segment_row(1, chunk.end_ms + 1_000, 60_000, channels, false),
                cut_checked: false,
                chunks: vec![ChunkInput {
                    start_ms: chunk.end_ms + 1_000,
                    end_ms: chunk.end_ms + 61_000,
                    cells: vec![Some(speech); 240],
                }],
            },
        ],
        recording_start_ms: 0,
        live_words: Vec::new(),
        settings: CutSettings {
            cut_minutes: Some(1),
            ..SETTINGS
        },
    };
    !detect(&input).unwrap().removed.is_empty()
}

fn percentile(values: &mut [f32], p: f32) -> f32 {
    if values.is_empty() {
        return f32::NAN;
    }
    values.sort_by(f32::total_cmp);
    let rank = (p * values.len() as f32).ceil() as usize;
    values[rank.clamp(1, values.len()) - 1]
}

fn levels(cells: &[&Cell], app: bool) -> Vec<f32> {
    cells
        .iter()
        .filter_map(|c| if app { c.app } else { Some(c.mic) })
        .map(|c| c.level_dbfs().max(-120.0))
        .collect()
}

fn summary(values: &mut [f32]) -> String {
    format!(
        "p50 {:.0} / p90 {:.0} / p95 {:.0} / max {:.0} dBFS",
        percentile(values, 0.5),
        percentile(values, 0.9),
        percentile(values, 0.95),
        percentile(values, 1.0)
    )
}

#[test]
#[ignore = "reads private recordings; set LONGHAND_CALIBRATION_WAV"]
fn calibrate_on_real_recordings() {
    let paths = std::env::var("LONGHAND_CALIBRATION_WAV").expect("LONGHAND_CALIBRATION_WAV");
    let mut failures = Vec::new();
    for (number, path) in paths.split(':').map(PathBuf::from).enumerate() {
        let temp = tempfile::tempdir().unwrap();
        let (channels, rate, duration_ms) = record(&path, temp.path(), 0);
        let chunks: Vec<ChunkInput> = recorder::list_chunks(temp.path())
            .unwrap()
            .iter()
            .map(|chunk| ChunkInput {
                start_ms: chunk.start_ms,
                end_ms: chunk.end_ms,
                cells: chunk_cells(chunk),
            })
            .collect();
        let cells: Vec<Cell> = chunks
            .iter()
            .flat_map(|c| c.cells.iter().flatten().copied())
            .collect();
        let reference = reference(&path);
        println!(
            "\n== file {number}: {channels} ch, {rate} Hz, {:.1} min, {} chunks",
            duration_ms as f64 / 60_000.0,
            chunks.len()
        );

        // The speech cells of each channel (KTD3), as the cut finds them.
        let mut speech_at: Vec<bool> = vec![false; cells.len()];
        for app in [false, true] {
            let mut window = SpeechWindow::default();
            let mut recent: VecDeque<(usize, bool)> = VecDeque::new();
            for (i, cell) in cells.iter().enumerate() {
                let Some(channel) = (if app { cell.app } else { Some(cell.mic) }) else {
                    continue;
                };
                let voiced = channel.is_voiced();
                recent.push_back((i, voiced));
                if recent.len() > SPEECH_WINDOW_CELLS as usize {
                    recent.pop_front();
                }
                if window.push(voiced) {
                    for &(j, v) in &recent {
                        speech_at[j] |= v;
                    }
                }
            }
        }

        // The silence check: the longest time with no speech.
        let meter = Arc::new(GuardMeter::default());
        let mut feed = GuardFeed::new(meter.clone());
        let mut longest_silence = 0;
        for cell in &cells {
            feed.cell(cell);
            longest_silence = longest_silence.max(meter.silence_ms());
        }
        println!(
            "silence check: longest time with no speech {:.1} min",
            longest_silence as f64 / 60_000.0
        );

        // Each full chunk by itself.
        let full: Vec<&ChunkInput> = chunks
            .iter()
            .filter(|c| c.end_ms - c.start_ms >= 60_000)
            .collect();
        let removable_chunks: Vec<&ChunkInput> = full
            .iter()
            .copied()
            .filter(|c| removable(c, channels))
            .collect();
        let mut chunk_p95_mic: Vec<f32> = Vec::new();
        for chunk in &full {
            let cells: Vec<&Cell> = chunk.cells.iter().flatten().collect();
            chunk_p95_mic.push(percentile(&mut levels(&cells, false), 0.95));
        }
        println!(
            "full chunks {}, removable by themselves {}; chunk p95 microphone levels: {}",
            full.len(),
            removable_chunks.len(),
            summary(&mut chunk_p95_mic.clone())
        );
        if channels == 2 {
            let mut app_p95: Vec<f32> = full
                .iter()
                .map(|chunk| {
                    let cells: Vec<&Cell> = chunk.cells.iter().flatten().collect();
                    percentile(&mut levels(&cells, true), 0.95)
                })
                .collect();
            println!("chunk p95 app levels: {}", summary(&mut app_p95));
        }

        // The cut on the whole file with the default settings.
        let whole = CutInput {
            segments: vec![SegmentInput {
                row: segment_row(0, 0, duration_ms, channels, false),
                cut_checked: false,
                chunks: chunks.clone(),
            }],
            recording_start_ms: 0,
            live_words: Vec::new(),
            settings: SETTINGS,
        };
        let plan = detect(&whole).unwrap();
        let removed_ms: u64 = plan.removed.iter().map(|c| c.duration_ms()).sum();
        println!(
            "cut with 10 min: {} chunks removed, {:.1} min",
            plan.removed.len(),
            removed_ms as f64 / 60_000.0
        );

        let Some(turns) = reference else {
            continue;
        };
        let speech_total: u64 = turns.iter().map(|t| t.1 - t.0).sum();
        // Chunks with reference speech.
        let mut speech_chunk_p95: Vec<f32> = Vec::new();
        let mut silent_chunk_p95: Vec<f32> = Vec::new();
        for (chunk, p95) in full.iter().zip(&chunk_p95_mic) {
            if overlap(&turns, chunk.start_ms, chunk.end_ms) > 0 {
                speech_chunk_p95.push(*p95);
            } else {
                silent_chunk_p95.push(*p95);
            }
        }
        println!(
            "reference speech {:.1} min in {} turns; chunks with reference speech {}: p95 mic {}",
            speech_total as f64 / 60_000.0,
            turns.len(),
            speech_chunk_p95.len(),
            summary(&mut speech_chunk_p95)
        );
        println!(
            "chunks with no reference speech: {}",
            silent_chunk_p95.len()
        );
        for chunk in &removable_chunks {
            let speech_ms = overlap(&turns, chunk.start_ms, chunk.end_ms);
            if speech_ms > 0 {
                failures.push(format!(
                    "file {number}: the chunk at {} min has {speech_ms} ms of reference speech and is removable",
                    chunk.start_ms / 60_000
                ));
            }
        }
        for chunk in &plan.removed {
            let speech_ms = overlap(&turns, chunk.start_ms, chunk.end_ms);
            if speech_ms > 0 {
                failures.push(format!(
                    "file {number}: the cut removes the chunk at {} min with {speech_ms} ms of reference speech",
                    chunk.start_ms / 60_000
                ));
            }
        }

        // The detector against the reference: the turns of at least 1 s
        // with a speech cell, and the speech cells far from reference
        // speech.
        let long_turns: Vec<&(u64, u64)> = turns.iter().filter(|t| t.1 - t.0 >= 1_000).collect();
        let found = long_turns
            .iter()
            .filter(|t| {
                let first = (t.0 / CELL_MS) as usize;
                let last = ((t.1 / CELL_MS) as usize).min(speech_at.len());
                (first..last).any(|i| speech_at[i])
            })
            .count();
        let padded: Vec<(u64, u64)> = turns
            .iter()
            .map(|t| (t.0.saturating_sub(1_000), t.1 + 1_000))
            .collect();
        let far_speech = speech_at
            .iter()
            .enumerate()
            .filter(|&(i, &s)| {
                let at = i as u64 * CELL_MS;
                s && overlap(&padded, at, at + CELL_MS) == 0
            })
            .count();
        println!(
            "detector: {found} of {} turns of at least 1 s have a speech cell; {} speech cells ({:.1} s) are more than 1 s from reference speech",
            long_turns.len(),
            far_speech,
            far_speech as f64 * CELL_MS as f64 / 1000.0
        );
        // The cells in reference gaps of at least 3 s, 0.5 s from speech.
        let mut gap_cells: Vec<&Cell> = Vec::new();
        let mut gap_ms = 0;
        let mut previous_end = 0;
        let mut gaps: BTreeSet<(u64, u64)> = BTreeSet::new();
        for &(start, end) in turns.iter().chain([(duration_ms, duration_ms)].iter()) {
            if start > previous_end + 3_000 {
                gaps.insert((previous_end + 500, start - 500));
                gap_ms += start - previous_end;
            }
            previous_end = previous_end.max(end);
        }
        for (i, cell) in cells.iter().enumerate() {
            let at = i as u64 * CELL_MS;
            if gaps.iter().any(|&(s, e)| at >= s && at + CELL_MS <= e) {
                gap_cells.push(cell);
            }
        }
        println!(
            "reference gaps of at least 3 s: {}, {:.1} min; cell levels in them: mic {}",
            gaps.len(),
            gap_ms as f64 / 60_000.0,
            summary(&mut levels(&gap_cells, false))
        );
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

const VARIANTS: [(f32, u32); 6] = [
    (0.5, 4),
    (0.5, 3),
    (0.375, 4),
    (0.375, 3),
    (0.25, 4),
    (0.25, 2),
];

/// Whether the microphone cells of `chunk` hold speech with another voiced
/// fraction and another count of voiced cells in 8.
fn speech_with(chunk: &ChunkInput, fraction: f32, needed: u32) -> bool {
    let mut recent = 0u32;
    chunk.cells.iter().flatten().any(|cell| {
        let voiced = cell.mic.voiced_fraction() >= fraction;
        recent = ((recent << 1) | u32::from(voiced)) & 0xff;
        recent.count_ones() >= needed
    })
}

fn read_mono(wav: &Path) -> Option<Vec<f32>> {
    let mut reader = hound::WavReader::open(wav).ok()?;
    let spec = reader.spec();
    (spec.channels == 1
        && spec.sample_rate == 16_000
        && spec.sample_format == hound::SampleFormat::Int
        && spec.bits_per_sample == 16)
        .then(|| {
            reader
                .samples::<i16>()
                .map(|s| f32::from(s.unwrap()) / 32_768.0)
                .collect()
        })
}

fn rms_dbfs(samples: &[f32]) -> f32 {
    let square: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (10.0 * (square / samples.len().max(1) as f64).log10()) as f32
}

/// Each short reference turn of a mono 16-kHz file, alone in one chunk of
/// the room tone of the same file, at its own level and 10 dB and 20 dB
/// lower: the quiet and far speech of the Success Criteria. Also chunks of
/// only room tone, for the silence check and the quiet test.
#[test]
#[ignore = "reads private recordings; set LONGHAND_CALIBRATION_WAV"]
fn a_short_turn_alone_in_room_tone() {
    let paths = std::env::var("LONGHAND_CALIBRATION_WAV").expect("LONGHAND_CALIBRATION_WAV");
    const RATE: usize = 16_000;
    const CHUNK: usize = 60 * RATE;
    const GAINS_DB: [f32; 3] = [0.0, -10.0, -20.0];
    const TURNS: usize = 40;
    const TONE_CHUNKS: usize = 10;
    let mut removed_speech = 0;
    for (number, path) in paths.split(':').map(PathBuf::from).enumerate() {
        let (Some(audio), Some(turns)) = (read_mono(&path), reference(&path)) else {
            continue;
        };
        // The room tone: the reference gaps of at least 3 s, 0.5 s from
        // speech.
        let mut tone: Vec<f32> = Vec::new();
        let mut previous_end = 0u64;
        for &(start, end) in &turns {
            if start > previous_end + 3_000 {
                let from = ((previous_end + 500) as usize * RATE / 1000).min(audio.len());
                let to = ((start - 500) as usize * RATE / 1000).min(audio.len());
                tone.extend_from_slice(&audio[from..to]);
            }
            previous_end = previous_end.max(end);
        }
        if tone.len() < 10 * RATE {
            continue;
        }
        let short: Vec<(u64, u64)> = turns
            .iter()
            .copied()
            .filter(|t| (400..=6_000).contains(&(t.1 - t.0)))
            .collect();
        let step = (short.len() / TURNS).max(1);
        let chosen: Vec<(u64, u64)> = short.into_iter().step_by(step).take(TURNS).collect();

        // The chunks: room tone only, then each turn at each gain.
        let mut items: Vec<Option<(f32, u64, f32)>> = vec![None; TONE_CHUNKS];
        for gain in GAINS_DB {
            for turn in &chosen {
                let from = turn.0 as usize * RATE / 1000;
                let to = (turn.1 as usize * RATE / 1000).min(audio.len());
                let level = rms_dbfs(&audio[from..to]) + gain;
                items.push(Some((gain, turn.1 - turn.0, level)));
            }
        }
        let mut tone_at = 0usize;
        let mut samples: Vec<f32> = Vec::with_capacity(items.len() * CHUNK);
        let mut turn_index = 0;
        for item in &items {
            let start = samples.len();
            for _ in 0..CHUNK {
                samples.push(tone[tone_at]);
                tone_at = (tone_at + 1) % tone.len();
            }
            if let Some((gain, _, _)) = item {
                let turn = chosen[turn_index % chosen.len()];
                turn_index += 1;
                let from = turn.0 as usize * RATE / 1000;
                let to = (turn.1 as usize * RATE / 1000).min(audio.len());
                let scale = 10f32.powf(gain / 20.0);
                for (k, &s) in audio[from..to].iter().enumerate() {
                    samples[start + 30 * RATE + k] += s * scale;
                }
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let wav = temp.path().join("stress.wav");
        let mut writer = hound::WavWriter::create(
            &wav,
            hound::WavSpec {
                channels: 1,
                sample_rate: RATE as u32,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for s in &samples {
            writer.write_sample(*s).unwrap();
        }
        writer.finalize().unwrap();
        drop(samples);
        let recovery = temp.path().join("recovery");
        record(&wav, &recovery, 0);
        let chunks: Vec<ChunkInput> = recorder::list_chunks(&recovery)
            .unwrap()
            .iter()
            .map(|chunk| ChunkInput {
                start_ms: chunk.start_ms,
                end_ms: chunk.end_ms,
                cells: chunk_cells(chunk),
            })
            .collect();
        assert_eq!(chunks.len(), items.len());

        println!(
            "\n== file {number}: room tone {:.1} min at {:.0} dBFS RMS; {} turns of 0.4 to 6 s",
            tone.len() as f64 / RATE as f64 / 60.0,
            rms_dbfs(&tone),
            chosen.len()
        );
        let mut tone_p95 = Vec::new();
        let mut tone_speech = 0;
        let mut tone_removable = 0;
        for (chunk, item) in chunks.iter().zip(&items) {
            let cells: Vec<&Cell> = chunk.cells.iter().flatten().collect();
            let mut window = SpeechWindow::default();
            let speech = cells.iter().any(|c| window.push(c.mic.is_voiced()));
            let p95 = percentile(&mut levels(&cells, false), 0.95);
            if item.is_none() {
                tone_p95.push(p95);
                tone_speech += usize::from(speech);
                tone_removable += usize::from(removable(chunk, 1));
            }
        }
        println!(
            "room tone chunks {TONE_CHUNKS}: chunk p95 {}; with speech {tone_speech}; removable {tone_removable}",
            summary(&mut tone_p95)
        );
        // Other speech values: (voiced fraction, voiced cells in 8). The
        // chunks with speech, for the room tone and for each gain.
        for (fraction, needed) in VARIANTS {
            let mut counts = Vec::new();
            for wanted in [None, Some(0.0), Some(-10.0), Some(-20.0)] {
                let count = chunks
                    .iter()
                    .zip(&items)
                    .filter(|(_, item)| item.map(|i| i.0) == wanted)
                    .filter(|(chunk, _)| speech_with(chunk, fraction, needed))
                    .count();
                counts.push(count);
            }
            println!(
                "  fraction {fraction:.3}, {needed} of 8: speech in tone {} / 0 dB {} / -10 dB {} / -20 dB {}",
                counts[0], counts[1], counts[2], counts[3]
            );
        }
        for gain in GAINS_DB {
            let mut count = 0;
            let mut with_speech = 0;
            let mut quiet = 0;
            let mut removable_turns: Vec<String> = Vec::new();
            let mut turn_levels = Vec::new();
            for (chunk, item) in chunks.iter().zip(&items) {
                let Some((g, length, level)) = item else {
                    continue;
                };
                if *g != gain {
                    continue;
                }
                count += 1;
                turn_levels.push(*level);
                let cells: Vec<&Cell> = chunk.cells.iter().flatten().collect();
                let mut window = SpeechWindow::default();
                with_speech += usize::from(cells.iter().any(|c| window.push(c.mic.is_voiced())));
                let p95 = percentile(&mut levels(&cells, false), 0.95);
                quiet += usize::from(p95 <= recorder::speech::QUIET_LEVEL_DBFS);
                if removable(chunk, 1) {
                    removable_turns.push(format!(
                        "{length} ms at {level:.0} dBFS, chunk p95 {p95:.0}"
                    ));
                }
            }
            println!(
                "gain {gain:>3} dB: {count} chunks, turn RMS {}; with speech {with_speech}; quiet {quiet}; removable {}: {:?}",
                summary(&mut turn_levels),
                removable_turns.len(),
                removable_turns
            );
            removed_speech += removable_turns.len();
        }
    }
    println!("\nremovable chunks with a reference turn: {removed_speech}");
}

#[test]
#[ignore = "reads a private recording; set LONGHAND_TIMING_WAV"]
fn time_the_cut_and_preparing_of_a_long_quiet_note() {
    const ID: &str = "timing";
    let wav = PathBuf::from(std::env::var("LONGHAND_TIMING_WAV").expect("LONGHAND_TIMING_WAV"));
    let quiet_minutes: u64 = std::env::var("LONGHAND_TIMING_QUIET_MINUTES")
        .ok()
        .and_then(|m| m.parse().ok())
        .unwrap_or(160);
    let cut = std::env::var("LONGHAND_TIMING_NO_CUT").is_err();
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join(ID);
    let store = common::store();
    let started = Instant::now();
    let (channels, rate, duration_ms) = record(&wav, &dir.join(RECOVERY_DIR), quiet_minutes);
    println!(
        "note: {channels} ch, {rate} Hz, {:.1} min ({quiet_minutes} min room tone), written in {:.1} s",
        duration_ms as f64 / 60_000.0,
        started.elapsed().as_secs_f64()
    );
    let model = "whisper-large-v3-turbo-q5km";
    let spec = resolve(&RecognitionConfig {
        asr_model: model.into(),
        speaker_source: SpeakerSource::Off,
        ..Default::default()
    })
    .unwrap();
    {
        let mut s = store.lock().unwrap();
        s.create_session(ID, common::at(-100), None).unwrap();
        s.put_segment(ID, &segment_row(0, 0, duration_ms, channels, false))
            .unwrap();
        if channels == 2 {
            s.mark_stereo_from(ID, 0).unwrap();
        }
        s.set_asr_model(ID, model).unwrap();
        if cut {
            s.set_recording_guards(
                ID,
                Some(r#"{"silence_check_minutes":30,"time_limit_hours":6,"cut_minutes":10,"countdown_minutes":2}"#),
            )
            .unwrap();
        }
        s.set_recognition_snapshot(ID, &serde_json::to_value(&spec).unwrap())
            .unwrap();
    }
    common::stop_session(&store, ID, duration_ms as i64 / 1000);

    let stop = Instant::now();
    pipeline::cut::run(&store, ID, &dir, &Unattended).unwrap();
    let cut_s = stop.elapsed().as_secs_f64();
    let segments = store.lock().unwrap().segments(ID).unwrap();
    let assemble = Instant::now();
    pipeline::assemble::run(&store, ID, &dir, &segments, None, &Unattended).unwrap();
    let assemble_s = assemble.elapsed().as_secs_f64();
    let kept_ms: u64 = segments.iter().map(|s| s.duration_ms).sum();
    println!(
        "cut {}: {:.2} s; Preparing the audio: {:.1} s; Stop to assembled: {:.1} s; kept {:.1} min in {} segments",
        if cut { "on" } else { "off" },
        cut_s,
        assemble_s,
        stop.elapsed().as_secs_f64(),
        kept_ms as f64 / 60_000.0,
        segments.len()
    );
}
