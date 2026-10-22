//! The job store (KTD10): sessions, segments, notes, the processing queue and
//! mid-stage checkpoints in SQLite with WAL and `synchronous=FULL`.
//!
//! Every lifecycle change goes through [`Store::apply`], which runs the state
//! machine and persists the result together with its queue side effects in
//! one transaction.

use std::path::Path;

use chrono::{DateTime, FixedOffset};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::folder::{PendingType, SessionDoc, SpeakerCount};
use crate::recognition;
use crate::state::{Event, IllegalTransition, SessionState, Stage};
use crate::timeline::Segment;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Migration(#[from] rusqlite_migration::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Illegal(#[from] IllegalTransition),
    #[error("no session {0}")]
    NoSuchSession(String),
    #[error("recognition snapshot is already set for {0}")]
    SnapshotExists(String),
    #[error("invalid recognition output: {0}")]
    Recognition(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: String,
    pub created_at: DateTime<FixedOffset>,
    pub state: SessionState,
    pub queue_seq: Option<i64>,
    pub speaker_count: Option<SpeakerCount>,
    /// The recognition model that the session records when it starts, or at
    /// Stop for a session that started before the update (KTD2).
    pub asr_model: Option<String>,
    pub recognition_snapshot: Option<serde_json::Value>,
    pub home_path: Option<String>,
    pub title: Option<Title>,
    /// The number of speakers that processing found (KTD11).
    pub found_speakers: Option<u32>,
    /// The session transcribes during recording (R18, KTD2). `None` for a
    /// session from before migration 0004.
    pub live_transcript: Option<bool>,
    /// The model of a queued or running Transcribe Again run, the same value
    /// as [`TranscribeAgainMarks::model`]. The run uses this model, not
    /// `asr_model` (KTD14).
    pub transcribe_again_model: Option<String>,
    /// The time the note opened (R16, migration 0016), for a note that this
    /// computer started with or without a recording. The note keeps it when
    /// it records later. `None` for a note from before migration 0016, a
    /// note from another computer that has segments, and a row from
    /// [`Store::create_session`]: see [`SessionRecord::start`].
    pub started_at: Option<DateTime<FixedOffset>>,
    /// The note type: the type folder that holds the home folder (KTD1).
    /// `None` is General. The library scan sets it from the path.
    pub note_type: Option<String>,
    /// The tags of the note: `longhand_tags` of `session.md` (KTD2).
    pub tags: Vec<String>,
    /// `longhand_pending_type` of `session.md` (KTD2, KTD3).
    pub pending_type: Option<PendingType>,
}

impl SessionRecord {
    /// The start time of the note (R16): the time it opened when the store
    /// has it, else the start of its first segment in `segments`, else the
    /// time the row was made.
    pub fn start(&self, segments: &[Segment]) -> DateTime<FixedOffset> {
        self.started_at
            .or_else(|| segments.first().map(|s| s.wall_start))
            .unwrap_or(self.created_at)
    }
}

/// What this computer last wrote into a note's home folder (KTD8): the
/// [`crate::note_folder::written_fingerprint`] of its `session.md` and
/// `longhand.json` in `session`, and the fingerprint of its `notes.md`.
/// `None` until this computer writes the files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WrittenFingerprints {
    pub session: Option<String>,
    pub notes: Option<String>,
}

/// The recognition model and the live mode that a session records when it
/// starts (KTD2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingModel {
    pub asr_model: String,
    /// True only when the model streams and the Live transcript switch is on.
    pub live_transcript: bool,
}

/// A microphone that a note chose: cpal's stable device ID, with the device
/// name as its label (KTD1 of the system audio plan).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Microphone {
    pub id: String,
    pub label: String,
}

/// The capture choice of a note (KTD2 of the system audio plan). The note
/// records it at its start, and a change on the recording page updates it.
/// Restart, resume and the capture watcher read it, not the current
/// Settings. The default, "System default", is also the choice of a note
/// from an older build. The app set of the note is stored apart from it
/// ([`Store::note_apps`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureChoice {
    /// `None` is "System default".
    pub microphone: Option<Microphone>,
}

/// An app whose audio a note records (KTD7 of the app audio plan).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecordedApp {
    /// On macOS, the bundle ID of the outermost `.app`. On Windows, the
    /// lowercase executable name without `.exe`.
    pub id: String,
    /// The display name of the app.
    pub name: String,
    /// The path of the app bundle or the executable.
    pub path: String,
}

/// The marks of Transcribe Again on a session (KTD14).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscribeAgainMarks {
    /// The model of a queued or running Transcribe Again run. `None` when
    /// no run waits or runs.
    pub model: Option<String>,
    /// Why the last run failed before its save. The session kept its
    /// previous transcript and summary.
    pub error: Option<String>,
    /// Why the summary of the last run that saved was not updated. The
    /// session has the new transcript and the previous summary.
    pub summary_error: Option<String>,
}

/// A saved note as a scan of its folder gives it (KTD1). [`Store::adopt`]
/// makes a Saved row from it. The per-computer data (the live mode, the
/// capture choice and the Transcribe Again marks) is not in the folder, so
/// an adopted row has the defaults (R4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adoption {
    pub id: String,
    /// The start of the note: `started` in `session.md`.
    pub created_at: DateTime<FixedOffset>,
    pub speaker_count: Option<SpeakerCount>,
    pub segments: Vec<Segment>,
    /// The text of `notes.md`.
    pub notes: String,
    pub title: Option<Title>,
    pub asr_model: Option<String>,
    pub recognition_snapshot: Option<serde_json::Value>,
    pub found_speakers: Option<u32>,
    /// The absolute path of the note folder.
    pub home_path: String,
    /// The [`fingerprint`](crate::folder::fingerprint) of the
    /// `transcript.md` that the scan read (KTD12). `None` when the folder
    /// has no `transcript.md`.
    pub transcript_fingerprint: Option<String>,
    /// The audio files of the segments whose folder has a removal marker
    /// (KTD9 of the audio removal plan). `session.md` does not record them,
    /// so [`Adoption::from_doc`] gives none, and the scan adds them.
    pub removed_audio: Vec<String>,
    /// The note type that the path of the folder gives (KTD1).
    /// [`Adoption::from_doc`] gives none, and the scan sets it.
    pub note_type: Option<String>,
    /// `longhand_tags` of `session.md` (KTD2).
    pub tags: Vec<String>,
    /// `longhand_pending_type` of `session.md` (KTD2).
    pub pending_type: Option<PendingType>,
}

impl Adoption {
    /// The adoption of the note that `doc`, the parsed `session.md`,
    /// describes, with the text of its `notes.md`, the path of its folder
    /// and the fingerprint of its `transcript.md`.
    pub fn from_doc(
        doc: &SessionDoc,
        notes: String,
        home_path: String,
        transcript_fingerprint: Option<String>,
    ) -> Self {
        Self {
            id: doc.session_id.clone(),
            created_at: doc.started,
            speaker_count: doc.speaker_count,
            segments: doc.timeline_segments(),
            notes,
            title: doc.title_and_source(),
            asr_model: doc.asr_model.clone(),
            recognition_snapshot: doc.recognition_snapshot.clone(),
            found_speakers: doc.found_speakers,
            home_path,
            transcript_fingerprint,
            removed_audio: Vec::new(),
            note_type: None,
            tags: doc.tags.clone(),
            pending_type: doc.pending_type.clone(),
        }
    }
}

/// The stages that hold this computer's recognition results for a note:
/// the decoded audio, the transcription (window rows and live parts) and
/// the speaker analysis (KTD12).
const RECOGNITION_STAGES: [Stage; 3] = [Stage::Assemble, Stage::Transcribe, Stage::Diarize];

/// The stages whose checkpoints a Transcribe Again run replaces: every
/// stage. Assemble is one of them because its completion mark would skip
/// the stage, and the run needs the processing files that Assemble decodes
/// again from the session's folder (KTD14).
const AGAIN_STAGES: [Stage; 7] = [
    Stage::Assemble,
    Stage::Diarize,
    Stage::Transcribe,
    Stage::Join,
    Stage::Verify,
    Stage::Summarize,
    Stage::Save,
];

/// The stages whose checkpoints [`Store::start_over`] removes: each stage
/// that derives the transcript from the recorded audio.
const START_OVER_STAGES: [Stage; 5] = [
    Stage::Assemble,
    Stage::Transcribe,
    Stage::Diarize,
    Stage::Join,
    Stage::Verify,
];

/// The stage name that holds a copy of `stage`'s checkpoints from before a
/// Transcribe Again run. [`Store::checkpoints`] never reads it.
fn previous_key(stage: Stage) -> String {
    format!("previous_{}", stage_key(stage))
}

/// Where a title came from (KTD7). `session.md` records it as `proposed`
/// or `user`, the same text as the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TitleSource {
    /// Processing proposed it.
    Proposed,
    /// The user typed it. A proposal never replaces it (R31).
    User,
}

/// `longhand.json` records the title that Longhand last wrote as this
/// value (KTD5).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Title {
    pub text: String,
    pub source: TitleSource,
}

/// A folder move that a rename recorded before it renamed the home folder
/// (KTD8). The paths are absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameIntent {
    /// The home folder before the rename.
    pub from: String,
    /// The home folder after the rename.
    pub to: String,
    /// The temporary name of a change of case alone, which moves the
    /// folder in two steps: `from` to `temp`, then `temp` to `to`.
    pub temp: Option<String>,
}

/// A type change whose move this computer owes (KTD5, KTD13 of the note
/// types and tags plan). It holds no path: the move finds the home folder
/// by session id at move time. `None` as a type is General.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeMove {
    /// The new type.
    pub to: Option<String>,
    /// The type folder that the home folder was in when the user changed
    /// the type.
    pub from: Option<String>,
    /// The time of the change.
    pub changed_at: DateTime<FixedOffset>,
    /// The time that this computer took over the move of another computer
    /// (KTD13). A taken-over move waits before it moves the folder.
    pub taken_over_at: Option<DateTime<FixedOffset>>,
    /// A type rename, merge or delete made the move: the old type folder
    /// goes when it holds nothing else (KTD7).
    pub remove_empty_from: bool,
    /// This computer wrote the pending type into `session.md`.
    pub field_written: bool,
}

/// A Settings edit that changes many notes (KTD7, KTD11 of the note types
/// and tags plan).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BulkEdit {
    /// A type rename. When `to` is the name of another type, the two types
    /// merge.
    RenameType { from: String, to: String },
    /// A type delete: the notes of the type go to General.
    DeleteType { name: String },
    /// A tag rename (KTD11): each note with the key of `from` gets `to`.
    /// When a note has a tag with the key of `to`, the two tags merge.
    RenameTag { from: String, to: String },
    /// A tag delete (KTD11): each note loses the tags with the key of
    /// `name`.
    DeleteTag { name: String },
}

impl BulkEdit {
    /// The edit changes tags. A tag edit has no shared-settings part.
    pub fn is_tag_edit(&self) -> bool {
        matches!(self, Self::RenameTag { .. } | Self::DeleteTag { .. })
    }
}

/// A [`BulkEdit`] that is not done yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenBulkEdit {
    pub id: i64,
    pub edit: BulkEdit,
    /// The notes that the edit still changes: the notes that matched when
    /// the user made the edit, less the notes that are done. A note that
    /// matches only after that keeps its tag or type: that is the user's
    /// new choice.
    pub notes: Vec<String>,
    /// The shared settings have the change.
    pub shared_saved: bool,
}

