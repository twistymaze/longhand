# Lens: data-loss

Find paths where user data is lost, damaged, or overwritten. In a recording app, this is the most important class of defect.

## Look for

- Writes that are not atomic: a file written in place, so that a crash or a full disk leaves it partly written. Look for write-to-temporary-then-rename, and check that the rename is in the same folder and that the data is flushed before the rename when the code needs it.
- Deletes and moves: a delete before the copy is confirmed, a move across volumes that is not a rename, cleanup that removes files that are still needed.
- Overwrites: a save that replaces newer data with older data, for example from another computer, an agent, or an editor that holds an old copy.
- A path or name collision: two items that map to the same file name, a case-only difference on a case-insensitive file system, or a name with characters that the file system does not accept.
- Recovery: what a crash, a force quit, or a power loss at each step leaves on disk, and whether the next start finds and recovers it.
- Formats: a reader that drops fields that it does not know, so a write after a read loses them.
- Settings: a value that resets to a default because a read failed, and is then saved.

## For each finding

Name the data that is lost and the step at which it is lost.
