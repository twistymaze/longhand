//! The long-silence cut on real chunk files (U10 of the forgotten-recording
//! plan): the files and the store after the cut, and the live results of
//! the kept audio.

mod common;

use std::fs::File;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use models::recognition::{ExecutionSpec, RecognitionConfig, SpeakerSource, resolve};
use pipeline::live::archive::{Archive, ArchiveReader, SourceProof};
use pipeline::live::{LiveConfig, LiveEnd, LiveRow};
use pipeline::{RECOVERY_DIR, SharedStore, Unattended, checkpoint, proc_file};
use recorder::speech::{Cell, ChannelCell, sidecar_of, write_sidecar};
use session_core::{Segment, Stage};
use tokio_util::sync::CancellationToken;

const ID: &str = "cut";
const MODEL: &str = "whisper-large-v3-turbo-q5km";
const MIN: u64 = 60_000;
/// The kinds of the ten chunks: speech (`S`) or quiet room tone (`Q`). With
/// a cut time of 3 minutes, the cut keeps one chunk of margin on each side
/// and removes the chunks 3 to 6.
const KINDS: &str = "SSQQQQQQSS";
const GUARDS: &str =
    r#"{"silence_check_minutes":30,"time_limit_hours":6,"cut_minutes":3,"countdown_minutes":2}"#;

struct Note {
    temp: tempfile::TempDir,
    store: SharedStore,
    spec: ExecutionSpec,
}

impl Note {
    fn dir(&self) -> PathBuf {
        self.temp.path().join(ID)
    }

    fn recovery(&self) -> PathBuf {
        self.dir().join(RECOVERY_DIR)
    }

    fn segments(&self) -> Vec<Segment> {
        self.store.lock().unwrap().segments(ID).unwrap()
    }

    fn cut(&self) {
        pipeline::cut::run(&self.store, ID, &self.dir(), &Unattended).unwrap();
    }

    /// The chunk and sidecar names in `audio-recovery/`, sorted.
    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.recovery())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// The segment index and start of each chunk, in time order.
    fn chunks(&self) -> Vec<(u32, u64)> {
        let mut chunks: Vec<(u32, u64)> = recorder::list_chunks(&self.recovery())
            .unwrap()
            .iter()
            .map(|c| (c.segment, c.start_ms))
            .collect();
        chunks.sort_by_key(|c| c.1);
        chunks
    }

    fn proof(&self, segment: u32) -> Option<SourceProof> {
        checkpoint::get(
            &self.store,
            ID,
            Stage::Transcribe,
            &pipeline::live::archive::key(segment),
        )
        .unwrap()
    }

    fn keys(&self, stage: Stage) -> Vec<String> {
        checkpoint::all(&self.store, ID, stage)
            .unwrap()
            .into_iter()
            .map(|(key, _)| key)
            .collect()
    }

    /// Save a live source proof over the first `seconds` of segment 0, and
    /// one live row with the words `words` (in milliseconds).
    fn live(&self, seconds: u64, words: &[(u64, &str)]) {
        let mut reader = ArchiveReader::new(Archive {
            dir: self.recovery(),
            segment: 0,
            offset_ms: 0,
            rate: 16_000,
        });
        while reader.position() < seconds * 16_000 && reader.next_frame().unwrap().is_some() {}
        let mut config = LiveConfig::new(ID, 0, MODEL, 16_000);
        config.snapshot = Some(self.spec.clone());
        pipeline::live::archive::save_proof(&self.store, &config, &reader).unwrap();
        pipeline::live::save_part(
            &self.store,
            ID,
            0,
            0,
            &asr::LivePart {
                start_ms: 0,
                end_ms: seconds * 1000,
                words: words.iter().map(|&(at, text)| word(at, text)).collect(),
            },
        )
        .unwrap();
    }
}

fn word(at: u64, text: &str) -> asr::LiveWord {
    asr::LiveWord {
        start_ms: at,
        end_ms: at + 400,
        text: text.into(),
    }
}

fn cell(kind: char) -> Cell {
    match kind {
        'S' => Cell {
            mic: ChannelCell {
                voiced: 255,
                level: 20,
            },
            app: None,
        },
        _ => Cell {
            mic: ChannelCell {
                voiced: 0,
                level: 70,
            },
            app: None,
        },
    }
}

