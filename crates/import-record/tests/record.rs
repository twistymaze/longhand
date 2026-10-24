//! The import record of one computer over one storage folder (U3).

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset};
use import_record::{
    ComputerId, Entry, Identity, Lookup, SCHEMA_VERSION, State, WriteError, entry_path,
    identity_folder, lookup, merge, write,
};
use serde_json::{Value, json};

const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn identity() -> Identity {
    Identity::parse(HASH).unwrap()
}

fn computer(id: &str) -> ComputerId {
    ComputerId::parse(id).unwrap()
}

fn time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-10-24T09:28:00+02:00").unwrap()
}

fn imported(id: &str) -> Entry {
    Entry::new(State::Imported, "Memo 12.m4a", 4_096, computer(id), time())
}

fn failed(id: &str, longhand_version: &str) -> Entry {
    Entry::failed(
        "Memo 12.m4a",
        4_096,
        computer(id),
        time(),
        longhand_version,
        "symphonia",
        "0.5.4",
    )
}

/// Every file under `root`, with its bytes, in sorted order.
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                out.push((path.clone(), fs::read(&path).unwrap()));
            } else {
                out.push((path, Vec::new()));
            }
        }
    }
    out.sort();
    out
}

fn put(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let folder = identity_folder(root, &identity());
    fs::create_dir_all(&folder).unwrap();
    let path = folder.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn an_imported_entry_reads_back_with_the_same_state_and_fields() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &identity(), &imported(A)).unwrap();
    let Lookup::Imported(entry) = lookup(root.path(), &identity()) else {
        panic!("the lookup does not give imported");
    };
    assert_eq!(entry, imported(A));
    assert_eq!(entry.schema_version, SCHEMA_VERSION);
    assert_eq!(entry.source_name, "Memo 12.m4a");
    assert_eq!(entry.size, 4_096);
    assert_eq!(entry.computer_id, A);
    assert_eq!(entry.time, time());
    assert!(entry_path(root.path(), &identity(), &computer(A)).is_file());
}

#[test]
fn a_folder_with_no_entry_gives_new() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(lookup(root.path(), &identity()), Lookup::New);
}

#[test]
fn an_unknown_field_stays_after_a_change_of_state() {
    let root = tempfile::tempdir().unwrap();
    let mut first = serde_json::to_value(failed(A, "1.4.0")).unwrap();
    first["note_hint"] = json!({"from": "a newer build"});
    put(
        root.path(),
        &format!("{A}.json"),
        &serde_json::to_vec(&first).unwrap(),
    );
    write(root.path(), &identity(), &imported(A)).unwrap();
    let path = entry_path(root.path(), &identity(), &computer(A));
    let raw: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(raw["state"], "imported");
    assert_eq!(raw["note_hint"], json!({"from": "a newer build"}));
    // The fields of a failed entry go with the state.
    assert!(raw.get("longhand_version").is_none());
}

#[test]
fn a_newer_schema_version_is_read_and_a_write_does_not_lower_it() {
    let root = tempfile::tempdir().unwrap();
    let mut newer = serde_json::to_value(imported(A)).unwrap();
    newer["schema_version"] = json!(SCHEMA_VERSION + 5);
    put(
        root.path(),
        &format!("{A}.json"),
        &serde_json::to_vec(&newer).unwrap(),
    );
    let Lookup::Imported(entry) = lookup(root.path(), &identity()) else {
        panic!("the lookup does not read the newer entry");
    };
    assert_eq!(entry.schema_version, SCHEMA_VERSION + 5);
    let written = write(root.path(), &identity(), &failed(A, "1.4.0")).unwrap();
    assert_eq!(written.schema_version, SCHEMA_VERSION + 5);
    let path = entry_path(root.path(), &identity(), &computer(A));
    let raw: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(raw["schema_version"], json!(SCHEMA_VERSION + 5));
}

#[test]
fn a_file_that_does_not_parse_makes_the_lookup_wait() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &identity(), &imported(A)).unwrap();
    put(root.path(), &format!("{B}.json"), b"{\"state\": \"fail");
    assert_eq!(lookup(root.path(), &identity()), Lookup::Wait);
}

#[test]
fn a_write_does_not_overwrite_a_file_that_does_not_parse() {
    let root = tempfile::tempdir().unwrap();
    let path = put(root.path(), &format!("{A}.json"), b"{\"state\": \"impo");
    let result = write(root.path(), &identity(), &failed(A, "1.4.0"));
    assert!(matches!(result, Err(WriteError::DoesNotParse(_))));
    assert_eq!(fs::read(&path).unwrap(), b"{\"state\": \"impo");
}

