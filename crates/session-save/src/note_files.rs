//! The writes of the two files that define a note folder (KTD2, KTD3):
//! `longhand.json`, which holds Longhand's records, and `session.md`,
//! where Longhand changes only the title and the tags. Each write is
//! atomic ([`crate::home_write`]).

use std::io;
use std::path::Path;

use session_core::folder::{FrontmatterPatch, SessionDoc};
use session_core::note_folder::{Layout, NoteFiles, SESSION_FILE, read_optional};
use session_core::note_record::{NOTE_RECORD_FILE, NoteRecord, NoteRecordRead};

use crate::home_write;
use crate::publication::OLD_LAYOUT;

/// The frontmatter key of the title in `session.md`.
pub const TITLE_KEY: &str = "title";

/// The frontmatter key of the tags in `session.md`.
pub const TAGS_KEY: &str = "longhand_tags";

/// Write `record` into `dir` as `longhand.json`. Returns the bytes that
/// it wrote.
pub fn write_record(dir: &Path, record: &NoteRecord) -> io::Result<Vec<u8>> {
    let bytes = record.render().into_bytes();
    home_write::write_atomic(dir, NOTE_RECORD_FILE, &bytes)?;
    Ok(bytes)
}

/// The records of the note in `dir`. A note in the old layout gives an
/// error with the text [`OLD_LAYOUT`] (KTD6).
pub fn new_layout_record(dir: &Path) -> io::Result<NoteRecord> {
    let files = NoteFiles::read(dir)?;
    if files.layout == Layout::Old {
        return Err(io::Error::other(OLD_LAYOUT));
    }
    Ok(files.record)
}

/// The patch that sets the title of `session.md`. `None` removes it.
pub fn title_patch(title: Option<&str>) -> FrontmatterPatch {
    match title {
        Some(title) => FrontmatterPatch::new().set(TITLE_KEY, title),
        None => FrontmatterPatch::new().remove(TITLE_KEY),
    }
}

/// `patch` with the tags of `session.md` set to `tags`. No tags remove
/// the key.
pub fn with_tags(patch: FrontmatterPatch, tags: &[String]) -> FrontmatterPatch {
    if tags.is_empty() {
        patch.remove(TAGS_KEY)
    } else {
        patch.set(TAGS_KEY, tags)
    }
}

/// The text of the `session.md` of a new note: a frontmatter with the
/// title and the tags, and no body (KTD3).
pub fn new_session_md(title: Option<&str>, tags: &[String]) -> String {
    let mut patch = FrontmatterPatch::new();
    if let Some(title) = title {
        patch = patch.set(TITLE_KEY, title);
    }
    if !tags.is_empty() {
        patch = patch.set(TAGS_KEY, tags);
    }
    patch.create()
}

/// Write the note of `doc` into `dir` in the new layout: a new
/// `session.md` with its title and tags, then `longhand.json` with its
/// records. The title of `doc`, with its source, is the last-written title.
/// A [`SessionDoc`] has no summary data, so the summary data of a
/// `longhand.json` that is in `dir` stays.
pub fn write_doc(dir: &Path, doc: &SessionDoc) -> io::Result<()> {
    let mut record = NoteRecord::from_doc(doc);
    if let Ok(text) = std::fs::read_to_string(dir.join(NOTE_RECORD_FILE))
        && let Some(current) = NoteRecord::parse(&text)
            .ok()
            .and_then(NoteRecordRead::current)
    {
        record.summary = current.summary;
    }
    let session_md = new_session_md(doc.title.as_deref(), &doc.tags);
    home_write::write_atomic(dir, SESSION_FILE, session_md.as_bytes())?;
    write_record(dir, &record)?;
    Ok(())
}

/// Apply `patch` to the `session.md` in `dir` (R4). A file that is not
/// there is made with only the keys of the patch. Returns the text of the
/// file after the patch, and writes the file only when the text changes.
/// A frontmatter that does not parse gives an error, and the file stays as
/// it is.
pub fn patch_session_md(dir: &Path, patch: &FrontmatterPatch) -> io::Result<String> {
    let path = dir.join(SESSION_FILE);
    let current = read_optional(&path)?;
    let text = match &current {
        Some(text) => patch
            .apply(SESSION_FILE, text)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?,
        None => patch.create(),
    };
    if current.as_deref() != Some(text.as_str()) {
        home_write::write_atomic(dir, SESSION_FILE, text.as_bytes())?;
    }
    Ok(text)
}
