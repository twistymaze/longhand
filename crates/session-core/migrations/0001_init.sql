-- Working state for unlanded sessions, the processing queue, and one retained
-- row per landed session (KTD10). Every row is rebuildable from the retained
-- destination roots (R25).

CREATE TABLE sessions (
    id              TEXT PRIMARY KEY,
    created_at      TEXT NOT NULL,        -- wall clock, RFC 3339, display only
    state           TEXT NOT NULL,        -- JSON SessionState
    -- Order in which the session entered the queue (R20c). Assigned when it
    -- first becomes Queued, kept when a dead claim returns it, cleared on resume.
    queue_seq       INTEGER,
    speaker_count   TEXT,                 -- JSON SpeakerCount (R36)
    asr_model       TEXT,                 -- model selected at stop time (R20c)
    landed_path     TEXT
);

CREATE INDEX sessions_queue ON sessions (queue_seq) WHERE queue_seq IS NOT NULL;

CREATE TABLE segments (
    session_id      TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    idx             INTEGER NOT NULL,
    offset_ms       INTEGER NOT NULL,
    duration_ms     INTEGER NOT NULL,
    wall_start      TEXT NOT NULL,
    wall_end        TEXT NOT NULL,
    audio_file      TEXT NOT NULL,
    PRIMARY KEY (session_id, idx)
);

-- A note with no session is pending: typed while no session was open (R13).
CREATE TABLE notes (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id      TEXT REFERENCES sessions (id) ON DELETE CASCADE,
    text            TEXT NOT NULL,
    anchor          TEXT NOT NULL         -- JSON Anchor; fixed once written
);

CREATE TABLE jobs (
    session_id      TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
    stage           TEXT NOT NULL,
    -- The run that holds the job. A run is one launch of the application;
    -- single instance (R34) means any claim not held by the current run is dead.
    claim_owner     TEXT,
    claimed_at      TEXT,
    heartbeat_at    TEXT
);

-- Mid-stage progress: one row per transcription window, diarized segment or
-- summarization chunk (KTD11).
CREATE TABLE checkpoints (
    session_id      TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    stage           TEXT NOT NULL,
    key             TEXT NOT NULL,
    payload         TEXT NOT NULL,
    PRIMARY KEY (session_id, stage, key)
);
