//! The speech and level timeline of capture (U3 of the forgotten-recording
//! plan): the cells, the sidecars beside the chunks, and the note's
//! counters.

use std::path::Path;
use std::sync::Arc;

use recorder::speech::{
    CELL_MS, CELLS_PER_CHUNK, Cell, EarshotDetector, GuardFeed, GuardMeter, QUIET_LEVEL_DBFS,
    VoiceDetector, chunk_cells, read_sidecar, sidecar_of,
};
use recorder::storage::DISK_RESERVE_BYTES;
use recorder::{ChunkedWriter, SegmentArgs, list_chunks, recover_partial_chunks, remove_segment};

const GB: u64 = 1 << 30;
const CAPTURE: u64 = 1_700_000_000_000;

/// A detector for the tests: a frame of 16 ms is voiced when its peak is
/// above 0.1.
struct Loud {
    frame: usize,
    buffer: Vec<f32>,
}

impl VoiceDetector for Loud {
    fn push(&mut self, samples: &[f32], verdict: &mut dyn FnMut(bool)) {
        for &sample in samples {
            self.buffer.push(sample);
            if self.buffer.len() == self.frame {
                verdict(self.buffer.iter().any(|s| s.abs() > 0.1));
                self.buffer.clear();
            }
        }
    }
}

fn loud(rate: u32) -> Box<dyn VoiceDetector> {
    Box::new(Loud {
        frame: (rate as usize * 16 / 1000).max(1),
        buffer: Vec::new(),
    })
}

fn earshot(rate: u32) -> Box<dyn VoiceDetector> {
    Box::new(EarshotDetector::new(rate))
}

fn writer(dir: &Path, segment: u32, offset_ms: u64, rate: u32, channels: u16) -> ChunkedWriter {
    ChunkedWriter::with_probe(
        SegmentArgs {
            dir: dir.to_path_buf(),
            segment,
            capture_started_at: CAPTURE,
            offset_ms,
            sample_rate: rate,
            channels,
            floor_bytes: DISK_RESERVE_BYTES,
        },
        Box::new(|_| Some(100 * GB)),
    )
    .unwrap()
}

/// A writer whose cells go to `meter`.
fn metered(
    dir: &Path,
    segment: u32,
    rate: u32,
    channels: u16,
    detector: fn(u32) -> Box<dyn VoiceDetector>,
    meter: &Arc<GuardMeter>,
) -> ChunkedWriter {
    let mut feed = GuardFeed::new(Arc::clone(meter));
    writer(dir, segment, 0, rate, channels).with_speech(detector, move |cell| feed.cell(cell))
}

/// Write in frames of 20 ms, as capture does.
fn feed(writer: &mut ChunkedWriter, samples: &[f32]) {
    let frame = writer.sample_rate() as usize / 50 * usize::from(writer.channels());
    for part in samples.chunks(frame) {
        writer.write(part).unwrap();
    }
}

fn silence(rate: u32, ms: u64) -> Vec<f32> {
    vec![0.0; (u64::from(rate) * ms / 1000) as usize]
}

fn tone(rate: u32, ms: u64, amplitude: f32) -> Vec<f32> {
    (0..u64::from(rate) * ms / 1000)
        .map(|i| amplitude * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin())
        .collect()
}

/// Mono cells in turn: loud (`true`) or silent, 250 ms each.
fn cells_of(rate: u32, pattern: &[bool]) -> Vec<f32> {
    pattern
        .iter()
        .flat_map(|&on| {
            if on {
                tone(rate, CELL_MS, 0.5)
            } else {
                silence(rate, CELL_MS)
            }
        })
        .collect()
}

/// Two channels as interleaved frames.
fn stereo(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter().zip(right).flat_map(|(&l, &r)| [l, r]).collect()
}

/// White noise with an RMS level of `dbfs`, from a fixed seed.
fn noise(rate: u32, ms: u64, dbfs: f32) -> Vec<f32> {
    let rms = 10f32.powf(dbfs / 20.0);
    // A uniform sample in -1..1 has an RMS of 1/sqrt(3).
    let scale = rms * 3f32.sqrt();
    let mut state: u32 = 0x1234_5678;
    (0..u64::from(rate) * ms / 1000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / u32::MAX as f32 * 2.0 - 1.0) * scale
        })
        .collect()
}

