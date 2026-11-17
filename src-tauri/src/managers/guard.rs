//! The silence check and the time limit of a recording, with the "Still
//! recording?" prompt and the automatic stop (R2–R9, R17, KTD5–KTD9 of the
//! forgotten-recording plan).
//!
//! [`GuardFlow`] makes all the decisions. It has no I/O and gets the UTC
//! time from its caller, so the tests need no waits. [`GuardManager`] keeps
//! the flow and the meter of the note that records, applies the actions of
//! the flow, and runs the `longhand-guard` thread. While the flow watches a
//! recording, the thread ticks once a second. Else it waits for the next
//! recording, because a tick then does nothing. The deadline of a prompt is UTC wall clock, because a monotonic
//! timer does not advance while the computer sleeps.
//!
//! Lock order: the capture lock, then the guard lock. The guard lock is
//! never held while the capture lock or the store lock is taken.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use recorder::speech::GuardMeter;
use serde::Serialize;
use session_core::note_record::AutoStop;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event as _;

use crate::managers::capture::CaptureManager;
use crate::managers::processing::ProcessingManager;
use crate::managers::settings::RecordingGuards;

/// The time between two ticks of the guard thread.
const TICK: Duration = Duration::from_secs(1);

const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;

/// The prompt "Still recording?" of the note that records (R5, R9). It
/// holds each check that triggered, with the time of the check in the
/// settings snapshot of the recording.
#[derive(Serialize, Clone, Type, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub note_id: String,
    /// A new number for each prompt. An answer with another number does
    /// nothing.
    pub generation: u64,
    /// The minutes of the silence check, when the silence check is in the
    /// prompt.
    pub silence_check_minutes: Option<u32>,
    /// The hours of the time limit, when the time limit is in the prompt.
    pub time_limit_hours: Option<u32>,
    /// The end of the countdown, in milliseconds since the Unix epoch (UTC).
    pub deadline_ms: i64,
    /// The length of the countdown.
    pub countdown_minutes: u32,
}

impl Prompt {
    fn deadline(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(self.deadline_ms).unwrap_or(DateTime::<Utc>::MAX_UTC)
    }

    /// The stop reason of the note: each check in the prompt (R8).
    fn reason(&self) -> AutoStop {
        AutoStop {
            silence_check_minutes: self.silence_check_minutes,
            time_limit_hours: self.time_limit_hours,
        }
    }
}

/// The prompt opened, changed or closed (R5, R9). `prompt` is `None` when
/// the prompt `generation` closed. A surface ignores an event of a
/// generation that is older than the prompt that it shows.
#[derive(Serialize, Clone, Type, tauri_specta::Event)]
pub struct RecordingPrompt {
    pub generation: u64,
    pub prompt: Option<Prompt>,
}

/// The automatic stop of a note at the end of the countdown (R7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopRequest {
    pub note_id: String,
    pub generation: u64,
    pub reason: AutoStop,
}

/// What the flow tells its owner to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Show a new prompt.
    Show(Prompt),
    /// A second check joined the open prompt. The deadline stays.
    Join(Prompt),
    /// Close the prompt `generation` on each surface, and remove the system
    /// notification.
    Close { generation: u64 },
    /// Set the silence time of the meter to zero (R6).
    ResetSilence,
    /// Stop the note with the reason (R7, KTD7).
    AutoStop(StopRequest),
}

/// The checks of the recording that records.
#[derive(Debug, Clone)]
struct Watch {
    guards: RecordingGuards,
    paused: bool,
    /// The recording time at which the time limit triggers. Keep Recording
    /// moves it a further limit after the recording time of the answer.
    next_limit_ms: Option<u64>,
    /// The recording time of the last tick.
    run_ms: u64,
}

/// The prompt flow of one note (KTD5). It has no I/O.
#[derive(Debug, Default)]
pub struct GuardFlow {
    /// The note of the current recording.
    note_id: Option<String>,
    /// `None` for a note with no settings snapshot: it gets no checks.
    watch: Option<Watch>,
    prompt: Option<Prompt>,
    /// The automatic stop that the countdown asked for and that did not
    /// end yet.
    stopping: Option<StopRequest>,
    /// The last generation. It never goes back, also for a new note.
    generations: u64,
}

impl GuardFlow {
    /// A new recording of `note_id`: Start, Resume or Record into a note
    /// (R3). An open prompt closes.
    pub fn start(&mut self, note_id: &str, guards: Option<RecordingGuards>) -> Vec<Action> {
        let actions = self.close();
        self.stopping = None;
        self.note_id = Some(note_id.to_string());
        self.watch = guards.map(|guards| Watch {
            next_limit_ms: guards.time_limit_hours.map(|h| u64::from(h) * HOUR_MS),
            guards,
            paused: false,
            run_ms: 0,
        });
        actions
    }

