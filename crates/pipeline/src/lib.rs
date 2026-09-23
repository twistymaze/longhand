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
pub mod join;
pub mod resample;
pub mod runner;
pub mod transcribe;

use std::sync::{Arc, Mutex, MutexGuard};

use session_core::Store;

pub use runner::{EngineProvider, Environment, Runner, RunnerEvent};

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
}

/// A control that never suspends or cancels, for tools and tests.
pub struct Unattended;

impl Control for Unattended {
    fn proceed(&self) -> Result<(), StageError> {
        Ok(())
    }
}

/// The processing audio's path for a segment, next to its archive name.
pub fn proc_file(audio_file: &str) -> String {
    let stem = std::path::Path::new(audio_file)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| audio_file.to_string());
    format!("{PROC_DIR}/{stem}.wav")
}
