//! The failed-capture cue (KD9, KTD14).
//!
//! The cue is synthesized and played through cpal 0.18, the version capture
//! already uses, rather than through rodio as anarlog's `sfx` plugin does:
//! rodio 0.22 pins cpal 0.17, which would put a second CoreAudio and WASAPI
//! binding in the build to play one sound.

mod cue;

pub use cue::{CueError, cue_samples, play_cue};
