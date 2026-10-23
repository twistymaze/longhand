//! The checked-in test storage folder `testdata/storage-folder/` (U10,
//! KTD13 of the safe agent edits plan). The Rust tests and the tests of the
//! skill scripts read the same folder.
//!
//! The real writers make the folder: [`generate`] writes it into a
//! temporary folder, and a test fails when the checked-in folder has other
//! files or other bytes. To write the folder again after a change to a
//! writer, run this test with `LONGHAND_UPDATE_TESTDATA=1`.
//!
//! All the times and ids are fixed, so each run writes the same bytes. The
//! in-use marker of the busy note has a fixed time, so in the checked-in
//! folder it is not fresh. A test that needs the busy note refreshes the
//! marker in a copy of the folder (see [`copy_with_fresh_marker`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock, RwLockReadGuard};

use chrono::{DateTime, FixedOffset, Local};
use library::scan::{
    Known, LibraryScan, ProblemKind, RemoteState, ScanOptions, Waiting, scan, write_marker,
};
use session_core::folder::{
    PendingType, ResultFlag, SegmentRecord, Summary, SummaryMeta, Transcript, TranscriptLine,
    fingerprint, render_notes,
};
use session_core::jobs::{Title, TitleSource};
use session_core::note_folder::{Layout, SESSION_FILE};
use session_core::note_record::NoteRecord;
use session_core::recognition::{self, ARTIFACT_FILE, ArtifactHeader, Fingerprints};
use session_save::home_write::write_atomic;
use session_save::in_use_marker;
use session_save::naming::folder_name;
use session_save::note_files::{new_session_md, write_record};
use shared_settings::file::{SharedValue, default_settings, save_settings, settings_path};
use shared_settings::prompts::{
    prompts_folder, render_prompt_file, render_prompt_file_with_title, safe_file_name, scan_prompts,
};

/// The environment variable that makes the drift test write the folder.
const UPDATE: &str = "LONGHAND_UPDATE_TESTDATA";

/// The type folder of the test storage folder.
const MEETINGS: &str = "Meetings";

/// The run id of the in-use marker of the busy note.
const BUSY_RUN: &str = "run-on-another-computer";

/// The ids of the prompts that hold an id.
const WEEKLY_REVIEW_ID: &str = "3f1c2a9e-0d4b-4c51-9e57-6a1f0c2b7d10";
const ACTION_ITEMS_ID: &str = "8b2e4f60-7a13-4d9c-b5e2-1c0d9f3a6e21";
const LONG_PROMPT_ID: &str = "c47d9e12-5b6a-4f08-8e3d-2a9b1f7c0e54";

/// The name of the conflict copy of the weekly review prompt, as a sync
/// client names it.
const WEEKLY_REVIEW_COPY: &str = "Weekly review (conflicted copy 2026-10-02).md";

/// The prompt file with no front matter, as a person writes it.
const PLAIN_PROMPT: &str = "Plain prompt.md";

/// The prompt text limit of the summarize crate, in characters. The test of
/// the prompts checks that the long prompt is over it.
const PROMPT_TEXT_LIMIT: usize = 4_000;

fn checked_in() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/storage-folder")
}

/// The tests of this file run in parallel. With `UPDATE` set, one test
/// writes the checked-in folder again, so each test that reads the folder
/// holds this lock for reading, and the writer holds it for writing.
static CHECKED_IN: RwLock<()> = RwLock::new(());

fn read_checked_in() -> RwLockReadGuard<'static, ()> {
    CHECKED_IN.read().unwrap_or_else(PoisonError::into_inner)
}

fn at(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

/// One note of the folder, before the write.
struct Note {
    note_type: Option<&'static str>,
    id: &'static str,
    started: &'static str,
    title: &'static str,
    title_source: TitleSource,
    tags: &'static [&'static str],
}

impl Note {
    fn started(&self) -> DateTime<FixedOffset> {
        at(self.started)
    }

