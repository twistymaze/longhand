//! Writes into a landed folder: whole files only, and never through a link.

use landing::post_landing::{write_atomic, write_atomic_via};

#[test]
fn a_write_replaces_the_file_whole_and_leaves_no_temporary() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.md"), "old").unwrap();
    write_atomic(dir.path(), "notes.md", b"new notes").unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("notes.md")).unwrap(),
        "new notes"
    );
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("notes.md")]);
}

#[cfg(unix)]
#[test]
fn a_temporary_path_pre_created_as_a_link_is_refused_and_its_target_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let victim = elsewhere.path().join("important.txt");
    std::fs::write(&victim, "keep me").unwrap();
    std::os::unix::fs::symlink(&victim, dir.path().join(".notes.md.fixed.tmp")).unwrap();

    let err = write_atomic_via(dir.path(), "notes.md", ".notes.md.fixed.tmp", b"x").unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
    assert!(!dir.path().join("notes.md").exists());
}

#[test]
fn temporary_names_differ_between_writes() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..50 {
        write_atomic(dir.path(), "summary.md", format!("v{i}").as_bytes()).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("summary.md")).unwrap(),
        "v49"
    );
}
