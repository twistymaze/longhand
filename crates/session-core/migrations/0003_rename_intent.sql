-- The folder rename intent (KTD8). A rename records the move here before
-- it renames the home folder, and clears the record after it sets
-- home_path. At launch, a record that is still here is resolved from disk.
--
-- temp_path is set only for a change of case alone: that move goes through
-- a temporary name first.
CREATE TABLE rename_intents (
    session_id      TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
    from_path       TEXT NOT NULL,
    to_path         TEXT NOT NULL,
    temp_path       TEXT
);
