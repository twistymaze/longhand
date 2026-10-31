//! Resolve, check and queue a Transcribe Again run (KTD1, KTD2, KTD4). The
//! run uses the recognition settings, the speaker count of the note and
//! After Stop timing. The run transcribes all audio of the note again. It
//! uses no saved timed text (R24).
use crate::{StageError, save};
use models::recognition::{
    ExecutionSpec, ProcessingMode, RecognitionConfig, ResolveError, SpeakerSource,
};
use session_core::{Event, SessionState, Store};
use session_save::note_files;
use std::path::Path;

/// Whether the artifacts of a run are on disk. `artifact` is the catalog id
/// of the first artifact that is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    Missing { artifact: String },
    Downloading { artifact: String },
}

/// The state that refuses a Transcribe Again run and that the runner reads
/// outside the store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Guards {
    /// The summary of the note is made again, or the note is deleted.
    pub regenerating: bool,
    pub capture_active: bool,
}

/// Why a Transcribe Again run cannot start, as the text for the user.
/// `in_settings` is true when the user can fix the cause in Settings.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Refusal {
    pub message: String,
    pub in_settings: bool,
}

impl From<StageError> for Refusal {
    fn from(e: StageError) -> Self {
        Self {
            message: e.to_string(),
            in_settings: false,
        }
    }
}

impl From<session_core::StoreError> for Refusal {
    fn from(e: session_core::StoreError) -> Self {
        StageError::from(e).into()
    }
}

impl From<serde_json::Error> for Refusal {
    fn from(e: serde_json::Error) -> Self {
        StageError::from(e).into()
    }
}

impl From<std::io::Error> for Refusal {
    fn from(e: std::io::Error) -> Self {
        StageError::from(e).into()
    }
}

/// Each cause that refuses a Transcribe Again run. [`Refusal::from`] gives
/// the text for each cause, so the check and the confirmation show the
/// same text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    NotDownloaded {
        artifact: String,
    },
    Downloading {
        artifact: String,
    },
    Language {
        model: String,
        language: String,
    },
    SpeakerLimit {
        source: String,
        requested: u32,
        maximum: u32,
    },
    Configuration,
    SettingsChanged,
    Regenerating,
    Recording,
    /// At least one segment has no audio file and no removal marker: the
    /// sync can still deliver it.
    MissingAudio,
    /// At least one segment has a removal marker (R14, KTD9).
    RemovedAudio,
    AlreadyQueued,
    NotSaved,
    /// An older version of Longhand saved the note: its records are in
    /// `session.md` until the conversion (KTD6).
    OldLayout,
}

impl From<Cause> for Refusal {
    fn from(cause: Cause) -> Self {
        let (message, in_settings) = match cause {
            Cause::NotDownloaded { artifact } => (
                format!(
                    "{} is not downloaded. Download it in Settings.",
                    model_name(&artifact)
                ),
                true,
            ),
            Cause::Downloading { artifact } => (
                format!(
                    "{} is still downloading. Try again when it is ready.",
                    model_name(&artifact)
                ),
                false,
            ),
            Cause::Language { model, language } => (
                format!(
                    "{} does not support {}. Choose another model or language in Settings.",
                    model_name(&model),
                    language_name(&language)
                ),
                true,
            ),
            Cause::SpeakerLimit {
                source,
                requested,
                maximum,
            } => (
                format!(
                    "This note has {requested} speakers. {source} supports up to {maximum}. \
                     Choose another speaker source in Settings."
                ),
                true,
            ),
            Cause::Configuration => (
                "The transcription settings cannot be used for this note. Check them in Settings."
                    .into(),
                true,
            ),
            Cause::SettingsChanged => (
                "The transcription settings changed. Choose Transcribe Again to use the new \
                 settings."
                    .into(),
                false,
            ),
            Cause::Regenerating => (
                "The summary of this note is being made again. Try again when it is done.".into(),
                false,
            ),
            Cause::Recording => (
                "A recording is in progress. Try again after it stops.".into(),
                false,
            ),
            Cause::MissingAudio => (
                "The note's folder does not have all of its audio, so it can't be transcribed \
                 again."
                    .into(),
                false,
            ),
            Cause::RemovedAudio => (
                "The audio of this note was removed, so it can't be transcribed again.".into(),
                false,
            ),
            Cause::AlreadyQueued => (
                "This note is already being transcribed again.".into(),
                false,
            ),
            Cause::NotSaved => (
                "This note can be transcribed again only after it is saved.".into(),
                false,
            ),
            Cause::OldLayout => (save::OLD_LAYOUT.into(), false),
        };
        Self {
            message,
            in_settings,
        }
    }
}