#[test]
fn one_cell_of_sound_is_not_speech() {
    let dir = tempfile::tempdir().unwrap();
    let meter = Arc::new(GuardMeter::default());
    let mut w = metered(dir.path(), 0, 16_000, 1, loud, &meter);
    feed(&mut w, &silence(16_000, 10_000));
    // A door or a cough: 250 ms.
    feed(&mut w, &cells_of(16_000, &[true]));
    feed(&mut w, &silence(16_000, 10_000));
    w.finish().unwrap();
    assert_eq!(meter.silence_ms(), 20_250);
    assert_eq!(meter.run_ms(), 20_250);
}

#[test]
fn one_second_of_voiced_cells_within_two_seconds_is_speech() {
    let dir = tempfile::tempdir().unwrap();
    let meter = Arc::new(GuardMeter::default());
    let mut w = metered(dir.path(), 0, 16_000, 1, loud, &meter);
    feed(&mut w, &silence(16_000, 10_000));
    // The fourth voiced cell is the seventh cell: speech.
    feed(
        &mut w,
        &cells_of(16_000, &[true, false, true, false, true, false, true]),
    );
    feed(&mut w, &silence(16_000, 2_000));
    w.finish().unwrap();
    // The window holds 4 voiced cells until the second silent cell after
    // the pattern. The 7 silent cells after that count.
    assert_eq!(meter.silence_ms(), 7 * CELL_MS);
    assert_eq!(meter.run_ms(), 10_000 + 7 * CELL_MS + 2_000);
}

#[test]
fn speech_on_the_app_channel_only_resets_the_silence_time() {
    let dir = tempfile::tempdir().unwrap();
    let meter = Arc::new(GuardMeter::default());
    let mut w = metered(dir.path(), 0, 16_000, 2, loud, &meter);
    let left = silence(16_000, 10_000 + 2_000 + 2_000);
    let right: Vec<f32> = [
        silence(16_000, 10_000),
        tone(16_000, 2_000, 0.5),
        silence(16_000, 2_000),
    ]
    .concat();
    feed(&mut w, &stereo(&left, &right));
    w.finish().unwrap();
    // 8 voiced cells: the window holds speech for 4 silent cells more. The
    // last 4 silent cells count.
    assert_eq!(meter.silence_ms(), 4 * CELL_MS);
    assert_eq!(meter.run_ms(), 14_000);
}

#[test]
fn a_new_segment_keeps_the_silence_time() {
    // A microphone change closes a segment and opens the next one. The
    // note's meter goes on.
    let dir = tempfile::tempdir().unwrap();
    let meter = Arc::new(GuardMeter::default());
    let mut first = metered(dir.path(), 0, 48_000, 1, loud, &meter);
    feed(&mut first, &silence(48_000, 3_000));
    first.finish().unwrap();
    let mut second = metered(dir.path(), 1, 16_000, 2, loud, &meter);
    feed(&mut second, &silence(16_000, 2 * 2_000));
    second.finish().unwrap();
    assert_eq!(meter.silence_ms(), 5_000);
    assert_eq!(meter.run_ms(), 5_000);
}

