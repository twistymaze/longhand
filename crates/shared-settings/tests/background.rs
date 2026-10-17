//! The Background file (U1): `.longhand/background.md` is read with its
//! exact name, and only a user save writes it (R1, R5, R6, KTD1, KTD2,
//! KTD3).

use std::fs;
use std::path::{Path, PathBuf};

use shared_settings::background::{
    BACKGROUND_FILE, BackgroundError, BackgroundRead, BackgroundUnavailable, read_background,
    save_background,
};
use shared_settings::cache::Cache;
use shared_settings::file::{Unavailable, default_settings};
use shared_settings::view::{Availability, cached, load};
use summarize::prompts::BACKGROUND_TEXT_LIMIT;

/// A storage folder with the default shared settings.
fn folder() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let longhand = root.path().join(".longhand");
    fs::create_dir_all(&longhand).unwrap();
    fs::write(
        longhand.join("settings.json"),
        serde_json::to_vec(&default_settings()).unwrap(),
    )
    .unwrap();
    root
}

fn background_path(root: &Path) -> PathBuf {
    root.join(".longhand").join(BACKGROUND_FILE)
}

fn write_background(root: &Path, bytes: impl AsRef<[u8]>) {
    fs::write(background_path(root), bytes).unwrap();
}

/// The file names in the hidden Longhand folder.
fn names_in_longhand(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root.join(".longhand"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn cache_with(source: &Path, background: &str) -> Cache {
    Cache {
        source: source.to_path_buf(),
        settings: default_settings(),
        prompts: Vec::new(),
        active_prompt: None,
        type_prompts: Vec::new(),
        background: background.into(),
    }
}

#[test]
fn the_file_name_is_background_md() {
    assert_eq!(BACKGROUND_FILE, "background.md");
}

#[test]
fn a_folder_with_a_background_file_reads_the_text() {
    let root = folder();
    write_background(root.path(), "I am Ann. My manager is Bo Lindqvist.\n");
    assert_eq!(
        read_background(root.path()),
        BackgroundRead::Text("I am Ann. My manager is Bo Lindqvist.\n".into())
    );
    let view = load(root.path(), None).view;
    assert_eq!(view.background, "I am Ann. My manager is Bo Lindqvist.\n");
    assert!(view.background_writable());
    assert_eq!(view.background_unavailable, None);
}

#[test]
fn a_missing_file_is_an_empty_background_that_the_user_can_edit() {
    let root = folder();
    assert_eq!(read_background(root.path()), BackgroundRead::Missing);
    let loaded = load(root.path(), None);
    assert_eq!(loaded.view.background, "");
    assert!(loaded.view.background_writable());
    // The read and the load write nothing.
    assert_eq!(names_in_longhand(root.path()), vec!["settings.json"]);
}

#[test]
fn a_file_at_the_limit_reads_the_text() {
    let root = folder();
    // Each character is two bytes, so the limit counts characters.
    let text = "é".repeat(BACKGROUND_TEXT_LIMIT);
    write_background(root.path(), text.as_bytes());
    assert_eq!(read_background(root.path()), BackgroundRead::Text(text));
}

#[test]
fn a_file_over_the_limit_is_not_available_and_a_save_does_not_change_it() {
    let root = folder();
    let long = "a".repeat(BACKGROUND_TEXT_LIMIT + 1);
    write_background(root.path(), long.as_bytes());
    assert_eq!(
        read_background(root.path()),
        BackgroundRead::NotAvailable(BackgroundUnavailable::TooLong)
    );
    let err = save_background(root.path(), "Short text.").unwrap_err();
    assert!(matches!(
        err,
        BackgroundError::FileNotAvailable(BackgroundUnavailable::TooLong)
    ));
    assert_eq!(
        fs::read(background_path(root.path())).unwrap(),
        long.as_bytes()
    );
}

#[test]
fn a_very_large_file_is_not_available_and_is_not_read_whole() {
    let root = folder();
    let huge = vec![b'a'; BACKGROUND_TEXT_LIMIT * 4 + 1];
    write_background(root.path(), &huge);
    assert_eq!(
        read_background(root.path()),
        BackgroundRead::NotAvailable(BackgroundUnavailable::TooLong)
    );
}

#[test]
fn a_file_that_is_not_utf8_is_not_available_and_nothing_writes_it() {
    let root = folder();
    let bytes = [b'A', 0xff, 0xfe, b'B'];
    write_background(root.path(), bytes);
    assert_eq!(
        read_background(root.path()),
        BackgroundRead::NotAvailable(BackgroundUnavailable::NotText)
    );
    assert!(save_background(root.path(), "New text.").is_err());
    assert_eq!(fs::read(background_path(root.path())).unwrap(), bytes);
}

#[test]
fn a_background_that_is_not_a_file_is_not_available() {
    let root = folder();
    fs::create_dir(background_path(root.path())).unwrap();
    assert!(matches!(
        read_background(root.path()),
        BackgroundRead::NotAvailable(BackgroundUnavailable::Unreadable(_))
    ));
    assert!(save_background(root.path(), "New text.").is_err());
    assert!(background_path(root.path()).is_dir());
}

#[test]
fn a_file_that_cannot_be_used_gives_the_cached_text_as_read_only() {
    let root = folder();
    write_background(root.path(), [0xff]);
    let cache = cache_with(root.path(), "The cached Background.");
    let loaded = load(root.path(), Some(&cache));
    assert_eq!(loaded.view.availability, Availability::Available);
    assert_eq!(loaded.view.background, "The cached Background.");
    assert_eq!(
        loaded.view.background_unavailable,
        Some(BackgroundUnavailable::NotText)
    );
    assert!(!loaded.view.background_writable());
    // The cache keeps the cached text.
    assert!(
        loaded
            .cache
            .is_none_or(|c| c.background == "The cached Background.")
    );
    assert_eq!(fs::read(background_path(root.path())).unwrap(), [0xff]);
}

#[test]
fn a_conflict_copy_is_ignored() {
    let root = folder();
    write_background(root.path(), "The real text.");
    fs::write(
        root.path().join(".longhand").join("background 2.md"),
        "The copy.",
    )
    .unwrap();
    assert_eq!(
        read_background(root.path()),
        BackgroundRead::Text("The real text.".into())
    );
}

#[test]
fn a_conflict_copy_alone_is_a_missing_file() {
    let root = folder();
    fs::write(
        root.path().join(".longhand").join("background 2.md"),
        "The copy.",
    )
    .unwrap();
    assert_eq!(read_background(root.path()), BackgroundRead::Missing);
    assert_eq!(load(root.path(), None).view.background, "");
}

#[test]
fn a_save_writes_the_text_and_a_read_gives_it_back() {
    let root = folder();
    let text = "# About me\n\n- Ann Ödegaard, product lead\n- *Longhand* is the app\n";
    save_background(root.path(), text).unwrap();
    assert_eq!(
        fs::read_to_string(background_path(root.path())).unwrap(),
        text
    );
    assert_eq!(
        read_background(root.path()),
        BackgroundRead::Text(text.into())
    );
    // The atomic write leaves no temporary file.
    assert_eq!(
        names_in_longhand(root.path()),
        vec!["background.md", "settings.json"]
    );
}

#[test]
fn a_settings_file_from_a_newer_version_still_reads_and_saves_the_background() {
    let root = folder();
    let mut newer = default_settings();
    newer.insert("schema_version".into(), serde_json::json!(999));
    let settings = root.path().join(".longhand").join("settings.json");
    fs::write(&settings, serde_json::to_vec(&newer).unwrap()).unwrap();
    let before = fs::read(&settings).unwrap();
    write_background(root.path(), "Old text.");
    assert_eq!(
        read_background(root.path()),
        BackgroundRead::Text("Old text.".into())
    );
    save_background(root.path(), "New text.").unwrap();
    assert_eq!(
        read_background(root.path()),
        BackgroundRead::Text("New text.".into())
    );
    // A Background save does not write the settings file.
    assert_eq!(fs::read(&settings).unwrap(), before);
}

#[test]
fn a_save_replaces_the_old_text() {
    let root = folder();
    save_background(root.path(), "Old text.").unwrap();
    save_background(root.path(), "New text.").unwrap();
    assert_eq!(
        read_background(root.path()),
        BackgroundRead::Text("New text.".into())
    );
}

#[test]
fn a_save_of_an_empty_text_writes_an_empty_file() {
    let root = folder();
    save_background(root.path(), "Some text.").unwrap();
    save_background(root.path(), "").unwrap();
    assert!(background_path(root.path()).is_file());
    assert_eq!(fs::read(background_path(root.path())).unwrap(), b"");
    assert_eq!(
        read_background(root.path()),
        BackgroundRead::Text(String::new())
    );
}

#[test]
fn a_save_over_the_limit_is_refused_and_writes_nothing() {
    let root = folder();
    save_background(root.path(), "Kept.").unwrap();
    let err = save_background(root.path(), &"a".repeat(BACKGROUND_TEXT_LIMIT + 1)).unwrap_err();
    assert!(matches!(err, BackgroundError::TooLong));
    assert_eq!(
        err.to_string(),
        "The Background is too long. Use 4,000 characters or fewer."
    );
    assert_eq!(
        fs::read_to_string(background_path(root.path())).unwrap(),
        "Kept."
    );
}

#[test]
fn a_save_when_the_settings_file_does_not_parse_writes_only_the_background() {
    // R22: the settings in other files still save.
    let root = folder();
    let settings = root.path().join(".longhand").join("settings.json");
    fs::write(&settings, "{ \"schema_version\": ").unwrap();
    save_background(root.path(), "Text.").unwrap();
    assert_eq!(
        fs::read_to_string(background_path(root.path())).unwrap(),
        "Text."
    );
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        "{ \"schema_version\": "
    );
}

#[test]
fn a_save_when_the_folder_has_no_settings_file_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let err = save_background(root.path(), "Text.").unwrap_err();
    assert!(matches!(err, BackgroundError::NoSettingsFile));
    assert!(!root.path().join(".longhand").exists());
}

