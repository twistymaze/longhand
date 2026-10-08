-- The type moves and the bulk edits that this computer owes (KTD5, KTD7,
-- KTD11, KTD13 of the note types and tags plan). These follow the owed
-- renames of migration 0008: a local record, written before the work, so
-- that a quit or a crash does not lose the work. Only the computer with
-- the record does the work.

-- A type change whose move this computer owes. The record holds no path:
-- the move finds the home folder by session id at move time.
CREATE TABLE type_moves (
    session_id        TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
    -- The new type. NULL is "No type".
    to_type           TEXT,
    -- The type folder that the home folder was in when the user changed
    -- the type. NULL is "No type".
    from_type         TEXT,
    -- The time of the change (RFC 3339).
    changed_at        TEXT NOT NULL,
    -- The time that this computer took over the move of another computer
    -- (KTD13), or NULL. A taken-over move waits 10 minutes.
    taken_over_at     TEXT,
    -- 1 when a type rename, merge or delete made the move: the old type
    -- folder goes when the last note leaves it and it holds nothing else.
    remove_empty_from INTEGER NOT NULL DEFAULT 0,
    -- 1 when this computer wrote the pending type into `session.md`. A
    -- field that is gone after that was cleared on another computer: the
    -- user changed the type back there.
    field_written     INTEGER NOT NULL DEFAULT 0
);

-- A Settings edit that changes many notes: a type rename, merge or delete,
-- or a tag rename or delete. The record is written before the first
-- change, and each launch and scan continues the edit until every note is
-- done.
CREATE TABLE bulk_edits (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The edit as JSON.
    edit         TEXT NOT NULL,
    -- 1 when the shared settings have the change, for an edit that changes
    -- them. The notes change after the shared settings.
    shared_saved INTEGER NOT NULL DEFAULT 0
);

-- The ID of this computer: the `by` of a pending type (KTD13). Each
-- database gets a random ID one time.
CREATE TABLE computer (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    computer_id TEXT NOT NULL
);
INSERT INTO computer (id, computer_id) VALUES (1, lower(hex(randomblob(16))));