    /// One tick at `now`, with the counters of the meter (R2, R3, R5, R7,
    /// R9). Speech during the countdown does not hold the stop.
    pub fn tick(&mut self, now: DateTime<Utc>, silence_ms: u64, run_ms: u64) -> Vec<Action> {
        if self.stopping.is_some() {
            return Vec::new();
        }
        if let Some(actions) = self.deadline(now) {
            return actions;
        }
        let Some(watch) = self.watch.as_mut() else {
            return Vec::new();
        };
        // The meter does not count while paused, but it keeps the silence
        // time until Continue sets it to zero (R4).
        if watch.paused {
            return Vec::new();
        }
        watch.run_ms = run_ms;
        let guards = watch.guards;
        let silence = guards
            .silence_check_minutes
            .filter(|m| silence_ms >= u64::from(*m) * MINUTE_MS);
        let limit = guards
            .time_limit_hours
            .filter(|_| watch.next_limit_ms.is_some_and(|limit| run_ms >= limit));
        if let Some(prompt) = self.prompt.as_mut() {
            // A second check joins the open prompt and keeps its deadline
            // (R9).
            let mut joined = false;
            if silence.is_some() && prompt.silence_check_minutes.is_none() {
                prompt.silence_check_minutes = silence;
                joined = true;
            }
            if limit.is_some() && prompt.time_limit_hours.is_none() {
                prompt.time_limit_hours = limit;
                joined = true;
            }
            return if joined {
                vec![Action::Join(prompt.clone())]
            } else {
                Vec::new()
            };
        }
        if silence.is_none() && limit.is_none() {
            return Vec::new();
        }
        self.generations += 1;
        let countdown = TimeDelta::minutes(i64::from(guards.countdown_minutes));
        let prompt = Prompt {
            note_id: self.note_id.clone().unwrap_or_default(),
            generation: self.generations,
            silence_check_minutes: silence,
            time_limit_hours: limit,
            deadline_ms: (now + countdown).timestamp_millis(),
            countdown_minutes: guards.countdown_minutes,
        };
        self.prompt = Some(prompt.clone());
        vec![Action::Show(prompt)]
    }

    /// The computer woke at `now` (R17, KTD9). When the countdown ended in
    /// the sleep, the note stops before the microphone opens again.
    pub fn wake(&mut self, now: DateTime<Utc>) -> Vec<Action> {
        if let Some(stop) = &self.stopping {
            return vec![Action::AutoStop(stop.clone())];
        }
        self.deadline(now).unwrap_or_default()
    }

    /// Keep Recording for the prompt `generation` (R6). An answer to a
    /// prompt that is not open does nothing.
    pub fn keep(&mut self, generation: u64) -> Vec<Action> {
        if !self.is_open(generation) {
            return Vec::new();
        }
        self.release()
    }

    /// Stop for the prompt `generation`: the prompt closes, and the caller
    /// stops the note of the prompt as a user Stop. Returns that note and
    /// the actions. `None` when the prompt is not open.
    pub fn answer_stop(&mut self, generation: u64) -> Option<(String, Vec<Action>)> {
        let note_id = self.prompt.as_ref()?.note_id.clone();
        self.is_open(generation).then(|| (note_id, self.close()))
    }

    /// The user paused the note (R4). An open prompt closes, and the pause
    /// counts as Keep Recording for each check in it.
    pub fn pause(&mut self) -> Vec<Action> {
        let actions = self.release();
        if let Some(watch) = self.watch.as_mut() {
            watch.paused = true;
        }
        actions
    }

    /// The user continued the paused note. The caller set the silence time
    /// of the meter to zero first (R4).
    pub fn resume(&mut self) {
        if let Some(watch) = self.watch.as_mut() {
            watch.paused = false;
        }
    }

    /// The note `note_id` stopped. Its prompt closes. A stop of another
    /// note does nothing.
    pub fn stopped(&mut self, note_id: &str) -> Vec<Action> {
        if self.note_id.as_deref() != Some(note_id) {
            return Vec::new();
        }
        let actions = self.close();
        *self = Self {
            generations: self.generations,
            ..Self::default()
        };
        actions
    }

    /// Whether the automatic stop of the prompt `generation` of `note_id`
    /// is still due (KTD7).
    pub fn stop_pending(&self, note_id: &str, generation: u64) -> bool {
        self.stopping
            .as_ref()
            .is_some_and(|s| s.note_id == note_id && s.generation == generation)
    }

    /// The automatic stop of the prompt `generation` did not stop the
    /// note. The checks continue.
    pub fn forget(&mut self, generation: u64) {
        if self
            .stopping
            .as_ref()
            .is_some_and(|s| s.generation == generation)
        {
            self.stopping = None;
        }
    }

