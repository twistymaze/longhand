-- The runs of the app on this computer, oldest first (the in-use markers
-- of KTD5 of the plan for notes without a recording). A run adds its ID at
-- launch. The marker of an earlier run on this computer is this
-- computer's own work, not work on another computer. The store keeps the
-- newest runs only.
CREATE TABLE runs (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE
);
