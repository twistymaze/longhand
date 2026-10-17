//! The live worker (U6, KTD3, KTD4, KTD7, KTD9) against a scripted stream
//! that hears clicks: each click it hears becomes one word at the click's
//! time in the stream.

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    /// From this number of clicks, the word at this index has a new text.
    revise: Mutex<Option<(usize, usize)>>,
    /// The streams give no timed words for the open part.
    no_open_words: AtomicBool,
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
        Ok(self.tokens())
    }

    fn reset(&mut self) {
        self.samples = 0;
        self.clicks.clear();
    }

    fn open_words(&self) -> Vec<LiveWord> {
        if self.script.no_open_words.load(Ordering::SeqCst) {
            return Vec::new();
        }
        asr::words_from_tokens(&self.tokens(), 0)
    }
}

impl ClickStream {
    /// One 50 ms token for each click, in stream time. A revised word has
    /// the text `rN` in place of `cN`.
    fn tokens(&self) -> Vec<TimedToken> {
        let revise = *self.script.revise.lock().unwrap();
        self.clicks
            .iter()
            .enumerate()
            .map(|(n, &at)| {
                let revised =
                    revise.is_some_and(|(after, index)| self.clicks.len() >= after && n == index);
                TimedToken {
                    text: format!(" {}{n}", if revised { 'r' } else { 'c' }),
                    start_ms: at / 16,
                    end_ms: at / 16 + 50,
                }
            })
            .collect()
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

/// One report of the open part's text.
#[derive(Debug, Clone)]
struct TextCall {
    part: u32,
    text: LiveText,
    word_start: u32,
    words: Vec<LiveWord>,
}

/// Records what the worker reports.
#[derive(Default, Clone)]
struct Seen {
    statuses: Arc<Mutex<Vec<LiveStatus>>>,
    saved: Arc<Mutex<Vec<LiveRow>>>,
    models: Arc<Mutex<Vec<Option<String>>>>,
    texts: Arc<AtomicUsize>,
    calls: Arc<Mutex<Vec<TextCall>>>,
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
    fn text(&mut self, part: u32, text: &LiveText, word_start: u32, words: &[LiveWord]) {
        self.texts.fetch_add(1, Ordering::SeqCst);
        self.calls.lock().unwrap().push(TextCall {
            part,
            text: text.clone(),
            word_start,
            words: words.to_vec(),
        });
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
            .contains(&LiveStatus::Behind)
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

/// The open-part words of `part` that the text calls sent, with each call
/// applied from its start index.
fn sent_words(calls: &[TextCall], part: u32) -> Vec<LiveWord> {
    let mut words = Vec::new();
    for call in calls.iter().filter(|c| c.part == part) {
        words.truncate(call.word_start as usize);
        words.extend(call.words.iter().cloned());
    }
    words
}

fn texts(words: &[LiveWord]) -> Vec<&str> {
    words.iter().map(|w| w.text.as_str()).collect()
}

/// One click 200 ms into each second, fed in frames of one second at
/// 16 kHz, so no word is cut at the end of the audio fed so far.
fn click_each_second(r: &Run, seconds: u64) {
    let rate = 16_000;
    let said: Vec<u64> = (0..seconds).map(|s| s * 1_000 + 200).collect();
    let audio = clicks(rate, seconds * 1_000, &said);
    r.run(
        config(rate),
        always_voiced(PartPolicy::default()),
        frames(&audio, rate, 16_000, &[]),
        LiveEnd::Keep,
    );
}

#[test]
fn each_text_call_sends_only_the_new_open_part_words() {
    let r = Run::new();
    click_each_second(&r, 3);
    let calls = r.seen.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 3, "{calls:?}");
    for (n, call) in calls.iter().enumerate() {
        assert_eq!(call.part, 0);
        assert_eq!(call.word_start, n as u32, "{calls:?}");
        let start_ms = n as u64 * 1_000 + 200;
        assert_eq!(
            call.words,
            vec![LiveWord {
                start_ms,
                end_ms: start_ms + 50,
                text: format!("c{n}"),
            }]
        );
        assert_eq!(call.text.committed, format!("{} clicks", n + 1));
    }
    // The saved part has the words that the text calls sent.
    let rows = r.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].words, sent_words(&calls, 0));
}

#[test]
fn a_revised_word_sends_the_words_from_its_index() {
    let r = Run::new();
    // The seventh click changes the text of word 5.
    *r.script.revise.lock().unwrap() = Some((7, 5));
    click_each_second(&r, 8);
    let calls = r.seen.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 8, "{calls:?}");
    assert_eq!(calls[6].word_start, 5);
    assert_eq!(texts(&calls[6].words), vec!["r5", "c6"]);
    assert_eq!(calls[7].word_start, 7);
    assert_eq!(texts(&calls[7].words), vec!["c7"]);
    let rows = r.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].words, sent_words(&calls, 0));
}