    fn dir(&self, root: &Path) -> PathBuf {
        let parent = match self.note_type {
            Some(note_type) => root.join(note_type),
            None => root.to_path_buf(),
        };
        let dir = parent.join(folder_name(self.started(), Some(self.title)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(&self) -> NoteRecord {
        NoteRecord {
            written_title: Some(Title {
                text: self.title.to_string(),
                source: self.title_source,
            }),
            ..NoteRecord::new(self.id, self.started())
        }
    }

    fn tags(&self) -> Vec<String> {
        self.tags.iter().map(|tag| tag.to_string()).collect()
    }

    fn write_session_md(&self, dir: &Path) {
        let text = new_session_md(Some(self.title), &self.tags());
        write_atomic(dir, SESSION_FILE, text.as_bytes()).unwrap();
    }
}

/// One segment of one minute, with its audio file.
fn segment(dir: &Path, started: DateTime<FixedOffset>, audio_present: bool) -> SegmentRecord {
    let audio = "audio/segment-01.opus".to_string();
    if audio_present {
        fs::create_dir_all(dir.join("audio")).unwrap();
        // The scan only asks whether the audio file exists, so a few bytes
        // are enough.
        write_atomic(&dir.join("audio"), "segment-01.opus", b"OggS test audio").unwrap();
    }
    SegmentRecord {
        index: 0,
        wall_start: started,
        wall_end: started + chrono::TimeDelta::minutes(1),
        offset_ms: 0,
        duration_ms: 60_000,
        audio,
        channels: 1,
        ends_in_pause: false,
    }
}

fn turn(position_ms: u64, speaker: u32, text: &str) -> TranscriptLine {
    TranscriptLine::Turn {
        position_ms,
        speaker: Some(speaker),
        text: text.to_string(),
    }
}

/// Write a note with a saved result: `notes.md`, `transcript.md`,
/// `summary.md`, `session.md` and `longhand.json`, in the order of a
/// publication. Returns the note folder and the records.
fn write_result_note(
    root: &Path,
    note: &Note,
    lines: Vec<TranscriptLine>,
    summary: &str,
    edit: impl FnOnce(&Path, &str, &mut NoteRecord),
) -> PathBuf {
    let dir = note.dir(root);
    let mut record = note.record();
    record.segments = vec![segment(&dir, note.started(), true)];
    record.asr_model = Some("parakeet-unified-en-0.6b-q80".to_string());
    record.found_speakers = Some(2);

    write_atomic(
        &dir,
        "notes.md",
        render_notes("- Ask about the schedule\n").as_bytes(),
    )
    .unwrap();
    let transcript = Transcript {
        has_speakers: true,
        lines,
    }
    .render();
    // `recognition.jsonl`, when the note has one, comes before
    // `transcript.md`.
    edit(&dir, &transcript, &mut record);
    write_atomic(&dir, "transcript.md", transcript.as_bytes()).unwrap();
    let meta = SummaryMeta::Complete {
        endpoint_host: "llm.example.com".to_string(),
        model: "example-model".to_string(),
        prompt_name: Some("Weekly review".to_string()),
        prompt_id: Some(WEEKLY_REVIEW_ID.to_string()),
    };
    let summary = Summary {
        meta: meta.clone(),
        body: summary.to_string(),
    }
    .render_body();
    write_atomic(&dir, "summary.md", summary.as_bytes()).unwrap();
    record.transcript_fingerprint = Some(fingerprint(transcript.as_bytes()));
    record.summary_fingerprint = Some(fingerprint(summary.as_bytes()));
    record.summary = Some(meta);
    note.write_session_md(&dir);
    write_record(&dir, &record).unwrap();
    dir
}

/// Write the test storage folder into the empty folder `root` with the
/// real writers.
fn generate(root: &Path) {
    assert!(write_marker(root));

    // A complete note in General, with a recognition file.
    let budget = Note {
        note_type: None,
        id: "note-budget-review",
        started: "2026-10-01T09:00:00-05:00",
        title: "Budget review",
        title_source: TitleSource::Proposed,
        tags: &["Finance", "Team::Planning"],
    };
    write_result_note(
        root,
        &budget,
        vec![
            turn(1_000, 1, "We have three items on the budget today."),
            turn(9_000, 2, "The travel line is lower than last quarter."),
            TranscriptLine::Silence {
                duration_ms: 12_000,
            },
            turn(31_000, 1, "Then we keep the training line as it is."),
        ],
        "## Topics\n\n- The travel line is lower than last quarter.\n- The training line stays.",
        |dir, transcript, record| {
            let header = ArtifactHeader {
                version: recognition::ARTIFACT_VERSION,
                generation: "generation-1".to_string(),
                snapshot: serde_json::json!({ "asr_model": "parakeet-unified-en-0.6b-q80" }),
                fingerprints: Fingerprints {
                    execution: "execution-1".to_string(),
                    asr: "asr-1".to_string(),
                    speakers: "speakers-1".to_string(),
                    audio: "audio-1".to_string(),
                    preprocessing: "preprocessing-1".to_string(),
                },
                transcript: recognition::digest(transcript.as_bytes()),
                limitations: Vec::new(),
                coverage: Vec::new(),
            };
            let reference =
                recognition::write_artifact(&dir.join(ARTIFACT_FILE), &header, []).unwrap();
            record.recognition_artifact = Some(reference);
        },
    );

    // A note in a type folder whose `session.md` has the same tag two
    // times, in other cases and spaces.
    let hiring = Note {
        note_type: Some(MEETINGS),
        id: "note-hiring-sync",
        started: "2026-10-02T14:00:00-05:00",
        title: "Hiring sync",
        title_source: TitleSource::User,
        tags: &["Hiring", "hiring", "Team :: Ops", "team::ops"],
    };
    write_result_note(
        root,
        &hiring,
        vec![
            turn(2_000, 1, "Two people applied for the support role."),
            turn(8_000, 2, "We can talk to both of them next week."),
        ],
        "## Next steps\n\n- Talk to the two people who applied next week.",
        |_, _, _| {},
    );

    // A note that another computer processes: a fresh in-use marker of
    // another run.
    let busy = Note {
        note_type: None,
        id: "note-design-review",
        started: "2026-10-03T10:00:00-05:00",
        title: "Design review",
        title_source: TitleSource::Proposed,
        tags: &[],
    };
    let dir = write_result_note(
        root,
        &busy,
        vec![turn(1_000, 1, "The new screen has two buttons.")],
        "## Decisions\n\n- Keep two buttons on the new screen.",
        |_, _, _| {},
    );
    assert!(in_use_marker::write_marker(&dir, BUSY_RUN, at("2026-10-03T10:05:00-05:00")).unwrap());

    // A note whose move to the type folder waits.
    let vendor = Note {
        note_type: None,
        id: "note-vendor-call",
        started: "2026-10-04T11:00:00-05:00",
        title: "Vendor call",
        title_source: TitleSource::Proposed,
        tags: &["Vendors"],
    };
    write_result_note(
        root,
        &vendor,
        vec![turn(1_000, 1, "The new price starts in January.")],
        "## Topics\n\n- The new price starts in January.",
        |_, _, record| {
            record.pending_type = Some(PendingType {
                to: Some(MEETINGS.to_string()),
                from: None,
                by: "computer-a".to_string(),
                at: at("2026-10-04T12:00:00-05:00"),
            });
        },
    );

    // A note whose `session.md` does not parse. Its `longhand.json` is
    // valid.
    let broken = Note {
        note_type: None,
        id: "note-broken-frontmatter",
        started: "2026-10-05T15:00:00-05:00",
        title: "Broken frontmatter",
        title_source: TitleSource::Proposed,
        tags: &[],
    };
    let dir = write_result_note(
        root,
        &broken,
        vec![turn(
            1_000,
            1,
            "This note has a session file that does not parse.",
        )],
        "## Topics\n\n- A session file that does not parse.",
        |_, _, _| {},
    );
    // A person or an agent left a list open.
    write_atomic(
        &dir,
        SESSION_FILE,
        b"---\ntitle: Broken frontmatter\nlonghand_tags: [Finance, Team\n---\n",
    )
    .unwrap();

    // A note whose recording is not saved yet.
    let standup = Note {
        note_type: Some(MEETINGS),
        id: "note-standup",
        started: "2026-10-06T08:00:00-05:00",
        title: "Standup",
        title_source: TitleSource::Proposed,
        tags: &[],
    };
    let dir = standup.dir(root);
    let mut record = standup.record();
    record.result = Some(ResultFlag::NotSaved);
    record.segments = vec![segment(&dir, standup.started(), false)];
    write_atomic(&dir, "notes.md", b"- Blocked on the test server\n").unwrap();
    standup.write_session_md(&dir);
    write_record(&dir, &record).unwrap();

    // A note that never recorded: only notes.
    let idea = Note {
        note_type: None,
        id: "note-quick-idea",
        started: "2026-10-07T13:00:00-05:00",
        title: "Quick idea",
        title_source: TitleSource::User,
        tags: &["Ideas"],
    };
    let dir = idea.dir(root);
    let mut record = idea.record();
    record.result = Some(ResultFlag::NoRecording);
    write_atomic(
        &dir,
        "notes.md",
        b"Put the export button on the note page.\n",
    )
    .unwrap();
    idea.write_session_md(&dir);
    write_record(&dir, &record).unwrap();

    // A note that a newer Longhand wrote: a layout that this build does
    // not know.
    let future = Note {
        note_type: None,
        id: "note-newer-layout",
        started: "2026-10-08T16:00:00-05:00",
        title: "Newer layout",
        title_source: TitleSource::Proposed,
        tags: &[],
    };
    let dir = future.dir(root);
    let record = NoteRecord {
        layout: session_core::note_record::LAYOUT + 1,
        ..future.record()
    };
    write_atomic(&dir, "notes.md", b"Written by a newer version.\n").unwrap();
    future.write_session_md(&dir);
    write_record(&dir, &record).unwrap();

    // A note whose `longhand.json` the sync has not delivered yet.
    let retro = Note {
        note_type: None,
        id: "note-retro",
        started: "2026-10-09T09:30:00-05:00",
        title: "Retro",
        title_source: TitleSource::Proposed,
        tags: &["Team::Planning"],
    };
    let dir = retro.dir(root);
    write_atomic(&dir, "notes.md", b"- What went well\n").unwrap();
    retro.write_session_md(&dir);

    // The prompts: two with an id, a sync conflict copy of one of them,
    // one with no id, and one over the text limit.
    let prompts = prompts_folder(root);
    fs::create_dir_all(&prompts).unwrap();
    let write_prompt = |name: &str, text: String| {
        write_atomic(&prompts, &safe_file_name(name), text.as_bytes()).unwrap();
    };
    write_prompt(
        "Weekly review",
        render_prompt_file(
            WEEKLY_REVIEW_ID,
            "Weekly review",
            "List the topics, the decisions and the next steps.",
        ),
    );
    write_atomic(
        &prompts,
        WEEKLY_REVIEW_COPY,
        render_prompt_file(
            WEEKLY_REVIEW_ID,
            "Weekly review",
            "List the topics and the decisions.",
        )
        .as_bytes(),
    )
    .unwrap();
    write_prompt(
        "Action items",
        render_prompt_file_with_title(
            ACTION_ITEMS_ID,
            "Action items",
            "List each action item with its owner.",
            "Use the main action item as the title.",
        ),
    );
    write_atomic(
        &prompts,
        PLAIN_PROMPT,
        b"Write a short summary in three sentences.\n",
    )
    .unwrap();
    let sentence = "Write each topic as a short paragraph. ";
    let long_text = sentence.repeat(PROMPT_TEXT_LIMIT / sentence.len() + 1);
    write_prompt(
        "Long prompt",
        render_prompt_file(LONG_PROMPT_ID, "Long prompt", long_text.trim_end()),
    );

    // The shared settings: the defaults, with one type, a prompt for the
    // type and a custom prompt as the active prompt.
    fs::write(
        settings_path(root),
        serde_json::to_vec(&default_settings()).unwrap(),
    )
    .unwrap();
    save_settings(
        root,
        &[
            SharedValue::ActiveSummaryPrompt(WEEKLY_REVIEW_ID.to_string()),
            SharedValue::NoteTypes(vec![MEETINGS.to_string()]),
            SharedValue::NoteTypePrompts(BTreeMap::from([(
                MEETINGS.to_string(),
                ACTION_ITEMS_ID.to_string(),
            )])),
        ],
    )
    .unwrap();
}

/// Each file in `root` and in its folders, hidden files too, by its path
/// with `/` between the parts.
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path.strip_prefix(root).unwrap();
                let name = relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.insert(name, fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    if root.is_dir() {
        walk(root, root, &mut out);
    }
    out
}

/// Write the files `files` into `root`. Each file gets a new file time.
fn write_files(root: &Path, files: &BTreeMap<String, Vec<u8>>) {
    for (name, bytes) in files {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
}

/// A copy of the checked-in folder whose busy note has a fresh in-use
/// marker. The files of the copy have new file times.
fn copy_with_fresh_marker() -> tempfile::TempDir {
    let copy = tempfile::tempdir().unwrap();
    if std::env::var_os(UPDATE).is_some() {
        // The checked-in folder is about to change: use what the writers
        // make now, which is what the folder gets.
        generate(copy.path());
    } else {
        let checked_in_files = {
            let _read = read_checked_in();
            files(&checked_in())
        };
        write_files(copy.path(), &checked_in_files);
    }
    let busy = copy.path().join("2026-10-03 1000 Design review");
    assert!(in_use_marker::write_marker(&busy, BUSY_RUN, Local::now().fixed_offset()).unwrap());
    copy
}

#[test]
fn the_checked_in_folder_matches_what_the_writers_make() {
    let made = tempfile::tempdir().unwrap();
    generate(made.path());
    let expected = files(made.path());
    if std::env::var_os(UPDATE).is_some() {
        let _write = CHECKED_IN.write().unwrap_or_else(PoisonError::into_inner);
        if checked_in().exists() {
            fs::remove_dir_all(checked_in()).unwrap();
        }
        write_files(&checked_in(), &expected);
    }
    let actual = {
        let _read = read_checked_in();
        files(&checked_in())
    };
    let expected_names: BTreeSet<&String> = expected.keys().collect();
    let actual_names: BTreeSet<&String> = actual.keys().collect();
    assert_eq!(
        actual_names, expected_names,
        "testdata/storage-folder has other files than the writers make. \
         Run this test with {UPDATE}=1 to write the folder again."
    );
    for (name, bytes) in &expected {
        assert!(
            actual[name] == *bytes,
            "testdata/storage-folder/{name} differs from what the writers make. \
             Run this test with {UPDATE}=1 to write the folder again."
        );
    }
}

/// The notes of `result` by their path in the storage folder `root`.
fn notes_by_path<'a>(
    root: &Path,
    result: &'a LibraryScan,
) -> BTreeMap<String, &'a library::scan::NoteFolder> {
    result
        .notes
        .iter()
        .map(|note| {
            let path = note.path.strip_prefix(root).unwrap();
            (path.to_string_lossy().replace('\\', "/"), note)
        })
        .collect()
}

#[test]
fn the_scan_reads_each_note_of_the_test_folder_with_its_state() {
    let copy = copy_with_fresh_marker();
    let root = copy.path();
    let result = scan(
        &[root.to_path_buf()],
        &Known::default(),
        &ScanOptions::default(),
    );
    assert!(result.is_available(root));
    assert!(!result.roots[0].marker_written, "the folder has its marker");
    assert!(result.unreadable.is_empty(), "{:?}", result.unreadable);

    let notes = notes_by_path(root, &result);
    assert_eq!(
        notes.keys().map(String::as_str).collect::<Vec<_>>(),
        vec![
            "2026-10-01 0900 Budget review",
            "2026-10-03 1000 Design review",
            "2026-10-04 1100 Vendor call",
            "2026-10-07 1300 Quick idea",
            "Meetings/2026-10-02 1400 Hiring sync",
            "Meetings/2026-10-06 0800 Standup",
        ]
    );
    for note in notes.values() {
        assert_eq!(note.layout, Layout::New, "{}", note.path.display());
        assert!(
            note.waiting.is_empty(),
            "{}: {:?}",
            note.path.display(),
            note.waiting
        );
        assert!(note.conflict_copies.is_empty());
        assert!(note.duplicate_folders.is_empty());
    }

    let budget = notes["2026-10-01 0900 Budget review"];
    assert_eq!(budget.doc.note_id, "note-budget-review");
    assert_eq!(budget.note_type, None);
    assert_eq!(budget.remote, None);
    assert_eq!(budget.doc.title.as_deref(), Some("Budget review"));
    assert_eq!(budget.doc.tags, vec!["Finance", "Team::Planning"]);
    assert!(budget.doc.recognition_artifact.is_some());
    assert!(budget.doc.pending_type.is_none());

    // The tag key of each tag is in `session.md` two times: the first
    // text of each key stays.
    let hiring = notes["Meetings/2026-10-02 1400 Hiring sync"];
    assert_eq!(hiring.note_type.as_deref(), Some(MEETINGS));
    assert_eq!(hiring.doc.tags, vec!["Hiring", "Team :: Ops"]);
    assert_eq!(hiring.remote, None);

    let busy = notes["2026-10-03 1000 Design review"];
    assert_eq!(busy.remote, Some(RemoteState::InUse));

    let vendor = notes["2026-10-04 1100 Vendor call"];
    assert_eq!(vendor.remote, None);
    let pending = vendor.doc.pending_type.as_ref().expect("a type move waits");
    assert_eq!(pending.to.as_deref(), Some(MEETINGS));
    assert_eq!(pending.from, None);

    // The copy gave `longhand.json` a new file time, so the marker of the
    // recording can still be on its way.
    let standup = notes["Meetings/2026-10-06 0800 Standup"];
    assert_eq!(standup.doc.result_flag(), Some(ResultFlag::NotSaved));
    assert_eq!(standup.remote, Some(RemoteState::InUse));
    assert!(!standup.waiting.contains(&Waiting::Transcript));

    let idea = notes["2026-10-07 1300 Quick idea"];
    assert_eq!(idea.doc.result_flag(), Some(ResultFlag::NoRecording));
    assert_eq!(idea.remote, None);
    assert_eq!(
        idea.notes.as_deref(),
        Some("Put the export button on the note page.\n")
    );

    let mut invalid: Vec<(String, Option<String>, ProblemKind, &str, bool)> = result
        .invalid
        .iter()
        .map(|folder| {
            let problem = folder
                .problem
                .as_ref()
                .expect("an invalid folder has a problem");
            (
                folder
                    .path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                folder.known_id.clone(),
                problem.kind,
                problem.file,
                problem.line.is_some(),
            )
        })
        .collect();
    invalid.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        invalid,
        vec![
            (
                "2026-10-05 1500 Broken frontmatter".to_string(),
                Some("note-broken-frontmatter".to_string()),
                ProblemKind::UserFile,
                SESSION_FILE,
                true,
            ),
            (
                "2026-10-08 1600 Newer layout".to_string(),
                Some("note-newer-layout".to_string()),
                ProblemKind::NewerRecord,
                "longhand.json",
                false,
            ),
        ]
    );

    let waiting: Vec<String> = result
        .waiting_for_record
        .iter()
        .map(|folder| {
            folder
                .path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(waiting, vec!["2026-10-09 0930 Retro"]);
}

#[test]
fn the_checked_in_busy_marker_is_not_fresh() {
    // The marker has a fixed time. Only a copy with a refreshed marker
    // shows the note as busy.
    let _read = read_checked_in();
    let marker = checked_in()
        .join("2026-10-03 1000 Design review")
        .join(in_use_marker::marker_name(BUSY_RUN));
    assert!(marker.is_file());
    assert!(!in_use_marker::marker_fresh(
        &marker,
        Local::now().fixed_offset()
    ));
}

#[test]
fn the_prompts_of_the_test_folder_have_each_kind() {
    let copy = copy_with_fresh_marker();
    let scan = scan_prompts(copy.path()).unwrap();
    let by_file: BTreeMap<&str, &shared_settings::prompts::PromptFile> = scan
        .prompts
        .iter()
        .map(|prompt| (prompt.file_name.as_str(), prompt))
        .collect();
    assert_eq!(
        by_file.keys().copied().collect::<Vec<_>>(),
        vec![
            "Action items.md",
            "Long prompt.md",
            PLAIN_PROMPT,
            WEEKLY_REVIEW_COPY,
            "Weekly review.md",
        ]
    );
    assert_eq!(by_file["Weekly review.md"].id, WEEKLY_REVIEW_ID);
    assert!(!by_file["Action items.md"].title_text.is_empty());
    // The conflict copy holds the id of its original and gets an id of its
    // own.
    let copy_prompt = by_file[WEEKLY_REVIEW_COPY];
    assert_eq!(copy_prompt.id_in_file.as_deref(), Some(WEEKLY_REVIEW_ID));
    assert_ne!(copy_prompt.id, WEEKLY_REVIEW_ID);
    assert!(scan.conflicted.contains(WEEKLY_REVIEW_ID));
    assert_eq!(by_file[PLAIN_PROMPT].id_in_file, None);
    assert!(by_file["Long prompt.md"].over_limit());
    assert!(!by_file["Long prompt.md"].too_large);
    assert!(!by_file["Weekly review.md"].over_limit());
}