/// A queued note with one 16-kHz segment of one chunk for each kind in
/// `kinds`, with speech sidecars and the recording settings snapshot.
fn note(kinds: &str) -> Note {
    let temp = tempfile::tempdir().unwrap();
    let store = common::store();
    let spec = resolve(&RecognitionConfig {
        asr_model: MODEL.into(),
        speaker_source: SpeakerSource::Off,
        ..Default::default()
    })
    .unwrap();
    let audio: Vec<f32> = (0..kinds.len() * 60 * 16_000)
        .map(|i| ((i * 7_919 % 2_003) as f32 / 2_003.0 - 0.5) * 0.5)
        .collect();
    common::queue_session(
        &store,
        temp.path(),
        ID,
        10_000,
        Some(MODEL),
        &[(16_000, audio)],
    );
    let recovery = temp.path().join(ID).join(RECOVERY_DIR);
    for chunk in recorder::list_chunks(&recovery).unwrap() {
        let kind = kinds.as_bytes()[(chunk.start_ms / MIN) as usize] as char;
        let cells = vec![cell(kind); recorder::speech::CELLS_PER_CHUNK as usize];
        write_sidecar(&sidecar_of(&chunk), 1, &cells, File::sync_all).unwrap();
    }
    {
        let s = store.lock().unwrap();
        s.set_recording_guards(ID, Some(GUARDS)).unwrap();
        s.set_recognition_snapshot(ID, &serde_json::to_value(&spec).unwrap())
            .unwrap();
    }
    Note { temp, store, spec }
}

fn assemble(note: &Note) -> Vec<Segment> {
    let segments = note.segments();
    pipeline::assemble::run(&note.store, ID, &note.dir(), &segments, None, &Unattended).unwrap();
    segments
}

fn reasons(note: &Note) -> Option<Vec<String>> {
    checkpoint::get(
        &note.store,
        ID,
        Stage::Transcribe,
        "live-invalidation-reasons",
    )
    .unwrap()
}

#[test]
fn the_cut_splits_the_segment_and_keeps_the_original_times() {
    let note = note(KINDS);
    note.cut();
    let segments = note.segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(
        (
            segments[0].index,
            segments[0].offset_ms,
            segments[0].duration_ms
        ),
        (0, 0, 3 * MIN)
    );
    assert_eq!(segments[0].cut_after_ms, 4 * MIN);
    assert_eq!(
        (
            segments[1].index,
            segments[1].offset_ms,
            segments[1].duration_ms
        ),
        (1, 7 * MIN, 3 * MIN)
    );
    assert_eq!(segments[1].audio_file, "audio/segment-02.opus");
    assert_eq!(
        note.store.lock().unwrap().cut_checked(ID).unwrap(),
        std::collections::BTreeSet::from([0, 1])
    );
    // The removed chunks and their sidecars are gone. The chunks of part B
    // have the index 1.
    assert_eq!(
        note.chunks(),
        [
            (0, 0),
            (0, MIN),
            (0, 2 * MIN),
            (1, 7 * MIN),
            (1, 8 * MIN),
            (1, 9 * MIN)
        ]
    );
    let files = note.files();
    assert_eq!(files.iter().filter(|n| n.ends_with(".speech")).count(), 6);
    assert!(
        files
            .iter()
            .any(|n| n.starts_with("1-") && n.ends_with("-420000.speech"))
    );
    assert_eq!(files.len(), 12, "{files:?}");
}

