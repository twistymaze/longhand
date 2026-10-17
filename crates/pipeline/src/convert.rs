//! The conversion of old notes (U5 of the safe agent edits plan, R9, R10,
//! R12, R13, KTD6).
//!
//! An old note has its records in `session.md` and its summary data in the
//! frontmatter of `summary.md`. The conversion moves them into
//! `longhand.json`, in this order:
//!
//! 1. `longhand.json`, from the unchanged `session.md` and `summary.md`,
//!    with the fingerprints of the files as they are after the conversion.
//! 2. `summary.md` without the summary data. Other frontmatter keys, the
//!    comments and the body stay. With no other key, the file is the body
//!    only.
//! 3. `session.md` with no records. The title, the tags, other keys and the
//!    body stay.
//!
//! Until step 3, `session.md` holds `session_id`, so the note is still in
//! the old layout. A second run after a stop at any step reads the same
//! records and writes the same files. The output has no write time, so two
//! computers that convert one note write the same bytes.
//!
//! Only the user starts the conversion: a read, a scan or a launch never
//! converts a note (R12).

use std::io;
use std::path::{Path, PathBuf};

use chrono::Local;
use session_core::folder::{FolderError, FrontmatterPatch, Summary, fingerprint};
use session_core::jobs::busy;
use session_core::note_folder::{
    Layout, NoteFiles, SESSION_FILE, read_optional, written_fingerprint,
};
use session_core::note_record::{NOTE_RECORD_FILE, NoteRecord, NoteRecordRead};
use session_save::{home_write, in_use_marker, note_files};

use crate::summarize::SUMMARY_FILE;
use crate::{SharedStore, lock};

/// The frontmatter keys of the records in an old-layout `session.md`. The
/// conversion removes them. `result_generation` is a record of an older
/// Longhand.
const RECORD_KEYS: &[&str] = &[
    "session_id",
    "started",
    "title_source",
    "longhand_pending_type",
    "speaker_count",
    "asr_model",
    "recognition_snapshot",
    "recognition_artifact",
    "recognition_audio",
    "found_speakers",
    "transcript_fingerprint",
    "summary_fingerprint",
    "result",
    "segments",
    "result_generation",
];

/// The frontmatter keys of the summary data in an old-layout `summary.md`
/// (the fields of `SummaryMeta`). The conversion removes them.
const SUMMARY_KEYS: &[&str] = &[
    "status",
    "endpoint_host",
    "model",
    "prompt_name",
    "prompt_id",
    "failed_stage",
];

/// A write step of the conversion of one folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Record,
    Summary,
    SessionMd,
}

/// What the conversion of one folder did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderConversion {
    /// The note is in the new layout now. The texts are the `session.md`
    /// and the `longhand.json` that the conversion wrote.
    Converted { session_md: String, record: Vec<u8> },
    /// The folder has no old-layout `session.md`.
    NotOld,
    /// `session.md` changed during the conversion, for example from a sync.
    /// The note stays in the old layout until the next run.
    Changed,
}

/// What one conversion run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConvertReport {
    /// The notes that are in the new layout now.
    pub converted: Vec<String>,
    /// The notes that are in use or changed during the run. The next run
    /// converts them. They are not failures.
    pub skipped: Vec<String>,
    /// The notes that did not convert, each with its folder and the reason.
    /// They stay in the old layout.
    pub failed: Vec<(String, PathBuf, String)>,
}

/// Convert the old notes `notes` (each id with its folder) that no computer
/// works on now. `own_runs` are this computer's runs, whose in-use markers
/// do not count. `claimed` tells whether a stage on this computer holds a
/// claim on a note. Each note converts under the store lock, so no other
/// write of this computer changes it in the meantime.
pub fn convert_notes(
    store: &SharedStore,
    notes: &[(String, PathBuf)],
    own_runs: &[String],
    claimed: &dyn Fn(&str) -> bool,
) -> ConvertReport {
    let mut report = ConvertReport::default();
    for (id, folder) in notes {
        let guard = lock(store);
        let record = match guard.session(id) {
            Ok(record) => record,
            Err(e) => {
                report
                    .failed
                    .push((id.clone(), folder.clone(), e.to_string()));
                continue;
            }
        };
        if record.as_ref().is_some_and(|r| busy(r, claimed(id))) {
            report.skipped.push(id.clone());
            continue;
        }
        match in_use_marker::in_use_except(folder, own_runs, Local::now().fixed_offset()) {
            Ok(false) => {}
            Ok(true) => {
                report.skipped.push(id.clone());
                continue;
            }
            Err(e) => {
                report
                    .failed
                    .push((id.clone(), folder.clone(), e.to_string()));
                continue;
            }
        }
        match convert_folder(folder, &mut |_| Ok(())) {
            Ok(FolderConversion::Converted {
                session_md,
                record: bytes,
            }) => {
                if record.is_some()
                    && let Err(e) = guard.set_written_session_fingerprint(
                        id,
                        Some(&written_fingerprint(session_md.as_bytes(), &bytes)),
                    )
                {
                    log::warn!("could not record the files of {id} after the conversion: {e}");
                }
                log::info!("{id} is converted to the new layout");
                report.converted.push(id.clone());
            }
            // A later scan finds that the note is not old.
            Ok(FolderConversion::NotOld) => {}
            Ok(FolderConversion::Changed) => report.skipped.push(id.clone()),
            Err(e) => {
                log::warn!("could not convert {}: {e}", folder.display());
                report
                    .failed
                    .push((id.clone(), folder.clone(), e.to_string()));
            }
        }
    }
    report
}