/// The catalog name of the model `id`, or `id` when the catalog does not
/// have it.
pub fn model_name(id: &str) -> String {
    models::model(id).map_or_else(|| id.to_string(), |m| m.name.clone())
}

/// The speaker source name that Settings shows.
fn source_name(config: &RecognitionConfig) -> String {
    match config.speaker_source {
        SpeakerSource::Off => "Off".into(),
        SpeakerSource::Pyannote => "pyannote / WeSpeaker".into(),
        SpeakerSource::Sortformer => "Sortformer".into(),
        SpeakerSource::Nemotron => "Nemotron 3".into(),
    }
}

/// The English name of each language code in the catalog.
#[rustfmt::skip]
const LANGUAGES: &[(&str, &str)] = &[
    ("af", "Afrikaans"), ("am", "Amharic"), ("ar", "Arabic"), ("as", "Assamese"),
    ("az", "Azerbaijani"), ("ba", "Bashkir"), ("be", "Belarusian"), ("bg", "Bulgarian"),
    ("bn", "Bangla"), ("bo", "Tibetan"), ("br", "Breton"), ("bs", "Bosnian"),
    ("ca", "Catalan"), ("cs", "Czech"), ("cy", "Welsh"), ("da", "Danish"), ("de", "German"),
    ("el", "Greek"), ("en", "English"), ("es", "Spanish"), ("et", "Estonian"),
    ("eu", "Basque"), ("fa", "Persian"), ("fi", "Finnish"), ("fo", "Faroese"),
    ("fr", "French"), ("gl", "Galician"), ("gu", "Gujarati"), ("ha", "Hausa"),
    ("haw", "Hawaiian"), ("he", "Hebrew"), ("hi", "Hindi"), ("hr", "Croatian"),
    ("ht", "Haitian Creole"), ("hu", "Hungarian"), ("hy", "Armenian"), ("id", "Indonesian"),
    ("is", "Icelandic"), ("it", "Italian"), ("ja", "Japanese"), ("jw", "Javanese"),
    ("ka", "Georgian"), ("kk", "Kazakh"), ("km", "Khmer"), ("kn", "Kannada"),
    ("ko", "Korean"), ("la", "Latin"), ("lb", "Luxembourgish"), ("ln", "Lingala"),
    ("lo", "Lao"), ("lt", "Lithuanian"), ("lv", "Latvian"), ("mg", "Malagasy"),
    ("mi", "Māori"), ("mk", "Macedonian"), ("ml", "Malayalam"), ("mn", "Mongolian"),
    ("mr", "Marathi"), ("ms", "Malay"), ("mt", "Maltese"), ("my", "Burmese"),
    ("nb", "Norwegian Bokmål"), ("ne", "Nepali"), ("nl", "Dutch"),
    ("nn", "Norwegian Nynorsk"), ("no", "Norwegian"), ("oc", "Occitan"), ("pa", "Punjabi"),
    ("pl", "Polish"), ("ps", "Pashto"), ("pt", "Portuguese"), ("ro", "Romanian"),
    ("ru", "Russian"), ("sa", "Sanskrit"), ("sd", "Sindhi"), ("si", "Sinhala"),
    ("sk", "Slovak"), ("sl", "Slovenian"), ("sn", "Shona"), ("so", "Somali"),
    ("sq", "Albanian"), ("sr", "Serbian"), ("su", "Sundanese"), ("sv", "Swedish"),
    ("sw", "Swahili"), ("ta", "Tamil"), ("te", "Telugu"), ("tg", "Tajik"), ("th", "Thai"),
    ("tk", "Turkmen"), ("tl", "Tagalog"), ("tr", "Turkish"), ("tt", "Tatar"),
    ("uk", "Ukrainian"), ("ur", "Urdu"), ("uz", "Uzbek"), ("vi", "Vietnamese"),
    ("yi", "Yiddish"), ("yo", "Yoruba"), ("yue", "Cantonese"), ("zh", "Chinese"),
];

