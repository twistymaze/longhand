//! Keeping a recording machine awake (KD3, KTD12, R4), and noticing when it
//! sleeps anyway (U13). `sleep/macos.rs` is anarlog's observer; see
//! PROVENANCE.md.

mod assertion;
mod processing;
mod sleep;

pub use assertion::{IdleGuard, REASON};
pub use sleep::{PowerCallback, PowerEvent, SleepWatch};

pub use processing::{ProcessingLease, ProcessingPower};
