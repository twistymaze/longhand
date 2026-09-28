CREATE TABLE recognition_output (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    generation TEXT NOT NULL,
    key TEXT NOT NULL,
    revision INTEGER NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (session_id, generation, key)
);
CREATE TABLE recognition_epochs (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    generation TEXT NOT NULL,
    next_epoch INTEGER NOT NULL,
    PRIMARY KEY (session_id, generation)
);
