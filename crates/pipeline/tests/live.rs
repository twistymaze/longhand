//! The live worker (U6, KTD3, KTD4, KTD7, KTD9) against a scripted stream
//! that hears clicks: each click it hears becomes one word at the click's
//! time in the stream.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use asr::{
    AsrError, AsrSegment, LiveStream, LiveText, LiveWord, PartPolicy, Parts, Recognizer, Span,
    TimedToken, WindowText,
};
use common::{store, write_segment};
use pipeline::live::{
    self, EngineAccess, EngineSlot, Inputs, LiveConfig, LiveEnd, LiveFrame, LiveRow, LiveSink,
    LiveStatus, Workers,
};
use pipeline::transcribe::{self, WindowPlan};
use pipeline::{RECOVERY_DIR, SharedStore, Unattended, assemble, checkpoint, verify};
use session_core::Stage;
use tokio_util::sync::CancellationToken;

const ID: &str = "s";
/// 20 ms at 48 kHz, as capture delivers it.
const FRAME_48K: usize = 960;

/// What the scripted recognizers do, shared by every stream they start.
#[derive(Default)]
struct Script {
    /// Model ids loaded, in order.
    loads: Mutex<Vec<String>>,
    streams: AtomicUsize,
    /// Stream number and stream time in ms at which a feed fails.
    fail_feed: Mutex<Option<(usize, u64)>>,
    /// Each finish waits for a message on this channel.
    finish_gate: Mutex<Option<mpsc::Receiver<()>>>,
}

struct Clicker {
    model: String,
    script: Arc<Script>,
}

impl Recognizer for Clicker {
    fn model_id(&self) -> &str {
        &self.model
    }

    fn transcribe(&mut self, _: &[f32]) -> Result<WindowText, AsrError> {
        Ok(WindowText::default())
    }

    fn start_stream(&mut self) -> Result<Option<Box<dyn LiveStream + '_>>, AsrError> {
        let number = self.script.streams.fetch_add(1, Ordering::SeqCst);
        Ok(Some(Box::new(ClickStream {
            number,
            script: Arc::clone(&self.script),
            samples: 0,
            clicks: Vec::new(),
        })))
    }
}

/// Hears a click where a sample passes 0.5, at most one in 100 ms.
struct ClickStream {
    number: usize,
    script: Arc<Script>,
    samples: u64,
    /// Stream sample positions of the clicks.
    clicks: Vec<u64>,
}

impl LiveStream for ClickStream {
    fn feed(&mut self, pcm: &[f32]) -> Result<bool, AsrError> {
        if let Some((number, at_ms)) = *self.script.fail_feed.lock().unwrap()
            && number == self.number
            && self.samples / 16 >= at_ms
        {
            return Err(AsrError::Run("the stream failed".into()));
        }
        let before = self.clicks.len();
        for (i, &s) in pcm.iter().enumerate() {
            let at = self.samples + i as u64;
            if s > 0.5 && self.clicks.last().is_none_or(|&last| at - last > 1_600) {
                self.clicks.push(at);
            }
        }
        self.samples += pcm.len() as u64;
        Ok(self.clicks.len() > before)
    }

    fn text(&self) -> LiveText {
        LiveText {
            committed: format!("{} clicks", self.clicks.len()),
            tentative: String::new(),
        }
    }

    fn finish(&mut self) -> Result<Vec<TimedToken>, AsrError> {
        let gate = self.script.finish_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.recv();
        }
        Ok(self
            .clicks
            .iter()
            .enumerate()
            .map(|(n, &at)| TimedToken {
                text: format!(" c{n}"),
                start_ms: at / 16,
                end_ms: at / 16 + 50,
            })
            .collect())
    }

    fn reset(&mut self) {
        self.samples = 0;
        self.clicks.clear();
    }
}

fn slot(model: Option<&str>, script: &Arc<Script>) -> EngineSlot {
    Arc::new(Mutex::new(model.map(|m| {
        Box::new(Clicker {
            model: m.into(),
            script: Arc::clone(script),
        }) as Box<dyn Recognizer>
    })))
}

fn access(slot: &EngineSlot, script: &Arc<Script>, shutdown: CancellationToken) -> EngineAccess {
    let script = Arc::clone(script);
    EngineAccess::new(
        Arc::clone(slot),
        Arc::new(move |model: &str| {
            script.loads.lock().unwrap().push(model.to_string());
            Ok(Box::new(Clicker {
                model: model.into(),
                script: Arc::clone(&script),
            }) as Box<dyn Recognizer>)
        }),
        shutdown,
    )
}

