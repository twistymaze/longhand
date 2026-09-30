//! Post-stop processing (KD1, R14, R20, R20c): a durable FIFO of sessions,
//! executed one stage of one session at a time, each stage writing a
//! checkpoint at every unit of progress so a quit, a crash or a failure
//! resumes where it stopped.
//!
//! Nothing here depends on Tauri; `src-tauri/src/managers/processing.rs`
//! wires the runner into the app.

pub mod assemble;
pub mod checkpoint;
pub mod diarize;
pub mod echo;
pub mod folder_rename;
pub mod follow;
pub mod in_use;
pub mod join;
pub mod live;
pub mod move_notes;
pub mod old_audio;
mod opus_ms;
pub mod progress;
pub mod recognition;
pub mod recover;
pub mod remove_audio;
pub mod remove_copies;
pub mod reprocess;
pub mod resample;
pub mod runner;
pub mod save;
pub mod summarize;
pub mod transcribe;
pub mod verify;

use std::sync::{Arc, Mutex, MutexGuard};

use session_core::Store;

pub use runner::{EndpointProblem, EngineProvider, Environment, Outcome, Runner, RunnerEvent};
pub use save::Saved;
pub use summarize::SummaryEndpoint;

/// The one job store connection, shared with capture so every lifecycle
/// transition is serialized through it.
pub type SharedStore = Arc<Mutex<Store>>;

pub(crate) fn lock(store: &SharedStore) -> MutexGuard<'_, Store> {
    store.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where a session's folders sit inside its working directory.
pub const RECOVERY_DIR: &str = "audio-recovery";
pub const PROC_DIR: &str = "proc";

#[derive(Debug, thiserror::Error)]
pub enum StageError {
    /// The session was taken back (resumed) or the app is shutting down.
    /// Completed checkpoints stand.
    #[error("cancelled")]
    Cancelled,
    /// Something the stage needs is not on disk yet (model weights). The
    /// session goes back to its place in the queue rather than failing.
    #[error("waiting for {0}")]
    Waiting(String),
    /// The secret store holding a credential is locked (R20b): the session
    /// waits for the user to unlock it, which is not a failure.
    #[error("blocked: {0}")]
    Blocked(String),
    /// The destination cannot take the session (R22b): it waits, whole in
    /// the working location, until the user fixes or changes it.
    #[error("{0}")]
    NoDestination(String),
    #[error("{0}")]
    Failed(String),
}

impl From<std::io::Error> for StageError {
    fn from(e: std::io::Error) -> Self {
        StageError::Failed(e.to_string())
    }
}

impl From<session_core::StoreError> for StageError {
    fn from(e: session_core::StoreError) -> Self {
        StageError::Failed(e.to_string())
    }
}

impl From<library::refresh::RefreshError> for StageError {
    fn from(e: library::refresh::RefreshError) -> Self {
        StageError::Failed(e.to_string())
    }
}

impl From<serde_json::Error> for StageError {
    fn from(e: serde_json::Error) -> Self {
        StageError::Failed(e.to_string())
    }
}

impl From<hound::Error> for StageError {
    fn from(e: hound::Error) -> Self {
        StageError::Failed(e.to_string())
    }
}

/// Consulted by a stage at every checkpoint boundary.
pub trait Control: Send + Sync {
    /// Return once the stage may continue, or `Cancelled`. Blocks while
    /// inference is suspended (KTD20: capture is active).
    fn proceed(&self) -> Result<(), StageError>;

    /// Whether work should stop now, without waiting out a suspension: for
    /// a stage polling while a network call is in flight.
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// A control that never suspends or cancels, for tools and tests.
pub struct Unattended;

impl Control for Unattended {
    fn proceed(&self) -> Result<(), StageError> {
        Ok(())
    }
}

/// Flush a written file to disk. The handle is opened for writing because
/// Windows refuses `FlushFileBuffers` on a read-only one ("Access is denied").
pub(crate) fn sync_file(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)?
        .sync_all()
}

/// The processing audio's path for a segment, next to its archive name.
pub fn proc_file(audio_file: &str) -> String {
    let stem = std::path::Path::new(audio_file)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| audio_file.to_string());
    format!("{PROC_DIR}/{stem}.wav")
}
