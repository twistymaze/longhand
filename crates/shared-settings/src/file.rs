//! The shared settings file `.longhand/settings.json` (KTD2, KTD3).
//!
//! The file is one JSON object. Each known field parses on its own, so a
//! value that this build does not know stays in the file and shows as
//! unknown, not as a default (R17). A save reads the file again, sets only
//! the fields of that save, and writes the result atomically (R15). A read
//! never writes. A file that exists but does not parse is not available:
//! a sync service can deliver a file in parts, and Longhand never writes
//! over such a file.

use std::io;
use std::path::{Path, PathBuf};

use models::recognition::{ProcessingMode, SpeakerCount, SpeakerSource};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// The hidden folder in the storage folder that holds Longhand's own
/// files. It is the same folder as `library::scan::LONGHAND_FOLDER`.
pub const LONGHAND_FOLDER: &str = ".longhand";

/// The name of the shared settings file. Longhand reads only this exact
/// name, so a sync conflict copy such as `settings 2.json` has no effect.
pub const SETTINGS_FILE: &str = "settings.json";

/// The schema version that this build writes. A save never writes a lower
/// version than the version in the file (R17).
pub const SCHEMA_VERSION: u64 = 1;

pub const KEY_SCHEMA_VERSION: &str = "schema_version";
pub const KEY_ACTIVE_SUMMARY_PROMPT: &str = "active_summary_prompt";
pub const KEY_DEFAULT_SPEAKER_COUNT: &str = "default_speaker_count";
pub const KEY_ASR_MODEL: &str = "asr_model";
pub const KEY_LIVE_TRANSCRIPT: &str = "live_transcript";
pub const KEY_SPEAKER_SOURCE: &str = "speaker_source";
pub const KEY_SPEAKER_PROCESSING_MODE: &str = "speaker_processing_mode";
pub const KEY_RECOGNITION_LANGUAGE: &str = "recognition_language";
pub const KEY_CHECK_UPDATES_AUTOMATICALLY: &str = "check_updates_automatically";

/// The hidden Longhand folder of the storage folder `root`.
pub fn longhand_folder(root: &Path) -> PathBuf {
    root.join(LONGHAND_FOLDER)
}

/// The path of the shared settings file of `root`.
pub fn settings_path(root: &Path) -> PathBuf {
    longhand_folder(root).join(SETTINGS_FILE)
}

/// One shared setting as the file holds it.
#[derive(Debug, Clone, PartialEq)]
pub enum Field<T> {
    /// The file has no value for this setting.
    Absent,
    Known(T),
    /// A value that this build does not know, such as a newer speaker
    /// source or a model that is not in the catalog. The app shows it and
    /// keeps it; it never replaces it with a default (R17).
    Unknown(Value),
}

impl<T> Field<T> {
    /// The value, when this build knows it.
    pub fn known(&self) -> Option<&T> {
        match self {
            Field::Known(value) => Some(value),
            Field::Absent | Field::Unknown(_) => None,
        }
    }
}

/// The typed view of the shared settings file (R12, R18). The custom
/// prompts are files of their own (see [`crate::prompts`]).
#[derive(Debug, Clone, PartialEq)]
pub struct SharedFields {
    /// The version in the file. `None` when the file has no version.
    pub schema_version: Option<u64>,
    /// The id of the prompt that the next summary uses.
    pub active_summary_prompt: Field<String>,
    /// `Known(None)` is "no count".
    pub default_speaker_count: Field<Option<SpeakerCount>>,
    /// The chosen transcription model: a catalog id of a transcription
    /// model.
    pub asr_model: Field<String>,
    pub live_transcript: Field<bool>,
    pub speaker_source: Field<SpeakerSource>,
    pub speaker_processing_mode: Field<ProcessingMode>,
    /// `Known(None)` is "detect the language".
    pub recognition_language: Field<Option<String>>,
    pub check_updates_automatically: Field<bool>,
}