/// Records what the worker reports.
#[derive(Default, Clone)]
struct Seen {
    statuses: Arc<Mutex<Vec<LiveStatus>>>,
    saved: Arc<Mutex<Vec<LiveRow>>>,
    models: Arc<Mutex<Vec<Option<String>>>>,
    texts: Arc<AtomicUsize>,
    /// Cancel this when the given number of parts is saved.
    cancel_after: Arc<Mutex<Option<(usize, CancellationToken)>>>,
}

impl LiveSink for Seen {
    fn model(&mut self, held: Option<&str>) {
        self.models.lock().unwrap().push(held.map(str::to_string));
    }
    fn status(&mut self, status: LiveStatus) {
        self.statuses.lock().unwrap().push(status);
    }
    fn text(&mut self, _: u32, _: &LiveText) {
        self.texts.fetch_add(1, Ordering::SeqCst);
    }
    fn saved(&mut self, row: &LiveRow) {
        let mut saved = self.saved.lock().unwrap();
        saved.push(row.clone());
        if let Some((after, cancel)) = self.cancel_after.lock().unwrap().as_ref()
            && saved.len() == *after
        {
            cancel.cancel();
        }
    }
}

/// Mono audio at `rate` with a 5 ms click at each time in `clicks_ms`.
fn clicks(rate: u32, length_ms: u64, clicks_ms: &[u64]) -> Vec<f32> {
    let per_ms = rate as usize / 1000;
    let mut audio = vec![0.0f32; length_ms as usize * per_ms];
    for &at in clicks_ms {
        let start = at as usize * per_ms;
        for s in &mut audio[start..start + 5 * per_ms] {
            *s = 0.9;
        }
    }
    audio
}

/// Frames of `frame` samples, without the frames that start in a dropped
/// span of segment time.
fn frames(audio: &[f32], rate: u32, frame: usize, dropped_ms: &[(u64, u64)]) -> Vec<LiveFrame> {
    audio
        .chunks(frame)
        .enumerate()
        .map(|(n, pcm)| LiveFrame {
            index: (n * frame) as u64,
            pcm: Arc::from(pcm),
        })
        .filter(|f| {
            let ms = f.index * 1000 / rate as u64;
            !dropped_ms.iter().any(|&(a, b)| ms >= a && ms < b)
        })
        .collect()
}

fn always_voiced(policy: PartPolicy) -> Parts {
    Parts::with_classifier(0, policy, |_: &[f32]| true)
}

struct Run {
    store: SharedStore,
    script: Arc<Script>,
    seen: Seen,
}

impl Run {
    fn new() -> Self {
        let store = store();
        store
            .lock()
            .unwrap()
            .create_session(ID, common::at(0), None)
            .unwrap();
        Self {
            store,
            script: Arc::default(),
            seen: Seen::default(),
        }
    }

    /// Run a worker over `frames` to the end, on this thread.
    fn run(&self, config: LiveConfig, parts: Parts, frames: Vec<LiveFrame>, end: LiveEnd) {
        let slot = slot(None, &self.script);
        let engine = access(&slot, &self.script, CancellationToken::new());
        let (tx, rx) = mpsc::channel();
        for frame in frames {
            tx.send(frame).unwrap();
        }
        drop(tx);
        let (end_tx, end_rx) = mpsc::channel();
        end_tx.send(end).unwrap();
        let cancel = CancellationToken::new();
        if let Some((_, token)) = self.seen.cancel_after.lock().unwrap().as_mut() {
            *token = cancel.clone();
        }
        let inputs = Inputs {
            frames: rx,
            end: end_rx,
            cancel,
        };
        let mut sink = self.seen.clone();
        live::run(&self.store, &engine, &config, parts, &inputs, &mut sink);
    }

    fn rows(&self) -> Vec<LiveRow> {
        live::rows(&self.store, ID).unwrap()
    }
}

fn config(rate: u32) -> LiveConfig {
    LiveConfig::new(ID, 0, "streams", rate)
}

fn spans(rows: &[LiveRow]) -> Vec<(u64, u64)> {
    rows.iter().map(|r| (r.start_ms, r.end_ms)).collect()
}

fn word_starts(rows: &[LiveRow]) -> Vec<u64> {
    rows.iter()
        .flat_map(|r| r.words.iter().map(|w| w.start_ms))
        .collect()
}

fn near(got: u64, want: u64, within: u64) -> bool {
    got.abs_diff(want) <= within
}

/// Where the clicks sit in a 16 kHz processing file, in ms.
fn clicks_in(pcm: &[f32]) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    let mut last: Option<usize> = None;
    for (i, &s) in pcm.iter().enumerate() {
        if s > 0.5 && last.is_none_or(|l| i - l > 1_600) {
            out.push(i as u64 / 16);
            last = Some(i);
        }
    }
    out
}

