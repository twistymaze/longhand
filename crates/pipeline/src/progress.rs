//! Where a session's processing stands, read from the store alone (U12,
//! AE4): a window shown mid-stage rebuilds its view from this, then follows
//! the runner's events.

use serde::{Deserialize, Serialize};
use session_core::Stage;

use crate::runner::STAGES;
use crate::{SharedStore, StageError, checkpoint, lock};

/// Under a stage: the message it last failed or was blocked with.
pub const ERROR: &str = "error";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Stages whose work is complete, in order.
    pub done: Vec<Stage>,
    /// The stage started last, if processing has begun.
    pub current: Option<Stage>,
    /// Transcription windows done and planned, once any are planned.
    pub windows: Option<(usize, usize)>,
    /// The speaker stage's last progress while that stage runs. `None`
    /// when another stage runs or the speaker stage is complete.
    pub speakers: Option<SpeakerProgress>,
}

pub fn snapshot(store: &SharedStore, note_id: &str) -> Result<Snapshot, StageError> {
    let segments = lock(store).segments(note_id)?.len();
    let mut done = Vec::new();
    for &stage in STAGES {
        if checkpoint::is_complete(store, note_id, stage, segments)? {
            done.push(stage);
        }
    }
    let current = lock(store).current_stage(note_id)?;
    let rows = checkpoint::all(store, note_id, Stage::Transcribe)?;
    let mut planned = 0;
    let mut finished = 0;
    for (key, payload) in &rows {
        if key.ends_with("-plan") {
            let plan: crate::transcribe::WindowPlan = serde_json::from_str(payload)?;
            planned += plan.windows.len();
        } else if key.contains("-win-") {
            finished += 1;
        }
    }
    // A failed, cancelled, or queued stage has no active meter.
    let speakers = if current == Some(Stage::Diarize)
        && !done.contains(&Stage::Diarize)
        && matches!(
            lock(store).session(note_id)?.map(|record| record.state),
            Some(session_core::SessionState::Processing { .. })
        ) {
        speakers(store, note_id)?
    } else {
        None
    };
    Ok(Snapshot {
        done,
        current,
        windows: (planned > 0).then_some((finished.min(planned), planned)),
        speakers,
    })
}

/// Remember why `stage` stopped, for a view opened later.
pub fn record_error(
    store: &SharedStore,
    note_id: &str,
    stage: Stage,
    message: &str,
) -> Result<(), StageError> {
    checkpoint::put(store, note_id, stage, ERROR, &message)
}

pub fn error(
    store: &SharedStore,
    note_id: &str,
    stage: Stage,
) -> Result<Option<String>, StageError> {
    checkpoint::get(store, note_id, stage, ERROR)
}

/// Under `Stage::Diarize`: the speaker stage's last progress, for a view
/// opened while the stage runs (KTD4).
pub const SPEAKER_KEY: &str = "speaker-progress-v1";

/// What the speaker stage can say about its work now (KTD1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerWork {
    /// The measured share of the whole note, 0 to 99. `None` before the
    /// first measurement. Only the stage checkpoint means complete (R5).
    pub percent: Option<u8>,
    /// False while work runs that gives no fraction, or while a replay
    /// is below the highest value reported (R4).
    pub measuring: bool,
}

/// [`SpeakerWork`] of one processing attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerProgress {
    pub attempt: String,
    #[serde(flatten)]
    pub work: SpeakerWork,
}

/// The highest measured percent before the stage checkpoint.
const MEASURED_LIMIT: u8 = 99;

/// Adds the measured work of the speaker stage over the whole note (KTD2).
/// The unit is audio samples, weighted by the route. The value that it
/// reports does not decrease, and it stays below 100 (KTD3).
pub struct SpeakerMeter<'a> {
    sink: &'a mut dyn FnMut(SpeakerWork),
    total: u64,
    /// Units of the parts that are complete.
    base: u64,
    /// The highest units reported.
    peak: u64,
    last: Option<SpeakerWork>,
}

impl<'a> SpeakerMeter<'a> {
    pub fn new(sink: &'a mut dyn FnMut(SpeakerWork)) -> Self {
        Self {
            sink,
            total: 0,
            base: 0,
            peak: 0,
            last: None,
        }
    }

    /// Set the units of the whole note, and the units that are complete
    /// before new work starts: a saved prefix, for example. The next
    /// report includes them.
    pub fn plan(&mut self, total: u64, complete: u64) {
        self.total = total;
        self.base = 0;
        self.saved(complete);
    }

    /// Add `units` of saved work before new work starts. The next report
    /// includes them.
    pub fn saved(&mut self, units: u64) {
        self.base = (self.base + units).min(self.total);
        self.peak = self.peak.max(self.base);
    }