    pub fn prompt(&self) -> Option<&Prompt> {
        self.prompt.as_ref()
    }

    fn is_open(&self, generation: u64) -> bool {
        self.prompt
            .as_ref()
            .is_some_and(|p| p.generation == generation)
    }

    /// The countdown ended at `now`: the prompt closes, and the note stops
    /// with each check in the prompt as its reason (R7).
    fn deadline(&mut self, now: DateTime<Utc>) -> Option<Vec<Action>> {
        let prompt = self.prompt.take_if(|p| now >= p.deadline())?;
        let stop = StopRequest {
            note_id: prompt.note_id.clone(),
            generation: prompt.generation,
            reason: prompt.reason(),
        };
        self.stopping = Some(stop.clone());
        Some(vec![
            Action::Close {
                generation: prompt.generation,
            },
            Action::AutoStop(stop),
        ])
    }

    /// Close the prompt and start each check in it again (R6).
    fn release(&mut self) -> Vec<Action> {
        let Some(prompt) = self.prompt.take() else {
            return Vec::new();
        };
        let mut actions = vec![Action::Close {
            generation: prompt.generation,
        }];
        if prompt.silence_check_minutes.is_some() {
            actions.push(Action::ResetSilence);
        }
        if let (Some(hours), Some(watch)) = (prompt.time_limit_hours, self.watch.as_mut()) {
            watch.next_limit_ms = Some(watch.run_ms + u64::from(hours) * HOUR_MS);
        }
        actions
    }

    fn close(&mut self) -> Vec<Action> {
        self.prompt
            .take()
            .map(|p| {
                vec![Action::Close {
                    generation: p.generation,
                }]
            })
            .unwrap_or_default()
    }
}

/// The system notification of the prompt (R5, R9). The app uses
/// [`crate::notify_prompt::NativeNotice`]. It has no buttons and plays no
/// sound.
pub trait PromptNotice: Send + Sync {
    /// Show the notification of `prompt`, or change it for a check that
    /// joined.
    fn show(&self, prompt: &Prompt);
    /// Remove the notification of the prompt `generation`.
    fn remove(&self, generation: u64);
}

/// No system notification: the prompt shows only in the app.
struct NoNotice;

impl PromptNotice for NoNotice {
    fn show(&self, _: &Prompt) {}
    fn remove(&self, _: u64) {}
}

struct GuardState {
    flow: GuardFlow,
    /// The meter of the current recording.
    meter: Option<Arc<GuardMeter>>,
}

/// Keeps the prompt flow of the note that records and applies its actions
/// (KTD5).
pub struct GuardManager {
    state: Mutex<GuardState>,
    /// Notified when the flow starts to watch a recording.
    watched: Condvar,
    notice: Box<dyn PromptNotice>,
}

impl Default for GuardManager {
    fn default() -> Self {
        Self::new(Box::new(NoNotice))
    }
}

impl GuardManager {
    pub fn new(notice: Box<dyn PromptNotice>) -> Self {
        Self {
            state: Mutex::new(GuardState {
                flow: GuardFlow::default(),
                meter: None,
            }),
            watched: Condvar::new(),
            notice,
        }
    }

    fn state(&self) -> MutexGuard<'_, GuardState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Apply the meter actions under the guard lock, so that a tick never
    /// reads a silence time from before a reset. Returns the other actions.
    fn applied(state: &GuardState, actions: Vec<Action>) -> Vec<Action> {
        actions
            .into_iter()
            .filter(|action| {
                if *action != Action::ResetSilence {
                    return true;
                }
                if let Some(meter) = &state.meter {
                    meter.reset_silence();
                }
                false
            })
            .collect()
    }

    /// A new recording of `note_id` with the snapshot `guards` and the new
    /// meter `meter` (R3, KTD14).
    pub fn start(
        &self,
        note_id: &str,
        guards: Option<RecordingGuards>,
        meter: Arc<GuardMeter>,
    ) -> Vec<Action> {
        let mut state = self.state();
        state.meter = Some(meter);
        let actions = state.flow.start(note_id, guards);
        let actions = Self::applied(&state, actions);
        self.watched.notify_all();
        actions
    }

    /// Wait until the flow watches a recording. A flow with no watch has no
    /// prompt, so a tick then does nothing.
    fn wait_for_watch(&self) {
        let state = self.state();
        drop(
            self.watched
                .wait_while(state, |state| state.flow.watch.is_none())
                .unwrap_or_else(|e| e.into_inner()),
        );
    }

    pub fn pause(&self) -> Vec<Action> {
        let mut state = self.state();
        let actions = state.flow.pause();
        Self::applied(&state, actions)
    }

    pub fn resume(&self) {
        self.state().flow.resume();
    }