#[test]
fn after_a_cut_at_the_limit_the_open_part_words_leave_out_the_replay() {
    let rate = 16_000;
    let policy = PartPolicy {
        min_ms: 1_000,
        max_ms: 2_000,
        pause_ms: 600,
        replay_ms: 500,
    };
    // The replay is 1.5 s to 2 s. The click at 1.6 s stays in the first
    // part; the click at 1.85 s goes to the second part.
    let audio = clicks(rate, 3_000, &[300, 1_600, 1_850, 2_300]);
    let r = Run::new();
    r.run(
        config(rate),
        always_voiced(policy),
        frames(&audio, rate, 320, &[]),
        LiveEnd::Keep,
    );
    let rows = r.rows();
    assert_eq!(spans(&rows), vec![(0, 2_000), (1_500, 3_000)]);
    let calls = r.seen.calls.lock().unwrap().clone();
    let second: Vec<&TextCall> = calls.iter().filter(|c| c.part == 1).collect();
    assert!(!second.is_empty());
    assert_eq!(second[0].word_start, 0, "a new part starts at index 0");
    let sent = sent_words(&calls, 1);
    let starts: Vec<u64> = sent.iter().map(|w| w.start_ms).collect();
    assert_eq!(starts, vec![1_850, 2_300], "{sent:?}");
    assert!(
        second
            .iter()
            .flat_map(|c| &c.words)
            .all(|w| w.start_ms >= 1_650),
        "{second:?}"
    );
    // A saved part keeps the words that the text calls sent, less the
    // words that its finish moved to the next part or changed.
    for row in &rows {
        let sent = sent_words(&calls, row.part);
        let row_starts: Vec<u64> = row.words.iter().map(|w| w.start_ms).collect();
        let sent_starts: Vec<u64> = sent.iter().map(|w| w.start_ms).collect();
        assert!(
            sent_starts.starts_with(&row_starts),
            "{row_starts:?} against {sent_starts:?}"
        );
        assert_eq!(
            texts(&row.words),
            texts(&sent[..row.words.len()]),
            "part {}",
            row.part
        );
    }
}