#[test]
fn a_file_over_the_size_limit_makes_the_lookup_wait() {
    let root = tempfile::tempdir().unwrap();
    let mut large = serde_json::to_value(imported(A)).unwrap();
    large["padding"] = json!("x".repeat(import_record::MAX_ENTRY_BYTES as usize));
    put(
        root.path(),
        &format!("{A}.json"),
        &serde_json::to_vec(&large).unwrap(),
    );
    assert_eq!(lookup(root.path(), &identity()), Lookup::Wait);
}

#[test]
fn a_conflict_copy_has_no_effect_on_the_lookup() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &identity(), &failed(A, "1.4.0")).unwrap();
    let copy = serde_json::to_vec(&imported(A)).unwrap();
    put(root.path(), &format!("{A} 2.json"), &copy);
    put(root.path(), &format!("{A} (conflicted copy).json"), b"{");
    assert!(matches!(
        lookup(root.path(), &identity()),
        Lookup::Failed(_)
    ));
}

#[test]
fn a_file_name_that_is_not_a_computer_id_is_ignored() {
    let root = tempfile::tempdir().unwrap();
    let entry = serde_json::to_vec(&imported(A)).unwrap();
    put(root.path(), &format!("{}.json", A.to_uppercase()), &entry);
    put(root.path(), "aaaa.json", &entry);
    put(root.path(), &format!("{A}.txt"), &entry);
    put(
        root.path(),
        &format!(".{A}.json.0123456789abcdef.tmp"),
        b"{",
    );
    assert_eq!(lookup(root.path(), &identity()), Lookup::New);
}

#[test]
fn an_identity_and_a_computer_id_accept_only_their_form() {
    assert!(Identity::parse(HASH).is_some());
    assert!(Identity::parse(&HASH.to_uppercase()).is_none());
    assert!(Identity::parse(&HASH[1..]).is_none());
    assert!(Identity::parse(&format!("{}g", &HASH[1..])).is_none());
    assert!(Identity::parse("../../etc").is_none());
    assert!(ComputerId::parse(A).is_some());
    assert!(ComputerId::parse(&A.to_uppercase()).is_none());
    assert!(ComputerId::parse(&A[1..]).is_none());
    assert!(ComputerId::parse("../aaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_none());
}

#[test]
fn a_write_with_a_computer_id_that_is_not_valid_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let mut entry = imported(A);
    entry.computer_id = "../x".into();
    assert!(matches!(
        write(root.path(), &identity(), &entry),
        Err(WriteError::InvalidComputerId)
    ));
    assert_eq!(snapshot(root.path()), Vec::new());
}

#[cfg(unix)]
#[test]
fn a_link_that_points_outside_the_storage_folder_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.json");
    fs::write(&secret, serde_json::to_vec(&imported(B)).unwrap()).unwrap();
    let folder = identity_folder(root.path(), &identity());
    fs::create_dir_all(&folder).unwrap();
    std::os::unix::fs::symlink(&secret, folder.join(format!("{B}.json"))).unwrap();
    // The lookup does not read the file outside, and does not take the
    // identity as new.
    assert_eq!(lookup(root.path(), &identity()), Lookup::Wait);
}

#[cfg(unix)]
#[test]
fn a_linked_identity_folder_outside_the_storage_folder_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(
        outside.path().join(format!("{B}.json")),
        serde_json::to_vec(&imported(B)).unwrap(),
    )
    .unwrap();
    let folder = identity_folder(root.path(), &identity());
    fs::create_dir_all(folder.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(outside.path(), &folder).unwrap();
    assert_eq!(lookup(root.path(), &identity()), Lookup::Wait);
    // A write does not go into the folder outside.
    assert!(write(root.path(), &identity(), &imported(A)).is_err());
    assert!(!outside.path().join(format!("{A}.json")).exists());
}

#[cfg(unix)]
#[test]
fn a_link_to_a_file_in_the_storage_folder_is_read() {
    let root = tempfile::tempdir().unwrap();
    let inside = root.path().join("kept.json");
    fs::write(&inside, serde_json::to_vec(&imported(B)).unwrap()).unwrap();
    let folder = identity_folder(root.path(), &identity());
    fs::create_dir_all(&folder).unwrap();
    std::os::unix::fs::symlink(&inside, folder.join(format!("{B}.json"))).unwrap();
    assert!(matches!(
        lookup(root.path(), &identity()),
        Lookup::Imported(_)
    ));
}