#[test]
fn live_word_times_match_the_assembled_file_before_and_after_a_dropped_second() {
    let rate = 48_000;
    let said = [1_000, 2_500, 4_200, 5_400, 7_300, 9_900, 12_100, 14_650];
    let audio = clicks(rate, 16_000, &said);

    // The assembled 16 kHz file of the same audio.
    let dir = tempfile::tempdir().unwrap();
    let segment = write_segment(dir.path(), 0, 0, rate, &audio, false);
    assemble::assemble_segment(
        &dir.path().join(RECOVERY_DIR),
        dir.path(),
        &segment,
        &Unattended,
    )
    .unwrap();
    let proc: Vec<f32> = assemble::open_proc(dir.path(), &segment)
        .unwrap()
        .into_samples::<f32>()
        .map(Result::unwrap)
        .collect();
    let assembled = clicks_in(&proc);
    assert_eq!(assembled.len(), said.len());

    // The channel was full from 5 s to 6 s: those frames never arrived.
    let r = Run::new();
    r.run(
        config(rate),
        always_voiced(PartPolicy::default()),
        frames(&audio, rate, FRAME_48K, &[(5_000, 6_000)]),
        LiveEnd::Keep,
    );
    let rows = r.rows();
    // The part before the gap ends at the last frame received, and the next
    // part starts at the next frame: the second between is a gap.
    assert_eq!(spans(&rows), vec![(0, 5_000), (6_000, 16_000)]);
    let live = word_starts(&rows);
    let expected: Vec<u64> = assembled
        .iter()
        .copied()
        .filter(|&t| !(5_000..6_000).contains(&t))
        .collect();
    assert_eq!(live.len(), expected.len(), "{live:?} against {expected:?}");
    for (got, want) in live.iter().zip(&expected) {
        assert!(near(*got, *want, 50), "live {got} ms, assembled {want} ms");
    }
    assert_eq!(r.script.streams.load(Ordering::SeqCst), 2);
    assert!(
        r.seen
            .statuses
            .lock()
            .unwrap()
            .contains(&LiveStatus::Paused)
    );
    // Each saved part was reported with the row that was saved.
    assert_eq!(*r.seen.saved.lock().unwrap(), rows);
}

#[test]
fn a_full_channel_drops_the_frame_and_never_waits() {
    let (tap, frames) = live::channel(2);
    let pcm: Arc<[f32]> = Arc::from(vec![0.0f32; 960]);
    let started = Instant::now();
    assert!(tap.offer(0, &pcm));
    assert!(tap.offer(960, &pcm));
    assert!(!tap.offer(1_920, &pcm), "the third frame is dropped");
    assert!(started.elapsed() < Duration::from_millis(100));
    assert_eq!(frames.recv().unwrap().index, 0);
    assert!(tap.offer(2_880, &pcm));
    drop(frames);
    assert!(!tap.offer(3_840, &pcm), "a stopped worker takes no frames");
}

#[test]
fn a_stream_error_ends_the_part_and_a_new_stream_goes_on() {
    let rate = 16_000;
    let audio = clicks(rate, 10_000, &[1_000, 2_000, 5_000, 8_000]);
    let r = Run::new();
    *r.script.fail_feed.lock().unwrap() = Some((0, 3_000));
    r.run(
        config(rate),
        always_voiced(PartPolicy::default()),
        frames(&audio, rate, 320, &[]),
        LiveEnd::Keep,
    );
    let rows = r.rows();
    // The first part ends at the last audio the stream took; the frame it
    // failed on is a gap, and the next part starts at the frame after it.
    assert_eq!(spans(&rows), vec![(0, 3_000), (3_020, 10_000)]);
    let live = word_starts(&rows);
    assert_eq!(live.len(), 4);
    for (got, want) in live.iter().zip([1_000, 2_000, 5_000, 8_000]) {
        assert!(near(*got, want, 50), "{got} against {want}");
    }
    assert_eq!(r.script.streams.load(Ordering::SeqCst), 2);
}