/// The JSON of the `edit` column of `bulk_edits`: the edit and its notes.
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredBulkEdit {
    #[serde(flatten)]
    edit: BulkEdit,
    #[serde(default)]
    notes: Vec<String>,
}

/// The step of one note in a Move (KTD9). The Move records each step
/// before it takes the next one. The paths are absolute.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum MoveStep {
    /// The note is in the old folder only.
    Pending,
    /// The folders are on different volumes: `staging`, in the new folder,
    /// holds a verified copy of `source`. `replaces` is a placed copy that
    /// differed from the source: it goes before the staged copy takes its
    /// place.
    Copied {
        source: std::path::PathBuf,
        staging: std::path::PathBuf,
        replaces: Option<std::path::PathBuf>,
    },
    /// The note is at `path` in the new folder. `source` is the note in the
    /// old folder when the Move copied it; `None` after a rename.
    Placed {
        source: Option<std::path::PathBuf>,
        path: std::path::PathBuf,
    },
    /// The store's home path is `path`. `source` is removed next.
    Homed {
        source: Option<std::path::PathBuf>,
        path: std::path::PathBuf,
    },
    Done,
}

/// Why a Move is paused (R10). Retry continues it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MovePause {
    /// The disk of `folder` is full.
    DiskFull { folder: String },
    /// `folder` is not reachable, for example on a disk that is not
    /// connected.
    NotReachable { folder: String },
    /// Another error stopped the Move.
    Failed { message: String },
}

/// The open Move journal (KTD9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveJournal {
    pub old_folder: std::path::PathBuf,
    pub new_folder: std::path::PathBuf,
    /// `None` until the Move finds whether the folders are on different
    /// volumes.
    pub cross_volume: Option<bool>,
    /// Set while the Move is paused.
    pub pause: Option<MovePause>,
}

/// How long a claim's heartbeat may be quiet before its owner, when it
/// cannot be shown dead, is taken to be gone (U17). The runner beats every
/// 10 s.
pub const CLAIM_STALE_AFTER: chrono::TimeDelta = chrono::TimeDelta::seconds(30);

/// How many runs of this computer the store keeps ([`Store::add_run`]). A
/// marker of an earlier run is fresh for 30 minutes after that run ends,
/// so only the runs of the last few launches can have a fresh marker. The
/// marker of an older run counts as work on another computer.
pub const RUNS_KEPT: usize = 16;

/// What launch knows of the other runs that hold claims.
#[derive(Debug, Clone, Copy)]
pub enum Liveness {
    /// This process holds the instance lock: no other run is alive.
    OnlyInstance,
    /// The lock could not be taken; judge each claim by its heartbeat.
    Unknown { now: DateTime<FixedOffset> },
}

pub struct Store {
    pub(crate) conn: Connection,
}

pub(crate) fn stage_key(stage: Stage) -> String {
    serde_json::to_value(stage)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .expect("stage serializes to a string")
}

/// Put one checkpoint row on `conn`. A caller can give a transaction.
pub(crate) fn put_checkpoint_in(
    conn: &Connection,
    session_id: &str,
    stage: Stage,
    key: &str,
    payload: &str,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO checkpoints (session_id, stage, key, payload) \
         VALUES (?1, ?2, ?3, ?4)",
        params![session_id, stage_key(stage), key, payload],
    )?;
    Ok(())
}