#[test]
fn a_lookup_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &identity(), &failed(A, "1.4.0")).unwrap();
    put(root.path(), &format!("{B}.json"), b"{");
    put(root.path(), &format!("{B} 2.json"), b"{");
    let before = snapshot(root.path());
    lookup(root.path(), &identity());
    lookup(root.path(), &Identity::parse(&"f".repeat(64)).unwrap());
    assert_eq!(snapshot(root.path()), before);
}

#[test]
fn a_failed_entry_of_1_4_allows_a_retry_on_1_5_and_not_on_1_4() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &identity(), &failed(A, "1.4.0")).unwrap();
    let Lookup::Failed(entry) = lookup(root.path(), &identity()) else {
        panic!("the lookup does not give failed");
    };
    assert_eq!(entry.longhand_version.as_deref(), Some("1.4.0"));
    assert_eq!(entry.decoder_name.as_deref(), Some("symphonia"));
    assert_eq!(entry.decoder_version.as_deref(), Some("0.5.4"));
    assert!(entry.allows_retry("1.5.0"));
    assert!(entry.allows_retry("1.10.0"));
    assert!(!entry.allows_retry("1.4.0"));
    assert!(!entry.allows_retry("1.3.9"));
}

#[test]
fn the_newest_failed_entry_decides_the_retry() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &identity(), &failed(A, "1.4.0")).unwrap();
    write(root.path(), &identity(), &failed(B, "1.5.0")).unwrap();
    let Lookup::Failed(entry) = lookup(root.path(), &identity()) else {
        panic!("the lookup does not give failed");
    };
    assert!(!entry.allows_retry("1.5.0"));
}

#[test]
fn imported_wins_over_failed() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &identity(), &imported(A)).unwrap();
    write(root.path(), &identity(), &failed(B, "1.4.0")).unwrap();
    assert!(matches!(
        lookup(root.path(), &identity()),
        Lookup::Imported(_)
    ));
}

#[test]
fn the_merge_copies_a_missing_entry_and_keeps_an_entry_that_the_new_folder_has() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    let other = Identity::parse(&"e".repeat(64)).unwrap();
    write(old.path(), &identity(), &imported(A)).unwrap();
    write(old.path(), &other, &failed(A, "1.4.0")).unwrap();
    write(
        new.path(),
        &other,
        &Entry::new(State::Skipped, "x.m4a", 1, computer(A), time()),
    )
    .unwrap();
    let merged = merge(old.path(), new.path()).unwrap();
    assert_eq!(merged.copied, 1);
    assert_eq!(merged.kept, 1);
    assert_eq!(
        fs::read(entry_path(new.path(), &identity(), &computer(A))).unwrap(),
        fs::read(entry_path(old.path(), &identity(), &computer(A))).unwrap()
    );
    assert!(matches!(lookup(new.path(), &other), Lookup::Skipped(_)));
}

#[test]
fn the_merge_ignores_names_that_are_not_an_identity_or_a_computer_id() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    let entry = serde_json::to_vec(&imported(A)).unwrap();
    let imports = identity_folder(old.path(), &identity())
        .parent()
        .unwrap()
        .to_path_buf();
    // A file system that ignores case takes an upper-case name of HASH as
    // the folder of HASH, so the upper-case name is of another identity.
    for folder in ["f".repeat(64).to_uppercase(), "abc".to_string()] {
        let folder = imports.join(folder);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(format!("{A}.json")), &entry).unwrap();
    }
    put(old.path(), &format!("{A} 2.json"), &entry);
    let merged = merge(old.path(), new.path()).unwrap();
    assert_eq!(merged.copied, 0);
    assert_eq!(snapshot(new.path()), Vec::new());
}

#[test]
fn the_entry_of_one_computer_reads_only_its_own_file() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        import_record::entry_of(root.path(), &identity(), &computer(A)),
        None
    );
    write(root.path(), &identity(), &imported(B)).unwrap();
    assert_eq!(
        import_record::entry_of(root.path(), &identity(), &computer(A)),
        None
    );
    write(root.path(), &identity(), &imported(A)).unwrap();
    assert_eq!(
        import_record::entry_of(root.path(), &identity(), &computer(A)),
        Some(imported(A))
    );
}