#[test]
fn a_stream_without_timed_words_sends_the_text_with_no_words() {
    let r = Run::new();
    r.script.no_open_words.store(true, Ordering::SeqCst);
    click_each_second(&r, 3);
    let calls = r.seen.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 3, "{calls:?}");
    for (n, call) in calls.iter().enumerate() {
        assert_eq!(call.word_start, 0);
        assert!(call.words.is_empty());
        assert_eq!(call.text.committed, format!("{} clicks", n + 1));
    }
    assert!(
        r.seen
            .statuses
            .lock()
            .unwrap()
            .contains(&LiveStatus::Running)
    );
    // The saved part still has its words.
    assert_eq!(r.rows()[0].words.len(), 3);
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
    assert!(statuses.contains(&LiveStatus::Behind), "{statuses:?}");
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
            speech: Some(vec![span(0, 60_000)]),
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
        self.transcribe_segments(std::slice::from_ref(&self.segment));
    }

    fn transcribe_segments(&self, segments: &[session_core::Segment]) {
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
            segments,
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
fn the_time_of_a_pause_between_two_segments_gets_no_gap_fill() {
    // R15 of the pause plan: the note paused for 15 min after the first
    // segment. The live text covers both segments, so no window runs.
    let mut g = Gaps::new(&[(0, 60_000, live_words(0, 60))]);
    g.segment.ends_in_pause = true;
    let audio = vec![0.0; 60 * 16_000];
    let second = write_segment(g.dir.path(), 1, 960_000, 16_000, &audio, false);
    assemble::assemble_segment(
        &g.dir.path().join(RECOVERY_DIR),
        g.dir.path(),
        &second,
        &Unattended,
    )
    .unwrap();
    let plan = WindowPlan {
        segment: 1,
        duration_ms: 60_000,
        voiced_ms: 60_000,
        voiced: vec![span(0, 60_000)],
        speech: Some(vec![span(0, 60_000)]),
        windows: (0..6).map(|i| span(i * 10_000, (i + 1) * 10_000)).collect(),
    };
    checkpoint::put(
        &g.r.store,
        ID,
        Stage::Transcribe,
        &checkpoint::plan_key(1),
        &plan,
    )
    .unwrap();
    let live = asr::LivePart {
        start_ms: 0,
        end_ms: 60_000,
        words: live_words(0, 60),
    };
    live::save_part(&g.r.store, ID, 1, 0, &live).unwrap();

    g.transcribe_segments(&[g.segment.clone(), second]);
    assert_eq!(g.calls(), 0);
    assert!(g.filled().is_empty());
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
fn a_gap_fill_window_that_decodes_nothing_passes_verify() {
    // Sound without words, for example background music. A retry runs the
    // same model on the same audio, so it cannot add text.
    let g = Gaps::new(&[
        (0, 20_000, live_words(0, 20)),
        (40_000, 60_000, live_words(40, 60)),
    ]);
    g.blank_on.lock().unwrap().push(1);
    g.transcribe();
    assert_eq!(g.calls(), 2);
    g.verify().unwrap();
}

#[test]
fn a_verify_failure_and_retry_keep_the_live_rows_and_redo_only_the_flagged_window() {
    let g = Gaps::new(&[
        (0, 20_000, live_words(0, 20)),
        (40_000, 60_000, live_words(40, 60)),
    ]);
    g.transcribe();
    assert_eq!(g.calls(), 2);
    // The result of the second gap-fill window is lost: the model did not
    // hear its voiced time.
    g.r.store
        .lock()
        .unwrap()
        .delete_checkpoint(ID, Stage::Transcribe, &checkpoint::window_key(0, 3))
        .unwrap();
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

#[test]
fn durable_input_recovers_dropped_notifications_and_flushes_stop_tail_once() {
    let r = Run::new();
    let dir = tempfile::tempdir().unwrap();
    let mut writer = recorder::ChunkedWriter::create(recorder::SegmentArgs {
        dir: dir.path().to_path_buf(),
        segment: 0,
        capture_started_at: 1,
        offset_ms: 7_000,
        sample_rate: 16_000,
        channels: 1,
        floor_bytes: 0,
    })
    .unwrap();
    let pcm = clicks(16_000, 61_200, &[1_000, 59_800, 60_800]);
    writer.write(&pcm).unwrap();
    writer.finish().unwrap();
    let config =
        LiveConfig::new(ID, 0, "clicker", 16_000).with_archive(dir.path().to_path_buf(), 7_000);
    // No audio notifications survive. The saved audio is authoritative.
    r.run(
        config,
        always_voiced(PartPolicy::default()),
        vec![],
        LiveEnd::Keep,
    );
    let rows = live::rows(&r.store, ID).unwrap();
    let words: Vec<_> = rows.iter().flat_map(|row| &row.words).collect();
    assert_eq!(words.len(), 3);
    assert_eq!(rows.first().unwrap().start_ms, 0);
    assert_eq!(rows.last().unwrap().end_ms, 61_200);
    assert_eq!(
        words.iter().map(|w| w.start_ms).collect::<Vec<_>>(),
        vec![1_000, 59_800, 60_800]
    );
}

#[test]
fn durable_input_emits_live_text_before_the_first_chunk_closes() {
    let r = Run::new();
    let dir = tempfile::tempdir().unwrap();
    let mut writer = recorder::ChunkedWriter::create(recorder::SegmentArgs {
        dir: dir.path().to_path_buf(),
        segment: 0,
        capture_started_at: 1,
        offset_ms: 0,
        sample_rate: 16_000,
        channels: 1,
        floor_bytes: 0,
    })
    .unwrap();
    let engine = access(&slot(None, &r.script), &r.script, CancellationToken::new());
    let workers = Workers::new(r.store.clone(), engine);
    let (tap, worker) = workers
        .start(
            LiveConfig::new(ID, 0, "clicker", 16_000).with_archive(dir.path().to_path_buf(), 0),
            always_voiced(PartPolicy::default()),
            Box::new(r.seen.clone()),
        )
        .unwrap();
    writer.write(&clicks(16_000, 1_000, &[100])).unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while r.seen.texts.load(Ordering::SeqCst) == 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    let early_text = r.seen.texts.load(Ordering::SeqCst);
    assert!(recorder::list_chunks(dir.path()).unwrap().is_empty());
    writer.finish().unwrap();
    drop(tap);
    worker.end(LiveEnd::Keep);
    let until = Instant::now() + Duration::from_secs(5);
    while workers.busy() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    workers.shutdown();
    assert!(
        early_text > 0,
        "the worker must read flushed .part audio while capture continues"
    );
}

#[test]
fn offline_asr_starts_speaker_work_without_loading_the_text_model() {
    let r = Run::new();
    let engine = access(&slot(None, &r.script), &r.script, CancellationToken::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let workers =
        Workers::new(r.store.clone(), engine).with_recognition(Arc::new(move |_, _, _, _, _| {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));
    let mut config = LiveConfig::new(ID, 0, "offline", 16_000);
    config.text_enabled = false;
    config.speakers_enabled = true;
    let (tap, worker) = workers
        .start(
            config,
            always_voiced(PartPolicy::default()),
            Box::new(r.seen.clone()),
        )
        .unwrap();
    drop(tap);
    worker.end(LiveEnd::Keep);
    let until = Instant::now() + Duration::from_secs(5);
    while workers.busy() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    workers.shutdown();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(r.script.loads.lock().unwrap().is_empty());
}

/// Start a speaker worker whose recognition hook waits until capture's end
/// is set, and records the end that it saw.
fn speaker_worker_that_waits_for_the_end(
    r: &Run,
) -> (
    Workers,
    live::LiveTap,
    live::LiveWorker,
    Arc<Mutex<Option<LiveEnd>>>,
) {
    let engine = access(&slot(None, &r.script), &r.script, CancellationToken::new());
    let saw = Arc::new(Mutex::new(None));
    let seen = Arc::clone(&saw);
    let workers = Workers::new(r.store.clone(), engine).with_recognition(Arc::new(
        move |_, _, cancel, end, _| {
            loop {
                if cancel.is_cancelled() {
                    return Err(pipeline::StageError::Cancelled);
                }
                let how = *end.lock().unwrap();
                if how.is_some() {
                    *seen.lock().unwrap() = how;
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        },
    ));
    let mut config = LiveConfig::new(ID, 0, "offline", 16_000);
    config.text_enabled = false;
    config.speakers_enabled = true;
    let (tap, worker) = workers
        .start(
            config,
            always_voiced(PartPolicy::default()),
            Box::new(r.seen.clone()),
        )
        .unwrap();
    (workers, tap, worker, saw)
}

#[test]
fn a_dropped_worker_ends_as_keep_for_every_reader() {
    let r = Run::new();
    let (workers, tap, worker, saw) = speaker_worker_that_waits_for_the_end(&r);
    // Capture closes the segment with an error: the worker drops with no
    // end().
    drop(tap);
    drop(worker);
    wait_until("the worker ends", || !workers.busy());
    assert_eq!(*saw.lock().unwrap(), Some(LiveEnd::Keep));
    workers.shutdown();
}

#[test]
fn a_dropped_worker_keeps_the_end_that_capture_sent() {
    let r = Run::new();
    let (workers, tap, worker, saw) = speaker_worker_that_waits_for_the_end(&r);
    drop(tap);
    worker.end(LiveEnd::Discard);
    wait_until("the worker ends", || !workers.busy());
    assert_eq!(*saw.lock().unwrap(), Some(LiveEnd::Discard));
    workers.shutdown();
}

#[test]
fn live_pyannote_extracts_bounded_windows_and_keeps_the_tail_for_clustering() {
    struct Analyzer(Arc<Mutex<Vec<usize>>>);
    impl diarize::SpeakerAnalyzer for Analyzer {
        fn analyze(
            &mut self,
            audio: &mut dyn diarize::AudioSource,
            _: &diarize::DiarizeRequest<'_>,
        ) -> Result<diarize::Analysis, diarize::DiarizeError> {
            let total = audio.sample_count()?;
            self.0.lock().unwrap().push(total);
            Ok(windowed_analysis(total))
        }
    }
    let r = Run::new();
    let dir = tempfile::tempdir().unwrap();
    let recovery = dir.path().join(pipeline::RECOVERY_DIR);
    let mut writer = recorder::ChunkedWriter::create(recorder::SegmentArgs {
        dir: recovery.clone(),
        segment: 0,
        capture_started_at: 1,
        offset_ms: 0,
        sample_rate: 16000,
        channels: 1,
        floor_bytes: 0,
    })
    .unwrap();
    writer.write(&vec![0.0; 31 * 16000]).unwrap();
    writer.finish().unwrap();
    let config = LiveConfig::new(ID, 0, "offline", 16000).with_archive(recovery, 0);
    let end = Arc::new(Mutex::new(Some(LiveEnd::Keep)));
    let calls = Arc::new(Mutex::new(vec![]));
    pipeline::live::features::extract(
        &r.store,
        &config,
        &mut Analyzer(calls.clone()),
        &CancellationToken::new(),
        &end,
    )
    .unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 12);
    assert!(calls.iter().all(|n| *n <= 160_000));
    assert!(calls.last().unwrap() < &160_000);
    let cached = pipeline::live::features::restore(&r.store, ID, dir.path(), 0)
        .unwrap()
        .unwrap();
    assert_eq!(cached.total, 31 * 16000);
    assert_eq!(cached.step, 31_860);
}

#[test]
fn live_text_loads_the_snapshot_artifact_not_a_catalog_alias() {
    let r = Run::new();
    let script = r.script.clone();
    let exact_calls = Arc::new(AtomicUsize::new(0));
    let calls = exact_calls.clone();
    let engine = access(&slot(None, &r.script), &r.script, CancellationToken::new())
        .with_exact_loader(Arc::new(move |spec| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Clicker {
                model: spec.config.asr_model.clone(),
                script: script.clone(),
            }))
        }));
    let workers = Workers::new(r.store.clone(), engine);
    let mut config = LiveConfig::new(ID, 0, "streams", 16000);
    config.snapshot = Some(
        models::recognition::resolve(&models::recognition::RecognitionConfig::default()).unwrap(),
    );
    let (tap, worker) = workers
        .start(
            config,
            always_voiced(PartPolicy::default()),
            Box::new(r.seen.clone()),
        )
        .unwrap();
    drop(tap);
    worker.end(LiveEnd::Keep);
    let until = Instant::now() + Duration::from_secs(5);
    while workers.busy() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    workers.shutdown();
    assert_eq!(exact_calls.load(Ordering::SeqCst), 1);
    assert!(r.script.loads.lock().unwrap().is_empty());
}

#[test]
fn durable_reader_never_feeds_a_later_chunk_across_a_gap() {
    let dir = tempfile::tempdir().unwrap();
    for offset_ms in [0, 20_000] {
        let mut writer = recorder::ChunkedWriter::create(recorder::SegmentArgs {
            dir: dir.path().to_path_buf(),
            segment: 0,
            capture_started_at: 1,
            offset_ms,
            sample_rate: 16000,
            channels: 1,
            floor_bytes: 0,
        })
        .unwrap();
        writer.write(&vec![0.1; 16000]).unwrap();
        writer.finish().unwrap();
    }
    let mut reader = live::archive::ArchiveReader::new(live::archive::Archive {
        dir: dir.path().to_path_buf(),
        segment: 0,
        offset_ms: 0,
        rate: 16000,
    });
    while reader.next_frame().unwrap().is_some() {}
    assert_eq!(reader.position(), 16000);
    assert!(reader.next_frame().unwrap().is_none());
}

#[test]
fn cancelling_durable_catchup_keeps_saved_parts_and_recorded_audio() {
    let r = Run::new();
    let dir = tempfile::tempdir().unwrap();
    let mut writer = recorder::ChunkedWriter::create(recorder::SegmentArgs {
        dir: dir.path().to_path_buf(),
        segment: 0,
        capture_started_at: 1,
        offset_ms: 0,
        sample_rate: 16000,
        channels: 1,
        floor_bytes: 0,
    })
    .unwrap();
    writer
        .write(&clicks(16000, 10_000, &[500, 2500, 4500, 6500, 8500]))
        .unwrap();
    writer.finish().unwrap();
    let path = recorder::list_chunks(dir.path()).unwrap()[0].path.clone();
    let before = session_core::recognition::file_digest(&path).unwrap();
    *r.seen.cancel_after.lock().unwrap() = Some((2, CancellationToken::new()));
    r.run(
        LiveConfig::new(ID, 0, "clicker", 16000).with_archive(dir.path().to_path_buf(), 0),
        always_voiced(PartPolicy {
            min_ms: 1000,
            max_ms: 2000,
            pause_ms: 600,
            replay_ms: 500,
        }),
        vec![],
        LiveEnd::Keep,
    );
    assert_eq!(r.rows().len(), 2);
    assert!(r.rows().last().unwrap().end_ms < 10_000);
    assert_eq!(
        session_core::recognition::file_digest(&path).unwrap(),
        before
    );
    let proof: live::archive::SourceProof =
        checkpoint::get(&r.store, ID, Stage::Transcribe, &live::archive::key(0))
            .unwrap()
            .unwrap();
    assert!(proof.samples < 160_000);
}

#[test]
fn durable_segments_keep_local_word_times_across_pause_and_device_rate_changes() {
    let r = Run::new();
    let dir = tempfile::tempdir().unwrap();
    let mut before = vec![];
    for (segment, offset, rate) in [(0, 0, 48000), (1, 30_000, 16000)] {
        let mut writer = recorder::ChunkedWriter::create(recorder::SegmentArgs {
            dir: dir.path().to_path_buf(),
            segment,
            capture_started_at: 1 + u64::from(segment),
            offset_ms: offset,
            sample_rate: rate,
            channels: 1,
            floor_bytes: 0,
        })
        .unwrap();
        writer.write(&clicks(rate, 2000, &[500, 1500])).unwrap();
        writer.finish().unwrap();
        let config = LiveConfig::new(ID, segment, "clicker", rate)
            .with_archive(dir.path().to_path_buf(), offset);
        r.run(
            config,
            always_voiced(PartPolicy::default()),
            vec![],
            LiveEnd::Keep,
        );
        let rows = live::segment_rows(&r.store, ID, segment).unwrap();
        let starts: Vec<_> = rows
            .iter()
            .flat_map(|r| &r.words)
            .map(|w| w.start_ms)
            .collect();
        assert_eq!(starts.len(), 2);
        assert!(near(starts[0], 500, 10));
        assert!(near(starts[1], 1500, 10));
        if segment == 0 {
            before = rows;
        }
    }
    assert_eq!(live::segment_rows(&r.store, ID, 0).unwrap(), before);
}

#[test]
fn after_stop_reuses_verified_pyannote_windows_after_live_interruption() {
    struct Analyzer {
        calls: Arc<Mutex<Vec<usize>>>,
        cancel: Option<CancellationToken>,
    }
    impl diarize::SpeakerAnalyzer for Analyzer {
        fn analyze(
            &mut self,
            audio: &mut dyn diarize::AudioSource,
            _: &diarize::DiarizeRequest<'_>,
        ) -> Result<diarize::Analysis, diarize::DiarizeError> {
            let total = audio.sample_count()?;
            let mut calls = self.calls.lock().unwrap();
            calls.push(total);
            if calls.len() == 2
                && let Some(cancel) = &self.cancel
            {
                cancel.cancel();
            }
            Ok(windowed_analysis(total))
        }
    }
    let r = Run::new();
    let dir = tempfile::tempdir().unwrap();
    let audio: Vec<f32> = (0..31 * 16000)
        .map(|n| ((n % 401) as f32 - 200.0) / 1024.0)
        .collect();
    let segment = write_segment(dir.path(), 0, 0, 16000, &audio, false);
    let config =
        LiveConfig::new(ID, 0, "offline", 16000).with_archive(dir.path().join(RECOVERY_DIR), 0);
    let calls = Arc::new(Mutex::new(vec![]));
    let cancel = CancellationToken::new();
    let outcome = pipeline::live::features::extract(
        &r.store,
        &config,
        &mut Analyzer {
            calls: calls.clone(),
            cancel: Some(cancel.clone()),
        },
        &cancel,
        &Arc::new(Mutex::new(Some(LiveEnd::Keep))),
    );
    assert!(matches!(outcome, Err(pipeline::StageError::Cancelled)));
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert!(
        pipeline::live::features::restore(&r.store, ID, dir.path(), 0)
            .unwrap()
            .is_none()
    );
    assemble::assemble_segment(
        &dir.path().join(RECOVERY_DIR),
        dir.path(),
        &segment,
        &Unattended,
    )
    .unwrap();
    let processed: Vec<f32> = assemble::open_proc(dir.path(), &segment)
        .unwrap()
        .into_samples::<f32>()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        processed, audio,
        "initial Assemble keeps raw-derived mono PCM beside the Opus archive"
    );
    let mut seen = Vec::new();
    let mut sink = |work| seen.push(work);
    let mut speakers = pipeline::progress::SpeakerMeter::new(&mut sink);
    speakers.plan(pipeline::diarize::segment_units(&segment), 0);
    pipeline::diarize::run(
        &r.store,
        ID,
        dir.path(),
        std::slice::from_ref(&segment),
        None,
        &mut Analyzer {
            calls: calls.clone(),
            cancel: None,
        },
        &Unattended,
        &mut speakers,
    )
    .unwrap();
    // Completing the live features is measured work (U2).
    let percents: Vec<u8> = seen.iter().filter_map(|w| w.percent).collect();
    assert!(
        percents.windows(2).all(|pair| pair[0] <= pair[1]),
        "{percents:?}"
    );
    assert!(
        seen.iter()
            .filter(|w| w.measuring)
            .filter_map(|w| w.percent)
            .filter(|p| (1..99).contains(p))
            .count()
            >= 5,
        "the missing windows advance the value: {seen:?}"
    );
    assert_eq!(
        seen.last(),
        Some(&pipeline::progress::SpeakerWork {
            percent: Some(99),
            measuring: false
        }),
        "clustering is active work below 100"
    );
    let calls = calls.lock().unwrap();
    assert_eq!(
        calls.len(),
        12,
        "reuse two verified windows and compute only the ten missing windows"
    );
    assert!(calls.iter().all(|count| *count <= 160_000));
    assert!(
        checkpoint::get::<Vec<diarize::Turn>>(
            &r.store,
            ID,
            Stage::Diarize,
            pipeline::diarize::TURNS
        )
        .unwrap()
        .is_some()
    );
}

/// An analysis of `total` samples with the layout that `analyze()` makes:
/// one window per 31 860-sample hop, each with one embedding.
fn windowed_analysis(total: usize) -> diarize::Analysis {
    let step = 31_860;
    let windows = match total {
        0 => 0,
        _ => total.saturating_sub(diarize::WINDOW_SAMPLES).div_ceil(step) + 1,
    };
    diarize::Analysis {
        total,
        step,
        activities: vec![
            pyannote_local::segmentation::WindowActivity {
                frames: vec![[true, false, false]; 10],
            };
            windows
        ],
        embeddings: (0..windows)
            .map(|window| diarize::WindowEmbedding {
                window,
                local_speaker: 0,
                embedding: vec![1.0, 0.0],
                anchor: true,
            })
            .collect(),
    }
}

/// Records the sample count of each analysis and returns [`windowed_analysis`].
struct Windowed(Arc<Mutex<Vec<usize>>>);
impl diarize::SpeakerAnalyzer for Windowed {
    fn analyze(
        &mut self,
        audio: &mut dyn diarize::AudioSource,
        _: &diarize::DiarizeRequest<'_>,
    ) -> Result<diarize::Analysis, diarize::DiarizeError> {
        let total = audio.sample_count()?;
        self.0.lock().unwrap().push(total);
        Ok(windowed_analysis(total))
    }
}

/// Run the diarize stage on `segment` with the units of two segments, so a
/// segment that is counted two times shows as more than 50 percent.
fn diarize_and_report(
    r: &Run,
    dir: &std::path::Path,
    segment: &session_core::Segment,
    calls: &Arc<Mutex<Vec<usize>>>,
) -> Result<Vec<u8>, pipeline::StageError> {
    let mut seen = Vec::new();
    let mut sink = |work: pipeline::progress::SpeakerWork| seen.extend(work.percent);
    let mut speakers = pipeline::progress::SpeakerMeter::new(&mut sink);
    speakers.plan(2 * pipeline::diarize::segment_units(segment), 0);
    pipeline::diarize::run(
        &r.store,
        ID,
        dir,
        std::slice::from_ref(segment),
        None,
        &mut Windowed(calls.clone()),
        &Unattended,
        &mut speakers,
    )?;
    Ok(seen)
}

fn has_turns(r: &Run) -> bool {
    checkpoint::get::<Vec<diarize::Turn>>(&r.store, ID, Stage::Diarize, pipeline::diarize::TURNS)
        .unwrap()
        .is_some()
}

/// Put `u64::MAX` as the window count of the analysis file at `path`.
fn damage_window_count(path: &std::path::Path) {
    let mut raw = std::fs::read(path).unwrap();
    raw[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
    std::fs::write(path, raw).unwrap();
}

#[test]
fn a_damaged_speaker_analysis_is_analysed_again_and_counted_once() {
    let r = Run::new();
    let dir = tempfile::tempdir().unwrap();
    let segment = write_segment(dir.path(), 0, 0, 16000, &vec![0.1; 31 * 16000], false);
    assemble::assemble_segment(
        &dir.path().join(RECOVERY_DIR),
        dir.path(),
        &segment,
        &Unattended,
    )
    .unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    diarize_and_report(&r, dir.path(), &segment, &calls).unwrap();
    assert_eq!(calls.lock().unwrap().len(), 1);
    let file = diarize::analysis_path(&dir.path().join(pipeline::proc_file(&segment.audio_file)));
    damage_window_count(&file);

    let percents = diarize_and_report(&r, dir.path(), &segment, &calls).unwrap();
    assert_eq!(
        calls.lock().unwrap().len(),
        2,
        "the damaged segment is analysed again"
    );
    assert!(percents.iter().all(|p| *p <= 50), "{percents:?}");
    assert!(
        diarize::read_analysis(&file).is_ok(),
        "the new analysis replaces the file"
    );
    assert!(has_turns(&r));
}

#[test]
fn a_damaged_live_feature_window_falls_back_to_the_full_analysis() {
    let r = Run::new();
    let dir = tempfile::tempdir().unwrap();
    let segment = write_segment(dir.path(), 0, 0, 16000, &vec![0.1; 31 * 16000], false);
    let config =
        LiveConfig::new(ID, 0, "offline", 16000).with_archive(dir.path().join(RECOVERY_DIR), 0);
    let calls = Arc::new(Mutex::new(vec![]));
    pipeline::live::features::extract(
        &r.store,
        &config,
        &mut Windowed(calls.clone()),
        &CancellationToken::new(),
        &Arc::new(Mutex::new(Some(LiveEnd::Keep))),
    )
    .unwrap();
    let live_windows = calls.lock().unwrap().len();
    assert_eq!(live_windows, 12);
    let mut files: Vec<_> = std::fs::read_dir(dir.path().join("live-features"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    damage_window_count(&files[3]);
    assemble::assemble_segment(
        &dir.path().join(RECOVERY_DIR),
        dir.path(),
        &segment,
        &Unattended,
    )
    .unwrap();

    let percents = diarize_and_report(&r, dir.path(), &segment, &calls).unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(
        calls[live_windows..],
        [31 * 16000],
        "one full analysis of the segment"
    );
    assert!(percents.iter().all(|p| *p <= 50), "{percents:?}");
    assert!(has_turns(&r));
}