#[test]
fn forty_five_seconds_of_speech_with_no_new_text_ends_the_part_as_a_gap() {
    // Covers AE11. Clicks until 10 s, then 45 s of speech that the stream
    // gives no text for.
    let rate = 16_000;
    let said: Vec<u64> = (1..=10).map(|s| s * 1_000 - 500).collect();
    let audio = clicks(rate, 60_000, &said);
    let r = Run::new();
    r.run(
        config(rate),
        always_voiced(PartPolicy::default()),
        frames(&audio, rate, 320, &[]),
        LiveEnd::Keep,
    );
    let rows = r.rows();
    assert_eq!(rows.len(), 2, "{:?}", spans(&rows));
    // The stalled part ends where its text last changed.
    assert_eq!(rows[0].start_ms, 0);
    assert!(near(rows[0].end_ms, 9_520, 40), "{:?}", spans(&rows));
    assert_eq!(rows[0].words.len(), 10);
    // The stall is found after 45 s of speech without new text; a new
    // stream takes over and the recording goes on.
    assert!(near(rows[1].start_ms, 54_520, 40), "{:?}", spans(&rows));
    assert_eq!(rows[1].end_ms, 60_000);
    assert_eq!(r.script.streams.load(Ordering::SeqCst), 2);
    let statuses = r.seen.statuses.lock().unwrap().clone();
    assert!(statuses.contains(&LiveStatus::Paused), "{statuses:?}");
}

#[test]
fn a_crash_after_two_saved_parts_leaves_two_rows_and_the_third_part_is_a_gap() {
    let rate = 16_000;
    let audio = clicks(rate, 10_000, &[500, 2_500, 4_500, 6_500, 8_500]);
    let policy = PartPolicy {
        min_ms: 1_000,
        max_ms: 2_000,
        pause_ms: 600,
        replay_ms: 500,
    };
    let r = Run::new();
    // The process stops as the second part is saved.
    *r.seen.cancel_after.lock().unwrap() = Some((2, CancellationToken::new()));
    r.run(
        config(rate),
        always_voiced(policy),
        frames(&audio, rate, 320, &[]),
        LiveEnd::Keep,
    );
    let rows = r.rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().map(|r| r.part).collect::<Vec<_>>(), vec![0, 1]);
    // Nothing after the second part is saved: the rest is a gap.
    assert!(rows[1].end_ms < 10_000);
    // Launch recovery keeps the saved rows of a kept segment.
    let dir = tempfile::tempdir().unwrap();
    write_segment(&dir.path().join(ID), 0, 0, rate, &audio, true);
    pipeline::recover::recover_crashed(&r.store, dir.path()).unwrap();
    assert_eq!(r.rows(), rows);
}

#[test]
fn a_segment_under_a_second_leaves_no_live_rows() {
    let rate = 16_000;
    let audio = clicks(rate, 800, &[100, 600]);
    let r = Run::new();
    // A dropped frame saves a first part before the segment is discarded.
    r.run(
        config(rate),
        always_voiced(PartPolicy::default()),
        frames(&audio, rate, 320, &[(400, 420)]),
        LiveEnd::Discard,
    );
    assert_eq!(r.seen.saved.lock().unwrap().len(), 1);
    assert!(r.rows().is_empty());
}

#[test]
fn launch_recovery_deletes_the_live_rows_of_a_segment_under_a_second() {
    let rate = 16_000;
    let r = Run::new();
    let dir = tempfile::tempdir().unwrap();
    write_segment(&dir.path().join(ID), 0, 0, rate, &vec![0.1; 8_000], true);
    let part = asr::LivePart {
        start_ms: 0,
        end_ms: 400,
        words: vec![],
    };
    live::save_part(&r.store, ID, 0, 0, &part).unwrap();
    pipeline::recover::recover_crashed(&r.store, dir.path()).unwrap();
    assert!(r.rows().is_empty());
}