    pub fn stopped(&self, note_id: &str) -> Vec<Action> {
        let mut state = self.state();
        let actions = state.flow.stopped(note_id);
        if state.flow.note_id.is_none() {
            state.meter = None;
        }
        actions
    }

    pub fn keep(&self, generation: u64) -> Vec<Action> {
        let mut state = self.state();
        let actions = state.flow.keep(generation);
        Self::applied(&state, actions)
    }

    pub fn answer_stop(&self, generation: u64) -> Option<(String, Vec<Action>)> {
        self.state().flow.answer_stop(generation)
    }

    /// One tick at `now`. The counters are read under the guard lock.
    pub fn tick(&self, now: DateTime<Utc>) -> Vec<Action> {
        let mut state = self.state();
        let (silence_ms, run_ms) = state
            .meter
            .as_ref()
            .map_or((0, 0), |m| (m.silence_ms(), m.run_ms()));
        let actions = state.flow.tick(now, silence_ms, run_ms);
        Self::applied(&state, actions)
    }

    pub fn wake(&self, now: DateTime<Utc>) -> Vec<Action> {
        let actions = self.state().flow.wake(now);
        self.watched.notify_all();
        actions
    }

    pub fn stop_pending(&self, note_id: &str, generation: u64) -> bool {
        self.state().flow.stop_pending(note_id, generation)
    }

    pub fn forget(&self, generation: u64) {
        self.state().flow.forget(generation);
    }

    /// The open prompt, for [`crate::managers::capture::RecordingStatus`].
    pub fn prompt(&self) -> Option<Prompt> {
        self.state().flow.prompt().cloned()
    }

    /// Show the notification of `prompt`. The show runs with no lock, so a
    /// Pause or a Stop can close the prompt and remove its notification
    /// first. Then the notification is removed again. Returns false when
    /// the prompt is closed.
    fn show_notice(&self, prompt: &Prompt) -> bool {
        self.notice.show(prompt);
        let open = self.state().flow.is_open(prompt.generation);
        if !open {
            self.notice.remove(prompt.generation);
        }
        open
    }

    /// Show, change and close the prompt on each surface (R5, R9). Returns
    /// the automatic stop when `actions` has one. The caller holds no lock.
    pub fn publish(&self, app: &AppHandle, actions: Vec<Action>) -> Option<StopRequest> {
        let mut stop = None;
        for action in actions {
            match action {
                Action::Show(prompt) | Action::Join(prompt) => {
                    log::info!(
                        "prompt {} of {} shows (silence check: {:?} min, time limit: {:?} h)",
                        prompt.generation,
                        prompt.note_id,
                        prompt.silence_check_minutes,
                        prompt.time_limit_hours
                    );
                    // A prompt that closed first gets no surface.
                    if !self.show_notice(&prompt) {
                        continue;
                    }
                    emit(
                        app,
                        RecordingPrompt {
                            generation: prompt.generation,
                            prompt: Some(prompt),
                        },
                    );
                }
                Action::Close { generation } => {
                    log::info!("prompt {generation} closes");
                    self.notice.remove(generation);
                    emit(
                        app,
                        RecordingPrompt {
                            generation,
                            prompt: None,
                        },
                    );
                }
                Action::AutoStop(request) => stop = Some(request),
                // The manager applies it under the guard lock.
                Action::ResetSilence => {}
            }
        }
        stop
    }

    /// Stop the note of `stop` on the user's Stop path (KTD7). Nothing
    /// stops when Keep Recording, a Pause or a new recording came first.
    pub fn auto_stop(&self, app: &AppHandle, capture: &CaptureManager, stop: StopRequest) {
        match capture.stop_session_if(app, &stop.note_id, stop.generation, &stop.reason) {
            Ok(Some(outcome)) => {
                log::info!(
                    "{} stopped at the end of the countdown: {outcome:?}",
                    stop.note_id
                );
                if let Some(processing) = app.try_state::<Arc<ProcessingManager>>() {
                    processing.wake();
                }
            }
            Ok(None) => {
                log::info!("the automatic stop of {} does not apply", stop.note_id);
                self.forget(stop.generation);
            }
            Err(e) => {
                log::error!("the automatic stop of {} failed: {e}", stop.note_id);
                self.forget(stop.generation);
            }
        }
    }

    /// Start the `longhand-guard` thread, which ticks once a second while
    /// the flow watches a recording.
    pub fn start_thread(self: &Arc<Self>, app: AppHandle, capture: Arc<CaptureManager>) {
        let guard = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("longhand-guard".into())
            .spawn(move || {
                loop {
                    guard.wait_for_watch();
                    std::thread::sleep(TICK);
                    let actions = guard.tick(Utc::now());
                    if let Some(stop) = guard.publish(&app, actions) {
                        guard.auto_stop(&app, &capture, stop);
                    }
                }
            });
        if let Err(e) = spawned {
            log::error!("could not start the recording checks: {e}");
        }
    }
}