impl SharedFields {
    /// Parse each known field of `raw` on its own.
    pub fn from_raw(raw: &Map<String, Value>) -> Self {
        Self {
            schema_version: raw.get(KEY_SCHEMA_VERSION).and_then(Value::as_u64),
            active_summary_prompt: parse(raw, KEY_ACTIVE_SUMMARY_PROMPT, |_: &String| true),
            default_speaker_count: parse(
                raw,
                KEY_DEFAULT_SPEAKER_COUNT,
                |count: &Option<SpeakerCount>| count.as_ref().is_none_or(valid_speaker_count),
            ),
            asr_model: parse(raw, KEY_ASR_MODEL, |id: &String| is_catalog_asr_model(id)),
            live_transcript: parse(raw, KEY_LIVE_TRANSCRIPT, |_| true),
            speaker_source: parse(raw, KEY_SPEAKER_SOURCE, |_| true),
            speaker_processing_mode: parse(raw, KEY_SPEAKER_PROCESSING_MODE, |_| true),
            recognition_language: parse(raw, KEY_RECOGNITION_LANGUAGE, |_| true),
            check_updates_automatically: parse(raw, KEY_CHECK_UPDATES_AUTOMATICALLY, |_| true),
        }
    }

    /// The file comes from a newer build, so it can hold settings that
    /// this build does not show.
    pub fn newer_schema(&self) -> bool {
        self.schema_version.is_some_and(|v| v > SCHEMA_VERSION)
    }

    /// Whether any field has a value that this build does not know.
    pub fn has_unknown(&self) -> bool {
        fn unknown<T>(field: &Field<T>) -> bool {
            matches!(field, Field::Unknown(_))
        }
        unknown(&self.active_summary_prompt)
            || unknown(&self.default_speaker_count)
            || unknown(&self.asr_model)
            || unknown(&self.live_transcript)
            || unknown(&self.speaker_source)
            || unknown(&self.speaker_processing_mode)
            || unknown(&self.recognition_language)
            || unknown(&self.check_updates_automatically)
    }
}

fn parse<T: DeserializeOwned>(
    raw: &Map<String, Value>,
    key: &str,
    valid: impl Fn(&T) -> bool,
) -> Field<T> {
    let Some(value) = raw.get(key) else {
        return Field::Absent;
    };
    match serde_json::from_value::<T>(value.clone()) {
        Ok(parsed) if valid(&parsed) => Field::Known(parsed),
        _ => Field::Unknown(value.clone()),
    }
}

fn valid_speaker_count(count: &SpeakerCount) -> bool {
    match *count {
        SpeakerCount::Exact { count } => count >= 1,
        SpeakerCount::Range { min, max } => min >= 1 && min <= max,
    }
}

/// Whether `id` is a transcription model in this build's catalog.
pub fn is_catalog_asr_model(id: &str) -> bool {
    models::CATALOG
        .iter()
        .any(|m| m.id == id && m.kind == models::ModelKind::Asr)
}

/// One setting that a save sets.
#[derive(Debug, Clone, PartialEq)]
pub enum SharedValue {
    ActiveSummaryPrompt(String),
    DefaultSpeakerCount(Option<SpeakerCount>),
    AsrModel(String),
    LiveTranscript(bool),
    SpeakerSource(SpeakerSource),
    SpeakerProcessingMode(ProcessingMode),
    RecognitionLanguage(Option<String>),
    CheckUpdatesAutomatically(bool),
}