#[test]
fn live_rows_have_their_own_keys_and_never_count_as_windows() {
    let r = Run::new();
    let part = asr::LivePart {
        start_ms: 0,
        end_ms: 1_000,
        words: vec![],
    };
    live::save_part(&r.store, ID, 3, 7, &part).unwrap();
    let keys: Vec<String> = checkpoint::all(&r.store, ID, Stage::Transcribe)
        .unwrap()
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(keys, vec!["seg-0003-live-0007".to_string()]);
    assert!(!keys[0].starts_with(&checkpoint::window_prefix(3)));
    // The results read the live row on purpose, as a live part (KTD7).
    let results = pipeline::transcribe::results(&r.store, ID).unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].live);
    assert_eq!((results[0].segment, results[0].window), (3, 7));
    assert_eq!(live::segment_rows(&r.store, ID, 3).unwrap().len(), 1);
    assert!(live::segment_rows(&r.store, ID, 2).unwrap().is_empty());
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn processing_stays_suspended_after_stop_until_the_last_part_is_saved() {
    let r = Run::new();
    let (release, gate) = mpsc::channel();
    *r.script.finish_gate.lock().unwrap() = Some(gate);
    // The slot holds another model: the worker loads the note's model in
    // its place.
    let slot = slot(Some("other"), &r.script);
    let woken = Arc::new(AtomicUsize::new(0));
    let wake = Arc::clone(&woken);
    let workers = Workers::new(
        Arc::clone(&r.store),
        access(&slot, &r.script, CancellationToken::new()),
    )
    .with_idle(move || {
        wake.fetch_add(1, Ordering::SeqCst);
    });
    let (tap, worker) = workers
        .start(
            config(16_000),
            always_voiced(PartPolicy::default()),
            Box::new(r.seen.clone()),
        )
        .unwrap();
    assert!(workers.busy(), "suspended from the start");
    let audio = clicks(16_000, 2_000, &[500]);
    for frame in frames(&audio, 16_000, 320, &[]) {
        assert!(tap.offer(frame.index, &frame.pcm));
    }
    // Stop: the frames end, and capture keeps the segment.
    drop(tap);
    worker.end(LiveEnd::Keep);
    std::thread::sleep(Duration::from_millis(200));
    assert!(workers.busy(), "the last part is not saved yet");
    assert!(r.rows().is_empty());
    assert_eq!(woken.load(Ordering::SeqCst), 0);

    release.send(()).unwrap();
    wait_until("the worker ends", || !workers.busy());
    assert_eq!(spans(&r.rows()), vec![(0, 2_000)]);
    // Processing is woken once the worker is done.
    wait_until("the idle call", || woken.load(Ordering::SeqCst) == 1);
    // The worker released the slot, and the slot keeps the note's model for
    // processing.
    let held = slot.try_lock().expect("the slot is free");
    assert_eq!(held.as_ref().unwrap().model_id(), "streams");
    assert_eq!(*r.script.loads.lock().unwrap(), vec!["streams".to_string()]);
    assert_eq!(
        *r.seen.models.lock().unwrap(),
        vec![Some("streams".to_string()), None]
    );
}

#[test]
fn workers_run_one_after_another_on_one_loaded_model() {
    let r = Run::new();
    let slot = slot(None, &r.script);
    let workers = Workers::new(
        Arc::clone(&r.store),
        access(&slot, &r.script, CancellationToken::new()),
    );
    let audio = clicks(16_000, 1_500, &[500]);
    for segment in 0..2 {
        let mut config = config(16_000);
        config.segment = segment;
        let (tap, worker) = workers
            .start(
                config,
                always_voiced(PartPolicy::default()),
                Box::new(r.seen.clone()),
            )
            .unwrap();
        for frame in frames(&audio, 16_000, 320, &[]) {
            tap.offer(frame.index, &frame.pcm);
        }
        drop(tap);
        worker.end(LiveEnd::Keep);
    }
    wait_until("both workers end", || !workers.busy());
    let rows = r.rows();
    assert_eq!(
        rows.iter().map(|r| (r.segment, r.part)).collect::<Vec<_>>(),
        vec![(0, 0), (1, 0)]
    );
    assert_eq!(r.script.loads.lock().unwrap().len(), 1);
}

#[test]
fn a_quit_stops_the_worker_and_frees_the_slot_without_saving_the_open_part() {
    let r = Run::new();
    let slot = slot(None, &r.script);
    let shutdown = CancellationToken::new();
    let workers = Workers::new(
        Arc::clone(&r.store),
        access(&slot, &r.script, shutdown.clone()),
    );
    let (tap, _worker) = workers
        .start(
            config(16_000),
            always_voiced(PartPolicy::default()),
            Box::new(r.seen.clone()),
        )
        .unwrap();
    let audio = clicks(16_000, 1_000, &[500]);
    for frame in frames(&audio, 16_000, 320, &[]) {
        tap.offer(frame.index, &frame.pcm);
    }
    wait_until("the stream hears the click", || {
        r.seen.texts.load(Ordering::SeqCst) > 0
    });
    // The recording goes on; the app quits.
    workers.shutdown();
    assert!(!workers.busy());
    assert!(r.rows().is_empty(), "the open part is a gap");
    assert!(slot.try_lock().is_ok(), "the slot is free for the runner");
    // After the runner's shutdown, a worker loads nothing.
    shutdown.cancel();
    slot.lock().unwrap().take();
    let engine = access(&slot, &r.script, shutdown);
    assert!(engine.hold("streams").unwrap().is_none());
    assert_eq!(r.script.loads.lock().unwrap().len(), 1);
    drop(tap);
}

// ------------------------------------------------------------ gap fill (U7)

/// A batch recognizer that says one word in each second of a window, at
/// 0–300 ms of that second. It counts its calls, and the calls in `blank_on`
/// (0-based) decode nothing. Gap-fill tokens come from the trait's default.
struct Batch {
    calls: Arc<AtomicUsize>,
    blank_on: Arc<Mutex<Vec<usize>>>,
}

impl Recognizer for Batch {
    fn model_id(&self) -> &str {
        "streams"
    }