/// The `sessions` columns that [`read_session`] reads, in its order.
const SESSION_COLUMNS: &str = "id, created_at, state, queue_seq, speaker_count, asr_model, \
     home_path, title, title_source, found_speakers, live_transcript, transcribe_again_model, recognition_snapshot, \
     started_at, note_type, tags, pending_type";

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        crate::schema::migrations().to_latest(&mut conn)?;
        crate::schema::record_applied_migrations(&conn, &crate::schema::migration_hashes())?;
        Ok(Self { conn })
    }

    pub fn journal_mode(&self) -> Result<String> {
        Ok(self
            .conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))?)
    }

    /// 2 is FULL.
    pub fn synchronous(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "synchronous", |r| r.get(0))?)
    }

    // ------------------------------------------------------------ sessions

    /// Create a capturing session with an empty notes document and no
    /// recorded model.
    pub fn create_session(
        &mut self,
        id: &str,
        created_at: DateTime<FixedOffset>,
        speaker_count: Option<SpeakerCount>,
    ) -> Result<()> {
        self.insert_session(
            id,
            created_at,
            speaker_count,
            None,
            &CaptureChoice::default(),
            None,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_session(
        &mut self,
        id: &str,
        created_at: DateTime<FixedOffset>,
        speaker_count: Option<SpeakerCount>,
        model: Option<&RecordingModel>,
        capture: &CaptureChoice,
        snapshot: Option<&serde_json::Value>,
        opened: bool,
    ) -> Result<()> {
        // `opened`: `created_at` is the time the note opened, and the note
        // keeps it as its start time (R16).
        self.conn.execute(
            "INSERT INTO sessions (id, created_at, state, speaker_count, asr_model, live_transcript,
               mic_id, mic_label, recognition_snapshot, started_here, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, CASE WHEN ?10 THEN ?2 END)",
            params![
                id,
                created_at.to_rfc3339(),
                serde_json::to_string(&SessionState::Capturing)?,
                speaker_count
                    .map(|c| serde_json::to_string(&c))
                    .transpose()?,
                model.map(|m| m.asr_model.as_str()),
                model.map(|m| m.live_transcript),
                capture.microphone.as_ref().map(|m| m.id.as_str()),
                capture.microphone.as_ref().map(|m| m.label.as_str()),
                snapshot.map(serde_json::to_string).transpose()?,
                opened,
            ],
        )?;
        Ok(())
    }

    /// Make the row of a note that opens with no recording (KTD2): a Saved
    /// row with zero segments, an empty notes document, `opened_at` as its
    /// start time (R16), and the mark that this computer started it (KTD8),
    /// in one INSERT. `home_path` is the note's home folder, or `None`
    /// until the folder is written: the row is busy until then
    /// ([`busy`]). The row does not go through the state machine and is not
    /// queued. Record into it is [`Event::UserRecord`].
    pub fn create_notes_only(
        &mut self,
        id: &str,
        opened_at: DateTime<FixedOffset>,
        home_path: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, created_at, state, home_path, started_at, started_here)
             VALUES (?1, ?2, ?3, ?4, ?2, 1)",
            params![
                id,
                opened_at.to_rfc3339(),
                serde_json::to_string(&SessionState::Saved)?,
                home_path,
            ],
        )?;
        Ok(())
    }

    /// Whether this computer started the note (KTD8). False for a note
    /// from another computer, and for a note from before migration 0016.
    pub fn started_here(&self, id: &str) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT started_here FROM sessions WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NoSuchSession(id.to_string()))
    }

    /// Start a new session from the shortcut (R6c): any session still capturing
    /// or interrupted is stopped and queued first (or discarded if it kept no
    /// audio), then the new one is created with `model` and `capture` (KTD2).
    pub fn start_session(
        &mut self,
        id: &str,
        now: DateTime<FixedOffset>,
        speaker_count: Option<SpeakerCount>,
        model: &RecordingModel,
        capture: &CaptureChoice,
    ) -> Result<Vec<String>> {
        self.start_session_with_recognition(id, now, speaker_count, model, capture, None, &[])
    }

    /// Store the recognition configuration with the new session in the same
    /// INSERT. The note records no summary prompt: the type of the note
    /// when the summary runs chooses it (R4 of the type prompt plan).
    ///
    /// The notes in `unregistered` keep audio that has no segment row. This
    /// start does not stop them: they stay as they are, and a later launch
    /// registers their audio (KTD3 of the capture recovery plan).
    #[allow(clippy::too_many_arguments)]
    pub fn start_session_with_recognition(
        &mut self,
        id: &str,
        now: DateTime<FixedOffset>,
        speaker_count: Option<SpeakerCount>,
        model: &RecordingModel,
        capture: &CaptureChoice,
        snapshot: Option<&serde_json::Value>,
        unregistered: &[String],
    ) -> Result<Vec<String>> {
        let mut stopped = Vec::new();
        for name in ["capturing", "interrupted"] {
            for record in self.sessions_in(name)? {
                if unregistered.contains(&record.id) {
                    continue;
                }
                let retained_audio = !self.segments(&record.id)?.is_empty();
                self.apply(
                    &record.id,
                    &Event::NewSessionStarted {
                        at: now,
                        retained_audio,
                    },
                )?;
                stopped.push(record.id);
            }
        }
        self.insert_session(id, now, speaker_count, Some(model), capture, snapshot, true)?;
        Ok(stopped)
    }

    pub fn recognition_snapshot(&self, id: &str) -> Result<Option<serde_json::Value>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT recognition_snapshot FROM sessions WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NoSuchSession(id.into()))?;
        Ok(value.map(|v| serde_json::from_str(&v)).transpose()?)
    }

    pub fn result_state(&self, id: &str, key: &str) -> Result<Option<serde_json::Value>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM result_state WHERE session_id = ?1 AND key = ?2",
                params![id, key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value.map(|v| serde_json::from_str(&v)).transpose()?)
    }

    pub fn set_result_state(&self, id: &str, key: &str, value: &serde_json::Value) -> Result<()> {
        self.conn.execute(
            "INSERT INTO result_state(session_id, key, value) VALUES (?1, ?2, ?3)
            ON CONFLICT(session_id, key) DO UPDATE SET value = excluded.value",
            params![id, key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }

    pub fn clear_result_state(&self, id: &str, key: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM result_state WHERE session_id = ?1 AND key = ?2",
            params![id, key],
        )?;
        Ok(())
    }

    pub fn promote_result_snapshot(
        &self,
        id: &str,
        snapshot: Option<&serde_json::Value>,
    ) -> Result<()> {
        if let Some(snapshot) = snapshot {
            self.conn.execute(
                "UPDATE sessions SET recognition_snapshot = ?2 WHERE id = ?1",
                params![id, serde_json::to_string(snapshot)?],
            )?;
        }
        for key in ["replacement-base", "pending-snapshot"] {
            self.clear_result_state(id, key)?;
        }
        Ok(())
    }

    /// Fill a legacy snapshot once. An existing snapshot is immutable.
    pub fn set_recognition_snapshot(&self, id: &str, value: &serde_json::Value) -> Result<()> {
        let written = self.conn.execute(
            "UPDATE sessions SET recognition_snapshot = ?2 WHERE id = ?1 AND recognition_snapshot IS NULL",
            params![id, serde_json::to_string(value)?],
        )?;
        if written == 0 {
            if self.session(id)?.is_none() {
                return Err(StoreError::NoSuchSession(id.into()));
            }
            return Err(StoreError::SnapshotExists(id.into()));
        }
        Ok(())
    }

    /// The capture choice of the session (KTD2). A session with no stored
    /// choice, from an older build, gets the default.
    pub fn capture_choice(&self, id: &str) -> Result<CaptureChoice> {
        self.conn
            .query_row(
                "SELECT mic_id, mic_label FROM sessions WHERE id = ?1",
                [id],
                |r| {
                    let mic_id: Option<String> = r.get(0)?;
                    let mic_label: Option<String> = r.get(1)?;
                    Ok(CaptureChoice {
                        microphone: mic_id.map(|id| Microphone {
                            id,
                            label: mic_label.unwrap_or_default(),
                        }),
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NoSuchSession(id.to_string()))
    }

    /// Replace the capture choice of the session: a change on the
    /// recording page (KTD2).
    pub fn set_capture_choice(&self, id: &str, capture: &CaptureChoice) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE sessions SET mic_id = ?2, mic_label = ?3 WHERE id = ?1",
            params![
                id,
                capture.microphone.as_ref().map(|m| m.id.as_str()),
                capture.microphone.as_ref().map(|m| m.label.as_str()),
            ],
        )?;
        if changed == 0 {
            return Err(StoreError::NoSuchSession(id.to_string()));
        }
        Ok(())
    }

    /// The app set of the session (R21, KTD7). A session with no stored set,
    /// from an older build, has an empty set.
    pub fn note_apps(&self, id: &str) -> Result<Vec<RecordedApp>> {
        let text: Option<String> = self
            .conn
            .query_row("SELECT note_apps FROM sessions WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| StoreError::NoSuchSession(id.to_string()))?;
        match text {
            Some(text) => Ok(serde_json::from_str(&text)?),
            None => Ok(Vec::new()),
        }
    }

    /// Replace the app set of the session: the copy of Settings at the start
    /// of a note, or a change in the note menu (R10, R21).
    pub fn set_note_apps(&self, id: &str, apps: &[RecordedApp]) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE sessions SET note_apps = ?2 WHERE id = ?1",
            params![id, serde_json::to_string(apps)?],
        )?;
        if changed == 0 {
            return Err(StoreError::NoSuchSession(id.to_string()));
        }
        Ok(())
    }

    /// Queue every interruption left untouched too long (R5e).
    pub fn tick(&mut self, now: DateTime<FixedOffset>) -> Result<Vec<String>> {
        let mut queued = Vec::new();
        for record in self.sessions_in("interrupted")? {
            if record.state.auto_queue_due(now) {
                let retained_audio = !self.segments(&record.id)?.is_empty();
                self.apply(
                    &record.id,
                    &Event::Tick {
                        now,
                        retained_audio,
                    },
                )?;
                queued.push(record.id);
            }
        }
        Ok(queued)
    }

    pub fn session(&self, id: &str) -> Result<Option<SessionRecord>> {
        self.conn
            .query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id = ?1"),
                [id],
                read_session,
            )
            .optional()?
            .transpose()
    }

    fn require(&self, id: &str) -> Result<SessionRecord> {
        self.session(id)?
            .ok_or_else(|| StoreError::NoSuchSession(id.to_string()))
    }

    pub fn set_speaker_count(&self, id: &str, count: Option<SpeakerCount>) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET speaker_count = ?2 WHERE id = ?1",
            params![id, count.map(|c| serde_json::to_string(&c)).transpose()?],
        )?;
        Ok(())
    }

    /// Record the recognition model of the session.
    pub fn set_asr_model(&self, id: &str, model: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET asr_model = ?2 WHERE id = ?1",
            params![id, model],
        )?;
        Ok(())
    }

    /// Record the model and the live mode of the session. A resume uses
    /// this when the session's model is no longer downloaded (KTD2).
    pub fn set_recording_model(&self, id: &str, model: &RecordingModel) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE sessions SET asr_model = ?2, live_transcript = ?3 WHERE id = ?1",
            params![id, model.asr_model, model.live_transcript],
        )?;
        if changed == 0 {
            return Err(StoreError::NoSuchSession(id.to_string()));
        }
        Ok(())
    }

    /// Record `model` for a session that has no recorded model: one that
    /// started before the update (KTD2). Such a session did not transcribe
    /// during recording, so its live mode becomes false. Returns false and
    /// changes nothing when the session already has a model.
    pub fn record_model_if_unset(&self, id: &str, model: &str) -> Result<bool> {
        self.require(id)?;
        let changed = self.conn.execute(
            "UPDATE sessions SET asr_model = ?2, live_transcript = COALESCE(live_transcript, 0)
             WHERE id = ?1 AND asr_model IS NULL",
            params![id, model],
        )?;
        Ok(changed > 0)
    }

    pub fn set_home_path(&self, id: &str, path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET home_path = ?2 WHERE id = ?1",
            params![id, path],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------ adopt and forget

    /// Make a Saved row for a note from another computer, from its folder
    /// (KTD1): its segments, notes, title, recognition model, speakers found,
    /// home path and transcript fingerprint, in one transaction. A note with
    /// no segments keeps `created_at` as its start time (R16). The row does
    /// not go through the state machine, because the note never recorded on
    /// this computer. Returns false and changes nothing when `note.id`
    /// already has a row.
    pub fn adopt(&mut self, note: &Adoption) -> Result<bool> {
        let stereo_from = note
            .segments
            .iter()
            .find(|s| s.channels == 2)
            .map(|s| s.index);
        let tx = self.conn.transaction()?;
        let inserted = tx.execute(
            "INSERT INTO sessions (id, created_at, state, speaker_count, asr_model, home_path,
               title, title_source, notes, found_speakers, stereo_from, transcript_fingerprint, recognition_snapshot,
               started_at, note_type, tags, pending_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
               CASE WHEN ?14 THEN ?2 END, ?15, ?16, ?17)
             ON CONFLICT (id) DO NOTHING",
            params![
                note.id,
                note.created_at.to_rfc3339(),
                serde_json::to_string(&SessionState::Saved)?,
                note.speaker_count
                    .map(|c| serde_json::to_string(&c))
                    .transpose()?,
                note.asr_model,
                note.home_path,
                note.title.as_ref().map(|t| t.text.as_str()),
                note.title.as_ref().map(|t| title_source_key(t.source)),
                note.notes,
                note.found_speakers,
                stereo_from,
                note.transcript_fingerprint,
                note.recognition_snapshot.as_ref().map(serde_json::to_string).transpose()?,
                // A note with no segments starts at `started` (R16).
                note.segments.is_empty(),
                note.note_type,
                serde_json::to_string(&note.tags)?,
                note.pending_type.as_ref().map(serde_json::to_string).transpose()?,
            ],
        )?;
        if inserted == 0 {
            return Ok(false);
        }
        for segment in &note.segments {
            insert_segment(&tx, &note.id, segment)?;
        }
        update_removed_audio(&tx, &note.id, &note.removed_audio)?;
        tx.commit()?;
        Ok(true)
    }

    /// Remove the row of a saved note, with its segments, notes, checkpoints
    /// and job (KTD1, KTD6). No folder changes. Returns false and changes
    /// nothing when the note has no row, is not Saved, or has a folder
    /// rename, a Move or an early write of its home folder ([`EARLY_HOME`],
    /// KTD4) in progress: then capture, processing, a rename, the Move or
    /// the early write can still need the row. The caller checks the claims
    /// of [`Claims`] first.
    pub fn forget(&mut self, id: &str) -> Result<bool> {
        let removed = self.conn.execute(
            "DELETE FROM sessions WHERE id = ?1 AND json_extract(state, '$.state') = 'saved'
             AND NOT EXISTS (SELECT 1 FROM rename_intents WHERE session_id = ?1)
             AND NOT EXISTS (SELECT 1 FROM move_notes WHERE session_id = ?1
                 AND json_extract(step, '$.step') != 'done')
             AND NOT EXISTS (SELECT 1 FROM result_state WHERE session_id = ?1
                 AND key = ?2 AND json_extract(value, '$.step') != 'done')",
            params![id, EARLY_HOME],
        )?;
        Ok(removed > 0)
    }

    // ------------------------------------------------------------ transcript fingerprint

    /// The fingerprint of the `transcript.md` that this computer last wrote
    /// or read for the note (KTD12). `None` when none is recorded.
    pub fn transcript_fingerprint(&self, id: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT transcript_fingerprint FROM sessions WHERE id = ?1",
                [id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NoSuchSession(id.to_string()))
    }

    pub fn set_transcript_fingerprint(&self, id: &str, fingerprint: Option<&str>) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE sessions SET transcript_fingerprint = ?2 WHERE id = ?1",
            params![id, fingerprint],
        )?;
        if changed == 0 {
            return Err(StoreError::NoSuchSession(id.to_string()));
        }
        Ok(())
    }

    // ------------------------------------------------------------ last-written fingerprints

    /// The fingerprints of the `session.md` and `notes.md` that this
    /// computer last wrote for the note (KTD8).
    pub fn written_fingerprints(&self, id: &str) -> Result<WrittenFingerprints> {
        self.conn
            .query_row(
                "SELECT written_session_fingerprint, written_notes_fingerprint
                 FROM sessions WHERE id = ?1",
                [id],
                |r| {
                    Ok(WrittenFingerprints {
                        session: r.get(0)?,
                        notes: r.get(1)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NoSuchSession(id.to_string()))
    }

    /// Record the [`crate::note_folder::written_fingerprint`] of the
    /// `session.md` and `longhand.json` that this computer wrote for the
    /// note. Call it after each write of the files.
    pub fn set_written_session_fingerprint(
        &self,
        id: &str,
        fingerprint: Option<&str>,
    ) -> Result<()> {
        self.set_written_fingerprint(id, "written_session_fingerprint", fingerprint)
    }

    /// Record the fingerprint of the `notes.md` that this computer wrote
    /// for the note. Call it after each write of the file.
    pub fn set_written_notes_fingerprint(&self, id: &str, fingerprint: Option<&str>) -> Result<()> {
        self.set_written_fingerprint(id, "written_notes_fingerprint", fingerprint)
    }

    /// `column` is one of the two fixed column names above, never input.
    fn set_written_fingerprint(
        &self,
        id: &str,
        column: &'static str,
        fingerprint: Option<&str>,
    ) -> Result<()> {
        let changed = self.conn.execute(
            &format!("UPDATE sessions SET {column} = ?2 WHERE id = ?1"),
            params![id, fingerprint],
        )?;
        if changed == 0 {
            return Err(StoreError::NoSuchSession(id.to_string()));
        }
        Ok(())
    }

    /// Remove this computer's recognition results for the note: its
    /// Assemble, Transcribe and Diarize checkpoints, also the copies that a
    /// Transcribe Again run set aside (KTD12). Another computer wrote the
    /// note's `transcript.md`, and a later Resume or Transcribe Again must
    /// not bring back the older transcript (R18).
    pub fn clear_recognition_results(&mut self, id: &str) -> Result<()> {
        self.require(id)?;
        let tx = self.conn.transaction()?;
        for stage in RECOGNITION_STAGES {
            for key in [stage_key(stage), previous_key(stage)] {
                tx.execute(
                    "DELETE FROM checkpoints WHERE session_id = ?1 AND stage = ?2",
                    params![id, key],
                )?;
            }
        }
        tx.execute("DELETE FROM recognition_output WHERE session_id=?1", [id])?;
        tx.execute("DELETE FROM recognition_epochs WHERE session_id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// Remove the recognition work of a failed note, so that the next run
    /// starts again from the recorded audio. The audio, the notes, the
    /// title and the copies that a Transcribe Again run set aside stay.
    /// Summarize and Save restart because the transcript identity is gone.
    pub fn start_over(&mut self, id: &str) -> Result<()> {
        self.require(id)?;
        let tx = self.conn.transaction()?;
        for stage in START_OVER_STAGES {
            tx.execute(
                "DELETE FROM checkpoints WHERE session_id = ?1 AND stage = ?2",
                params![id, stage_key(stage)],
            )?;
        }
        tx.execute("DELETE FROM recognition_output WHERE session_id=?1", [id])?;
        tx.execute("DELETE FROM recognition_epochs WHERE session_id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    // ------------------------------------------------------------ transcribe again

    pub fn transcribe_again_marks(&self, id: &str) -> Result<TranscribeAgainMarks> {
        self.conn
            .query_row(
                "SELECT transcribe_again_model, transcribe_again_error, transcribe_again_summary_error
                 FROM sessions WHERE id = ?1",
                [id],
                |r| {
                    Ok(TranscribeAgainMarks {
                        model: r.get(0)?,
                        error: r.get(1)?,
                        summary_error: r.get(2)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NoSuchSession(id.to_string()))
    }

    /// Queue a saved session for a Transcribe Again run with `model` (R15,
    /// KTD14). `event` is the [`Event::UserTranscribeAgain`] with its guards.
    /// In one transaction: the session is queued, the checkpoints of every
    /// stage are set aside, the model is recorded and the
    /// notices of an earlier run are cleared. The session's `asr_model`
    /// stays until the run saves. A refused event changes nothing.
    pub fn transcribe_again(
        &mut self,
        id: &str,
        model: &str,
        event: &Event,
    ) -> Result<SessionState> {
        self.begin_reprocess(id, model, event, None, None)
    }

    pub fn begin_reprocess(
        &mut self,
        id: &str,
        model: &str,
        event: &Event,
        snapshot: Option<&serde_json::Value>,
        baseline: Option<&serde_json::Value>,
    ) -> Result<SessionState> {
        if !matches!(event, Event::UserTranscribeAgain { .. }) {
            return Err(StoreError::Illegal(IllegalTransition {
                state: "saved".into(),
                event: format!("{event:?}"),
            }));
        }
        let current = self.require(id)?;
        let next = current.state.transition(event)?;
        let tx = self.conn.transaction()?;
        write_state(&tx, id, &next)?;
        apply_queue_effects(&tx, id, &current.state, &next, event)?;
        for stage in AGAIN_STAGES {
            move_checkpoints(&tx, id, &stage_key(stage), &previous_key(stage))?;
        }
        tx.execute(
            "UPDATE sessions SET transcribe_again_model = ?2, transcribe_again_error = NULL,
             transcribe_again_summary_error = NULL WHERE id = ?1",
            params![id, model],
        )?;
        for (key, value) in [
            ("pending-snapshot", snapshot),
            ("replacement-base", baseline),
        ] {
            tx.execute(
                "DELETE FROM result_state WHERE session_id = ?1 AND key = ?2",
                params![id, key],
            )?;
            if let Some(value) = value {
                tx.execute(
                    "INSERT INTO result_state(session_id, key, value) VALUES (?1, ?2, ?3)",
                    params![id, key, serde_json::to_string(value)?],
                )?;
            }
        }
        tx.commit()?;
        Ok(next)
    }

    /// A Transcribe Again run failed before its save (R19): the session is
    /// saved again with the checkpoints it had before the run, and `error`
    /// is recorded for the note page. The caller removes the working
    /// directory. Refused for a session that runs no Transcribe Again.
    pub fn fail_transcribe_again(&mut self, id: &str, error: &str) -> Result<SessionState> {
        let current = self.require(id)?;
        if self.transcribe_again_marks(id)?.model.is_none() {
            return Err(StoreError::Illegal(IllegalTransition {
                state: current.state.name().to_string(),
                event: "TranscribeAgainFailed with no Transcribe Again run".into(),
            }));
        }
        let event = Event::TranscribeAgainFailed;
        let next = current.state.transition(&event)?;
        let tx = self.conn.transaction()?;
        write_state(&tx, id, &next)?;
        apply_queue_effects(&tx, id, &current.state, &next, &event)?;
        drop_run_generation(&tx, id)?;
        for stage in AGAIN_STAGES {
            restore_previous(&tx, id, stage)?;
        }
        tx.execute(
            "UPDATE sessions SET transcribe_again_model = NULL, transcribe_again_error = ?2
             WHERE id = ?1",
            params![id, error],
        )?;
        tx.execute("DELETE FROM result_state WHERE session_id = ?1 AND key IN ('pending-snapshot', 'replacement-base')", [id])?;
        tx.commit()?;
        Ok(next)
    }

    /// Called by the save stage when a save is complete, before the session
    /// becomes Saved. For a Transcribe Again run, the session's `asr_model`
    /// becomes the run's model, the checkpoints from before the run are
    /// dropped, and `summary_error` records why the previous summary stays.
    /// Every save clears the notices of an earlier run.
    pub fn finish_transcribe_again(&self, id: &str, summary_error: Option<&str>) -> Result<()> {
        let model = self.transcribe_again_marks(id)?.model;
        let tx = self.conn.unchecked_transaction()?;
        if let Some(model) = &model {
            tx.execute(
                "UPDATE sessions SET asr_model = ?2 WHERE id = ?1",
                params![id, model],
            )?;
            delete_previous(&tx, id)?;
        }
        tx.execute(
            "UPDATE sessions SET transcribe_again_model = NULL, transcribe_again_error = NULL,
             transcribe_again_summary_error = ?2 WHERE id = ?1",
            params![id, model.and(summary_error)],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The summary was made again (R19): the note has no summary from a
    /// previous transcript now.
    pub fn clear_summary_not_updated(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET transcribe_again_summary_error = NULL WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    /// Run `event` through the state machine and persist the result with its
    /// queue side effects.
    pub fn apply(&mut self, id: &str, event: &Event) -> Result<SessionState> {
        self.apply_with(id, event, |_| Ok(()))
    }

    /// [`Store::apply`], with the writes of `also` in the same transaction.
    fn apply_with(
        &mut self,
        id: &str,
        event: &Event,
        also: impl FnOnce(&Transaction) -> Result<()>,
    ) -> Result<SessionState> {
        let current = self.require(id)?;
        let next = current.state.transition(event)?;
        let tx = self.conn.transaction()?;
        write_state(&tx, id, &next)?;
        apply_queue_effects(&tx, id, &current.state, &next, event)?;
        also(&tx)?;
        // A resume puts a new segment after the last one, so the last
        // segment does not end in a pause now. An earlier segment keeps its
        // flag: a Continue followed it (KTD4 of the pause plan).
        if matches!(event, Event::UserResume { .. }) {
            tx.execute(
                "UPDATE segments SET ends_in_pause = 0
                 WHERE session_id = ?1 AND idx =
                   (SELECT idx FROM segments WHERE session_id = ?1
                    ORDER BY offset_ms DESC LIMIT 1)",
                [id],
            )?;
        }
        tx.commit()?;
        Ok(next)
    }

    /// Every session, oldest first.
    pub fn sessions(&self) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions ORDER BY created_at, rowid"
        ))?;
        let rows = stmt.query_map([], read_session)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    pub fn sessions_in(&self, state_name: &str) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions ORDER BY created_at, rowid"
        ))?;
        let rows = stmt.query_map([], read_session)?;
        let mut out = Vec::new();
        for row in rows {
            let record = row??;
            if record.state.name() == state_name {
                out.push(record);
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------ segments

    pub fn put_segment(&self, session_id: &str, segment: &Segment) -> Result<()> {
        insert_segment(&self.conn, session_id, segment)
    }

    pub fn segments(&self, session_id: &str) -> Result<Vec<Segment>> {
        let mut stmt = self.conn.prepare(
            "SELECT idx, offset_ms, duration_ms, wall_start, wall_end, audio_file, channels,
               ends_in_pause
             FROM segments WHERE session_id = ?1 ORDER BY offset_ms",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, u16>(6)?,
                r.get::<_, bool>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (index, offset, duration, start, end, audio_file, channels, ends_in_pause) = row?;
            out.push(Segment {
                index,
                offset_ms: offset as u64,
                duration_ms: duration as u64,
                wall_start: parse_time(&start)?,
                wall_end: parse_time(&end)?,
                audio_file,
                channels,
                ends_in_pause,
            });
        }
        Ok(out)
    }

    /// Mark the note's last kept segment as the end of a pause (KTD4 of the
    /// pause plan). Returns false when the note has no segment.
    pub fn mark_pause(&self, id: &str) -> Result<bool> {
        self.require(id)?;
        let changed = self.conn.execute(
            "UPDATE segments SET ends_in_pause = 1
             WHERE session_id = ?1 AND idx =
               (SELECT idx FROM segments WHERE session_id = ?1
                ORDER BY offset_ms DESC LIMIT 1)",
            [id],
        )?;
        Ok(changed > 0)
    }

    /// The audio files of the note's segments whose audio was removed: the
    /// folder has their removal markers (KTD9 of the audio removal plan).
    /// Sorted.
    pub fn removed_audio(&self, session_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT audio_file FROM segments
             WHERE session_id = ?1 AND audio_removed = 1 ORDER BY audio_file",
        )?;
        let rows = stmt.query_map([session_id], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Set the removed flag of each segment of the note whose audio file is
    /// in `audio`, and clear it for the others. The flags then match the
    /// removal markers of the note folder. Returns whether a flag changed.
    pub fn set_removed_audio(&self, session_id: &str, audio: &[String]) -> Result<bool> {
        self.require(session_id)?;
        Ok(update_removed_audio(&self.conn, session_id, audio)? > 0)
    }

    /// The index of the note's first stereo segment: the stereo mark
    /// (KTD2 of the system audio plan). `None` while every segment is mono.
    /// After the mark, every segment of the note opens stereo (KTD3).
    pub fn stereo_from(&self, id: &str) -> Result<Option<u32>> {
        self.conn
            .query_row(
                "SELECT stereo_from FROM sessions WHERE id = ?1",
                [id],
                |r| r.get::<_, Option<u32>>(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NoSuchSession(id.to_string()))
    }

    /// Set the stereo mark to `index` when the note has none: its first
    /// stereo segment is kept. Returns false when the mark was already set;
    /// it does not move.
    pub fn mark_stereo_from(&self, id: &str, index: u32) -> Result<bool> {
        self.require(id)?;
        let changed = self.conn.execute(
            "UPDATE sessions SET stereo_from = ?2 WHERE id = ?1 AND stereo_from IS NULL",
            params![id, index],
        )?;
        Ok(changed > 0)
    }

    // ------------------------------------------------------------ title

    /// Set the session's title. Returns false when the store refuses it: a
    /// proposal never replaces a title that the user set (R31), and an empty
    /// proposal is not a title. An empty title from the user clears the title
    /// and its source, so processing can propose one later (R35).
    pub fn set_title(&self, id: &str, title: &str, source: TitleSource) -> Result<bool> {
        self.require(id)?;
        let title = title.trim();
        let changed = match source {
            TitleSource::User if title.is_empty() => self.conn.execute(
                "UPDATE sessions SET title = NULL, title_source = NULL WHERE id = ?1",
                [id],
            )?,
            TitleSource::User => self.conn.execute(
                "UPDATE sessions SET title = ?2, title_source = 'user' WHERE id = ?1",
                params![id, title],
            )?,
            TitleSource::Proposed if title.is_empty() => 0,
            TitleSource::Proposed => self.conn.execute(
                "UPDATE sessions SET title = ?2, title_source = 'proposed'
                 WHERE id = ?1 AND title_source IS NOT 'user'",
                params![id, title],
            )?,
        };
        Ok(changed > 0)
    }

    pub fn title(&self, id: &str) -> Result<Option<Title>> {
        Ok(self.require(id)?.title)
    }

    // ------------------------------------------------------------ rename intent

    /// Record the folder move that a rename is about to make (KTD8). A
    /// second record for the session replaces the first.
    pub fn set_rename_intent(&self, id: &str, intent: &RenameIntent) -> Result<()> {
        self.require(id)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO rename_intents (session_id, from_path, to_path, temp_path)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, intent.from, intent.to, intent.temp],
        )?;
        Ok(())
    }

    pub fn rename_intent(&self, id: &str) -> Result<Option<RenameIntent>> {
        Ok(self
            .conn
            .query_row(
                "SELECT from_path, to_path, temp_path FROM rename_intents WHERE session_id = ?1",
                [id],
                read_intent,
            )
            .optional()?)
    }

    /// Every recorded intent, with its session id.
    pub fn rename_intents(&self) -> Result<Vec<(String, RenameIntent)>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, from_path, to_path, temp_path FROM rename_intents
             ORDER BY session_id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    RenameIntent {
                        from: r.get(1)?,
                        to: r.get(2)?,
                        temp: r.get(3)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn clear_rename_intent(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM rename_intents WHERE session_id = ?1", [id])?;
        Ok(())
    }

    // ------------------------------------------------------------ owed rename

    /// Record that this computer owes the session's folder a rename: a
    /// reconcile could not give the folder the name its title gives.
    pub fn set_rename_owed(&self, id: &str, owed: bool) -> Result<()> {
        self.require(id)?;
        if owed {
            self.conn.execute(
                "INSERT OR IGNORE INTO owed_renames (session_id) VALUES (?1)",
                [id],
            )?;
        } else {
            self.conn
                .execute("DELETE FROM owed_renames WHERE session_id = ?1", [id])?;
        }
        Ok(())
    }

    /// The ids of the sessions whose folders this computer owes a rename.
    pub fn owed_renames(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id FROM owed_renames ORDER BY session_id")?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    // ------------------------------------------------------------ type moves

    /// The ID of this computer (KTD13 of the note types and tags plan).
    pub fn computer_id(&self) -> Result<String> {
        Ok(self
            .conn
            .query_row("SELECT computer_id FROM computer WHERE id = 1", [], |r| {
                r.get(0)
            })?)
    }

    /// Record that this computer owes the session a type move (KTD5). A
    /// second record for the session replaces the first.
    pub fn set_type_move(&self, id: &str, type_move: &TypeMove) -> Result<()> {
        self.require(id)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO type_moves
             (session_id, to_type, from_type, changed_at, taken_over_at, remove_empty_from,
              field_written)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                type_move.to,
                type_move.from,
                type_move.changed_at.to_rfc3339(),
                type_move.taken_over_at.map(|at| at.to_rfc3339()),
                type_move.remove_empty_from,
                type_move.field_written,
            ],
        )?;
        Ok(())
    }

    pub fn type_move(&self, id: &str) -> Result<Option<TypeMove>> {
        Ok(self
            .type_moves_where("WHERE session_id = ?1", [id])?
            .pop()
            .map(|(_, type_move)| type_move))
    }

    /// Every type move that this computer owes, with its session id.
    pub fn type_moves(&self) -> Result<Vec<(String, TypeMove)>> {
        self.type_moves_where("", [])
    }

    fn type_moves_where<P: rusqlite::Params>(
        &self,
        filter: &str,
        values: P,
    ) -> Result<Vec<(String, TypeMove)>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT session_id, to_type, from_type, changed_at, taken_over_at, remove_empty_from,
                 field_written
             FROM type_moves {filter} ORDER BY session_id"
        ))?;
        let rows = stmt
            .query_map(values, |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, bool>(5)?,
                    r.get::<_, bool>(6)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(
                |(id, to, from, changed_at, taken_over_at, remove_empty_from, field_written)| {
                    Ok((
                        id,
                        TypeMove {
                            to,
                            from,
                            changed_at: parse_time(&changed_at)?,
                            taken_over_at: taken_over_at.as_deref().map(parse_time).transpose()?,
                            remove_empty_from,
                            field_written,
                        },
                    ))
                },
            )
            .collect()
    }

    pub fn clear_type_move(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM type_moves WHERE session_id = ?1", [id])?;
        Ok(())
    }

    // ------------------------------------------------------------ bulk edits

    /// Record a Settings edit that changes the notes `notes` (KTD7, KTD11),
    /// before the first change. Returns its id.
    pub fn add_bulk_edit(&self, edit: &BulkEdit, notes: &[String]) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO bulk_edits (edit) VALUES (?1)",
            [serde_json::to_string(&StoredBulkEdit {
                edit: edit.clone(),
                notes: notes.to_vec(),
            })?],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Record that the edit `id` still changes only the notes `notes`.
    pub fn set_bulk_edit_notes(&self, id: i64, notes: &[String]) -> Result<()> {
        let edit: String =
            self.conn
                .query_row("SELECT edit FROM bulk_edits WHERE id = ?1", [id], |r| {
                    r.get(0)
                })?;
        let mut stored: StoredBulkEdit = serde_json::from_str(&edit)?;
        stored.notes = notes.to_vec();
        self.conn.execute(
            "UPDATE bulk_edits SET edit = ?1 WHERE id = ?2",
            rusqlite::params![serde_json::to_string(&stored)?, id],
        )?;
        Ok(())
    }

    /// The edits that are not done, oldest first. An edit that this build
    /// cannot read is left out and logged.
    pub fn bulk_edits(&self) -> Result<Vec<OpenBulkEdit>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, edit, shared_saved FROM bulk_edits ORDER BY id")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, edit, shared_saved)| {
                match serde_json::from_str::<StoredBulkEdit>(&edit) {
                    Ok(StoredBulkEdit { edit, notes }) => Some(OpenBulkEdit {
                        id,
                        edit,
                        notes,
                        shared_saved,
                    }),
                    Err(e) => {
                        log::warn!("the bulk edit {id} cannot be read: {e}");
                        None
                    }
                }
            })
            .collect())
    }

    /// Record that the shared settings have the change of the edit `id`.
    pub fn set_bulk_edit_shared_saved(&self, id: i64) -> Result<()> {
        self.conn
            .execute("UPDATE bulk_edits SET shared_saved = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The edit `id` is done.
    pub fn remove_bulk_edit(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM bulk_edits WHERE id = ?1", [id])?;
        Ok(())
    }

    // ------------------------------------------------------------ move journal

    /// Open the Move journal from `old` to `new` (KTD9), with a Pending row
    /// for each note whose home folder is in `old`, directly or in a type
    /// folder (KTD14 of the note types plan). Returns false and changes
    /// nothing when a journal is open.
    pub fn open_move(&mut self, old: &Path, new: &Path) -> Result<bool> {
        let ids: Vec<String> = self
            .sessions()?
            .into_iter()
            .filter(|r| {
                r.home_path
                    .as_deref()
                    .is_some_and(|home| crate::folder::note_type_in(old, Path::new(home)).is_some())
            })
            .map(|r| r.id)
            .collect();
        let tx = self.conn.transaction()?;
        let opened = tx.execute(
            "INSERT INTO move_journal (id, old_folder, new_folder, state)
             VALUES (1, ?1, ?2, 'running') ON CONFLICT (id) DO NOTHING",
            params![old.to_string_lossy(), new.to_string_lossy()],
        )?;
        if opened == 0 {
            return Ok(false);
        }
        for id in ids {
            insert_pending_move_note(&tx, &id)?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// The open Move journal, or `None` when no Move runs or is paused.
    pub fn move_journal(&self) -> Result<Option<MoveJournal>> {
        let row = self
            .conn
            .query_row(
                "SELECT old_folder, new_folder, cross_volume, pause FROM move_journal WHERE id = 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<bool>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((old, new, cross_volume, pause)) = row else {
            return Ok(None);
        };
        Ok(Some(MoveJournal {
            old_folder: old.into(),
            new_folder: new.into(),
            cross_volume,
            pause: pause.as_deref().map(serde_json::from_str).transpose()?,
        }))
    }

    /// Record whether the two folders of the Move are on different volumes.
    pub fn set_move_cross_volume(&self, cross: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE move_journal SET cross_volume = ?1 WHERE id = 1",
            [cross],
        )?;
        Ok(())
    }

    /// Pause the Move with its cause, or continue it with `None` (Retry).
    pub fn set_move_pause(&self, pause: Option<&MovePause>) -> Result<()> {
        let (state, pause) = match pause {
            Some(pause) => ("paused", Some(serde_json::to_string(pause)?)),
            None => ("running", None),
        };
        self.conn.execute(
            "UPDATE move_journal SET state = ?1, pause = ?2 WHERE id = 1",
            params![state, pause],
        )?;
        Ok(())
    }

    /// Every note of the Move with its step, in id order.
    pub fn move_notes(&self) -> Result<Vec<(String, MoveStep)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id, step FROM move_notes ORDER BY session_id")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, step)| Ok((id, serde_json::from_str(&step)?)))
            .collect()
    }

    /// The step of `id` in the Move, or `None` when the Move has no row
    /// for it.
    pub fn move_step(&self, id: &str) -> Result<Option<MoveStep>> {
        let step = self
            .conn
            .query_row(
                "SELECT step FROM move_notes WHERE session_id = ?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(step.as_deref().map(serde_json::from_str).transpose()?)
    }

    /// The ids of the notes that the Move has not finished (KTD10). The
    /// library reconcile skips them, as it skips a rename intent.
    pub fn moving_notes(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id FROM move_notes
             WHERE json_extract(step, '$.step') != 'done' ORDER BY session_id",
        )?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Add a Pending row for `id` to the open Move. Returns false when the
    /// Move already has a row for it.
    pub fn add_move_note(&self, id: &str) -> Result<bool> {
        Ok(insert_pending_move_note(&self.conn, id)? > 0)
    }

    pub fn set_move_step(&self, id: &str, step: &MoveStep) -> Result<()> {
        self.conn.execute(
            "UPDATE move_notes SET step = ?2 WHERE session_id = ?1",
            params![id, serde_json::to_string(step)?],
        )?;
        Ok(())
    }

    /// Set the home path of `id` and record its step in one transaction:
    /// the home change of the Move has no window between its effect and
    /// its record.
    pub fn set_move_home(&mut self, id: &str, path: &Path, step: &MoveStep) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE sessions SET home_path = ?2 WHERE id = ?1",
            params![id, path.to_string_lossy()],
        )?;
        tx.execute(
            "UPDATE move_notes SET step = ?2 WHERE session_id = ?1",
            params![id, serde_json::to_string(step)?],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Remove the Move's row for `id`: its source is gone.
    pub fn drop_move_note(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM move_notes WHERE session_id = ?1", [id])?;
        Ok(())
    }

    /// Close the Move journal and remove its rows.
    pub fn close_move(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM move_notes", [])?;
        tx.execute("DELETE FROM move_journal", [])?;
        tx.commit()?;
        Ok(())
    }

    /// The sessions that are not Saved and whose save has put, or will put,
    /// the session in `folder`: the save state names a staging directory or
    /// a folder there, directly or in a type folder (KTD4 of the note
    /// types plan). A Move from `folder` waits for them (KTD10).
    pub fn saves_into(&self, folder: &Path) -> Result<Vec<String>> {
        #[derive(serde::Deserialize)]
        struct Paths {
            staging: Option<std::path::PathBuf>,
            path: Option<std::path::PathBuf>,
        }
        let mut ids = Vec::new();
        for record in self.sessions()? {
            if record.state == SessionState::Saved {
                continue;
            }
            let into = self
                .checkpoints(&record.id, Stage::Save)?
                .into_iter()
                .filter(|(key, _)| key == SAVE_STATE)
                .filter_map(|(_, payload)| serde_json::from_str::<Paths>(&payload).ok())
                .any(|p| {
                    p.staging
                        .iter()
                        .chain(&p.path)
                        .any(|path| crate::folder::note_type_in(folder, path).is_some())
                });
            if into {
                ids.push(record.id);
            }
        }
        Ok(ids)
    }

    // ------------------------------------------------------------ notes

    /// The session's notes: one markdown document (KTD5).
    pub fn notes(&self, id: &str) -> Result<String> {
        self.conn
            .query_row("SELECT notes FROM sessions WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| StoreError::NoSuchSession(id.to_string()))
    }

    /// Replace the session's notes document with `document`, unchanged.
    pub fn set_notes(&self, id: &str, document: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE sessions SET notes = ?2 WHERE id = ?1",
            params![id, document],
        )?;
        if changed == 0 {
            return Err(StoreError::NoSuchSession(id.to_string()));
        }
        Ok(())
    }

    // ------------------------------------------------------------ note type and tags

    /// Record the note type that the path of the home folder gives (KTD1).
    pub fn set_note_type(&self, id: &str, note_type: Option<&str>) -> Result<()> {
        self.update_one(
            id,
            "UPDATE sessions SET note_type = ?2 WHERE id = ?1",
            note_type,
        )
    }

    /// Record the tags of `session.md` (KTD2).
    pub fn set_tags(&self, id: &str, tags: &[String]) -> Result<()> {
        let tags = serde_json::to_string(tags)?;
        self.update_one(id, "UPDATE sessions SET tags = ?2 WHERE id = ?1", tags)
    }

    /// Record the pending type of `session.md` (KTD2).
    pub fn set_pending_type(&self, id: &str, pending: Option<&PendingType>) -> Result<()> {
        let pending = pending.map(serde_json::to_string).transpose()?;
        self.update_one(
            id,
            "UPDATE sessions SET pending_type = ?2 WHERE id = ?1",
            pending,
        )
    }

    /// Run the update `sql` with `id` as `?1` and `value` as `?2`. An `id`
    /// with no row is an error.
    fn update_one(&self, id: &str, sql: &str, value: impl rusqlite::ToSql) -> Result<()> {
        if self.conn.execute(sql, params![id, value])? == 0 {
            return Err(StoreError::NoSuchSession(id.to_string()));
        }
        Ok(())
    }

    // ------------------------------------------------------------ speakers

    /// Record the number of speakers that processing found (KTD11). It is
    /// separate from the user's bound on the speaker count (R36).
    pub fn set_found_speakers(&self, id: &str, count: Option<u32>) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE sessions SET found_speakers = ?2 WHERE id = ?1",
            params![id, count],
        )?;
        if changed == 0 {
            return Err(StoreError::NoSuchSession(id.to_string()));
        }
        Ok(())
    }

    pub fn found_speakers(&self, id: &str) -> Result<Option<u32>> {
        Ok(self.require(id)?.found_speakers)
    }

    // ------------------------------------------------------------ queue

    /// Take the oldest queued session (R20c) and claim it for `owner`.
    pub fn dequeue(&mut self, owner: &str, now: DateTime<FixedOffset>) -> Result<Option<String>> {
        let next: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM sessions WHERE queue_seq IS NOT NULL
                 AND json_extract(state, '$.state') = 'queued'
                 ORDER BY queue_seq LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        match next {
            Some(id) => self.dequeue_session(&id, owner, now),
            None => Ok(None),
        }
    }

    /// Take the queued session `id` and claim it for `owner`, whatever its
    /// place in the queue: the runner takes a later session when the head
    /// session waits for its model (KTD13). `None` when `id` is not queued.
    pub fn dequeue_session(
        &mut self,
        id: &str,
        owner: &str,
        now: DateTime<FixedOffset>,
    ) -> Result<Option<String>> {
        let queued: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM sessions WHERE id = ?1 AND queue_seq IS NOT NULL
                 AND json_extract(state, '$.state') = 'queued'",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = queued else {
            return Ok(None);
        };
        // The state and the claim are one transaction. A Processing note
        // with no claim is never taken back at launch.
        self.apply_with(&id, &Event::Dequeued, |tx| {
            tx.execute(
                "UPDATE jobs SET claim_owner = ?2, claimed_at = ?3, heartbeat_at = ?3 WHERE session_id = ?1",
                params![id, owner, now.to_rfc3339()],
            )?;
            Ok(())
        })?;
        Ok(Some(id))
    }

    /// Queued session ids, head first.
    pub fn queue(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM sessions WHERE queue_seq IS NOT NULL
             AND json_extract(state, '$.state') = 'queued' ORDER BY queue_seq",
        )?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    pub fn claim_owner(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT claim_owner FROM jobs WHERE session_id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    pub fn set_stage(&self, session_id: &str, stage: Stage) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET stage = ?2 WHERE session_id = ?1",
            params![session_id, stage_key(stage)],
        )?;
        Ok(())
    }

    /// The stage a processing session last started, if it has a job.
    pub fn current_stage(&self, session_id: &str) -> Result<Option<Stage>> {
        let stage: Option<String> = self
            .conn
            .query_row(
                "SELECT stage FROM jobs WHERE session_id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(stage.and_then(|s| serde_json::from_value(serde_json::Value::String(s)).ok()))
    }

    pub fn heartbeat(
        &self,
        session_id: &str,
        owner: &str,
        now: DateTime<FixedOffset>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET heartbeat_at = ?3 WHERE session_id = ?1 AND claim_owner = ?2",
            params![session_id, owner, now.to_rfc3339()],
        )?;
        Ok(())
    }

    /// At launch, return sessions claimed by another run to the queue with
    /// their checkpoints intact (U17). With the instance lock held, no other
    /// run is alive, so a foreign claim is a dead one however fresh its
    /// heartbeat looks, and waiting out the heartbeat would strand a session
    /// relaunched within half a minute of a crash. Without the lock, only
    /// claims whose heartbeat has gone quiet for [`CLAIM_STALE_AFTER`] are
    /// taken.
    pub fn reclaim_at_launch(
        &mut self,
        current_owner: &str,
        liveness: Liveness,
    ) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, heartbeat_at FROM jobs
             WHERE claim_owner IS NOT NULL AND claim_owner != ?1",
        )?;
        let claims: Vec<(String, Option<String>)> = stmt
            .query_map([current_owner], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        let ids = claims
            .into_iter()
            .filter_map(|(id, heartbeat)| match liveness {
                Liveness::OnlyInstance => Some(id),
                Liveness::Unknown { now } => {
                    let beat = heartbeat.and_then(|h| DateTime::parse_from_rfc3339(&h).ok());
                    let quiet = beat.is_none_or(|beat| now - beat > CLAIM_STALE_AFTER);
                    quiet.then_some(id)
                }
            });
        let ids: Vec<String> = ids.collect();
        let mut reclaimed = Vec::new();
        for id in ids {
            let state = self.require(&id)?.state;
            if matches!(
                state,
                SessionState::Processing { .. } | SessionState::Blocked { .. }
            ) {
                self.apply(&id, &Event::ClaimLost)?;
                reclaimed.push(id);
            } else {
                self.conn.execute(
                    "UPDATE jobs SET claim_owner = NULL, claimed_at = NULL WHERE session_id = ?1",
                    [&id],
                )?;
            }
        }
        Ok(reclaimed)
    }

    /// At launch, add the run `run_id` to the runs of this computer.
    /// Returns the earlier runs, newest first. The store keeps the newest
    /// [`RUNS_KEPT`] runs, this run included.
    pub fn add_run(&mut self, run_id: &str) -> Result<Vec<String>> {
        let tx = self.conn.transaction()?;
        tx.execute("INSERT OR IGNORE INTO runs (id) VALUES (?1)", [run_id])?;
        tx.execute(
            "DELETE FROM runs WHERE seq NOT IN
             (SELECT seq FROM runs ORDER BY seq DESC LIMIT ?1)",
            [RUNS_KEPT as i64],
        )?;
        let earlier = {
            let mut stmt = tx.prepare("SELECT id FROM runs WHERE id != ?1 ORDER BY seq DESC")?;
            stmt.query_map([run_id], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?
        };
        tx.commit()?;
        Ok(earlier)
    }

    // ------------------------------------------------------------ checkpoints

    pub fn put_checkpoint(
        &self,
        session_id: &str,
        stage: Stage,
        key: &str,
        payload: &str,
    ) -> Result<()> {
        put_checkpoint_in(&self.conn, session_id, stage, key, payload)
    }

    pub fn checkpoint_value(
        &self,
        session_id: &str,
        stage: Stage,
        key: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT payload FROM checkpoints WHERE session_id=?1 AND stage=?2 AND key=?3",
                params![session_id, stage_key(stage), key],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn checkpoints(&self, session_id: &str, stage: Stage) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT key, payload FROM checkpoints WHERE session_id = ?1 AND stage = ?2 ORDER BY key",
        )?;
        let rows = stmt
            .query_map(params![session_id, stage_key(stage)], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn clear_checkpoints(&self, session_id: &str, stage: Stage) -> Result<()> {
        self.conn.execute(
            "DELETE FROM checkpoints WHERE session_id = ?1 AND stage = ?2",
            params![session_id, stage_key(stage)],
        )?;
        Ok(())
    }

    pub fn delete_checkpoint(&self, session_id: &str, stage: Stage, key: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM checkpoints WHERE session_id = ?1 AND stage = ?2 AND key = ?3",
            params![session_id, stage_key(stage), key],
        )?;
        Ok(())
    }
}

/// Add `id` to the Move with the step Pending, unless the Move has it.
/// Returns the number of rows that it added.
fn insert_pending_move_note(conn: &Connection, id: &str) -> Result<usize> {
    Ok(conn.execute(
        "INSERT OR IGNORE INTO move_notes (session_id, step) VALUES (?1, ?2)",
        params![id, serde_json::to_string(&MoveStep::Pending)?],
    )?)
}

/// Write `segment`. A segment that the row already has keeps its removed
/// flag: only the scan, the refresh and the adopt set it.
fn insert_segment(conn: &Connection, session_id: &str, segment: &Segment) -> Result<()> {
    conn.execute(
        "INSERT INTO segments
         (session_id, idx, offset_ms, duration_ms, wall_start, wall_end, audio_file, channels,
          ends_in_pause)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (session_id, idx) DO UPDATE SET
           offset_ms = excluded.offset_ms, duration_ms = excluded.duration_ms,
           wall_start = excluded.wall_start, wall_end = excluded.wall_end,
           audio_file = excluded.audio_file, channels = excluded.channels,
           ends_in_pause = excluded.ends_in_pause",
        params![
            session_id,
            segment.index,
            segment.offset_ms as i64,
            segment.duration_ms as i64,
            segment.wall_start.to_rfc3339(),
            segment.wall_end.to_rfc3339(),
            segment.audio_file,
            segment.channels,
            segment.ends_in_pause,
        ],
    )?;
    Ok(())
}

/// Set the removed flag of each segment of `session_id` whose audio file is
/// in `audio`, and clear it for the others. Returns the number of segments
/// whose flag changed.
fn update_removed_audio(conn: &Connection, session_id: &str, audio: &[String]) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE segments SET audio_removed = (audio_file IN (SELECT value FROM json_each(?2)))
         WHERE session_id = ?1
           AND audio_removed != (audio_file IN (SELECT value FROM json_each(?2)))",
        params![session_id, serde_json::to_string(audio)?],
    )?)
}

