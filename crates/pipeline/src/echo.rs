//! The echo spans row of a stereo segment.
//!
//! Assemble does not remove echo: the processing file of a stereo segment
//! is the plain mix of its two channels (KTD9 of the app audio plan). The
//! row stays as the record that Assemble finished the segment, with no
//! spans and gains of 1 (KTD10). The Assemble checkpoint reads it through
//! [`saved`], and the Transcribe merge keeps the live words outside its
//! spans.

use asr::Span;
use serde::{Deserialize, Serialize};
use session_core::Stage;

use crate::{SharedStore, StageError, checkpoint};

/// The echo spans checkpoint of one stereo segment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EchoSpans {
    pub segment: u32,
    /// The spans of the segment where the microphone holds echo of the
    /// system audio, in ms from the segment start. Assemble saves none.
    pub spans: Vec<Span>,
    /// The gains that the mix gives the microphone and the system channel.
    /// Assemble saves 1 for each.
    pub mic_gain: f32,
    pub system_gain: f32,
}

impl EchoSpans {
    /// The row that Assemble saves for a stereo segment: no spans and gains
    /// of 1.
    pub fn plain_mix(segment: u32) -> Self {
        Self {
            segment,
            spans: Vec::new(),
            mic_gain: 1.0,
            system_gain: 1.0,
        }
    }
}

/// The saved echo spans of a segment. `None` for a mono segment, and for a
/// stereo segment that Assemble has not finished.
pub fn saved(
    store: &SharedStore,
    note_id: &str,
    segment: u32,
) -> Result<Option<EchoSpans>, StageError> {
    checkpoint::get(
        store,
        note_id,
        Stage::Assemble,
        &checkpoint::echo_key(segment),
    )
}
