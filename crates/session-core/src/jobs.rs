//! The job store (KTD10): sessions, segments, notes, the processing queue and
//! mid-stage checkpoints in SQLite with WAL and `synchronous=FULL`.
//!
//! Every lifecycle change goes through [`Store::apply`], which runs the state
//! machine and persists the result together with its queue side effects in
//! one transaction.

use std::path::Path;

use chrono::{DateTime, FixedOffset};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::folder::SpeakerCount;
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
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: String,
    pub created_at: DateTime<FixedOffset>,
    pub state: SessionState,
    pub queue_seq: Option<i64>,
    pub speaker_count: Option<SpeakerCount>,
    pub asr_model: Option<String>,
    pub home_path: Option<String>,
    pub title: Option<Title>,
    /// The number of speakers that processing found (KTD11).
    pub found_speakers: Option<u32>,
}

/// Where a title came from (KTD7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleSource {
    /// Processing proposed it.
    Proposed,
    /// The user typed it. A proposal never replaces it (R31).
    User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

/// How long a claim's heartbeat may be quiet before its owner, when it
/// cannot be shown dead, is taken to be gone (U17). The runner beats every
/// 10 s.
pub const CLAIM_STALE_AFTER: chrono::TimeDelta = chrono::TimeDelta::seconds(30);

/// What launch knows of the other runs that hold claims.
#[derive(Debug, Clone, Copy)]
pub enum Liveness {
    /// This process holds the instance lock: no other run is alive.
    OnlyInstance,
    /// The lock could not be taken; judge each claim by its heartbeat.
    Unknown { now: DateTime<FixedOffset> },
}

pub struct Store {
    conn: Connection,
}

fn stage_key(stage: Stage) -> String {
    serde_json::to_value(stage)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .expect("stage serializes to a string")
}

/// The `sessions` columns that [`read_session`] reads, in its order.
const SESSION_COLUMNS: &str = "id, created_at, state, queue_seq, speaker_count, asr_model, \
     home_path, title, title_source, found_speakers";

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

    /// Create a capturing session with an empty notes document.
    pub fn create_session(
        &mut self,
        id: &str,
        created_at: DateTime<FixedOffset>,
        speaker_count: Option<SpeakerCount>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, created_at, state, speaker_count) VALUES (?1, ?2, ?3, ?4)",
            params![
                id,
                created_at.to_rfc3339(),
                serde_json::to_string(&SessionState::Capturing)?,
                speaker_count
                    .map(|c| serde_json::to_string(&c))
                    .transpose()?,
            ],
        )?;
        Ok(())
    }

    /// Start a new session from the shortcut (R6c): any session still capturing
    /// or interrupted is stopped and queued first (or discarded if it kept no
    /// audio), then the new one is created.
    pub fn start_session(
        &mut self,
        id: &str,
        now: DateTime<FixedOffset>,
        speaker_count: Option<SpeakerCount>,
    ) -> Result<Vec<String>> {
        let mut stopped = Vec::new();
        for name in ["capturing", "interrupted"] {
            for record in self.sessions_in(name)? {
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
        self.create_session(id, now, speaker_count)?;
        Ok(stopped)
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

    /// Record the recognition model selected when the session stopped (R20c).
    pub fn set_asr_model(&self, id: &str, model: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET asr_model = ?2 WHERE id = ?1",
            params![id, model],
        )?;
        Ok(())
    }

    pub fn set_home_path(&self, id: &str, path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET home_path = ?2 WHERE id = ?1",
            params![id, path],
        )?;
        Ok(())
    }

    /// Run `event` through the state machine and persist the result with its
    /// queue side effects.
    pub fn apply(&mut self, id: &str, event: &Event) -> Result<SessionState> {
        let current = self.require(id)?;
        let next = current.state.transition(event)?;
        let tx = self.conn.transaction()?;
        write_state(&tx, id, &next)?;
        apply_queue_effects(&tx, id, &current.state, &next, event)?;
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
        self.conn.execute(
            "INSERT OR REPLACE INTO segments
             (session_id, idx, offset_ms, duration_ms, wall_start, wall_end, audio_file)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session_id,
                segment.index,
                segment.offset_ms as i64,
                segment.duration_ms as i64,
                segment.wall_start.to_rfc3339(),
                segment.wall_end.to_rfc3339(),
                segment.audio_file,
            ],
        )?;
        Ok(())
    }

    pub fn segments(&self, session_id: &str) -> Result<Vec<Segment>> {
        let mut stmt = self.conn.prepare(
            "SELECT idx, offset_ms, duration_ms, wall_start, wall_end, audio_file
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
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (index, offset, duration, start, end, audio_file) = row?;
            out.push(Segment {
                index,
                offset_ms: offset as u64,
                duration_ms: duration as u64,
                wall_start: parse_time(&start)?,
                wall_end: parse_time(&end)?,
                audio_file,
            });
        }
        Ok(out)
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
        let Some(id) = next else {
            return Ok(None);
        };
        self.apply(&id, &Event::Dequeued)?;
        self.conn.execute(
            "UPDATE jobs SET claim_owner = ?2, claimed_at = ?3, heartbeat_at = ?3 WHERE session_id = ?1",
            params![id, owner, now.to_rfc3339()],
        )?;
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

    // ------------------------------------------------------------ checkpoints

    pub fn put_checkpoint(
        &self,
        session_id: &str,
        stage: Stage,
        key: &str,
        payload: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO checkpoints (session_id, stage, key, payload) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, stage_key(stage), key, payload],
        )?;
        Ok(())
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