impl SharedValue {
    /// The key of this setting in the file.
    pub fn key(&self) -> &'static str {
        match self {
            SharedValue::ActiveSummaryPrompt(_) => KEY_ACTIVE_SUMMARY_PROMPT,
            SharedValue::DefaultSpeakerCount(_) => KEY_DEFAULT_SPEAKER_COUNT,
            SharedValue::AsrModel(_) => KEY_ASR_MODEL,
            SharedValue::LiveTranscript(_) => KEY_LIVE_TRANSCRIPT,
            SharedValue::SpeakerSource(_) => KEY_SPEAKER_SOURCE,
            SharedValue::SpeakerProcessingMode(_) => KEY_SPEAKER_PROCESSING_MODE,
            SharedValue::RecognitionLanguage(_) => KEY_RECOGNITION_LANGUAGE,
            SharedValue::CheckUpdatesAutomatically(_) => KEY_CHECK_UPDATES_AUTOMATICALLY,
        }
    }

    /// The value of this setting as the file holds it.
    pub fn value(&self) -> Value {
        let value = match self {
            SharedValue::ActiveSummaryPrompt(id) => serde_json::to_value(id),
            SharedValue::DefaultSpeakerCount(count) => serde_json::to_value(count),
            SharedValue::AsrModel(id) => serde_json::to_value(id),
            SharedValue::LiveTranscript(on) => serde_json::to_value(on),
            SharedValue::SpeakerSource(source) => serde_json::to_value(source),
            SharedValue::SpeakerProcessingMode(mode) => serde_json::to_value(mode),
            SharedValue::RecognitionLanguage(language) => serde_json::to_value(language),
            SharedValue::CheckUpdatesAutomatically(on) => serde_json::to_value(on),
        };
        value.expect("a shared setting serializes")
    }
}

/// A shared settings file that parsed: the whole object as it is on disk,
/// and the typed view of it.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsFile {
    pub raw: Map<String, Value>,
    pub fields: SharedFields,
}

impl SettingsFile {
    pub fn from_raw(raw: Map<String, Value>) -> Self {
        let fields = SharedFields::from_raw(&raw);
        Self { raw, fields }
    }
}

/// Why the shared settings are not available (R16, KTD3).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unavailable {
    /// The storage folder does not exist, for example because its disk is
    /// not connected.
    #[error("the storage folder does not exist")]
    FolderMissing,
    #[error("the shared settings cannot be read: {0}")]
    Unreadable(String),
    /// The file exists but is not one complete JSON object, for example
    /// because a sync delivered only a part of it.
    #[error("the shared settings file does not parse: {0}")]
    DoesNotParse(String),
}

/// The result of a read of the shared settings file.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant, reason = "one value for each read")]
pub enum FileRead {
    Parsed(SettingsFile),
    /// The storage folder exists and has no settings file.
    Missing,
    NotAvailable(Unavailable),
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// The shared settings are read-only. The save wrote nothing.
    #[error("{0}")]
    NotAvailable(Unavailable),
    /// The folder has no settings file. The join fills it first (KTD8).
    #[error("the storage folder has no shared settings file")]
    Missing,
    #[error("the shared settings file cannot be written: {0}")]
    Io(#[from] io::Error),
}

/// Read the shared settings file of the storage folder `root`. This never
/// writes.
pub fn read_settings(root: &Path) -> FileRead {
    match std::fs::metadata(root) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return FileRead::NotAvailable(Unavailable::FolderMissing),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return FileRead::NotAvailable(Unavailable::FolderMissing);
        }
        Err(e) => return FileRead::NotAvailable(Unavailable::Unreadable(e.to_string())),
    }
    let bytes = match std::fs::read(settings_path(root)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return FileRead::Missing,
        Err(e) => return FileRead::NotAvailable(Unavailable::Unreadable(e.to_string())),
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(raw)) => FileRead::Parsed(SettingsFile::from_raw(raw)),
        Ok(_) => FileRead::NotAvailable(Unavailable::DoesNotParse(
            "the file is not a JSON object".into(),
        )),
        Err(e) => FileRead::NotAvailable(Unavailable::DoesNotParse(e.to_string())),
    }
}