    fn transcribe(&mut self, pcm: &[f32]) -> Result<WindowText, AsrError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.blank_on.lock().unwrap().contains(&call) {
            return Ok(WindowText::default());
        }
        let segments: Vec<AsrSegment> = (0..pcm.len() as u64 / 16_000)
            .map(|n| AsrSegment {
                start_ms: n * 1_000,
                end_ms: n * 1_000 + 300,
                text: format!("b{call}x{n}"),
            })
            .collect();
        let text: Vec<&str> = segments.iter().map(|s| s.text.as_str()).collect();
        Ok(WindowText {
            text: text.join(" "),
            segments,
        })
    }
}

/// One live word in each second from `from_s` to `to_s`, at 0–300 ms.
fn live_words(from_s: u64, to_s: u64) -> Vec<LiveWord> {
    (from_s..to_s)
        .map(|s| word(s * 1_000, s * 1_000 + 300, &format!("live{s}")))
        .collect()
}

fn word(start_ms: u64, end_ms: u64, text: &str) -> LiveWord {
    LiveWord {
        start_ms,
        end_ms,
        text: text.into(),
    }
}

fn span(start_ms: u64, end_ms: u64) -> Span {
    Span { start_ms, end_ms }
}

/// A stopped segment of 60 s, assembled, with a plan of six 10 s windows
/// over one voiced span, and the given live parts saved.
struct Gaps {
    r: Run,
    dir: tempfile::TempDir,
    segment: session_core::Segment,
    calls: Arc<AtomicUsize>,
    blank_on: Arc<Mutex<Vec<usize>>>,
}

impl Gaps {
    fn new(parts: &[(u64, u64, Vec<LiveWord>)]) -> Self {
        let r = Run::new();
        let dir = tempfile::tempdir().unwrap();
        let audio = vec![0.0; 60 * 16_000];
        let segment = write_segment(dir.path(), 0, 0, 16_000, &audio, false);
        assemble::assemble_segment(
            &dir.path().join(RECOVERY_DIR),
            dir.path(),
            &segment,
            &Unattended,
        )
        .unwrap();
        let plan = WindowPlan {
            segment: 0,
            duration_ms: 60_000,
            voiced_ms: 60_000,
            voiced: vec![span(0, 60_000)],
            windows: (0..6).map(|i| span(i * 10_000, (i + 1) * 10_000)).collect(),
        };
        let key = checkpoint::plan_key(0);
        checkpoint::put(&r.store, ID, Stage::Transcribe, &key, &plan).unwrap();
        for (part, (start_ms, end_ms, words)) in (0u32..).zip(parts) {
            let live = asr::LivePart {
                start_ms: *start_ms,
                end_ms: *end_ms,
                words: words.clone(),
            };
            live::save_part(&r.store, ID, 0, part, &live).unwrap();
        }
        Self {
            r,
            dir,
            segment,
            calls: Arc::default(),
            blank_on: Arc::default(),
        }
    }

    fn transcribe(&self) {
        let slot: Mutex<Option<Box<dyn Recognizer>>> = Mutex::new(None);
        let (calls, blank_on) = (Arc::clone(&self.calls), Arc::clone(&self.blank_on));
        let load = move |_: &str| {
            Ok(Box::new(Batch {
                calls: Arc::clone(&calls),
                blank_on: Arc::clone(&blank_on),
            }) as Box<dyn Recognizer>)
        };
        let slot = transcribe::Slot {
            slot: &slot,
            model: "streams",
            load: &load,
        };
        transcribe::run(
            &self.r.store,
            ID,
            self.dir.path(),
            std::slice::from_ref(&self.segment),
            &Default::default(),
            &slot,
            &Unattended,
            &mut |_| {},
        )
        .unwrap();
    }