fn proc_samples(note: &Note, segment: &Segment) -> Vec<f32> {
    pipeline::assemble::open_proc(&note.dir(), segment)
        .unwrap()
        .into_samples::<f32>()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn assemble_reads_the_rows_after_the_cut_and_prepares_only_the_kept_audio() {
    // The same note with no cut gives the processing samples of the whole
    // segment.
    let whole = note(KINDS);
    let whole = proc_samples(&whole, &assemble(&whole)[0]);
    let note = note(KINDS);
    note.cut();
    let segments = assemble(&note);
    for segment in &segments {
        let samples = proc_samples(&note, segment);
        let from = (segment.offset_ms * 16) as usize;
        assert_eq!(samples.len(), (segment.duration_ms * 16) as usize);
        assert!(
            samples == whole[from..from + samples.len()],
            "segment {}",
            segment.index
        );
    }
    assert!(!note.dir().join(proc_file("audio/segment-03.opus")).exists());
}

#[test]
fn after_a_cut_the_live_results_of_the_kept_audio_stay_valid() {
    let note = note(KINDS);
    note.live(
        600,
        &[(30_000, "hello"), (480_000, "again"), (590_000, "end")],
    );
    note.cut();
    // The live row splits at the cut edges. Each piece keeps the words in
    // its part, and the piece of part B has the times of part B.
    let rows = pipeline::live::rows(&note.store, ID).unwrap();
    assert_eq!(
        rows,
        [
            LiveRow {
                segment: 0,
                part: 0,
                start_ms: 0,
                end_ms: 3 * MIN,
                words: vec![word(30_000, "hello")],
            },
            LiveRow {
                segment: 1,
                part: 0,
                start_ms: 0,
                end_ms: 3 * MIN,
                words: vec![word(60_000, "again"), word(170_000, "end")],
            },
        ]
    );
    assert_eq!(note.proof(0).unwrap().samples, 3 * 60 * 16_000);
    assert_eq!(note.proof(1).unwrap().samples, 3 * 60 * 16_000);
    let segments = assemble(&note);
    pipeline::recognition::durable::prepare(&note.store, ID, &note.dir(), &segments, &note.spec)
        .unwrap();
    assert_eq!(reasons(&note), None);
    assert_eq!(pipeline::live::rows(&note.store, ID).unwrap(), rows);
    let keys = note.keys(Stage::Transcribe);
    for key in [
        "live-source-00000000",
        "live-source-00000001",
        "live-snapshot-00000000",
        "live-snapshot-00000001",
    ] {
        assert!(keys.iter().any(|k| k == key), "{key} in {keys:?}");
    }
}

#[test]
fn a_live_worker_that_was_behind_gives_proofs_only_for_the_covered_audio() {
    let note = note(KINDS);
    // The live worker read the first 500 s.
    note.live(500, &[(30_000, "hello"), (480_000, "again")]);
    note.cut();
    assert_eq!(note.proof(0).unwrap().samples, 3 * 60 * 16_000);
    assert_eq!(note.proof(1).unwrap().samples, 80 * 16_000);
    let segments = assemble(&note);
    pipeline::recognition::durable::prepare(&note.store, ID, &note.dir(), &segments, &note.spec)
        .unwrap();
    assert_eq!(reasons(&note), None);
    assert_eq!(pipeline::live::rows(&note.store, ID).unwrap().len(), 2);
}

#[test]
fn a_proof_that_does_not_verify_gives_no_new_proofs() {
    let note = note(KINDS);
    note.live(600, &[(30_000, "hello"), (480_000, "again")]);
    let mut proof = note.proof(0).unwrap();
    proof.digest = "0".repeat(64);
    checkpoint::put(
        &note.store,
        ID,
        Stage::Transcribe,
        &pipeline::live::archive::key(0),
        &proof,
    )
    .unwrap();
    note.cut();
    assert_eq!(note.proof(0), None);
    assert_eq!(note.proof(1), None);
    // Recognition of the recorded audio replaces the live text, as for a
    // note with no cut.
    let segments = assemble(&note);
    pipeline::recognition::durable::prepare(&note.store, ID, &note.dir(), &segments, &note.spec)
        .unwrap();
    assert!(reasons(&note).is_some_and(|r| !r.is_empty()));
    assert!(pipeline::live::rows(&note.store, ID).unwrap().is_empty());
}

#[test]
fn an_epoch_record_clips_to_the_kept_audio() {
    let note = note(KINDS);
    note.live(600, &[]);
    // An epoch in part A, an epoch from part A over the removed audio into
    // part B, an epoch in the removed audio, and an epoch from the removed
    // audio into part B.
    for (start, end) in [
        (0, 2 * MIN),
        (2 * MIN, 8 * MIN),
        (4 * MIN, 6 * MIN),
        (6 * MIN, 9 * MIN),
    ] {
        checkpoint::put(
            &note.store,
            ID,
            Stage::Transcribe,
            &format!("recognition-epoch-{:020}", start * 16),
            &serde_json::json!({"range": {"start": start * 16, "end": end * 16}, "rows": 0}),
        )
        .unwrap();
    }
    note.cut();
    let epochs: Vec<(String, serde_json::Value)> =
        checkpoint::all(&note.store, ID, Stage::Transcribe)
            .unwrap()
            .into_iter()
            .filter(|(key, _)| key.starts_with("recognition-epoch-"))
            .map(|(key, value)| (key, serde_json::from_str(&value).unwrap()))
            .collect();
    let ranges: Vec<(&str, u64, u64)> = epochs
        .iter()
        .map(|(key, value)| {
            (
                key.as_str(),
                value["range"]["start"].as_u64().unwrap() / 16,
                value["range"]["end"].as_u64().unwrap() / 16,
            )
        })
        .collect();
    assert_eq!(
        ranges,
        [
            ("recognition-epoch-00000000000000000000", 0, 2 * MIN),
            ("recognition-epoch-00000000000001920000", 2 * MIN, 8 * MIN),
            ("recognition-epoch-00000000000005760000", 7 * MIN, 9 * MIN),
        ]
    );
    // The epoch that continues in part B is checked in part A only up to
    // the end of part A.
    let segments = assemble(&note);
    pipeline::recognition::durable::prepare(&note.store, ID, &note.dir(), &segments, &note.spec)
        .unwrap();
    assert_eq!(reasons(&note), None);
}

/// Records the sample count of each analysis.
struct Counting(Arc<Mutex<Vec<usize>>>);
impl diarize::SpeakerAnalyzer for Counting {
    fn analyze(
        &mut self,
        audio: &mut dyn diarize::AudioSource,
        _: &diarize::DiarizeRequest<'_>,
    ) -> Result<diarize::Analysis, diarize::DiarizeError> {
        let total = audio.sample_count()?;
        self.0.lock().unwrap().push(total);
        Ok(diarize::Analysis {
            total,
            step: 31_860,
            activities: vec![pyannote_local::segmentation::WindowActivity { frames: vec![] }],
            ..Default::default()
        })
    }
}

#[test]
fn a_part_computes_again_only_the_feature_windows_at_its_edges() {
    let note = note(KINDS);
    let mut config = LiveConfig::new(ID, 0, MODEL, 16_000).with_archive(note.recovery(), 0);
    config.snapshot = pipeline::recognition::snapshot(&note.store, ID).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    pipeline::live::features::extract(
        &note.store,
        &config,
        &mut Counting(calls.clone()),
        &CancellationToken::new(),
        &Arc::new(Mutex::new(Some(LiveEnd::Keep))),
    )
    .unwrap();
    // 10 minutes give 298 windows on a hop of 31 860 samples.
    assert_eq!(calls.lock().unwrap().len(), 298);
    calls.lock().unwrap().clear();
    let windows = std::fs::read_dir(note.dir().join("live-features"))
        .unwrap()
        .count();
    assert_eq!(windows, 298);

    note.cut();
    let segments = assemble(&note);
    let mut analyses = Vec::new();
    for segment in &segments {
        let analysis = pipeline::live::features::complete(
            &note.store,
            ID,
            &note.dir(),
            segment,
            &mut Counting(calls.clone()),
            &Unattended,
            &mut |_| {},
        )
        .unwrap()
        .unwrap();
        analyses.push(analysis);
    }
    // Part A computes its last window, which ends at the cut. Part B
    // computes its first window, which starts before the cut.
    assert_eq!(*calls.lock().unwrap(), [140_040, 160_000]);
    assert_eq!(analyses[0].grid_start, 0);
    assert_eq!(analyses[0].activities.len(), 87);
    // Original window 210 starts 29 400 samples before part B.
    assert_eq!(analyses[1].grid_start, -29_400);
    assert_eq!(analyses[1].activities.len(), 88);
    // The windows that the cut deleted have no file.
    let windows = std::fs::read_dir(note.dir().join("live-features"))
        .unwrap()
        .count();
    assert_eq!(windows, 87 + 88);
}

#[test]
fn the_cut_does_not_delete_a_symlink_or_a_file_with_another_name() {
    let note = note(KINDS);
    let outside = note.temp.path().join("outside.wav");
    std::fs::write(&outside, b"keep").unwrap();
    // The chunk at 4 minutes is now a symlink, and a copy of the chunk at
    // 5 minutes has a name that Longhand does not write.
    let chunks = recorder::list_chunks(&note.recovery()).unwrap();
    let at = |ms: u64| {
        chunks
            .iter()
            .find(|c| c.start_ms == ms)
            .unwrap()
            .path
            .clone()
    };
    std::fs::remove_file(at(4 * MIN)).unwrap();
    std::os::unix::fs::symlink(&outside, at(4 * MIN)).unwrap();
    let other = note.recovery().join(format!(
        "00{}",
        at(5 * MIN).file_name().unwrap().to_string_lossy()
    ));
    std::fs::copy(at(5 * MIN), &other).unwrap();
    note.cut();
    assert!(std::fs::symlink_metadata(at(4 * MIN)).is_ok());
    assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    assert!(other.exists());
    assert!(!at(5 * MIN).exists());
}

#[test]
fn start_over_after_a_cut_does_not_cut_again() {
    let note = note(KINDS);
    note.cut();
    let segments = note.segments();
    let files = note.files();
    note.store.lock().unwrap().start_over(ID).unwrap();
    note.cut();
    assert_eq!(note.segments(), segments);
    assert_eq!(note.files(), files);
}

#[test]
fn a_note_without_a_snapshot_is_not_cut() {
    let note = note(KINDS);
    note.store
        .lock()
        .unwrap()
        .set_recording_guards(ID, None)
        .unwrap();
    let segments = note.segments();
    let files = note.files();
    note.cut();
    assert_eq!(note.segments(), segments);
    assert_eq!(note.files(), files);
    assert!(
        note.store
            .lock()
            .unwrap()
            .cut_checked(ID)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_cut_that_removes_nothing_marks_the_segments() {
    let note = note("SSSSSSSSSS");
    let segments = note.segments();
    let files = note.files();
    note.cut();
    assert_eq!(note.segments(), segments);
    assert_eq!(note.files(), files);
    assert_eq!(
        note.store.lock().unwrap().cut_checked(ID).unwrap(),
        std::collections::BTreeSet::from([0])
    );
}