/// The text of `source` in the `title_source` column.
fn title_source_key(source: TitleSource) -> &'static str {
    match source {
        TitleSource::Proposed => "proposed",
        TitleSource::User => "user",
    }
}

fn parse_time(text: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(text).map_err(|e| {
        StoreError::Sqlite(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(e),
        ))
    })
}

fn read_intent(r: &rusqlite::Row) -> rusqlite::Result<RenameIntent> {
    Ok(RenameIntent {
        from: r.get(0)?,
        to: r.get(1)?,
        temp: r.get(2)?,
    })
}

fn read_session(r: &rusqlite::Row) -> rusqlite::Result<Result<SessionRecord>> {
    let id: String = r.get(0)?;
    let created_at: String = r.get(1)?;
    let state: String = r.get(2)?;
    let queue_seq: Option<i64> = r.get(3)?;
    let speaker_count: Option<String> = r.get(4)?;
    let asr_model: Option<String> = r.get(5)?;
    let home_path: Option<String> = r.get(6)?;
    let title: Option<String> = r.get(7)?;
    let title_source: Option<String> = r.get(8)?;
    let found_speakers: Option<u32> = r.get(9)?;
    let live_transcript: Option<bool> = r.get(10)?;
    let transcribe_again_model: Option<String> = r.get(11)?;
    let recognition_snapshot: Option<String> = r.get(12)?;
    let started_at: Option<String> = r.get(13)?;
    let note_type: Option<String> = r.get(14)?;
    let tags: String = r.get(15)?;
    let pending_type: Option<String> = r.get(16)?;
    let title = title.map(|text| Title {
        text,
        // The store writes a title only together with its source. A title
        // with no source counts as a proposal, which a user title replaces.
        source: match title_source.as_deref() {
            Some("user") => TitleSource::User,
            _ => TitleSource::Proposed,
        },
    });
    Ok((|| {
        Ok(SessionRecord {
            id,
            created_at: parse_time(&created_at)?,
            state: serde_json::from_str(&state)?,
            queue_seq,
            speaker_count: speaker_count
                .map(|c| serde_json::from_str(&c))
                .transpose()?,
            asr_model,
            home_path,
            title,
            found_speakers,
            live_transcript,
            transcribe_again_model,
            recognition_snapshot: recognition_snapshot
                .map(|v| serde_json::from_str(&v))
                .transpose()?,
            started_at: started_at.as_deref().map(parse_time).transpose()?,
            note_type,
            tags: serde_json::from_str(&tags)?,
            pending_type: pending_type.map(|v| serde_json::from_str(&v)).transpose()?,
        })
    })())
}