#[test]
fn the_sidecars_cover_new_audio_only() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 44_100;
    let mut w = writer(dir.path(), 2, 120_000, rate, 1).with_speech(loud, |_| {});
    // The last 2 s of the first chunk are loud, so the lookback of the
    // second chunk is loud.
    feed(&mut w, &silence(rate, 58_000));
    feed(&mut w, &tone(rate, 2_000, 0.5));
    feed(&mut w, &silence(rate, 1_000));
    w.finish().unwrap();

    let chunks = list_chunks(dir.path()).unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(
        sidecar_of(&chunks[0])
            .file_name()
            .unwrap()
            .to_string_lossy(),
        format!("2-{CAPTURE}-120000.speech")
    );
    let first = read_sidecar(&sidecar_of(&chunks[0])).unwrap().unwrap();
    assert_eq!(first.channels, 1);
    assert_eq!(first.cells.len() as u64, CELLS_PER_CHUNK);
    let levels: Vec<f32> = first.cells.iter().map(|c| c.mic.level_dbfs()).collect();
    assert!(levels[..232].iter().all(|&l| l == f32::NEG_INFINITY));
    // A sine of amplitude 0.5 has an RMS level of -9 dBFS.
    assert!(levels[232..].iter().all(|&l| l == -9.0), "{levels:?}");

    // The second chunk starts with 2 s of loud lookback. Its cells hold
    // only the 1 s of new, silent audio.
    let second = chunk_cells(&chunks[1]);
    assert_eq!(second.len(), 4);
    assert!(
        second
            .iter()
            .all(|c| c.unwrap().mic.level_dbfs() == f32::NEG_INFINITY)
    );
}

#[test]
fn a_crash_leaves_no_sidecar_for_the_open_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = writer(dir.path(), 0, 0, 16_000, 1).with_speech(loud, |_| {});
    feed(&mut w, &silence(16_000, 70_000));
    // A crash: no finish. The drop waits for the sync of the first chunk.
    drop(w);
    recover_partial_chunks(dir.path()).unwrap();
    let chunks = list_chunks(dir.path()).unwrap();
    assert_eq!(chunks.len(), 2);
    let first = chunk_cells(&chunks[0]);
    assert_eq!(first.len() as u64, CELLS_PER_CHUNK);
    assert!(first.iter().all(Option::is_some));
    assert!(!sidecar_of(&chunks[1]).exists());
    let open = chunk_cells(&chunks[1]);
    assert_eq!(
        open.len(),
        (chunks[1].end_ms - 60_000).div_ceil(CELL_MS) as usize
    );
    assert!(!open.is_empty());
    assert!(open.iter().all(Option::is_none));
}

#[test]
fn a_chunk_with_a_short_sidecar_has_cells_with_no_data() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = writer(dir.path(), 0, 0, 16_000, 1).with_speech(loud, |_| {});
    feed(&mut w, &silence(16_000, 5_000));
    w.finish().unwrap();
    let chunks = list_chunks(dir.path()).unwrap();
    let path = sidecar_of(&chunks[0]);
    let bytes = std::fs::read(&path).unwrap();
    // Keep the header and 3 cells.
    std::fs::write(&path, &bytes[..2 + 3 * 2]).unwrap();
    let cells = chunk_cells(&chunks[0]);
    assert_eq!(cells.len(), 20);
    assert!(cells[..3].iter().all(Option::is_some));
    assert!(cells[3..].iter().all(Option::is_none));
    // A sidecar that cannot be read gives no data.
    std::fs::write(&path, [0u8; 3]).unwrap();
    assert!(chunk_cells(&chunks[0]).iter().all(Option::is_none));
}

#[test]
fn remove_segment_removes_its_sidecars() {
    let dir = tempfile::tempdir().unwrap();
    for segment in [0, 1] {
        let mut w = writer(dir.path(), segment, 0, 16_000, 1).with_speech(loud, |_| {});
        feed(&mut w, &silence(16_000, 61_000));
        w.finish().unwrap();
    }
    std::fs::write(dir.path().join(format!("0-{CAPTURE}-0.speech.tmp")), [1]).unwrap();
    remove_segment(dir.path(), 0).unwrap();
    let mut left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left.len(), 4, "{left:?}");
    assert!(left.iter().all(|name| name.starts_with("1-")), "{left:?}");
    assert_eq!(
        left.iter().filter(|name| name.ends_with(".speech")).count(),
        2
    );
}

/// The AMI excerpt of the VAD tests: far-field meeting speech, mono 16 kHz.
fn meeting_speech() -> Vec<f32> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vad/tests/fixtures/ami-sdm-speech.wav"
    );
    let reader = hound::WavReader::open(path).unwrap();
    assert_eq!(reader.spec().sample_rate, 16_000);
    reader
        .into_samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / 32_768.0)
        .collect()
}

