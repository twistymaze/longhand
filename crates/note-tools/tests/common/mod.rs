//! A copy of the checked-in test storage folder for each test.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

pub const BUDGET: &str = "2026-10-01 0900 Budget review";
pub const DESIGN: &str = "2026-10-03 1000 Design review";
pub const QUICK: &str = "2026-10-07 1300 Quick idea";

pub fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/storage-folder")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// A temporary copy of the test storage folder.
pub fn storage() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture(), dir.path());
    dir
}

/// Copy the note folder `name` to `new_name` with the note id `id`.
pub fn copy_note(root: &Path, name: &str, new_name: &str, id: Option<&str>) -> PathBuf {
    let target = root.join(new_name);
    copy_dir(&root.join(name), &target);
    if let Some(id) = id {
        let record = target.join("longhand.json");
        let text = fs::read_to_string(&record).unwrap();
        let old = text
            .split("\"note_id\": \"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap()
            .to_string();
        fs::write(&record, text.replace(&old, id)).unwrap();
    }
    target
}

/// A transcript of `minutes` minutes with one line every 30 seconds.
pub fn long_transcript(minutes: u64) -> String {
    let mut out = String::from("# Transcript\n\n");
    for i in 0..minutes * 2 {
        let s = i * 30;
        out.push_str(&format!(
            "- [{:02}:{:02}:{:02}] Speaker {}: line {i} of the talk.\n",
            s / 3600,
            (s / 60) % 60,
            s % 60,
            i % 2 + 1
        ));
    }
    out
}
