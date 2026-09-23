//! Longhand's session core: the lifecycle state machine, the monotonic
//! session timeline, note anchors, the SQLite job store, and the session
//! folder's markdown. Pure Rust; nothing here depends on Tauri.

pub mod anchors;
pub mod folder;
pub mod jobs;
pub mod schema;
pub mod state;
pub mod timeline;

pub use anchors::{Anchor, DEFAULT_LOOKBACK_MS, resolve};
pub use folder::SpeakerCount;
pub use jobs::{Note, SessionRecord, Store, StoreError};
pub use state::{Event, IllegalTransition, InterruptReason, RESUME_WINDOW, SessionState, Stage};
pub use timeline::{Closed, Location, MIN_SEGMENT_MS, Segment, Timeline, TimelineError};