/// 16 kHz to 48 kHz by linear interpolation.
fn to_48k(samples: &[f32]) -> Vec<f32> {
    samples
        .windows(2)
        .flat_map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            [a, a + (b - a) / 3.0, a + 2.0 * (b - a) / 3.0]
        })
        .collect()
}

fn voiced_cells(dir: &Path) -> (usize, Vec<Cell>) {
    let cells: Vec<Cell> = list_chunks(dir)
        .unwrap()
        .iter()
        .flat_map(chunk_cells)
        .map(Option::unwrap)
        .collect();
    let voiced = cells
        .iter()
        .filter(|c| c.channels().any(|ch| ch.is_voiced()))
        .count();
    (voiced, cells)
}

#[test]
fn earshot_finds_meeting_speech_on_the_app_channel_at_48_khz() {
    let dir = tempfile::tempdir().unwrap();
    let meter = Arc::new(GuardMeter::default());
    let mut w = metered(dir.path(), 0, 48_000, 2, earshot, &meter);
    let speech = to_48k(&meeting_speech());
    let quiet = silence(48_000, 40_000);
    let right: Vec<f32> = [quiet.clone(), speech.clone()].concat();
    let left = vec![0.0; right.len()];
    feed(&mut w, &stereo(&left, &right));
    w.finish().unwrap();

    let (voiced, cells) = voiced_cells(dir.path());
    // The excerpt is 34 s of speech with pauses.
    assert!(voiced >= 40, "{voiced} voiced cells");
    // Nothing in the 40 s of silence is voiced.
    assert!(
        cells[..160]
            .iter()
            .all(|c| c.channels().all(|ch| !ch.is_voiced()))
    );
    // Speech reset the silence time after the 40 s of silence.
    assert!(meter.silence_ms() < 10_000, "{}", meter.silence_ms());
    assert!(meter.run_ms() > 70_000);
}

#[test]
fn earshot_finds_no_speech_in_white_noise() {
    let dir = tempfile::tempdir().unwrap();
    let meter = Arc::new(GuardMeter::default());
    let mut w = metered(dir.path(), 0, 48_000, 1, earshot, &meter);
    feed(&mut w, &noise(48_000, 30_000, -40.0));
    w.finish().unwrap();

    let (voiced, cells) = voiced_cells(dir.path());
    assert_eq!(voiced, 0);
    assert_eq!(meter.silence_ms(), 30_000);
    // The level is -40 dBFS, above the quiet level.
    for cell in &cells {
        let level = cell.mic.level_dbfs();
        assert!((-41.0..=-39.0).contains(&level), "{level}");
        assert!(level > QUIET_LEVEL_DBFS);
    }
}

/// The CPU time of the detector on 10 minutes of 48 kHz stereo (KTD1).
/// Run in a release build:
/// `cargo test -p recorder --release --test speech -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement for a release build"]
fn detector_cpu_time_on_ten_minutes_of_48_khz_stereo() {
    use recorder::speech::CellBuilder;
    let rate = 48_000;
    let mut builder = CellBuilder::new(rate, vec![earshot(rate), earshot(rate)]);
    let left = to_48k(&meeting_speech());
    let right = noise(rate, left.len() as u64 * 1000 / u64::from(rate), -40.0);
    // `stereo` stops at the shorter channel.
    let block = stereo(&left, &right);
    let seconds = left.len() as f64 / f64::from(rate);
    let rounds = (600.0 / seconds).ceil() as usize;
    let mut cells = 0usize;
    let started = std::time::Instant::now();
    for _ in 0..rounds {
        for frame in block.chunks(rate as usize / 50 * 2) {
            builder.push(frame, &mut |_| cells += 1);
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    let audio = seconds * rounds as f64;
    println!(
        "{audio:.0} s of 48 kHz stereo in {elapsed:.3} s: {:.4}% of one core, {cells} cells",
        elapsed / audio * 100.0
    );
}