fn write_state(tx: &Transaction, id: &str, state: &SessionState) -> Result<()> {
    tx.execute(
        "UPDATE sessions SET state = ?2 WHERE id = ?1",
        params![id, serde_json::to_string(state)?],
    )?;
    Ok(())
}

/// Replace `stage`'s checkpoints with the copy set aside before a
/// Transcribe Again run.
fn restore_previous(tx: &Transaction, id: &str, stage: Stage) -> Result<()> {
    move_checkpoints(tx, id, &previous_key(stage), &stage_key(stage))
}

/// Replace the checkpoints stored under the stage name `to` with those
/// stored under `from`.
fn move_checkpoints(tx: &Transaction, id: &str, from: &str, to: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM checkpoints WHERE session_id = ?1 AND stage = ?2",
        params![id, to],
    )?;
    tx.execute(
        "UPDATE checkpoints SET stage = ?3 WHERE session_id = ?1 AND stage = ?2",
        params![id, from, to],
    )?;
    Ok(())
}

/// Delete the recognition rows of the generation that the Transcribe
/// checkpoints under the stage name `from` hold, when the checkpoints under
/// `kept` hold a different generation. The values are compared
/// exactly, so the rows of the kept generation always stay.
fn delete_replaced_generation(tx: &Transaction, id: &str, from: &str, kept: &str) -> Result<()> {
    let Some(replaced) = recognition::generation_in(tx, id, from)? else {
        return Ok(());
    };
    if recognition::generation_in(tx, id, kept)?.as_deref() != Some(replaced.as_str()) {
        recognition::delete_generation(tx, id, &replaced)?;
    }
    Ok(())
}

