//! Longhand's session core: the lifecycle state machine, the monotonic
//! session timeline, the SQLite job store, and the session
//! folder's markdown. Pure Rust; nothing here depends on Tauri.

pub mod backup;
pub mod folder;
pub mod inspect;
pub mod jobs;
pub mod lock;
pub mod recognition;
pub mod schema;
pub mod state;
pub mod timeline;

pub use folder::SpeakerCount;
pub use jobs::{
    Adoption, CaptureChoice, DeleteError, Microphone, MoveJournal, MovePause, MoveStep,
    PromptSnapshot, RecordingModel, RenameIntent, SessionRecord, Store, StoreError, Title,
    TitleSource, TranscribeAgainMarks,
};
pub use state::{AUTO_QUEUE_AFTER, Event, IllegalTransition, InterruptReason, SessionState, Stage};
pub use timeline::{Closed, Location, MIN_SEGMENT_MS, Segment, Timeline, TimelineError};