/// Apply `patch` to `raw`: set each value of the patch, keep every other
/// key, and set the schema version to the larger of the file's version
/// and [`SCHEMA_VERSION`] (R17).
pub fn apply_patch(raw: &Map<String, Value>, patch: &[SharedValue]) -> Map<String, Value> {
    let mut out = raw.clone();
    for value in patch {
        out.insert(value.key().to_string(), value.value());
    }
    let version = raw
        .get(KEY_SCHEMA_VERSION)
        .and_then(Value::as_u64)
        .map_or(SCHEMA_VERSION, |v| v.max(SCHEMA_VERSION));
    out.insert(KEY_SCHEMA_VERSION.to_string(), Value::from(version));
    out
}

/// Save the settings of `patch` into the shared settings file of `root`
/// (R15). The save reads the file from disk again, so a value that another
/// computer wrote since the last read stays. Returns the file as written.
pub fn save_settings(root: &Path, patch: &[SharedValue]) -> Result<SettingsFile, SaveError> {
    let current = match read_settings(root) {
        FileRead::Parsed(file) => file,
        FileRead::Missing => return Err(SaveError::Missing),
        FileRead::NotAvailable(cause) => return Err(SaveError::NotAvailable(cause)),
    };
    let raw = apply_patch(&current.raw, patch);
    write_settings(root, &raw)?;
    Ok(SettingsFile::from_raw(raw))
}

/// Write `raw` as the shared settings file of `root`, and make the hidden
/// Longhand folder when it does not exist.
pub(crate) fn write_settings(root: &Path, raw: &Map<String, Value>) -> io::Result<()> {
    let folder = longhand_folder(root);
    std::fs::create_dir_all(&folder)?;
    let mut bytes = serde_json::to_vec_pretty(raw).expect("a JSON object serializes");
    bytes.push(b'\n');
    session_save::home_write::write_atomic(&folder, SETTINGS_FILE, &bytes)
}

/// Write `raw` as the shared settings file of `root` only when the folder
/// has no settings file. A settings file that a sync delivers in the
/// meantime stays (KTD8). Returns false when the folder has a settings
/// file.
pub(crate) fn write_new_settings(root: &Path, raw: &Map<String, Value>) -> io::Result<bool> {
    let folder = longhand_folder(root);
    std::fs::create_dir_all(&folder)?;
    let mut bytes = serde_json::to_vec_pretty(raw).expect("a JSON object serializes");
    bytes.push(b'\n');
    write_new(&folder, SETTINGS_FILE, &bytes)
}

/// Write `contents` as `dir/name` only when nothing is at that name. The
/// file is complete before it gets its name, as with `write_atomic`. The
/// temporary name starts with `.`, so no scan reads it. Returns false, and
/// writes nothing, when the name is taken.
pub(crate) fn write_new(dir: &Path, name: &str, contents: &[u8]) -> io::Result<bool> {
    let staged = format!(".{name}.{}.new", uuid::Uuid::new_v4().simple());
    session_save::home_write::write_atomic(dir, &staged, contents)?;
    let placed = session_save::rename::rename_noreplace(&dir.join(&staged), &dir.join(name));
    if let Err(e) = placed {
        let _ = std::fs::remove_file(dir.join(&staged));
        return if session_save::rename::is_name_taken(&e) {
            Ok(false)
        } else {
            Err(e)
        };
    }
    session_save::fsync::sync_dir(dir)?;
    Ok(true)
}

/// The shared settings of a new install. They match the defaults of the
/// app's settings.
pub fn default_settings() -> Map<String, Value> {
    let defaults = [
        SharedValue::ActiveSummaryPrompt(summarize::prompts::TOPIC_SUMMARY_ID.to_string()),
        SharedValue::DefaultSpeakerCount(None),
        SharedValue::AsrModel(models::DEFAULT_ASR_MODEL.to_string()),
        SharedValue::LiveTranscript(true),
        SharedValue::SpeakerSource(SpeakerSource::default()),
        SharedValue::SpeakerProcessingMode(ProcessingMode::default()),
        SharedValue::RecognitionLanguage(None),
        SharedValue::CheckUpdatesAutomatically(true),
    ];
    apply_patch(&Map::new(), &defaults)
}
