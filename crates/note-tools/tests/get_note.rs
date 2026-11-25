//! Get note on the test storage folder (R4, R5, R7, AE2, AE3, KTD11).

mod common;

use std::fs;

use chrono::DateTime;
use common::{BUDGET, QUICK, copy_note, long_transcript, storage};
use note_tools::get_note::{GetNoteInput, GetNoteOutput, TranscriptRequest};
use note_tools::storage::PartState;
use note_tools::{ErrorKind, GetNote, Tool};
use session_core::folder::SessionDoc;

fn get(root: &std::path::Path, max_chars: usize, input: GetNoteInput) -> GetNoteOutput {
    GetNote { max_chars }.call(root, input).unwrap()
}

fn input(id: &str, transcript: Option<TranscriptRequest>) -> GetNoteInput {
    GetNoteInput {
        note_id: id.into(),
        transcript,
        ..Default::default()
    }
}

#[test]
fn the_details_and_the_asked_parts_come_back() {
    let dir = storage();
    let out = get(
        dir.path(),
        10_000,
        GetNoteInput {
            note_id: "note-budget-review".into(),
            manual_notes: true,
            summary: true,
            transcript: Some(TranscriptRequest::default()),
        },
    );
    assert_eq!(out.title.as_deref(), Some("Budget review"));
    assert_eq!(out.note_type, "General");
    assert_eq!(out.transcript_state, PartState::Present);
    assert_eq!(out.transcript_lines, Some(8));
    assert!(out.manual_notes.unwrap().text.is_some());
    let summary = out.summary.unwrap().text.unwrap();
    assert!(!summary.starts_with("---"));
    let transcript = out.transcript.unwrap();
    assert_eq!(transcript.hunks.len(), 1);
    assert_eq!(transcript.hunks[0].first_line, 1);
    assert_eq!(transcript.hunks[0].last_line, 8);
    assert!(transcript.hunks[0].text.contains("- [00:00:09] Speaker 2:"));
    assert!(!transcript.truncated);
    // Only the details, when nothing else is asked for.
    let bare = get(dir.path(), 10_000, input("note-budget-review", None));
    assert!(bare.transcript.is_none() && bare.summary.is_none() && bare.manual_notes.is_none());
}

#[test]
fn a_time_range_gives_only_the_lines_in_it_with_their_time_stamps() {
    let dir = storage();
    fs::write(
        dir.path().join(BUDGET).join("transcript.md"),
        long_transcript(240),
    )
    .unwrap();
    let out = get(
        dir.path(),
        100_000,
        input(
            "note-budget-review",
            Some(TranscriptRequest {
                from_time: Some("03:55:00".into()),
                to_time: Some("04:00:00".into()),
                ..Default::default()
            }),
        ),
    );
    let part = out.transcript.unwrap();
    assert_eq!(part.hunks.len(), 1);
    let lines: Vec<&str> = part.hunks[0].text.lines().collect();
    assert_eq!(
        lines.first().unwrap(),
        &"- [03:55:00] Speaker 1: line 470 of the talk."
    );
    assert_eq!(
        lines.last().unwrap(),
        &"- [03:59:30] Speaker 2: line 479 of the talk."
    );
    assert_eq!(lines.len(), 10);
}

#[test]
fn a_pattern_gives_each_match_with_its_context_and_overlaps_once() {
    let dir = storage();
    fs::write(
        dir.path().join(BUDGET).join("transcript.md"),
        long_transcript(30),
    )
    .unwrap();
    // Lines 3, 4 and 5 are the talk lines 0, 1 and 2: matches at 10 and 12
    // overlap with a context of 2, and the match at 40 stands alone.
    let out = get(
        dir.path(),
        100_000,
        input(
            "note-budget-review",
            Some(TranscriptRequest {
                pattern: Some(r"line (7|9|37) of".into()),
                context: Some(2),
                ..Default::default()
            }),
        ),
    );
    let part = out.transcript.unwrap();
    assert_eq!(part.matches, Some(3));
    let ranges: Vec<(usize, usize)> = part
        .hunks
        .iter()
        .map(|h| (h.first_line, h.last_line))
        .collect();
    assert_eq!(ranges, [(8, 14), (38, 42)]);
}

#[test]
fn a_long_transcript_comes_back_one_page_at_a_time() {
    let dir = storage();
    fs::write(
        dir.path().join(BUDGET).join("transcript.md"),
        long_transcript(240),
    )
    .unwrap();
    let first = get(
        dir.path(),
        2_000,
        input("note-budget-review", Some(TranscriptRequest::default())),
    )
    .transcript
    .unwrap();
    assert!(first.truncated);
    let next = first.next_line.unwrap();
    assert_eq!(first.hunks.last().unwrap().last_line + 1, next);
    let text: usize = first.hunks.iter().map(|h| h.text.chars().count() + 1).sum();
    assert!(text <= 2_000);
    let second = get(
        dir.path(),
        2_000,
        input(
            "note-budget-review",
            Some(TranscriptRequest {
                from_line: Some(next),
                ..Default::default()
            }),
        ),
    )
    .transcript
    .unwrap();
    assert_eq!(second.hunks[0].first_line, next);
}