/// Convert the note folder `folder` to the new layout. `before` runs before
/// each write step, and an error from it stops the conversion there: the
/// tests use it to stop the conversion as a crash does.
pub fn convert_folder(
    folder: &Path,
    before: &mut dyn FnMut(Step) -> io::Result<()>,
) -> io::Result<FolderConversion> {
    let invalid = |e: FolderError| io::Error::new(io::ErrorKind::InvalidData, e.to_string());
    let Some(original) = read_optional(&folder.join(SESSION_FILE))? else {
        return Ok(FolderConversion::NotOld);
    };
    let existing = read_optional(&folder.join(NOTE_RECORD_FILE))?;
    let files = match NoteFiles::parse(Some(&original), existing.as_deref()) {
        Ok(files) if files.layout == Layout::Old => files,
        Ok(_) => return Ok(FolderConversion::NotOld),
        Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
    };
    let doc = files.doc;
    // The records of a `longhand.json` that an earlier run of the
    // conversion wrote. They give the summary data after a stop between
    // the `summary.md` write and the `session.md` write.
    let earlier = existing
        .as_deref()
        .and_then(|text| NoteRecord::parse(text).ok())
        .and_then(NoteRecordRead::current);

    let mut record = files.record;
    let summary = read_optional(&folder.join(SUMMARY_FILE))?;
    let final_summary = match &summary {
        Some(text) => {
            let current = Some(fingerprint(text.as_bytes()));
            let (meta, final_text) = match Summary::parse(text) {
                Ok(parsed) => (
                    Some(parsed.meta.clone()),
                    without_summary_data(text).map_err(invalid)?,
                ),
                // A `summary.md` with no summary data: the conversion
                // stopped after its write, or the note has a body only.
                // The `longhand.json` of an earlier run holds the summary
                // data, also for a summary that differs from its
                // fingerprint.
                Err(_) => (
                    earlier.as_ref().and_then(|e| e.summary.clone()),
                    text.clone(),
                ),
            };
            record.summary = meta;
            // A summary that matched its fingerprint matches after the
            // conversion too. A summary that differed still differs.
            let now_matches = doc.summary_fingerprint == current
                || earlier
                    .as_ref()
                    .is_some_and(|e| e.summary_fingerprint == current);
            if now_matches {
                record.summary_fingerprint = Some(fingerprint(final_text.as_bytes()));
            }
            Some((text.clone(), final_text))
        }
        None => None,
    };

    before(Step::Record)?;
    let bytes = note_files::write_record(folder, &record)?;

    if let Some((text, final_text)) = &final_summary
        && text != final_text
    {
        if !unchanged(folder, &original)? {
            return Ok(FolderConversion::Changed);
        }
        before(Step::Summary)?;
        home_write::write_atomic(folder, SUMMARY_FILE, final_text.as_bytes())?;
    }

    if !unchanged(folder, &original)? {
        return Ok(FolderConversion::Changed);
    }
    before(Step::SessionMd)?;
    let patch = RECORD_KEYS
        .iter()
        .fold(FrontmatterPatch::new(), |patch, key| patch.remove(key));
    let session_md = patch.apply(SESSION_FILE, &original).map_err(invalid)?;
    if holds_records(&session_md) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the records cannot be removed from this session.md",
        ));
    }
    home_write::write_atomic(folder, SESSION_FILE, session_md.as_bytes())?;
    Ok(FolderConversion::Converted {
        session_md,
        record: bytes,
    })
}

/// The text of the old-layout `summary.md` text `text` without the summary
/// data. The other keys, the comments and the body stay. When no other key
/// or comment stays, the text is the body only, written as
/// [`Summary::body_text`] writes it.
fn without_summary_data(text: &str) -> Result<String, FolderError> {
    let patched = SUMMARY_KEYS
        .iter()
        .fold(FrontmatterPatch::new(), |patch, key| patch.remove(key))
        .apply(SUMMARY_FILE, text)?;
    Ok(if empty_frontmatter(&patched) {
        Summary::body_text(Summary::body_of(&patched))
    } else {
        patched
    })
}