#[test]
fn a_save_when_the_folder_is_missing_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("not-here");
    let err = save_background(&missing, "Text.").unwrap_err();
    assert!(matches!(
        err,
        BackgroundError::NotAvailable(Unavailable::FolderMissing)
    ));
    assert!(!missing.exists());
}

#[test]
fn a_read_of_a_missing_folder_is_not_available() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("not-here");
    assert!(matches!(
        read_background(&missing),
        BackgroundRead::NotAvailable(BackgroundUnavailable::Folder(Unavailable::FolderMissing))
    ));
}

#[test]
fn a_settings_file_that_does_not_parse_gives_the_folder_background_and_keeps_the_cache() {
    // R22: the Background is in another file, so it still changes.
    let root = folder();
    write_background(root.path(), "The folder's text.");
    fs::write(root.path().join(".longhand").join("settings.json"), "{").unwrap();
    let cache = cache_with(root.path(), "The cached text.");
    let loaded = load(root.path(), Some(&cache));
    assert!(matches!(
        loaded.view.availability,
        Availability::NotAvailable(_)
    ));
    assert_eq!(loaded.view.background, "The folder's text.");
    assert!(loaded.view.background_writable());
    assert_eq!(loaded.cache, None);
}

#[test]
fn a_load_puts_the_background_in_the_new_cache() {
    let root = folder();
    write_background(root.path(), "Text for the cache.");
    let cache = load(root.path(), None).cache.unwrap();
    assert_eq!(cache.background, "Text for the cache.");
    // A cache with the same text gives no new cache.
    assert_eq!(load(root.path(), Some(&cache)).cache, None);
}