/// The English name of the language `code`, or `code` when the table does
/// not have it.
fn language_name(code: &str) -> String {
    LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .map_or_else(|| code.to_string(), |(_, name)| (*name).to_string())
}

/// Check if a Transcribe Again run can start on `note_id` now, and give
/// its execution. This is read-only: the row does not follow its folder.
pub fn check(
    store: &Store,
    note_id: &str,
    guards: Guards,
    settings: &RecognitionConfig,
    ready: &dyn Fn(&ExecutionSpec) -> Readiness,
) -> Result<ExecutionSpec, Refusal> {
    resolve_run(store, note_id, guards, settings, None, ready).map(|(spec, _)| spec)
}

/// Queue a saved note for a Transcribe Again run (R15, KTD14). The row
/// follows its folder first (KTD3): the run keeps the notes, the title and
/// the speaker count that another computer wrote. Then every check of
/// [`check`] runs again. `model_id` is the model that the user confirmed:
/// when the settings now select another model, the run is refused. The
/// caller holds the store lock.
pub fn queue(
    store: &mut Store,
    note_id: &str,
    guards: Guards,
    settings: &RecognitionConfig,
    model_id: &str,
    ready: &dyn Fn(&ExecutionSpec) -> Readiness,
) -> Result<ExecutionSpec, Refusal> {
    if !guards.regenerating {
        crate::follow::follow_folder(store, note_id, false)?;
    }
    let (spec, home) = resolve_run(store, note_id, guards, settings, Some(model_id), ready)?;
    // The save compares the records of the note with this value (KTD4). A
    // change to the records that another computer makes during the run then
    // keeps the previous result. An edit of a `.md` file, a title proposal
    // or a type move does not count (R3).
    let baseline = match home {
        Some(home) => {
            let record = note_files::new_layout_record(Path::new(&home))?;
            Some(serde_json::to_value((home, record.run_fingerprint()))?)
        }
        None => None,
    };
    store.begin_reprocess(
        note_id,
        &spec.config.asr_model,
        &Event::UserTranscribeAgain {
            at: chrono::Local::now().fixed_offset(),
            audio_intact: true,
            capture_active: guards.capture_active,
            regenerating: guards.regenerating,
        },
        Some(&serde_json::to_value(&spec)?),
        baseline.as_ref(),
    )?;
    Ok(spec)
}

/// The refusals in order, then the execution of the run: `settings` with
/// After Stop timing and the speaker count of the note. Also gives the home
/// folder of the note.
fn resolve_run(
    store: &Store,
    note_id: &str,
    guards: Guards,
    settings: &RecognitionConfig,
    model_id: Option<&str>,
    ready: &dyn Fn(&ExecutionSpec) -> Readiness,
) -> Result<(ExecutionSpec, Option<String>), Refusal> {
    let record = store
        .session(note_id)?
        .ok_or_else(|| StageError::Failed(format!("no session {note_id}")))?;
    if guards.regenerating {
        return Err(Cause::Regenerating.into());
    }
    if guards.capture_active {
        return Err(Cause::Recording.into());
    }
    if record.state != SessionState::Saved {
        return Err(if store.transcribe_again_marks(note_id)?.model.is_some() {
            Cause::AlreadyQueued
        } else {
            Cause::NotSaved
        }
        .into());
    }
    // A note in the old layout waits for the conversion (KTD6).
    if record
        .home_path
        .as_deref()
        .is_some_and(|home| save::old_layout(Path::new(home)))
    {
        return Err(Cause::OldLayout.into());
    }
    // Removed audio does not come back, so its reason differs from the
    // reason for audio that the sync can still deliver (R14).
    match save::audio_state(store, note_id)? {
        save::AudioState::Present => {}
        save::AudioState::Removed => return Err(Cause::RemovedAudio.into()),
        save::AudioState::Waiting => return Err(Cause::MissingAudio.into()),
    }
    let config = RecognitionConfig {
        processing_mode: ProcessingMode::AfterStop,
        speaker_processing_mode: ProcessingMode::AfterStop,
        speaker_count: record.speaker_count.map(|count| match count {
            session_core::SpeakerCount::Exact(count) => {
                models::recognition::SpeakerCount::Exact { count }
            }
            session_core::SpeakerCount::Range { min, max } => {
                models::recognition::SpeakerCount::Range { min, max }
            }
        }),
        ..settings.clone()
    };
    if model_id.is_some_and(|id| id != config.asr_model) {
        return Err(Cause::SettingsChanged.into());
    }
    let spec = models::recognition::resolve(&config).map_err(|e| {
        Refusal::from(match e {
            ResolveError::Language { model, language } => Cause::Language { model, language },
            ResolveError::SpeakerLimit { maximum, requested } => Cause::SpeakerLimit {
                source: source_name(&config),
                requested,
                maximum,
            },
            _ => Cause::Configuration,
        })
    })?;
    crate::recognition::TaskGraph::resolve(&spec).map_err(|_| Cause::Configuration)?;
    match ready(&spec) {
        Readiness::Ready => Ok((spec, record.home_path)),
        Readiness::Missing { artifact } => Err(Cause::NotDownloaded { artifact }.into()),
        Readiness::Downloading { artifact } => Err(Cause::Downloading { artifact }.into()),
    }
}