fn emit(app: &AppHandle, event: RecordingPrompt) {
    if let Err(e) = event.emit(app) {
        log::warn!("could not send the prompt event: {e}");
    }
}

/// The guard manager of the app, when it is set up.
pub fn of(app: &AppHandle) -> Option<Arc<GuardManager>> {
    app.try_state::<Arc<GuardManager>>()
        .map(|guard| Arc::clone(&guard))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    /// The time `secs` after the start of the recording.
    fn at(secs: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 16, 15, 0, 0).unwrap() + TimeDelta::seconds(secs)
    }

    const MIN: u64 = 60_000;

    fn guards(silence: Option<u32>, limit: Option<u32>) -> RecordingGuards {
        RecordingGuards {
            silence_check_minutes: silence,
            time_limit_hours: limit,
            cut_minutes: Some(10),
            countdown_minutes: 2,
        }
    }

    fn defaults() -> RecordingGuards {
        guards(Some(30), Some(6))
    }

    fn started(guards: Option<RecordingGuards>) -> GuardFlow {
        let mut flow = GuardFlow::default();
        assert!(flow.start("n", guards).is_empty());
        flow
    }

    fn shown(actions: &[Action]) -> Prompt {
        match actions {
            [Action::Show(prompt)] => prompt.clone(),
            other => panic!("no prompt shows: {other:?}"),
        }
    }

    fn stop_of(actions: &[Action]) -> StopRequest {
        match actions {
            [Action::Close { .. }, Action::AutoStop(stop)] => stop.clone(),
            other => panic!("no automatic stop: {other:?}"),
        }
    }

    /// The silence check after 30 minutes with no speech: one prompt, at
    /// 30 minutes and not before.
    fn silence_prompt(flow: &mut GuardFlow) -> Prompt {
        assert!(flow.tick(at(29 * 60), 29 * MIN, 29 * MIN).is_empty());
        shown(&flow.tick(at(30 * 60), 30 * MIN, 30 * MIN))
    }

    // Covers R2, R5 (Watching → Prompt).
    #[test]
    fn the_silence_check_shows_one_prompt_with_a_countdown_at_its_time() {
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        assert_eq!(prompt.note_id, "n");
        assert_eq!(prompt.silence_check_minutes, Some(30));
        assert_eq!(prompt.time_limit_hours, None);
        assert_eq!(prompt.countdown_minutes, 2);
        assert_eq!(prompt.deadline_ms, at(32 * 60).timestamp_millis());
        // The open prompt does not show again.
        assert!(flow.tick(at(31 * 60), 31 * MIN, 31 * MIN).is_empty());
        assert_eq!(flow.prompt(), Some(&prompt));
    }

    // Covers R3.
    #[test]
    fn the_time_limit_shows_a_prompt_after_its_recording_time() {
        let mut flow = started(Some(defaults()));
        // Speech keeps the silence time short.
        assert!(flow.tick(at(1), 0, 6 * 60 * MIN - 1).is_empty());
        let prompt = shown(&flow.tick(at(2), 0, 6 * 60 * MIN));
        assert_eq!(prompt.time_limit_hours, Some(6));
        assert_eq!(prompt.silence_check_minutes, None);
    }

    // Covers R9 (Prompt → Prompt: the second check joins).
    #[test]
    fn a_second_check_joins_the_open_prompt_with_the_same_deadline() {
        let mut flow = started(Some(guards(Some(30), Some(1))));
        let first = shown(&flow.tick(at(0), 30 * MIN, 59 * MIN));
        let joined = flow.tick(at(60), 31 * MIN, 60 * MIN);
        let [Action::Join(prompt)] = joined.as_slice() else {
            panic!("the time limit does not join: {joined:?}");
        };
        assert_eq!(prompt.generation, first.generation);
        assert_eq!(prompt.deadline_ms, first.deadline_ms);
        assert_eq!(prompt.silence_check_minutes, Some(30));
        assert_eq!(prompt.time_limit_hours, Some(1));
        // The stop names both checks (R8).
        let stop = stop_of(&flow.tick(at(120), 32 * MIN, 61 * MIN));
        assert_eq!(
            stop.reason,
            AutoStop {
                silence_check_minutes: Some(30),
                time_limit_hours: Some(1),
            }
        );
    }

    // Covers R7 (Prompt → Stopped: deadline passed), AE1, AE4.
    #[test]
    fn the_end_of_the_countdown_closes_the_prompt_and_stops_the_note_also_with_speech() {
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        // Speech in the countdown does not hold the stop.
        assert!(flow.tick(at(31 * 60), 0, 31 * MIN).is_empty());
        let actions = flow.tick(at(32 * 60), 0, 32 * MIN);
        assert_eq!(
            actions,
            vec![
                Action::Close {
                    generation: prompt.generation
                },
                Action::AutoStop(StopRequest {
                    note_id: "n".into(),
                    generation: prompt.generation,
                    reason: AutoStop {
                        silence_check_minutes: Some(30),
                        time_limit_hours: None,
                    },
                }),
            ]
        );
        assert!(flow.stop_pending("n", prompt.generation));
        // No second stop while the stop runs.
        assert!(flow.tick(at(33 * 60), 33 * MIN, 33 * MIN).is_empty());
        assert!(flow.prompt().is_none());
    }

    // Covers R6 (Prompt → Watching: Keep Recording).
    #[test]
    fn keep_recording_starts_the_silence_check_again() {
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        assert_eq!(
            flow.keep(prompt.generation),
            vec![
                Action::Close {
                    generation: prompt.generation
                },
                Action::ResetSilence,
            ]
        );
        assert!(flow.prompt().is_none());
        // The meter counts from zero, and the check asks again after a
        // further 30 minutes.
        assert!(flow.tick(at(50 * 60), 29 * MIN, 60 * MIN).is_empty());
        let again = shown(&flow.tick(at(51 * 60), 30 * MIN, 61 * MIN));
        assert!(again.generation > prompt.generation);
        // No stop at the deadline of the old prompt.
        assert!(flow.tick(at(52 * 60), 31 * MIN, 62 * MIN).is_empty());
    }

    // Covers R6: Keep Recording resets only the checks in the prompt, and
    // the time limit asks again after a further limit.
    #[test]
    fn keep_recording_resets_only_the_checks_in_the_prompt() {
        let mut flow = started(Some(guards(Some(30), Some(1))));
        let prompt = shown(&flow.tick(at(0), 10 * MIN, 60 * MIN));
        assert_eq!(prompt.time_limit_hours, Some(1));
        // The silence check is not in the prompt: no reset of the meter.
        assert_eq!(
            flow.keep(prompt.generation),
            vec![Action::Close {
                generation: prompt.generation
            }]
        );
        assert!(flow.tick(at(60), 11 * MIN, 61 * MIN).is_empty());
        // The time limit asks again one hour of recording time after the
        // answer.
        assert!(flow.tick(at(3600), 0, 120 * MIN - 1).is_empty());
        let again = shown(&flow.tick(at(3601), 0, 120 * MIN));
        assert_eq!(again.time_limit_hours, Some(1));
    }

    // Covers R6, R9: an answer to a closed prompt does nothing.
    #[test]
    fn an_answer_with_an_old_generation_does_nothing() {
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        assert!(flow.keep(prompt.generation + 1).is_empty());
        assert!(flow.answer_stop(prompt.generation + 1).is_none());
        assert_eq!(flow.prompt(), Some(&prompt));
    }

    // Covers R7 (Prompt → Stopped: Stop answer, a user stop with no reason).
    #[test]
    fn the_stop_answer_closes_the_prompt_and_asks_for_no_automatic_stop() {
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        // The answer names the note of the prompt: the caller stops only
        // that note.
        assert_eq!(
            flow.answer_stop(prompt.generation),
            Some((
                "n".to_string(),
                vec![Action::Close {
                    generation: prompt.generation
                }]
            ))
        );
        assert!(!flow.stop_pending("n", prompt.generation));
        assert!(flow.prompt().is_none());
    }

    // Covers R7, R9, KTD7: the deadline and Keep Recording at the same
    // moment give one result, by generation.
    #[test]
    fn the_deadline_and_keep_recording_at_the_same_moment_give_one_result() {
        // The deadline first: the late Keep Recording does nothing.
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        let stop = stop_of(&flow.tick(at(32 * 60), 32 * MIN, 32 * MIN));
        assert!(flow.keep(prompt.generation).is_empty());
        assert!(flow.stop_pending("n", stop.generation));

        // Keep Recording first: the tick at the deadline stops nothing.
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        assert!(!flow.keep(prompt.generation).is_empty());
        assert!(flow.tick(at(32 * 60), 0, 32 * MIN).is_empty());
        assert!(!flow.stop_pending("n", prompt.generation));
    }

    // Covers KTD7: a stop that is due is no longer due for a new recording.
    #[test]
    fn a_new_recording_ends_a_stop_that_is_due() {
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        stop_of(&flow.tick(at(32 * 60), 32 * MIN, 32 * MIN));
        assert!(flow.start("m", Some(defaults())).is_empty());
        assert!(!flow.stop_pending("n", prompt.generation));
        assert!(!flow.stop_pending("m", prompt.generation));
    }

    // Covers R3: a new recording closes the open prompt.
    #[test]
    fn a_new_recording_closes_the_open_prompt() {
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        assert_eq!(
            flow.start("n", Some(defaults())),
            vec![Action::Close {
                generation: prompt.generation
            }]
        );
        assert!(flow.tick(at(33 * 60), 0, 0).is_empty());
    }

    // Covers R4 (Prompt → Paused: Pause counts as Keep Recording).
    #[test]
    fn pause_closes_the_prompt_as_keep_recording() {
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        assert_eq!(
            flow.pause(),
            vec![
                Action::Close {
                    generation: prompt.generation
                },
                Action::ResetSilence,
            ]
        );
        // No stop at the deadline while paused.
        assert!(flow.tick(at(60 * 60), 30 * MIN, 30 * MIN).is_empty());
    }

    // Covers R4, AE7 (Watching → Paused → Watching).
    #[test]
    fn a_pause_after_twenty_quiet_minutes_shows_no_prompt_at_continue() {
        let mut flow = started(Some(defaults()));
        assert!(flow.tick(at(20 * 60), 20 * MIN, 20 * MIN).is_empty());
        assert!(flow.pause().is_empty());
        // The paused meter keeps its silence time until Continue: no
        // check counts paused time.
        assert!(flow.tick(at(80 * 60), 20 * MIN, 20 * MIN).is_empty());
        flow.resume();
        // Continue set the silence time to zero.
        assert!(flow.tick(at(80 * 60 + 1), 0, 20 * MIN).is_empty());
        assert!(flow.tick(at(109 * 60), 29 * MIN, 49 * MIN).is_empty());
        shown(&flow.tick(at(110 * 60), 30 * MIN, 50 * MIN));
    }

    // Covers R4: paused time does not count, also not for checks that
    // a meter that stands still would trigger.
    #[test]
    fn a_paused_note_gets_no_prompt() {
        let mut flow = started(Some(defaults()));
        flow.pause();
        assert!(flow.tick(at(60 * 60), 40 * MIN, 7 * 60 * MIN).is_empty());
    }

    // Covers R4: a time-limit prompt that a Pause closed does not show
    // again at Continue.
    #[test]
    fn a_time_limit_prompt_closed_by_pause_does_not_show_again_at_continue() {
        let mut flow = started(Some(guards(Some(30), Some(1))));
        let prompt = shown(&flow.tick(at(0), 0, 60 * MIN));
        assert_eq!(prompt.time_limit_hours, Some(1));
        assert_eq!(
            flow.pause(),
            vec![Action::Close {
                generation: prompt.generation
            }]
        );
        flow.resume();
        // The time limit continues from the recording time of the pause.
        assert!(flow.tick(at(3600), 0, 60 * MIN).is_empty());
        assert!(flow.tick(at(7000), 0, 120 * MIN - 1).is_empty());
        shown(&flow.tick(at(7001), 0, 120 * MIN));
    }

    // Covers R17, AE8 (Asleep → Stopped: wake after the deadline).
    #[test]
    fn a_wake_after_the_deadline_stops_the_note() {
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        // The lid closes 30 seconds later, and the computer wakes 10
        // minutes later.
        let stop = stop_of(&flow.wake(at(30 * 60 + 30 + 600)));
        assert_eq!(stop.generation, prompt.generation);
        assert_eq!(stop.reason.silence_check_minutes, Some(30));
        // The guard thread can tick first after the wake: the wake still
        // gets the stop that is due, so the microphone does not open.
        assert_eq!(
            flow.wake(at(30 * 60 + 31 + 600)),
            vec![Action::AutoStop(stop)]
        );
    }

    // Covers R17 (Asleep → Prompt: wake before the deadline).
    #[test]
    fn a_wake_before_the_deadline_keeps_the_prompt() {
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        assert!(flow.wake(at(31 * 60)).is_empty());
        assert_eq!(flow.prompt(), Some(&prompt));
        assert!(flow.wake(at(31 * 60)).is_empty());
    }

    // Covers R17, KTD9: a sleep past the 30 minutes of the automatic
    // queue, with a deadline that passed, stops the note with its reason.
    #[test]
    fn a_wake_after_a_long_sleep_stops_the_note_with_its_reason() {
        let mut flow = started(Some(defaults()));
        silence_prompt(&mut flow);
        let stop = stop_of(&flow.wake(at(30 * 60 + 45 * 60)));
        assert_eq!(
            stop.reason,
            AutoStop {
                silence_check_minutes: Some(30),
                time_limit_hours: None,
            }
        );
    }

    // Covers KTD9: the countdown continues while the note is interrupted:
    // the meter stands still, and the deadline stops the note.
    #[test]
    fn the_countdown_ends_while_the_input_is_lost() {
        let mut flow = started(Some(defaults()));
        silence_prompt(&mut flow);
        assert!(flow.tick(at(31 * 60), 30 * MIN, 30 * MIN).is_empty());
        stop_of(&flow.tick(at(32 * 60), 30 * MIN, 30 * MIN));
    }

    // Covers R10: checks that are off never trigger.
    #[test]
    fn checks_that_are_off_never_trigger() {
        let mut flow = started(Some(guards(None, None)));
        assert!(flow.tick(at(0), 240 * MIN, 24 * 60 * MIN).is_empty());
        assert!(flow.tick(at(1), 10_000 * MIN, 100 * 60 * MIN).is_empty());
    }

    // Covers KTD14: a note with no snapshot gets no checks.
    #[test]
    fn a_note_with_no_snapshot_gets_no_checks() {
        let mut flow = started(None);
        assert!(flow.tick(at(0), 240 * MIN, 24 * 60 * MIN).is_empty());
    }

    // Covers R7, R9 (Watching → Stopped: user Stop).
    #[test]
    fn a_stop_closes_the_prompt_of_its_note_only() {
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        assert!(flow.stopped("other").is_empty());
        assert_eq!(
            flow.stopped("n"),
            vec![Action::Close {
                generation: prompt.generation
            }]
        );
        assert!(flow.tick(at(40 * 60), 40 * MIN, 40 * MIN).is_empty());
    }

    // Covers KTD7: a stop that did not apply lets the checks continue.
    #[test]
    fn a_forgotten_stop_lets_the_checks_continue() {
        let mut flow = started(Some(defaults()));
        let prompt = silence_prompt(&mut flow);
        stop_of(&flow.tick(at(32 * 60), 32 * MIN, 32 * MIN));
        flow.forget(prompt.generation);
        assert!(!flow.stop_pending("n", prompt.generation));
        shown(&flow.tick(at(33 * 60), 33 * MIN, 33 * MIN));
    }

    // Covers R6, R9: the manager resets the meter under its lock at Keep
    // Recording, so the next tick reads zero.
    #[test]
    fn keep_recording_sets_the_silence_time_of_the_meter_to_zero() {
        let guard = GuardManager::default();
        let meter = Arc::new(GuardMeter::default());
        guard.start("n", Some(defaults()), Arc::clone(&meter));
        meter.add(30 * MIN, false);
        let actions = guard.tick(at(30 * 60));
        let prompt = shown(&actions);
        assert_eq!(guard.prompt(), Some(prompt.clone()));
        let closed = guard.keep(prompt.generation);
        assert_eq!(
            closed,
            vec![Action::Close {
                generation: prompt.generation
            }]
        );
        assert_eq!(meter.silence_ms(), 0);
        assert_eq!(meter.run_ms(), 30 * MIN);
        assert!(guard.tick(at(30 * 60 + 1)).is_empty());
    }

    // The guard thread waits while no recording is watched, and a start
    // ends the wait. The test waits for the end of the wait, not for a
    // time.
    #[test]
    fn a_start_ends_the_wait_of_the_guard_thread() {
        let guard = Arc::new(GuardManager::default());
        let waiter = {
            let guard = Arc::clone(&guard);
            std::thread::spawn(move || guard.wait_for_watch())
        };
        guard.start("n", Some(defaults()), Arc::new(GuardMeter::default()));
        waiter.join().unwrap();
        // A watched flow does not wait.
        guard.wait_for_watch();
    }

    /// A notice that records each call.
    #[derive(Clone, Default)]
    struct Calls(Arc<Mutex<Vec<String>>>);

    impl PromptNotice for Calls {
        fn show(&self, prompt: &Prompt) {
            self.0
                .lock()
                .unwrap()
                .push(format!("show {}", prompt.generation));
        }
        fn remove(&self, generation: u64) {
            self.0.lock().unwrap().push(format!("remove {generation}"));
        }
    }

    // Covers R9: a Pause that closes the prompt before its notification
    // shows leaves no notification.
    #[test]
    fn a_notification_of_a_prompt_that_closed_first_is_removed() {
        let calls = Calls::default();
        let guard = GuardManager::new(Box::new(calls.clone()));
        let meter = Arc::new(GuardMeter::default());
        guard.start("n", Some(defaults()), Arc::clone(&meter));
        meter.add(30 * MIN, false);
        let prompt = shown(&guard.tick(at(30 * 60)));
        assert!(guard.show_notice(&prompt));
        assert_eq!(
            *calls.0.lock().unwrap(),
            vec![format!("show {}", prompt.generation)]
        );

        // The Pause closes the prompt, and then the show runs.
        guard.pause();
        assert!(!guard.show_notice(&prompt));
        assert_eq!(
            *calls.0.lock().unwrap(),
            vec![
                format!("show {}", prompt.generation),
                format!("show {}", prompt.generation),
                format!("remove {}", prompt.generation),
            ]
        );
    }
}