#[test]
fn a_view_with_no_folder_uses_the_cached_background() {
    let cache = cache_with(Path::new("/nowhere"), "Cached.");
    let view = cached(
        Some(&cache),
        Availability::NotAvailable(Unavailable::FolderMissing),
    );
    assert_eq!(view.background, "Cached.");
    assert!(!view.background_writable());
    assert_eq!(cached(None, Availability::MissingFile).background, "");
}

#[test]
fn a_cache_from_an_older_version_has_an_empty_background() {
    let json = serde_json::json!({
        "source": "/folder",
        "settings": {},
        "prompts": [],
    });
    let cache: Cache = serde_json::from_value(json).unwrap();
    assert_eq!(cache.background, "");
}

/// A folder outside the storage folder with the file `name` that holds
/// `text`.
#[cfg(unix)]
fn outside_file(name: &str, text: &str) -> (tempfile::TempDir, PathBuf) {
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join(name);
    fs::write(&path, text).unwrap();
    (outside, path)
}

#[cfg(unix)]
#[test]
fn a_background_that_links_outside_the_folder_is_not_available() {
    let root = folder();
    let (_outside, target) = outside_file("secret.md", "Outside text.");
    std::os::unix::fs::symlink(&target, background_path(root.path())).unwrap();
    assert!(matches!(
        read_background(root.path()),
        BackgroundRead::NotAvailable(BackgroundUnavailable::Unreadable(_))
    ));
    let loaded = load(root.path(), None);
    assert_ne!(loaded.view.background, "Outside text.");
    assert!(loaded.view.background_unavailable.is_some());
    // Settings lets the user save, and the save replaces the link.
    assert!(loaded.view.background_writable());
    assert!(
        fs::symlink_metadata(background_path(root.path()))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "Outside text.");
}

#[cfg(unix)]
#[test]
fn a_background_that_links_to_a_file_in_the_folder_reads_the_text() {
    let root = folder();
    let target = root.path().join("Background text.md");
    fs::write(&target, "Inside text.").unwrap();
    std::os::unix::fs::symlink(&target, background_path(root.path())).unwrap();
    assert_eq!(
        read_background(root.path()),
        BackgroundRead::Text("Inside text.".into())
    );
}

#[cfg(unix)]
#[test]
fn a_background_that_links_to_a_file_that_does_not_exist_is_not_available() {
    let root = folder();
    std::os::unix::fs::symlink(root.path().join("gone.md"), background_path(root.path())).unwrap();
    assert!(matches!(
        read_background(root.path()),
        BackgroundRead::NotAvailable(BackgroundUnavailable::Unreadable(_))
    ));
}

#[cfg(unix)]
#[test]
fn a_save_replaces_a_link_outside_the_folder_with_a_file() {
    let root = folder();
    let (_outside, target) = outside_file("secret.md", "Outside text.");
    std::os::unix::fs::symlink(&target, background_path(root.path())).unwrap();
    save_background(root.path(), "New text.").unwrap();
    let meta = fs::symlink_metadata(background_path(root.path())).unwrap();
    assert!(meta.file_type().is_file());
    assert_eq!(
        read_background(root.path()),
        BackgroundRead::Text("New text.".into())
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "Outside text.");
}

#[cfg(unix)]
#[test]
fn a_save_into_a_longhand_folder_that_links_outside_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let settings = serde_json::to_vec(&default_settings()).unwrap();
    fs::write(outside.path().join("settings.json"), settings).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join(".longhand")).unwrap();
    assert!(save_background(root.path(), "New text.").is_err());
    let names: Vec<_> = fs::read_dir(outside.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["settings.json"]);
}