/// A failed or cancelled Transcribe Again run: delete the rows of the
/// generation that the run wrote. The previous checkpoints come back after
/// this, so the rows of the previous generation stay.
fn drop_run_generation(tx: &Transaction, id: &str) -> Result<()> {
    delete_replaced_generation(
        tx,
        id,
        &stage_key(Stage::Transcribe),
        &previous_key(Stage::Transcribe),
    )
}

/// Delete the checkpoints set aside before a Transcribe Again run, and the
/// rows of their generation when the run replaced it.
fn delete_previous(tx: &Transaction, id: &str) -> Result<()> {
    delete_replaced_generation(
        tx,
        id,
        &previous_key(Stage::Transcribe),
        &stage_key(Stage::Transcribe),
    )?;
    for stage in AGAIN_STAGES {
        tx.execute(
            "DELETE FROM checkpoints WHERE session_id = ?1 AND stage = ?2",
            params![id, previous_key(stage)],
        )?;
    }
    Ok(())
}

/// A resume takes a queued or running Transcribe Again run back (KTD14):
/// the session then resumes as a saved session does. Its assembly and
/// transcription from before the run come back, and the stages after them
/// run again.
fn cancel_transcribe_again(tx: &Transaction, id: &str) -> Result<()> {
    let model: Option<String> = tx.query_row(
        "SELECT transcribe_again_model FROM sessions WHERE id = ?1",
        [id],
        |r| r.get(0),
    )?;
    if model.is_none() {
        return Ok(());
    }
    drop_run_generation(tx, id)?;
    for stage in AGAIN_STAGES {
        tx.execute(
            "DELETE FROM checkpoints WHERE session_id = ?1 AND stage = ?2",
            params![id, stage_key(stage)],
        )?;
    }
    restore_previous(tx, id, Stage::Assemble)?;
    restore_previous(tx, id, Stage::Transcribe)?;
    delete_previous(tx, id)?;
    tx.execute(
        "UPDATE sessions SET transcribe_again_model = NULL WHERE id = ?1",
        [id],
    )?;
    tx.execute("DELETE FROM result_state WHERE session_id = ?1 AND key IN ('pending-snapshot', 'replacement-base')", [id])?;
    Ok(())
}