    /// A part of `units` is complete.
    pub fn finished(&mut self, units: u64) {
        self.base = (self.base + units).min(self.total);
        self.report(self.base);
    }

    /// The current part has `units` of measured work complete.
    pub fn at(&mut self, units: u64) {
        self.report((self.base + units).min(self.total));
    }

    /// The current part is complete to `fraction` of its `units`.
    pub fn at_fraction(&mut self, units: u64, fraction: f32) {
        let fraction = f64::from(fraction.clamp(0.0, 1.0));
        self.at((units as f64 * fraction) as u64);
    }

    /// Work runs that gives no fraction (KTD3).
    pub fn unmeasured(&mut self) {
        self.send(SpeakerWork {
            percent: self.percent(),
            measuring: false,
        });
    }

    fn report(&mut self, done: u64) {
        if self.total == 0 {
            return self.unmeasured();
        }
        let measuring = done >= self.peak;
        self.peak = self.peak.max(done);
        self.send(SpeakerWork {
            percent: self.percent(),
            measuring,
        });
    }

    fn percent(&self) -> Option<u8> {
        if self.total == 0 {
            return None;
        }
        let percent = (u128::from(self.peak) * 100 / u128::from(self.total)) as u8;
        Some(percent.min(MEASURED_LIMIT))
    }

    fn send(&mut self, work: SpeakerWork) {
        if self.last != Some(work) {
            self.last = Some(work);
            (self.sink)(work);
        }
    }
}

/// The last progress that the speaker stage saved, if any.
pub fn speakers(store: &SharedStore, note_id: &str) -> Result<Option<SpeakerProgress>, StageError> {
    checkpoint::get(store, note_id, Stage::Diarize, SPEAKER_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(steps: impl FnOnce(&mut SpeakerMeter)) -> Vec<SpeakerWork> {
        let mut seen = Vec::new();
        let mut sink = |work| seen.push(work);
        steps(&mut SpeakerMeter::new(&mut sink));
        seen
    }

    fn measured(percent: u8) -> SpeakerWork {
        SpeakerWork {
            percent: Some(percent),
            measuring: true,
        }
    }

    fn held(percent: u8) -> SpeakerWork {
        SpeakerWork {
            percent: Some(percent),
            measuring: false,
        }
    }

    const NOTHING_YET: SpeakerWork = SpeakerWork {
        percent: None,
        measuring: false,
    };

    #[test]
    fn parts_add_up_over_the_whole_note_without_a_reset() {
        let seen = measure(|m| {
            m.plan(400, 0);
            m.at(0);
            m.at(100);
            m.finished(100);
            // The second part starts where the first stopped.
            m.at(0);
            m.at(100);
        });
        assert_eq!(seen, vec![measured(0), measured(25), measured(50)]);
    }

    #[test]
    fn work_complete_before_the_run_counts_in_the_first_report() {
        let seen = measure(|m| {
            m.plan(400, 100);
            m.saved(100);
            m.saved(100);
            m.at(0);
        });
        assert_eq!(seen, vec![measured(75)]);
    }

    #[test]
    fn active_work_before_the_first_measurement_shows_the_saved_work() {
        let seen = measure(|m| {
            m.plan(400, 100);
            m.unmeasured();
        });
        assert_eq!(seen, vec![held(25)]);
    }

    #[test]
    fn the_last_measured_work_does_not_show_100() {
        let seen = measure(|m| {
            m.plan(100, 0);
            m.at(0);
            m.at(100);
            m.finished(100);
            m.unmeasured();
        });
        assert_eq!(seen, vec![measured(0), measured(99), held(99)]);
    }

    #[test]
    fn a_replay_holds_the_value_until_it_passes_it() {
        let seen = measure(|m| {
            m.plan(100, 0);
            m.at(0);
            m.at(60);
            m.at(10);
            m.at(59);
            m.at(70);
        });
        assert_eq!(
            seen,
            vec![measured(0), measured(60), held(60), measured(70)]
        );
    }

    #[test]
    fn unmeasured_work_keeps_the_last_value() {
        let seen = measure(|m| {
            m.unmeasured();
            m.plan(10, 0);
            m.at(4);
            m.unmeasured();
            m.at(5);
        });
        assert_eq!(
            seen,
            vec![NOTHING_YET, measured(40), held(40), measured(50)]
        );
    }

    #[test]
    fn a_note_with_no_audio_has_no_measured_value() {
        let seen = measure(|m| {
            m.plan(0, 0);
            m.finished(0);
        });
        assert_eq!(seen, vec![NOTHING_YET]);
    }

    #[test]
    fn a_fraction_of_a_part_is_measured_in_its_units() {
        let seen = measure(|m| {
            m.plan(200, 100);
            m.at_fraction(100, 0.5);
            m.at_fraction(100, 7.0);
        });
        assert_eq!(seen, vec![measured(75), measured(99)]);
    }
}