pub(crate) fn speaker_count(spec: &ExecutionSpec) -> Option<session_core::SpeakerCount> {
    spec.config.speaker_count.map(|count| match count {
        models::recognition::SpeakerCount::Exact { count } => {
            session_core::SpeakerCount::Exact(count)
        }
        models::recognition::SpeakerCount::Range { min, max } => {
            session_core::SpeakerCount::Range { min, max }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_cause_gives_its_text_and_says_if_the_fix_is_in_settings() {
        let text = |cause: Cause| {
            let refusal = Refusal::from(cause);
            (refusal.message, refusal.in_settings)
        };
        assert_eq!(
            text(Cause::NotSaved),
            (
                "This note can be transcribed again only after it is saved.".into(),
                false
            )
        );
        assert_eq!(
            text(Cause::NotDownloaded {
                artifact: "whisper-large-v3-turbo-q5km".into()
            }),
            (
                "Whisper Large v3 Turbo is not downloaded. Download it in Settings.".into(),
                true
            )
        );
        assert_eq!(
            text(Cause::Downloading {
                artifact: "whisper-large-v3-turbo-q5km".into()
            }),
            (
                "Whisper Large v3 Turbo is still downloading. Try again when it is ready.".into(),
                false
            )
        );
        assert_eq!(
            text(Cause::Language {
                model: "parakeet-unified-en-0.6b-q80".into(),
                language: "fr".into()
            }),
            (
                "Parakeet Unified EN 0.6B does not support French. Choose another model or \
                 language in Settings."
                    .into(),
                true
            )
        );
        assert_eq!(
            text(Cause::SettingsChanged),
            (
                "The transcription settings changed. Choose Transcribe Again to use the new \
                 settings."
                    .into(),
                false
            )
        );
        assert_eq!(
            text(Cause::Configuration),
            (
                "The transcription settings cannot be used for this note. Check them in \
                 Settings."
                    .into(),
                true
            )
        );
        assert_eq!(
            text(Cause::AlreadyQueued),
            (
                "This note is already being transcribed again.".into(),
                false
            )
        );
    }

    #[test]
    fn the_speaker_limit_names_the_speaker_source() {
        let config = RecognitionConfig {
            speaker_source: SpeakerSource::Sortformer,
            ..Default::default()
        };
        assert_eq!(
            Refusal::from(Cause::SpeakerLimit {
                source: source_name(&config),
                requested: 5,
                maximum: 4
            }),
            Refusal {
                message: "This note has 5 speakers. Sortformer supports up to 4. Choose \
                          another speaker source in Settings."
                    .into(),
                in_settings: true
            }
        );
    }

    #[test]
    fn every_catalog_language_has_a_name() {
        for model in models::CATALOG.iter() {
            for code in &model.languages {
                assert_ne!(language_name(code), *code, "{code}");
            }
        }
    }
}
