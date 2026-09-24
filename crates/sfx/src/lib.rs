//! The failed-capture cue, and the check that it can be heard (KD9, KTD14,
//! R5a).
//!
//! The cue is synthesized and played through cpal 0.18, the version capture
//! already uses, rather than through rodio as anarlog's `sfx` plugin does:
//! rodio 0.22 pins cpal 0.17, which would put a second CoreAudio and WASAPI
//! binding in the build to play one sound. The output probe is Handy's; see
//! PROVENANCE.md.

mod cue;
mod output;

pub use cue::{CueError, cue_samples, play_cue};
pub use output::{OutputProblem, OutputState, VOLUME_FLOOR, default_output_name, output_state};
