//! Two computers over one synced storage folder (U3). Each computer has
//! its own copy of the storage folder. A sync copies each file of one copy
//! into the other, as a sync service does.

use std::fs;
use std::path::Path;

use chrono::{DateTime, FixedOffset};
use import_record::{ComputerId, Entry, Identity, Lookup, State, lookup, write};

const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn identity() -> Identity {
    Identity::parse(HASH).unwrap()
}

fn time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-10-24T09:28:00+02:00").unwrap()
}

fn entry(id: &str, state: State) -> Entry {
    let computer = ComputerId::parse(id).unwrap();
    match state {
        State::Failed => Entry::failed(
            "Memo 12.m4a",
            4_096,
            computer,
            time(),
            "1.4.0",
            "symphonia",
            "0.5.4",
        ),
        state => Entry::new(state, "Memo 12.m4a", 4_096, computer, time()),
    }
}

/// Copy each file under `from` that `to` does not have into `to`.
fn sync(from: &Path, to: &Path) {
    for item in fs::read_dir(from).unwrap() {
        let item = item.unwrap();
        let target = to.join(item.file_name());
        if item.file_type().unwrap().is_dir() {
            fs::create_dir_all(&target).unwrap();
            sync(&item.path(), &target);
        } else if !target.exists() {
            fs::copy(item.path(), target).unwrap();
        }
    }
}

/// Computer A writes `on_a` and computer B writes `on_b` for one identity,
/// each in its own copy of the storage folder. Then both copies sync.
/// Returns the lookup on A and the lookup on B.
fn both_write_then_sync(on_a: State, on_b: State) -> (Lookup, Lookup) {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(a.path(), &identity(), &entry(A, on_a)).unwrap();
    write(b.path(), &identity(), &entry(B, on_b)).unwrap();
    sync(a.path(), b.path());
    sync(b.path(), a.path());
    (lookup(a.path(), &identity()), lookup(b.path(), &identity()))
}

#[test]
fn skipped_on_a_and_failed_on_b_give_skipped_on_each() {
    let (on_a, on_b) = both_write_then_sync(State::Skipped, State::Failed);
    assert!(matches!(on_a, Lookup::Skipped(_)), "{on_a:?}");
    assert!(matches!(on_b, Lookup::Skipped(_)), "{on_b:?}");
}

#[test]
fn imported_on_a_and_failed_on_b_give_imported_on_each() {
    let (on_a, on_b) = both_write_then_sync(State::Imported, State::Failed);
    assert!(matches!(on_a, Lookup::Imported(_)), "{on_a:?}");
    assert!(matches!(on_b, Lookup::Imported(_)), "{on_b:?}");
}

#[test]
fn imported_on_a_shows_on_b_after_the_sync() {
    // R13: B reads the record before an import, and does not import the
    // file again.
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(a.path(), &identity(), &entry(A, State::Imported)).unwrap();
    assert_eq!(lookup(b.path(), &identity()), Lookup::New);
    sync(a.path(), b.path());
    let Lookup::Imported(on_b) = lookup(b.path(), &identity()) else {
        panic!("B does not see the import of A");
    };
    assert_eq!(on_b.computer_id, A);
}

#[test]
fn each_computer_writes_only_its_own_file() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path(), &identity(), &entry(A, State::Imported)).unwrap();
    write(folder.path(), &identity(), &entry(B, State::Failed)).unwrap();
    let a_file =
        import_record::entry_path(folder.path(), &identity(), &ComputerId::parse(A).unwrap());
    let a_bytes = fs::read(&a_file).unwrap();
    write(folder.path(), &identity(), &entry(B, State::Skipped)).unwrap();
    assert_eq!(fs::read(&a_file).unwrap(), a_bytes);
}
