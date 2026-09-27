-- The folder renames that this computer owes (one-library plan). A reconcile
-- that cannot give a folder the name its title gives records the session
-- here: the session was busy, or the move failed. The launch reconcile
-- renames only these folders. It does not rename a folder that another
-- computer renamed, when the new session.md has not synced yet.
CREATE TABLE owed_renames (
    session_id      TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE
);