    fn verify(&self) -> Result<(), pipeline::StageError> {
        verify::run(
            &self.r.store,
            ID,
            self.dir.path(),
            std::slice::from_ref(&self.segment),
        )
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// The numbers of the windows that were transcribed.
    fn filled(&self) -> Vec<usize> {
        transcribe::results(&self.r.store, ID)
            .unwrap()
            .into_iter()
            .filter(|w| !w.live)
            .map(|w| w.window)
            .collect()
    }

    /// Every word of the results, in the order that Join reads them.
    fn words(&self) -> Vec<LiveWord> {
        transcribe::results(&self.r.store, ID)
            .unwrap()
            .into_iter()
            .flat_map(|w| w.words.expect("timed words"))
            .collect()
    }
}

#[test]
fn a_segment_that_the_live_text_covers_runs_no_batch_window() {
    // Covers AE3.
    let g = Gaps::new(&[
        (0, 30_000, live_words(0, 30)),
        (30_000, 60_000, live_words(30, 60)),
    ]);
    g.transcribe();
    assert_eq!(g.calls(), 0);
    assert!(g.filled().is_empty());
    assert_eq!(g.words(), live_words(0, 60));
    g.verify().unwrap();
}

#[test]
fn one_uncovered_span_of_twenty_seconds_transcribes_only_the_windows_that_overlap_it() {
    let g = Gaps::new(&[
        (0, 20_000, live_words(0, 20)),
        (40_000, 60_000, live_words(40, 60)),
    ]);
    g.transcribe();
    assert_eq!(g.filled(), vec![2, 3]);
    assert_eq!(g.calls(), 2);
    // Live words and gap-fill words in time order, each second once.
    let starts: Vec<u64> = g.words().iter().map(|w| w.start_ms).collect();
    assert_eq!(starts, (0..60).map(|s| s * 1_000).collect::<Vec<_>>());
    // Verify passes after the gap fill.
    g.verify().unwrap();
}

#[test]
fn two_saved_parts_and_a_lost_third_transcribe_only_the_third_parts_windows() {
    // Covers AE4: the app quit 40 s into the segment.
    let g = Gaps::new(&[
        (0, 20_000, live_words(0, 20)),
        (20_000, 40_000, live_words(20, 40)),
    ]);
    g.transcribe();
    assert_eq!(g.filled(), vec![4, 5]);
    assert_eq!(g.calls(), 2);
    assert_eq!(g.words().len(), 60);
    g.verify().unwrap();
}

#[test]
fn a_dropped_span_shorter_than_the_slack_between_two_parts_gets_a_gap_fill() {
    // The frames from 25.0 to 25.6 s were dropped. The slack of the live
    // words at 24 s and 26 s covers the span, but no saved part does.
    let g = Gaps::new(&[
        (0, 25_000, live_words(0, 25)),
        (25_600, 60_000, live_words(26, 60)),
    ]);
    g.transcribe();
    assert_eq!(g.filled(), vec![2]);
    assert_eq!(g.calls(), 1);
    // The gap-fill word at 25 s is kept; its repeats of 24 s and 26 s are not.
    let starts: Vec<u64> = g.words().iter().map(|w| w.start_ms).collect();
    assert_eq!(starts, (0..60).map(|s| s * 1_000).collect::<Vec<_>>());
    g.verify().unwrap();
}

#[test]
fn parts_that_overlap_by_the_replay_run_no_batch_window() {
    // A part cut at its length limit: the next part starts 2 s earlier.
    let g = Gaps::new(&[
        (0, 32_000, live_words(0, 32)),
        (30_000, 60_000, live_words(32, 60)),
    ]);
    g.transcribe();
    assert_eq!(g.calls(), 0);
    assert!(g.filled().is_empty());
}

#[test]
fn speech_that_a_part_heard_but_gave_no_text_gets_a_gap_fill() {
    // The part spans the whole segment, but its words stop at 30 s.
    let g = Gaps::new(&[(0, 60_000, live_words(0, 30))]);
    g.transcribe();
    assert_eq!(g.filled(), vec![3, 4, 5]);
    g.verify().unwrap();
}

#[test]
fn a_segment_with_no_live_rows_transcribes_every_window() {
    let g = Gaps::new(&[]);
    g.transcribe();
    assert_eq!(g.filled(), vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(g.calls(), 6);
    let results = transcribe::results(&g.r.store, ID).unwrap();
    assert!(
        results
            .iter()
            .all(|w| w.words.is_none() && !w.segments.is_empty())
    );
}

#[test]
fn a_verify_failure_and_retry_keep_the_live_rows_and_redo_only_the_flagged_window() {
    let g = Gaps::new(&[
        (0, 20_000, live_words(0, 20)),
        (40_000, 60_000, live_words(40, 60)),
    ]);
    // The second gap-fill window decodes nothing.
    g.blank_on.lock().unwrap().push(1);
    g.transcribe();
    assert_eq!(g.calls(), 2);
    assert!(g.verify().is_err());
    let rows = live::rows(&g.r.store, ID).unwrap();

    assert!(verify::prepare_retry(&g.r.store, ID).unwrap());
    assert_eq!(live::rows(&g.r.store, ID).unwrap(), rows);
    assert_eq!(g.filled(), vec![2]);
    g.transcribe();
    assert_eq!(g.calls(), 3, "only window 3 is transcribed again");
    assert_eq!(g.filled(), vec![2, 3]);
    g.verify().unwrap();
}

#[test]
fn the_merge_keeps_gap_words_and_drops_repeats_and_words_outside_the_gaps() {
    let live = vec![word(9_000, 9_400, "before"), word(30_000, 30_400, "after")];
    let gaps = transcribe::gaps(&[span(0, 31_000)], &[span(0, 31_000)], &live);
    // Voiced time more than the slack away from the live words is a gap.
    assert_eq!(gaps, vec![span(0, 8_000), span(10_400, 29_000)]);
    let batch = vec![
        // Within 150 ms of a live word: the same word again.
        word(9_450, 9_700, "before"),
        // Between the live word and the gap: the live text missed it.
        word(9_700, 10_000, "missed"),
        word(15_000, 15_400, "inside"),
        // In the slack before the live word "after", but not near it.
        word(29_500, 29_800, "late"),
        // Outside every gap: the live text covers this time.
        word(31_600, 31_900, "outside"),
    ];
    let kept: Vec<String> = transcribe::merge(batch, &gaps, &live)
        .into_iter()
        .map(|w| w.text)
        .collect();
    assert_eq!(kept, vec!["missed", "inside", "late"]);
}

// ------------------------------------------ echo spans as gaps (U10, KTD10)

impl Gaps {
    /// Save the echo spans checkpoint that Assemble saves for a stereo
    /// segment (KTD8 step 7).
    fn echo(&self, spans: &[Span]) {
        let echo = pipeline::echo::EchoSpans {
            segment: 0,
            spans: spans.to_vec(),
            mic_gain: 1.0,
            system_gain: 1.0,
        };
        checkpoint::put(
            &self.r.store,
            ID,
            Stage::Assemble,
            &checkpoint::echo_key(0),
            &echo,
        )
        .unwrap();
    }

    /// Every word of the results in time order, as Join places them.
    fn texts(&self) -> Vec<(u64, String)> {
        let mut texts: Vec<(u64, String)> = self
            .words()
            .into_iter()
            .map(|w| (w.start_ms, w.text))
            .collect();
        texts.sort();
        texts
    }
}

#[test]
fn live_words_inside_an_echo_span_give_way_to_words_from_the_cleaned_file() {
    // Covers AE2: the live text of 20 to 30 s came from echoed audio.
    let g = Gaps::new(&[(0, 60_000, live_words(0, 60))]);
    g.echo(&[span(20_000, 30_000)]);
    g.transcribe();
    assert_eq!(g.filled(), vec![2]);
    assert_eq!(g.calls(), 1);
    let texts = g.texts();
    let starts: Vec<u64> = texts.iter().map(|(s, _)| *s).collect();
    assert_eq!(starts, (0..60).map(|s| s * 1_000).collect::<Vec<_>>());
    for (start, text) in &texts {
        let inside = (20_000..30_000).contains(start);
        assert_eq!(text.starts_with("live"), !inside, "{start} ms: {text}");
    }
    g.verify().unwrap();
}

#[test]
fn a_segment_with_no_echo_spans_keeps_its_live_text_and_runs_no_batch_window() {
    // Covers AE1: a call on headphones.
    let g = Gaps::new(&[(0, 60_000, live_words(0, 60))]);
    g.echo(&[]);
    g.transcribe();
    assert_eq!(g.calls(), 0);
    assert!(g.filled().is_empty());
    assert_eq!(g.words(), live_words(0, 60));
    g.verify().unwrap();
}

#[test]
fn a_stream_gap_and_an_echo_span_that_overlap_get_one_pass_over_their_union() {
    // Frames from 25 to 28 s were dropped, and echo covers 26 to 35.5 s.
    let g = Gaps::new(&[
        (0, 25_000, live_words(0, 25)),
        (28_000, 60_000, live_words(28, 60)),
    ]);
    g.echo(&[span(26_000, 35_500)]);
    g.transcribe();
    assert_eq!(g.filled(), vec![2, 3]);
    assert_eq!(g.calls(), 2, "each window once");
    let texts = g.texts();
    let starts: Vec<u64> = texts.iter().map(|(s, _)| *s).collect();
    assert_eq!(
        starts,
        (0..60).map(|s| s * 1_000).collect::<Vec<_>>(),
        "each second once: {texts:?}"
    );
    for (start, text) in &texts {
        let batch = (25_000..36_000).contains(start);
        assert_eq!(text.starts_with("live"), !batch, "{start} ms: {text}");
    }
    g.verify().unwrap();
}

#[test]
fn windows_in_finds_the_windows_that_overlap_a_gap() {
    let windows = [span(0, 10_000), span(10_000, 20_000), span(20_000, 30_000)];
    assert_eq!(
        transcribe::windows_in(&windows, &[span(12_000, 21_000)]),
        vec![1, 2]
    );
    assert!(transcribe::windows_in(&windows, &[]).is_empty());
}