/// Work on a session that its state does not show: a summary that is
/// regenerated and a delete that removes the session's directories. The
/// two exclude each other, and a folder rename waits for both (R8, KTD8).
/// Each claim is taken and checked under one lock, so a regeneration or a
/// delete that starts later sees the claim that came first.
#[derive(Clone, Default)]
pub struct Claims {
    set: std::sync::Arc<std::sync::Mutex<ClaimSet>>,
}

#[derive(Default)]
struct ClaimSet {
    /// One entry for each regeneration that runs or waits for its turn.
    regenerating: Vec<String>,
    /// One entry for each delete that runs.
    deleting: Vec<String>,
}

/// Which work a [`Claim`] is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClaimKind {
    Regenerate,
    Delete,
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

    /// Claim the session for a summary regeneration. None while a delete
    /// runs on it.
    pub fn regenerate(&self, session_id: &str) -> Option<Claim> {
        let mut set = self.set();
        if set.deleting.iter().any(|id| id == session_id) {
            return None;
        }
        set.regenerating.push(session_id.to_string());
        Some(self.claim(session_id, ClaimKind::Regenerate))
    }

    /// Claim the session for a delete. None while its summary is
    /// regenerated or waits to be.
    pub fn delete(&self, session_id: &str) -> Option<Claim> {
        let mut set = self.set();
        if set.regenerating.iter().any(|id| id == session_id) {
            return None;
        }
        set.deleting.push(session_id.to_string());
        Some(self.claim(session_id, ClaimKind::Delete))
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
        self.set().regenerating.iter().any(|id| id == session_id)
    }

    /// True while a regeneration or a delete holds a claim on the session.
    /// A folder rename then waits (KTD8).
    pub fn busy(&self, session_id: &str) -> bool {
        let set = self.set();
        set.regenerating
            .iter()
            .chain(&set.deleting)
            .any(|id| id == session_id)
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut set = self.claims.set();
        let ids = match self.kind {
            ClaimKind::Regenerate => &mut set.regenerating,
            ClaimKind::Delete => &mut set.deleting,
        };
        if let Some(i) = ids.iter().position(|id| *id == self.session_id) {
            ids.swap_remove(i);
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
    let lock = || store.lock().unwrap_or_else(|e| e.into_inner());
    // The claim comes before the home folder is read. A rename that holds
    // the store lock now completes first, and the read gets its new path.
    let Some(_claim) = claims.delete(id) else {
        return Err(DeleteError::Busy {
            state: "regenerating",
        });
    };
    let (record, staging) = {
        let store = lock();
        let record = store.require(id)?;
        (record, store.save_staging(id)?)
    };
    if let Some(state) = delete_refusal(&record.state) {
        return Err(DeleteError::Busy { state });
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
