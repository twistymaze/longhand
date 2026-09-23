//! The job store (KTD10): sessions, segments, notes, the processing queue and
//! mid-stage checkpoints in SQLite with WAL and `synchronous=FULL`.
//!
//! Every lifecycle change goes through [`Store::apply`], which runs the state
//! machine and persists the result together with its queue side effects in
//! one transaction.

use std::path::Path;

use chrono::{DateTime, FixedOffset};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::anchors::Anchor;
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
    #[error("no note {0}")]
    NoSuchNote(i64),
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
    pub landed_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: i64,
    pub session_id: Option<String>,
    pub text: String,
    pub anchor: Anchor,
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

    /// Create a capturing session and adopt every pending note as one of its
    /// un-anchored notes (R13).
    pub fn create_session(
        &mut self,
        id: &str,
        created_at: DateTime<FixedOffset>,
        speaker_count: Option<SpeakerCount>,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
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
        tx.execute(
            "UPDATE notes SET session_id = ?1, anchor = ?2 WHERE session_id IS NULL",
            params![id, serde_json::to_string(&Anchor::Unanchored)?],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Start a new session from the shortcut (R6c): any session still capturing
    /// or interrupted is stopped and queued first (or discarded if it kept no
    /// audio), then the new one is created and adopts the pending notes.
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

    /// Queue every interruption whose resume window has elapsed (R5e).
    pub fn tick(&mut self, now: DateTime<FixedOffset>) -> Result<Vec<String>> {
        let mut queued = Vec::new();
        for record in self.sessions_in("interrupted")? {
            if !record.state.within_resume_window(now) {
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
                "SELECT id, created_at, state, queue_seq, speaker_count, asr_model, landed_path
                 FROM sessions WHERE id = ?1",
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

    /// The most recently created session, which is the only one Resume is
    /// offered on (R6a).
    pub fn most_recent_session(&self) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM sessions ORDER BY created_at DESC, rowid DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn is_most_recent(&self, id: &str) -> Result<bool> {
        Ok(self.most_recent_session()?.as_deref() == Some(id))
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

    pub fn set_landed_path(&self, id: &str, path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET landed_path = ?2 WHERE id = ?1",
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

    pub fn sessions_in(&self, state_name: &str) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, state, queue_seq, speaker_count, asr_model, landed_path
             FROM sessions ORDER BY created_at, rowid",
        )?;
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

    // ------------------------------------------------------------ notes

    /// A note typed while no session is open (R13).
    pub fn add_pending_note(&self, text: &str) -> Result<i64> {
        self.insert_note(None, text, Anchor::Unanchored)
    }

    pub fn add_note(&self, session_id: &str, text: &str, anchor: Anchor) -> Result<i64> {
        self.insert_note(Some(session_id), text, anchor)
    }

    fn insert_note(&self, session_id: Option<&str>, text: &str, anchor: Anchor) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO notes (session_id, text, anchor) VALUES (?1, ?2, ?3)",
            params![session_id, text, serde_json::to_string(&anchor)?],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Change a note's text. Its anchor never moves (R12).
    pub fn edit_note(&self, note_id: i64, text: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE notes SET text = ?2 WHERE id = ?1",
            params![note_id, text],
        )?;
        if changed == 0 {
            return Err(StoreError::NoSuchNote(note_id));
        }
        Ok(())
    }

    pub fn notes(&self, session_id: &str) -> Result<Vec<Note>> {
        self.query_notes(
            "SELECT id, session_id, text, anchor FROM notes WHERE session_id = ?1 ORDER BY id",
            Some(session_id),
        )
    }

    pub fn pending_notes(&self) -> Result<Vec<Note>> {
        self.query_notes(
            "SELECT id, session_id, text, anchor FROM notes WHERE session_id IS NULL ORDER BY id",
            None,
        )
    }

    fn query_notes(&self, sql: &str, session_id: Option<&str>) -> Result<Vec<Note>> {
        let mut stmt = self.conn.prepare(sql)?;
        let map = |r: &rusqlite::Row| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        };
        let rows = match session_id {
            Some(id) => stmt.query_map([id], map)?.collect::<Vec<_>>(),
            None => stmt.query_map([], map)?.collect::<Vec<_>>(),
        };
        rows.into_iter()
            .map(|row| {
                let (id, session_id, text, anchor) = row?;
                Ok(Note {
                    id,
                    session_id,
                    text,
                    anchor: serde_json::from_str(&anchor)?,
                })
            })
            .collect()
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

    /// At launch, return every session claimed by another run to the queue
    /// with its checkpoints intact. Single instance (R34) guarantees no other
    /// run is alive, so a foreign claim is a dead one however fresh its
    /// heartbeat looks.
    pub fn reclaim_at_launch(&mut self, current_owner: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id FROM jobs WHERE claim_owner IS NOT NULL AND claim_owner != ?1",
        )?;
        let ids: Vec<String> = stmt
            .query_map([current_owner], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
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

fn read_session(r: &rusqlite::Row) -> rusqlite::Result<Result<SessionRecord>> {
    let id: String = r.get(0)?;
    let created_at: String = r.get(1)?;
    let state: String = r.get(2)?;
    let queue_seq: Option<i64> = r.get(3)?;
    let speaker_count: Option<String> = r.get(4)?;
    let asr_model: Option<String> = r.get(5)?;
    let landed_path: Option<String> = r.get(6)?;
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
            landed_path,
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
        SessionState::Capturing if before.resume_window_start().is_some() => {
            tx.execute("UPDATE sessions SET queue_seq = NULL WHERE id = ?1", [id])?;
            tx.execute("DELETE FROM jobs WHERE session_id = ?1", [id])?;
            tx.execute(
                "DELETE FROM checkpoints WHERE session_id = ?1 AND stage = ?2",
                params![id, stage_key(Stage::Summarize)],
            )?;
        }
        SessionState::Landed | SessionState::Discarded => {
            tx.execute("UPDATE sessions SET queue_seq = NULL WHERE id = ?1", [id])?;
            tx.execute("DELETE FROM jobs WHERE session_id = ?1", [id])?;
        }
        _ => {}
    }
    Ok(())
}