fn apply_queue_effects(
    tx: &Transaction,
    id: &str,
    before: &SessionState,
    after: &SessionState,
    event: &Event,
) -> Result<()> {
    match after {
        // Entering the queue from capture takes the next place in line. A
        // session handed back by a dead claim keeps the place it had.
        SessionState::Queued { .. } if !matches!(event, Event::ClaimLost) => {
            tx.execute(
                "UPDATE sessions SET queue_seq =
                   (SELECT COALESCE(MAX(queue_seq), 0) + 1 FROM sessions) WHERE id = ?1",
                [id],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO jobs (session_id, stage) VALUES (?1, ?2)",
                params![id, stage_key(Stage::Assemble)],
            )?;
        }
        SessionState::Queued { .. } => {
            tx.execute(
                "UPDATE jobs SET claim_owner = NULL, claimed_at = NULL WHERE session_id = ?1",
                [id],
            )?;
        }
        // Resuming a stopped session takes it out of the queue (R20c) and
        // discards summarization output, keeping transcription and
        // diarization checkpoints for the segments already recorded (R6b).
        SessionState::Capturing if before.is_stopped() => {
            cancel_transcribe_again(tx, id)?;
            tx.execute("UPDATE sessions SET queue_seq = NULL WHERE id = ?1", [id])?;
            tx.execute("DELETE FROM jobs WHERE session_id = ?1", [id])?;
            tx.execute(
                "DELETE FROM checkpoints WHERE session_id = ?1 AND stage = ?2",
                params![id, stage_key(Stage::Summarize)],
            )?;
        }
        // Resuming a completed session (R6d): its speaker analyses left with
        // the working directory, so diarization and everything after it run
        // again; assembly and transcription of the recorded segments are
        // kept, and the folder it completed into stays its home.
        SessionState::Capturing if *before == SessionState::Saved => {
            for stage in [
                Stage::Diarize,
                Stage::Join,
                Stage::Verify,
                Stage::Summarize,
                Stage::Save,
            ] {
                tx.execute(
                    "DELETE FROM checkpoints WHERE session_id = ?1 AND stage = ?2",
                    params![id, stage_key(stage)],
                )?;
            }
        }
        SessionState::Saved | SessionState::Discarded => {
            tx.execute("UPDATE sessions SET queue_seq = NULL WHERE id = ?1", [id])?;
            tx.execute("DELETE FROM jobs WHERE session_id = ?1", [id])?;
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------- delete

/// Why a delete did not complete (KTD9).
#[derive(Debug, thiserror::Error)]
pub enum DeleteError {
    /// The session records, waits to resume, waits in the queue, is
    /// processed, or has its summary regenerated. Nothing changed.
    #[error("the session is {state}")]
    Busy { state: &'static str },
    /// A directory could not be removed. The directories before it are
    /// removed; the directories after it and the rows remain.
    #[error("could not remove {}: {message}", path.display())]
    Remove {
        path: std::path::PathBuf,
        message: String,
    },
    /// The id is not in the form that Longhand writes
    /// ([`crate::note_record::valid_session_id`]): the working directory
    /// that the caller joins to it can be outside the working root.
    /// Nothing changed.
    #[error("the note id is not valid")]
    InvalidId,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// The state name when `state` refuses a delete (R8): capture or
/// processing can still write to the session.
pub fn delete_refusal(state: &SessionState) -> Option<&'static str> {
    match state {
        SessionState::Capturing
        | SessionState::Interrupted { .. }
        | SessionState::Queued { .. }
        | SessionState::Processing { .. } => Some(state.name()),
        _ => None,
    }
}

/// Whether a session is busy (KTD8): it records, is queued, processes, has
/// no home folder, or `claimed` is true because a summary regeneration or
/// a delete holds a claim on it ([`Claims::busy`]). A folder rename waits
/// while a session is busy, and the library reconcile neither refreshes
/// nor forgets a busy row. The caller reads `claimed` under the store
/// lock, so work that starts later sees what the busy check changed.
pub fn busy(record: &SessionRecord, claimed: bool) -> bool {
    record.state != SessionState::Saved || record.home_path.is_none() || claimed
}

/// Work on a session that its state does not show: a summary that is
/// regenerated, a delete that removes the session's directories, and a
/// Move of the note to a new storage folder (KTD9). A regeneration and a
/// delete exclude each other, and a folder rename waits for all of them
/// (R8, KTD8). Each claim is taken and checked under one lock, so work that
/// starts later sees the claim that came first.
///
/// The Move takes two claims. The move claim covers each step of the note
/// and excludes a delete. The hold covers the Move's final check and its
/// home change: note edits and renames wait for it
/// ([`Claims::wait_unheld`]), and a regeneration cannot start.
#[derive(Clone, Default)]
pub struct Claims {
    set: std::sync::Arc<std::sync::Mutex<ClaimSet>>,
    /// Signalled when a hold is released.
    released: std::sync::Arc<std::sync::Condvar>,
}

#[derive(Default)]
struct ClaimSet {
    /// One entry for each regeneration that runs or waits for its turn.
    regenerating: Vec<String>,
    /// One entry for each delete that runs.
    deleting: Vec<String>,
    /// The note that the Move works on.
    moving: Vec<String>,
    /// The note that the Move holds from its final check to its home change.
    holding: Vec<String>,
}

impl ClaimSet {
    fn has(ids: &[String], session_id: &str) -> bool {
        ids.iter().any(|id| id == session_id)
    }
}

/// Which work a [`Claim`] is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClaimKind {
    Regenerate,
    Delete,
    Move,
    Hold,
}

/// Why [`Claims::regenerate`] refuses a regeneration of a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegenerateRefusal {
    /// Another regeneration of the session runs or waits for its turn.
    Regenerating,
    /// A delete runs on the session.
    Deleting,
    /// The Move holds the session for its final check.
    Held,
}

/// A claim on a session, released when it is dropped, also when the work
/// fails or its future is dropped.
pub struct Claim {
    claims: Claims,
    session_id: String,
    kind: ClaimKind,
}

impl Claims {
    fn set(&self) -> std::sync::MutexGuard<'_, ClaimSet> {
        self.set.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Claim the session for a summary regeneration. The check and the new
    /// claim happen under one lock (KTD8). The error tells which work
    /// refuses the regeneration: another regeneration of the session runs
    /// or waits for its turn (R9), a delete runs on it, or the Move holds
    /// it.
    pub fn regenerate(&self, session_id: &str) -> std::result::Result<Claim, RegenerateRefusal> {
        let mut set = self.set();
        if ClaimSet::has(&set.regenerating, session_id) {
            return Err(RegenerateRefusal::Regenerating);
        }
        if ClaimSet::has(&set.deleting, session_id) {
            return Err(RegenerateRefusal::Deleting);
        }
        if ClaimSet::has(&set.holding, session_id) {
            return Err(RegenerateRefusal::Held);
        }
        set.regenerating.push(session_id.to_string());
        Ok(self.claim(session_id, ClaimKind::Regenerate))
    }

    /// Claim the session for a delete. The error is the name of the work
    /// that refuses the delete: "regenerating" while its summary is
    /// regenerated or waits to be, "moving" while the Move works on it.
    pub fn delete(&self, session_id: &str) -> std::result::Result<Claim, &'static str> {
        let mut set = self.set();
        if ClaimSet::has(&set.regenerating, session_id) {
            return Err("regenerating");
        }
        if ClaimSet::has(&set.moving, session_id) || ClaimSet::has(&set.holding, session_id) {
            return Err("moving");
        }
        set.deleting.push(session_id.to_string());
        Ok(self.claim(session_id, ClaimKind::Delete))
    }

    /// Claim the note for a step of the Move. None while its summary is
    /// regenerated or a delete runs on it: the note moves later.
    pub fn moving(&self, session_id: &str) -> Option<Claim> {
        let mut set = self.set();
        if ClaimSet::has(&set.regenerating, session_id)
            || ClaimSet::has(&set.deleting, session_id)
            || ClaimSet::has(&set.moving, session_id)
        {
            return None;
        }
        set.moving.push(session_id.to_string());
        Some(self.claim(session_id, ClaimKind::Move))
    }

    /// Hold the note for the Move's final check and home change. None while
    /// its summary is regenerated or a delete runs on it. The caller holds
    /// the store lock, so an edit that holds that lock ends first.
    pub fn hold(&self, session_id: &str) -> Option<Claim> {
        let mut set = self.set();
        if ClaimSet::has(&set.regenerating, session_id)
            || ClaimSet::has(&set.deleting, session_id)
            || ClaimSet::has(&set.holding, session_id)
        {
            return None;
        }
        set.holding.push(session_id.to_string());
        Some(self.claim(session_id, ClaimKind::Hold))
    }

    fn claim(&self, session_id: &str, kind: ClaimKind) -> Claim {
        Claim {
            claims: self.clone(),
            session_id: session_id.to_string(),
            kind,
        }
    }

    /// True while the session's summary is regenerated or waits to be.
    pub fn is_regenerating(&self, session_id: &str) -> bool {
        ClaimSet::has(&self.set().regenerating, session_id)
    }

    /// True while a delete holds a claim on the session. Record and Resume
    /// are then refused.
    pub fn is_deleting(&self, session_id: &str) -> bool {
        ClaimSet::has(&self.set().deleting, session_id)
    }

    /// True while a regeneration or a delete holds a claim on the session.
    /// The Move then waits, and Transcribe Again is refused.
    pub fn is_regenerating_or_deleting(&self, session_id: &str) -> bool {
        let set = self.set();
        ClaimSet::has(&set.regenerating, session_id) || ClaimSet::has(&set.deleting, session_id)
    }

    /// True while the Move holds the note (see [`Claims::hold`]).
    pub fn is_held(&self, session_id: &str) -> bool {
        ClaimSet::has(&self.set().holding, session_id)
    }

    /// Return when the Move does not hold the note. The caller must not
    /// hold the store lock.
    pub fn wait_unheld(&self, session_id: &str) {
        let mut set = self.set();
        while ClaimSet::has(&set.holding, session_id) {
            set = self.released.wait(set).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// True while a regeneration, a delete or the Move holds a claim on the
    /// session. A folder rename then waits (KTD8).
    pub fn busy(&self, session_id: &str) -> bool {
        let set = self.set();
        set.regenerating
            .iter()
            .chain(&set.deleting)
            .chain(&set.moving)
            .chain(&set.holding)
            .any(|id| id == session_id)
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut set = self.claims.set();
        let ids = match self.kind {
            ClaimKind::Regenerate => &mut set.regenerating,
            ClaimKind::Delete => &mut set.deleting,
            ClaimKind::Move => &mut set.moving,
            ClaimKind::Hold => &mut set.holding,
        };
        if let Some(i) = ids.iter().position(|id| *id == self.session_id) {
            ids.swap_remove(i);
        }
        if self.kind == ClaimKind::Hold {
            self.claims.released.notify_all();
        }
    }
}

/// Delete a session (R8, R33): remove its home folder (when it has one),
/// then the staging directory of a save that stopped before its final
/// rename (when there is one), then its working directory, then its rows.
/// Child rows cascade.
///
/// `remove` moves a directory to the Trash or deletes it permanently. The
/// store is not locked while it runs. A directory that is already missing
/// counts as removed, so a delete that stopped part way completes when it
/// is tried again. The delete holds a claim in `claims` from before it
/// reads the home folder until it deletes the rows: a folder rename waits
/// for it, and a summary regeneration refuses it and is refused by it.
pub fn delete_session<R>(
    store: &std::sync::Mutex<Store>,
    id: &str,
    working_dir: &Path,
    claims: &Claims,
    mut remove: R,
) -> std::result::Result<(), DeleteError>
where
    R: FnMut(&Path) -> std::result::Result<(), String>,
{
    if !crate::note_record::valid_session_id(id) {
        return Err(DeleteError::InvalidId);
    }
    let lock = || store.lock().unwrap_or_else(|e| e.into_inner());
    // The claim comes before the home folder is read. A rename that holds
    // the store lock now completes first, and the read gets its new path.
    let _claim = claims
        .delete(id)
        .map_err(|state| DeleteError::Busy { state })?;
    let (record, staging, moving) = {
        let store = lock();
        let record = store.require(id)?;
        let moving = !matches!(
            store.move_step(id)?,
            None | Some(MoveStep::Pending | MoveStep::Done)
        );
        (record, store.save_staging(id)?, moving)
    };
    if let Some(state) = delete_refusal(&record.state) {
        return Err(DeleteError::Busy { state });
    }
    // A note that the Move has copied or placed has a second folder. The
    // delete waits until the Move is done with it.
    if moving {
        return Err(DeleteError::Busy { state: "moving" });
    }
    let home = record.home_path.as_deref().map(Path::new);
    for dir in home
        .into_iter()
        .chain(staging.as_deref())
        .chain([working_dir])
    {
        remove_dir(dir, &mut remove)?;
    }
    let store = lock();
    // The state can change while the directories are removed; check again.
    let record = store.require(id)?;
    if let Some(state) = delete_refusal(&record.state) {
        return Err(DeleteError::Busy { state });
    }
    store
        .conn
        .execute("DELETE FROM sessions WHERE id = ?1", [id])
        .map_err(StoreError::from)?;
    Ok(())
}

impl Store {
    /// The staging directory that the save stage recorded for a save that
    /// stopped before its final rename: the save state names a `staging`
    /// path only while it copies or holds a complete staged copy.
    fn save_staging(&self, id: &str) -> Result<Option<std::path::PathBuf>> {
        #[derive(serde::Deserialize)]
        struct Staging {
            staging: Option<std::path::PathBuf>,
        }
        Ok(self
            .checkpoints(id, Stage::Save)?
            .into_iter()
            .filter(|(key, _)| key == SAVE_STATE)
            .find_map(|(_, payload)| serde_json::from_str::<Staging>(&payload).ok())
            .and_then(|s| s.staging))
    }
}

/// The key of the save stage's checkpoint that holds its state
/// (`pipeline::save::STATE`).
pub const SAVE_STATE: &str = "state";

/// The `result_state` key that holds the state of the early write of a
/// note's home folder (`pipeline::early_home`, KTD4). Its `step` field is
/// `done` once the store records the home path.
pub const EARLY_HOME: &str = "early-home";

fn remove_dir<R>(dir: &Path, remove: &mut R) -> std::result::Result<(), DeleteError>
where
    R: FnMut(&Path) -> std::result::Result<(), String>,
{
    let missing = |dir: &Path| matches!(std::fs::symlink_metadata(dir), Err(e) if e.kind() == std::io::ErrorKind::NotFound);
    if missing(dir) {
        return Ok(());
    }
    match remove(dir) {
        Ok(()) => Ok(()),
        // Something else removed it first.
        Err(_) if missing(dir) => Ok(()),
        Err(message) => Err(DeleteError::Remove {
            path: dir.to_path_buf(),
            message,
        }),
    }
}

/// The `result_state` key that marks a note that existed before its
/// recording: Record started the recording in a note with no audio. A
/// Stop or a crash recovery that keeps no segment keeps such a note, and
/// the removal of empty notes never removes it (R19).
pub const RECORDED_INTO: &str = "recorded_into";