/// Whether the frontmatter at the start of `text` has only blank lines.
fn empty_frontmatter(text: &str) -> bool {
    text.strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .is_some_and(|rest| {
            rest.lines()
                .take_while(|line| line.trim_end() != "---")
                .all(|line| line.trim().is_empty())
        })
}

/// Whether the `session.md` in `folder` still has the text `original`.
fn unchanged(folder: &Path, original: &str) -> io::Result<bool> {
    Ok(read_optional(&folder.join(SESSION_FILE))?.as_deref() == Some(original))
}

/// Whether the text of a `session.md` still gives an old-layout note.
fn holds_records(session_md: &str) -> bool {
    matches!(
        NoteFiles::parse(Some(session_md), None),
        Ok(files) if files.layout == Layout::Old
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::DateTime;
    use session_core::TitleSource;
    use session_core::folder::{PendingType, ResultFlag, SegmentRecord, SessionDoc, SpeakerCount};
    use session_core::recognition::ArtifactReference;
    use session_save::note_files::{TAGS_KEY, TITLE_KEY};

    use session_core::folder::{Summary, SummaryMeta};

    use super::{RECORD_KEYS, SUMMARY_KEYS};

    /// Each key of the summary data of an old-layout `summary.md` is a key
    /// that the conversion removes.
    #[test]
    fn summary_keys_cover_each_field_of_the_summary_data() {
        for meta in [
            SummaryMeta::Complete {
                endpoint_host: "api.example.com".into(),
                model: "m".into(),
                prompt_name: Some("Team sync".into()),
                prompt_id: Some("b1".into()),
            },
            SummaryMeta::stub(),
        ] {
            let text = Summary {
                meta,
                body: "Body".into(),
            }
            .render();
            let keys = frontmatter_keys(&text);
            let unknown: Vec<_> = keys
                .iter()
                .filter(|key| !SUMMARY_KEYS.contains(&key.as_str()))
                .collect();
            assert!(
                unknown.is_empty(),
                "keys that the conversion keeps: {unknown:?}"
            );
        }
    }

    /// The top-level keys of the frontmatter of `text`.
    fn frontmatter_keys(text: &str) -> Vec<String> {
        let rest = text.strip_prefix("---\n").expect("a frontmatter");
        let (yaml, _) = rest.split_once("\n---\n").expect("a frontmatter end");
        yaml.lines()
            .filter(|line| !line.starts_with([' ', '-', '#']))
            .filter_map(|line| line.split_once(':').map(|(key, _)| key.to_string()))
            .collect()
    }

    /// Each key of an old-layout `session.md` is a record that the
    /// conversion removes, or a user field that stays. The document sets
    /// every optional field, so that each key is in the file. A new field
    /// of `SessionDoc` does not compile here until it is set.
    #[test]
    fn record_keys_cover_each_record_of_session_md() {
        let at = DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap();
        let doc = SessionDoc {
            session_id: "s".into(),
            started: at,
            title: Some("Budget".into()),
            title_source: Some(TitleSource::User),
            tags: vec!["work".into()],
            pending_type: Some(PendingType {
                to: Some("Meetings".into()),
                from: None,
                by: "computer".into(),
                at,
            }),
            speaker_count: Some(SpeakerCount::Exact(2)),
            asr_model: Some("model".into()),
            recognition_snapshot: Some(serde_json::json!({ "model": "model" })),
            recognition_artifact: Some(ArtifactReference {
                version: 1,
                filename: "recognition.jsonl".into(),
                sha256: "0".repeat(64),
            }),
            recognition_audio: Some(BTreeMap::from([("a.wav".into(), "digest".into())])),
            found_speakers: Some(2),
            transcript_fingerprint: Some("transcript".into()),
            summary_fingerprint: Some("summary".into()),
            result: Some(ResultFlag::NotSaved),
            segments: vec![SegmentRecord {
                index: 0,
                wall_start: at,
                wall_end: at,
                offset_ms: 0,
                duration_ms: 1_000,
                audio: "a.wav".into(),
                channels: 2,
                ends_in_pause: true,
            }],
        };
        let keys = frontmatter_keys(&doc.render());
        assert_eq!(keys.len(), 16, "each field gives a key: {keys:?}");
        let unknown: Vec<_> = keys
            .iter()
            .filter(|key| {
                !RECORD_KEYS.contains(&key.as_str())
                    && ![TITLE_KEY, TAGS_KEY].contains(&key.as_str())
            })
            .collect();
        assert!(
            unknown.is_empty(),
            "keys that the conversion keeps: {unknown:?}"
        );
    }
}