#[test]
fn a_line_longer_than_the_cap_comes_back_cut_and_marked_truncated() {
    let dir = storage();
    let text = format!(
        "[00:00:01] Speaker 1: {}\n[00:00:02] Speaker 1: next\n",
        "word ".repeat(1_000)
    );
    fs::write(dir.path().join(BUDGET).join("transcript.md"), text).unwrap();
    let part = get(
        dir.path(),
        500,
        input("note-budget-review", Some(TranscriptRequest::default())),
    )
    .transcript
    .unwrap();
    assert!(part.truncated);
    let sent: usize = part.hunks.iter().map(|h| h.text.chars().count()).sum();
    assert!(sent < 500, "{sent}");
    assert_eq!(part.next_line, Some(2));
}

#[test]
fn a_bad_unknown_or_shared_note_id_is_refused() {
    let dir = storage();
    for id in [
        "../2026-10-01 0900 Budget review",
        "/etc",
        "Note-Budget-Review",
        "",
    ] {
        let error = GetNote { max_chars: 1_000 }
            .call(dir.path(), input(id, None))
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidInput, "{id}");
    }
    let error = GetNote { max_chars: 1_000 }
        .call(dir.path(), input("note-nobody", None))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotFound);
    copy_note(
        dir.path(),
        BUDGET,
        "2026-10-01 0900 Budget review copy",
        None,
    );
    let error = GetNote { max_chars: 1_000 }
        .call(dir.path(), input("note-budget-review", None))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Ambiguous);
}

#[test]
fn a_note_with_no_transcript_or_a_removed_one_says_so() {
    let dir = storage();
    let quick = get(
        dir.path(),
        1_000,
        input("note-quick-idea", Some(TranscriptRequest::default())),
    );
    assert_eq!(quick.transcript_state, PartState::Missing);
    assert!(quick.transcript.is_none());
    fs::write(
        dir.path()
            .join(BUDGET)
            .join(session_core::folder::TRANSCRIPT_REMOVED_MARKER),
        session_core::folder::TRANSCRIPT_REMOVED_TEXT,
    )
    .unwrap();
    let removed = get(dir.path(), 1_000, input("note-budget-review", None));
    assert_eq!(removed.transcript_state, PartState::Removed);
}

#[test]
fn an_old_layout_note_gives_its_summary_body_without_the_old_frontmatter() {
    let dir = storage();
    let note = dir.path().join("2026-09-30 0800 Old note");
    fs::create_dir_all(&note).unwrap();
    let started = DateTime::parse_from_rfc3339("2026-09-30T08:00:00+02:00").unwrap();
    let mut doc = SessionDoc::without_audio("old-note", started, None);
    doc.title = Some("Old note".into());
    doc.tags = vec!["Legacy".into()];
    fs::write(note.join("session.md"), doc.render()).unwrap();
    fs::write(
        note.join("summary.md"),
        "---\nmodel: m\n---\n\nThe old summary.\n",
    )
    .unwrap();
    let out = get(
        dir.path(),
        1_000,
        GetNoteInput {
            note_id: "old-note".into(),
            summary: true,
            ..Default::default()
        },
    );
    assert_eq!(out.tags, ["Legacy"]);
    assert_eq!(
        out.summary.unwrap().text.as_deref(),
        Some("The old summary.")
    );
}

#[cfg(unix)]
#[test]
fn a_part_that_links_outside_the_storage_folder_is_not_read() {
    let dir = storage();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.md");
    fs::write(&secret, "- [00:00:01] Speaker 1: a secret outside.\n").unwrap();
    let note = copy_note(
        dir.path(),
        QUICK,
        "2026-10-12 0900 Linked parts",
        Some("note-linked"),
    );
    std::os::unix::fs::symlink(&secret, note.join("transcript.md")).unwrap();
    std::os::unix::fs::symlink(&secret, note.join("summary.md")).unwrap();
    let out = get(
        dir.path(),
        1_000,
        GetNoteInput {
            note_id: "note-linked".into(),
            manual_notes: true,
            summary: true,
            transcript: Some(TranscriptRequest::default()),
        },
    );
    assert!(!serde_json::to_string(&out).unwrap().contains("secret"));
    assert_eq!(out.transcript_state, PartState::Unreadable);
    assert!(out.transcript.is_none());
    assert_eq!(out.summary.unwrap().state, PartState::Unreadable);
    assert_eq!(out.manual_notes.unwrap().state, PartState::Present);
}
