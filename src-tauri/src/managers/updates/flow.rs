//! The update flow as pure decisions (KTD6). [`Flow::step`] takes one input
//! and the facts at that time, changes the state, and returns the effects
//! for the update manager to do. It makes no network request, opens no
//! window and reads no clock, so the tests cover it directly.

use serde::{Deserialize, Serialize};
use specta::Type;

/// After "Remind Me Later", no automatic offer for 24 hours (R15). After
/// "Later", no ready question for 24 hours (R17).
pub const DELAY_SECONDS: i64 = 24 * 60 * 60;

/// What the update window shows, and what the manager does in the
/// background. A state with a view shows the update window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateState {
    /// Nothing to show.
    Idle,
    /// A check runs. `manual` is true for "Check for Updates…".
    Checking { manual: bool },
    /// A newer build is offered (R13).
    Offered { version: String, current: String },
    /// A newer build is found while busy. The offer shows when busy ends (R14).
    Held { version: String },
    /// The result of a manual check: no newer build.
    UpToDate { current: String },
    /// The result of a manual check that failed.
    CheckFailed { error: String },
    /// The result of a manual check in a build with no track.
    LocalBuild,
    /// The build downloads (R16). `total` is none when the server sends no size.
    Downloading {
        version: String,
        downloaded: u64,
        total: Option<u64>,
    },
    /// The download or the signature check failed (R18).
    Failed { version: String, error: String },
    /// The build is downloaded while busy. The ready question shows when busy ends (R17).
    ReadyHeld { version: String },
    /// The build is downloaded: "Relaunch Now" or "Later" (R17).
    Ready { version: String },
    /// After "Later": the build installs at the next quit.
    WaitingForQuit { version: String },
    /// The build installs, and the app restarts.
    Installing { version: String },
}

/// A choice in the update window. A close of the window is `Close`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    Install,
    RemindLater,
    RelaunchNow,
    Later,
    TryAgain,
    Close,
}

/// The delays that stay across a restart (KTD8), in `updates.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delays {
    /// No automatic offer before this UTC time, in seconds since 1970.
    pub remind_until: Option<i64>,
    /// No ready question before this UTC time, in seconds since 1970.
    pub later_until: Option<i64>,
}

/// One input to the flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// The launch delay, the hourly timer or a wake (R9).
    Timer,
    /// "Check for Updates…" (R11).
    ManualCheck,
    /// The result of a check: the newer version, none, or the error.
    Checked(Result<Option<String>, String>),
    /// Busy started or ended (KTD7).
    Busy(bool),
    Choice(Choice),
    Progress {
        downloaded: u64,
        total: Option<u64>,
    },
    /// The download ended. Ok means that the archive is verified and saved.
    Downloaded(Result<(), String>),
}

/// The facts at the time of an input.
#[derive(Debug, Clone, Copy)]
pub struct Facts {
    /// UTC seconds since 1970.
    pub now: i64,
    pub busy: bool,
    pub automatic_checks: bool,
    /// False for a local build (KTD8).
    pub has_track: bool,
}

/// What the update manager does after a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Check,
    Download {
        version: String,
    },
    /// Install the saved archive and restart.
    Restart,
    /// Write the delays to the store.
    SaveDelays,
}

#[derive(Debug, Clone)]
pub struct Flow {
    state: UpdateState,
    delays: Delays,
    current: String,
    /// The version of the verified archive in the cache, if any.
    saved: Option<String>,
    /// The window of a download is hidden after a close (KTD9).
    download_hidden: bool,
    /// The state that a check returns to when it finds nothing.
    before_check: UpdateState,
}

impl Flow {
    pub fn new(current: impl Into<String>, delays: Delays, saved: Option<String>) -> Self {
        Self {
            state: UpdateState::Idle,
            delays,
            current: current.into(),
            saved,
            download_hidden: false,
            before_check: UpdateState::Idle,
        }
    }

    pub fn state(&self) -> &UpdateState {
        &self.state
    }

    pub fn delays(&self) -> &Delays {
        &self.delays
    }

    /// True when the update window shows the state.
    pub fn window_visible(&self) -> bool {
        match &self.state {
            UpdateState::Offered { .. }
            | UpdateState::UpToDate { .. }
            | UpdateState::CheckFailed { .. }
            | UpdateState::LocalBuild
            | UpdateState::Failed { .. }
            | UpdateState::Ready { .. } => true,
            UpdateState::Downloading { .. } => !self.download_hidden,
            UpdateState::Checking { manual } => *manual,
            _ => false,
        }
    }

    /// True when the user asked for the build and a quit must install it (KTD10).
    pub fn install_at_quit(&self) -> Option<&str> {
        match &self.state {
            UpdateState::ReadyHeld { version }
            | UpdateState::Ready { version }
            | UpdateState::WaitingForQuit { version }
            | UpdateState::Installing { version } => Some(version),
            // A check and its result keep the build that waits for a quit.
            UpdateState::Checking { .. }
            | UpdateState::UpToDate { .. }
            | UpdateState::CheckFailed { .. } => match &self.before_check {
                UpdateState::WaitingForQuit { version } => Some(version),
                _ => None,
            },
            _ => None,
        }
    }

    /// Idle, or the ready build that waits for a quit. A check and its
    /// result return to the state before the check.
    fn rest(&self) -> UpdateState {
        match &self.state {
            UpdateState::WaitingForQuit { .. } => self.state.clone(),
            UpdateState::Checking { .. }
            | UpdateState::UpToDate { .. }
            | UpdateState::CheckFailed { .. } => self.before_check.clone(),
            _ => UpdateState::Idle,
        }
    }

    /// Go to the rest state, and forget the state before the check.
    fn settle(&mut self) {
        self.state = self.rest();
        self.before_check = UpdateState::Idle;
    }

    fn active(until: Option<i64>, now: i64) -> bool {
        until.is_some_and(|t| now < t)
    }

    pub fn step(&mut self, input: Input, facts: Facts) -> Vec<Effect> {
        match input {
            Input::Timer => self.timer(facts),
            Input::ManualCheck => self.manual_check(facts),
            Input::Checked(result) => self.checked(result, facts),
            Input::Busy(busy) => self.busy(busy),
            Input::Choice(choice) => self.choice(choice, facts),
            Input::Progress { downloaded, total } => {
                if let UpdateState::Downloading { version, .. } = &self.state {
                    self.state = UpdateState::Downloading {
                        version: version.clone(),
                        downloaded,
                        total,
                    };
                }
                vec![]
            }
            Input::Downloaded(result) => self.downloaded(result, facts),
        }
    }

    /// An automatic check (R9, R10, R15, R17).
    fn timer(&mut self, facts: Facts) -> Vec<Effect> {
        if !facts.automatic_checks || !facts.has_track {
            return vec![];
        }
        let due = match &self.state {
            UpdateState::Idle => !Self::active(self.delays.remind_until, facts.now),
            UpdateState::WaitingForQuit { .. } => !Self::active(self.delays.later_until, facts.now),
            _ => false,
        };
        if !due {
            return vec![];
        }
        self.before_check = self.rest();
        self.state = UpdateState::Checking { manual: false };
        vec![Effect::Check]
    }

    /// "Check for Updates…" ignores the setting and the delays (R11, AE4).
    fn manual_check(&mut self, facts: Facts) -> Vec<Effect> {
        match &self.state {
            // The window shows the work that runs.
            UpdateState::Downloading { .. } => {
                self.download_hidden = false;
                return vec![];
            }
            UpdateState::Checking { .. }
            | UpdateState::ReadyHeld { .. }
            | UpdateState::Ready { .. }
            | UpdateState::Failed { .. }
            | UpdateState::Installing { .. } => {
                if let UpdateState::ReadyHeld { version } = &self.state {
                    // The user asked, so the question shows while busy.
                    self.state = UpdateState::Ready {
                        version: version.clone(),
                    };
                }
                if let UpdateState::Checking { .. } = self.state {
                    self.state = UpdateState::Checking { manual: true };
                }
                return vec![];
            }
            _ => {}
        }
        if !facts.has_track {
            self.state = UpdateState::LocalBuild;
            return vec![];
        }
        self.before_check = self.rest();
        self.state = UpdateState::Checking { manual: true };
        vec![Effect::Check]
    }

    fn checked(&mut self, result: Result<Option<String>, String>, facts: Facts) -> Vec<Effect> {
        let UpdateState::Checking { manual } = self.state else {
            return vec![];
        };
        // A manual result without a build keeps `before_check` until the
        // window closes.
        let rest = match (&result, manual) {
            (Ok(None) | Err(_), true) => self.before_check.clone(),
            _ => std::mem::replace(&mut self.before_check, UpdateState::Idle),
        };
        match result {
            Ok(Some(version)) if self.saved.as_deref() == Some(version.as_str()) => {
                // The build is already downloaded and verified.
                let waiting = Self::active(self.delays.later_until, facts.now);
                self.state = if !manual && waiting {
                    UpdateState::WaitingForQuit { version }
                } else if facts.busy && !manual {
                    UpdateState::ReadyHeld { version }
                } else {
                    UpdateState::Ready { version }
                };
            }
            Ok(Some(version)) => {
                self.state = if facts.busy && !manual {
                    UpdateState::Held { version }
                } else {
                    UpdateState::Offered {
                        version,
                        current: self.current.clone(),
                    }
                };
            }
            Ok(None) if manual => {
                self.state = UpdateState::UpToDate {
                    current: self.current.clone(),
                }
            }
            Err(error) if manual => self.state = UpdateState::CheckFailed { error },
            // An automatic check shows nothing (R12).
            Ok(None) | Err(_) => self.state = rest,
        }
        vec![]
    }

    /// A recording start closes the window at once (AE2), and the offer or
    /// the question shows again when busy ends (R14, R17).
    fn busy(&mut self, busy: bool) -> Vec<Effect> {
        if busy {
            match &self.state {
                UpdateState::Offered { version, .. } => {
                    self.state = UpdateState::Held {
                        version: version.clone(),
                    }
                }
                // An install that could not claim the restart waits too.
                UpdateState::Ready { version } | UpdateState::Installing { version } => {
                    self.state = UpdateState::ReadyHeld {
                        version: version.clone(),
                    }
                }
                UpdateState::Downloading { .. } => self.download_hidden = true,
                // The check continues, and its result is held or shows nothing.
                UpdateState::Checking { manual: true } => {
                    self.state = UpdateState::Checking { manual: false }
                }
                UpdateState::UpToDate { .. }
                | UpdateState::CheckFailed { .. }
                | UpdateState::LocalBuild
                | UpdateState::Failed { .. } => self.settle(),
                _ => {}
            }
        } else {
            match &self.state {
                UpdateState::Held { version } => {
                    self.state = UpdateState::Offered {
                        version: version.clone(),
                        current: self.current.clone(),
                    }
                }
                UpdateState::ReadyHeld { version } => {
                    self.state = UpdateState::Ready {
                        version: version.clone(),
                    }
                }
                _ => {}
            }
        }
        vec![]
    }

    fn choice(&mut self, choice: Choice, facts: Facts) -> Vec<Effect> {
        match (choice, &self.state) {
            (Choice::Install, UpdateState::Offered { version, .. })
            | (Choice::TryAgain, UpdateState::Failed { version, .. }) => {
                let version = version.clone();
                self.delays.remind_until = None;
                self.download_hidden = false;
                self.state = UpdateState::Downloading {
                    version: version.clone(),
                    downloaded: 0,
                    total: None,
                };
                vec![Effect::Download { version }, Effect::SaveDelays]
            }
            (Choice::RemindLater | Choice::Close, UpdateState::Offered { .. }) => {
                self.delays.remind_until = Some(facts.now + DELAY_SECONDS);
                self.state = UpdateState::Idle;
                vec![Effect::SaveDelays]
            }
            (Choice::Close, UpdateState::Downloading { .. }) => {
                self.download_hidden = true;
                vec![]
            }
            (Choice::RelaunchNow, UpdateState::Ready { version }) => {
                self.state = UpdateState::Installing {
                    version: version.clone(),
                };
                vec![Effect::Restart]
            }
            (Choice::Later | Choice::Close, UpdateState::Ready { version }) => {
                self.delays.later_until = Some(facts.now + DELAY_SECONDS);
                self.state = UpdateState::WaitingForQuit {
                    version: version.clone(),
                };
                vec![Effect::SaveDelays]
            }
            (
                Choice::Close,
                UpdateState::UpToDate { .. }
                | UpdateState::CheckFailed { .. }
                | UpdateState::LocalBuild
                | UpdateState::Failed { .. }
                | UpdateState::Checking { manual: true },
            ) => {
                // After a cancel, the state is not Checking, so the result
                // of the check has no effect.
                self.settle();
                vec![]
            }
            _ => vec![],
        }
    }

    fn downloaded(&mut self, result: Result<(), String>, facts: Facts) -> Vec<Effect> {
        let UpdateState::Downloading { version, .. } = &self.state else {
            return vec![];
        };
        let version = version.clone();
        self.download_hidden = false;
        match result {
            Ok(()) => {
                self.saved = Some(version.clone());
                self.delays.later_until = None;
                if facts.busy {
                    self.state = UpdateState::ReadyHeld { version };
                    vec![Effect::SaveDelays]
                } else {
                    self.state = UpdateState::Installing { version };
                    vec![Effect::SaveDelays, Effect::Restart]
                }
            }
            Err(error) => {
                self.state = UpdateState::Failed { version, error };
                vec![]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURRENT: &str = "0.1.0-preview.20260928143012";
    const NEW: &str = "0.1.0-preview.20260929080000";
    const NEWER: &str = "0.1.0-preview.20260929160000";
    const HOUR: i64 = 60 * 60;
    /// 2026-09-29 09:00 UTC.
    const NINE: i64 = 1_790_672_400;

    fn facts(now: i64) -> Facts {
        Facts {
            now,
            busy: false,
            automatic_checks: true,
            has_track: true,
        }
    }

    fn busy(now: i64) -> Facts {
        Facts {
            busy: true,
            ..facts(now)
        }
    }

    fn flow() -> Flow {
        Flow::new(CURRENT, Delays::default(), None)
    }

    fn offered(version: &str) -> UpdateState {
        UpdateState::Offered {
            version: version.into(),
            current: CURRENT.into(),
        }
    }

    /// A flow that shows the offer for `version`.
    fn offering(version: &str) -> Flow {
        let mut f = flow();
        f.step(Input::Timer, facts(NINE));
        f.step(Input::Checked(Ok(Some(version.into()))), facts(NINE));
        assert_eq!(f.state(), &offered(version));
        f
    }

    /// A flow that asks the ready question for NEW.
    fn ready() -> Flow {
        let mut f = offering(NEW);
        f.step(Input::Choice(Choice::Install), facts(NINE));
        f.step(Input::Downloaded(Ok(())), busy(NINE));
        f.step(Input::Busy(false), facts(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::Ready {
                version: NEW.into()
            }
        );
        f
    }

    #[test]
    fn covers_ae5_automatic_checks_off_make_no_check_and_a_manual_check_still_checks() {
        let mut f = flow();
        let off = Facts {
            automatic_checks: false,
            ..facts(NINE)
        };
        assert_eq!(f.step(Input::Timer, off), vec![]);
        assert_eq!(f.state(), &UpdateState::Idle);
        assert_eq!(f.step(Input::ManualCheck, off), vec![Effect::Check]);
        assert_eq!(f.state(), &UpdateState::Checking { manual: true });
    }

    #[test]
    fn a_build_with_no_track_makes_no_automatic_check_and_says_so_at_a_manual_check() {
        let mut f = flow();
        let local = Facts {
            has_track: false,
            ..facts(NINE)
        };
        assert_eq!(f.step(Input::Timer, local), vec![]);
        assert_eq!(f.step(Input::ManualCheck, local), vec![]);
        assert_eq!(f.state(), &UpdateState::LocalBuild);
        assert!(f.window_visible());
    }

    #[test]
    fn covers_ae1_a_build_found_while_busy_is_held_until_busy_ends() {
        let mut f = flow();
        f.step(Input::Timer, facts(NINE));
        f.step(Input::Checked(Ok(Some(NEW.into()))), busy(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::Held {
                version: NEW.into()
            }
        );
        assert!(!f.window_visible());
        f.step(Input::Busy(false), facts(NINE));
        assert_eq!(f.state(), &offered(NEW));
        assert!(f.window_visible());
    }

    #[test]
    fn covers_ae2_a_recording_start_closes_the_offer_and_holds_it() {
        let mut f = offering(NEW);
        f.step(Input::Busy(true), busy(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::Held {
                version: NEW.into()
            }
        );
        assert!(!f.window_visible());
        f.step(Input::Busy(false), facts(NINE));
        assert_eq!(f.state(), &offered(NEW));
    }

    #[test]
    fn covers_ae3_remind_me_later_waits_24_hours_then_offers_the_newest_build() {
        let mut f = offering(NEW);
        assert_eq!(
            f.step(Input::Choice(Choice::RemindLater), facts(NINE)),
            vec![Effect::SaveDelays]
        );
        assert_eq!(f.state(), &UpdateState::Idle);
        for later in [NINE + 3 * HOUR, NINE + 8 * HOUR, NINE + 24 * HOUR - 1] {
            assert_eq!(f.step(Input::Timer, facts(later)), vec![], "at {later}");
        }
        let next_day = NINE + 24 * HOUR;
        assert_eq!(f.step(Input::Timer, facts(next_day)), vec![Effect::Check]);
        f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(next_day));
        assert_eq!(f.state(), &offered(NEWER));
    }

    #[test]
    fn a_close_of_the_offer_counts_as_remind_me_later() {
        let mut f = offering(NEW);
        f.step(Input::Choice(Choice::Close), facts(NINE));
        assert_eq!(f.delays().remind_until, Some(NINE + DELAY_SECONDS));
    }

    #[test]
    fn covers_ae4_a_manual_check_during_the_remind_delay_shows_at_once() {
        let mut f = offering(NEW);
        f.step(Input::Choice(Choice::RemindLater), facts(NINE));
        assert_eq!(
            f.step(Input::ManualCheck, facts(NINE + HOUR)),
            vec![Effect::Check]
        );
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE + HOUR));
        assert_eq!(f.state(), &offered(NEW));
    }

    #[test]
    fn a_manual_check_shows_its_result_while_busy() {
        let mut f = flow();
        f.step(Input::ManualCheck, busy(NINE));
        f.step(Input::Checked(Ok(Some(NEW.into()))), busy(NINE));
        assert_eq!(f.state(), &offered(NEW));
    }

    #[test]
    fn an_automatic_check_that_fails_or_finds_nothing_shows_nothing() {
        let mut f = flow();
        f.step(Input::Timer, facts(NINE));
        f.step(Input::Checked(Err("offline".into())), facts(NINE));
        assert_eq!(f.state(), &UpdateState::Idle);
        f.step(Input::Timer, facts(NINE + HOUR));
        f.step(Input::Checked(Ok(None)), facts(NINE + HOUR));
        assert_eq!(f.state(), &UpdateState::Idle);
        assert!(!f.window_visible());
    }

    #[test]
    fn a_manual_check_shows_up_to_date_or_the_error() {
        let mut f = flow();
        f.step(Input::ManualCheck, facts(NINE));
        f.step(Input::Checked(Ok(None)), facts(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::UpToDate {
                current: CURRENT.into()
            }
        );
        f.step(Input::Choice(Choice::Close), facts(NINE));
        f.step(Input::ManualCheck, facts(NINE));
        f.step(Input::Checked(Err("offline".into())), facts(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::CheckFailed {
                error: "offline".into()
            }
        );
    }

    #[test]
    fn a_stored_remind_time_is_kept_and_a_past_time_counts_as_ended() {
        let stored = Delays {
            remind_until: Some(NINE + HOUR),
            later_until: None,
        };
        let mut f = Flow::new(CURRENT, stored.clone(), None);
        assert_eq!(f.step(Input::Timer, facts(NINE)), vec![]);
        let mut f = Flow::new(CURRENT, stored, None);
        assert_eq!(
            f.step(Input::Timer, facts(NINE + HOUR)),
            vec![Effect::Check]
        );
    }

    #[test]
    fn install_downloads_and_a_download_that_ends_when_not_busy_restarts() {
        let mut f = offering(NEW);
        assert_eq!(
            f.step(Input::Choice(Choice::Install), facts(NINE)),
            vec![
                Effect::Download {
                    version: NEW.into()
                },
                Effect::SaveDelays
            ]
        );
        f.step(
            Input::Progress {
                downloaded: 10,
                total: Some(100),
            },
            facts(NINE),
        );
        assert_eq!(
            f.state(),
            &UpdateState::Downloading {
                version: NEW.into(),
                downloaded: 10,
                total: Some(100)
            }
        );
        let effects = f.step(Input::Downloaded(Ok(())), facts(NINE));
        assert!(effects.contains(&Effect::Restart));
        assert_eq!(
            f.state(),
            &UpdateState::Installing {
                version: NEW.into()
            }
        );
    }

    #[test]
    fn covers_ae8_a_download_that_ends_while_busy_waits_and_does_not_restart() {
        let mut f = flow();
        f.step(Input::ManualCheck, busy(NINE));
        f.step(Input::Checked(Ok(Some(NEW.into()))), busy(NINE));
        f.step(Input::Choice(Choice::Install), busy(NINE));
        let effects = f.step(Input::Downloaded(Ok(())), busy(NINE));
        assert!(!effects.contains(&Effect::Restart));
        assert_eq!(
            f.state(),
            &UpdateState::ReadyHeld {
                version: NEW.into()
            }
        );
        assert!(!f.window_visible());
    }

    #[test]
    fn covers_f3_a_download_that_started_before_a_recording_finishes_and_asks_after_it() {
        let mut f = offering(NEW);
        f.step(Input::Choice(Choice::Install), facts(NINE));
        f.step(Input::Busy(true), busy(NINE));
        assert!(!f.window_visible(), "a recording start hides the progress");
        f.step(Input::Downloaded(Ok(())), busy(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::ReadyHeld {
                version: NEW.into()
            }
        );
        f.step(Input::Busy(false), facts(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::Ready {
                version: NEW.into()
            }
        );
        assert!(f.window_visible());
    }

    #[test]
    fn relaunch_now_installs_and_restarts() {
        let mut f = ready();
        assert_eq!(
            f.step(Input::Choice(Choice::RelaunchNow), facts(NINE)),
            vec![Effect::Restart]
        );
        assert_eq!(f.install_at_quit(), Some(NEW));
    }

    #[test]
    fn covers_ae7_later_installs_at_the_next_quit() {
        let mut f = ready();
        f.step(Input::Choice(Choice::Later), facts(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::WaitingForQuit {
                version: NEW.into()
            }
        );
        assert!(!f.window_visible());
        assert_eq!(f.install_at_quit(), Some(NEW));
    }

    #[test]
    fn with_no_quit_24_hours_after_later_the_next_check_asks_again() {
        let mut f = ready();
        f.step(Input::Choice(Choice::Later), facts(NINE));
        assert_eq!(f.step(Input::Timer, facts(NINE + 23 * HOUR)), vec![]);
        let next_day = NINE + 24 * HOUR;
        assert_eq!(f.step(Input::Timer, facts(next_day)), vec![Effect::Check]);
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(next_day));
        assert_eq!(
            f.state(),
            &UpdateState::Ready {
                version: NEW.into()
            }
        );
    }

    #[test]
    fn a_newer_build_found_while_waiting_for_quit_is_offered_in_place_of_the_ready_build() {
        let mut f = ready();
        f.step(Input::Choice(Choice::Later), facts(NINE));
        let next_day = NINE + 24 * HOUR;
        f.step(Input::Timer, facts(next_day));
        f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(next_day));
        assert_eq!(f.state(), &offered(NEWER));
    }

    #[test]
    fn a_failed_check_while_waiting_for_quit_keeps_the_ready_build() {
        let mut f = ready();
        f.step(Input::Choice(Choice::Later), facts(NINE));
        let next_day = NINE + 24 * HOUR;
        f.step(Input::Timer, facts(next_day));
        f.step(Input::Checked(Err("offline".into())), facts(next_day));
        assert_eq!(
            f.state(),
            &UpdateState::WaitingForQuit {
                version: NEW.into()
            }
        );
    }

    #[test]
    fn a_failed_download_shows_the_error_and_try_again_downloads_again() {
        let mut f = offering(NEW);
        f.step(Input::Choice(Choice::Install), facts(NINE));
        f.step(Input::Choice(Choice::Close), facts(NINE));
        assert!(!f.window_visible(), "a close hides the progress");
        f.step(Input::Downloaded(Err("bad signature".into())), facts(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::Failed {
                version: NEW.into(),
                error: "bad signature".into()
            }
        );
        assert!(f.window_visible(), "the window shows again at Failed");
        assert_eq!(f.install_at_quit(), None);
        assert_eq!(
            f.step(Input::Choice(Choice::TryAgain), facts(NINE)),
            vec![
                Effect::Download {
                    version: NEW.into()
                },
                Effect::SaveDelays
            ]
        );
    }

    #[test]
    fn a_check_that_finds_the_saved_build_waits_for_quit_during_the_later_delay() {
        let delays = Delays {
            remind_until: None,
            later_until: Some(NINE + HOUR),
        };
        let mut f = Flow::new(CURRENT, delays, Some(NEW.into()));
        f.step(Input::Timer, facts(NINE));
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::WaitingForQuit {
                version: NEW.into()
            }
        );
        assert!(!f.window_visible());
    }

    #[test]
    fn a_check_that_finds_the_saved_build_asks_the_ready_question() {
        let mut f = Flow::new(CURRENT, Delays::default(), Some(NEW.into()));
        f.step(Input::Timer, facts(NINE));
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::Ready {
                version: NEW.into()
            }
        );
    }

    /// A flow where the build NEW waits for a quit after "Later".
    fn waiting_for_quit() -> Flow {
        let mut f = ready();
        f.step(Input::Choice(Choice::Later), facts(NINE));
        assert_eq!(f.install_at_quit(), Some(NEW));
        f
    }

    #[test]
    fn a_manual_check_that_fails_or_finds_nothing_keeps_the_install_at_quit() {
        for result in [Err("offline".to_string()), Ok(None)] {
            let mut f = waiting_for_quit();
            f.step(Input::ManualCheck, facts(NINE));
            assert_eq!(f.install_at_quit(), Some(NEW));
            f.step(Input::Checked(result), facts(NINE));
            assert!(f.window_visible());
            assert_eq!(f.install_at_quit(), Some(NEW));
            f.step(Input::Choice(Choice::Close), facts(NINE));
            assert_eq!(
                f.state(),
                &UpdateState::WaitingForQuit {
                    version: NEW.into()
                }
            );
        }
    }

    #[test]
    fn an_automatic_check_keeps_the_install_at_quit_while_it_runs() {
        let mut f = waiting_for_quit();
        f.step(Input::Timer, facts(NINE + 25 * HOUR));
        assert_eq!(f.state(), &UpdateState::Checking { manual: false });
        assert_eq!(f.install_at_quit(), Some(NEW));
    }

    #[test]
    fn a_cancelled_manual_check_shows_nothing_when_its_result_arrives() {
        let mut f = flow();
        f.step(Input::ManualCheck, facts(NINE));
        f.step(Input::Choice(Choice::Close), facts(NINE));
        assert_eq!(f.state(), &UpdateState::Idle);
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE));
        assert_eq!(f.state(), &UpdateState::Idle);
        assert!(!f.window_visible());
    }

    #[test]
    fn a_recording_start_closes_the_manual_check_window_and_holds_its_result() {
        let mut f = flow();
        f.step(Input::ManualCheck, facts(NINE));
        f.step(Input::Busy(true), busy(NINE));
        assert!(!f.window_visible());
        f.step(Input::Checked(Ok(Some(NEW.into()))), busy(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::Held {
                version: NEW.into()
            }
        );
        f.step(Input::Busy(false), facts(NINE));
        assert_eq!(f.state(), &offered(NEW));
    }

    #[test]
    fn a_recording_that_starts_before_the_restart_holds_the_install() {
        let mut f = ready();
        assert_eq!(
            f.step(Input::Choice(Choice::RelaunchNow), facts(NINE)),
            vec![Effect::Restart]
        );
        // The restart claim failed, and the recording start arrives.
        f.step(Input::Busy(true), busy(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::ReadyHeld {
                version: NEW.into()
            }
        );
        assert_eq!(f.install_at_quit(), Some(NEW));
        f.step(Input::Busy(false), facts(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::Ready {
                version: NEW.into()
            }
        );
    }

    #[test]
    fn a_recording_start_during_a_manual_check_shows_no_error_later() {
        let mut f = waiting_for_quit();
        f.step(Input::ManualCheck, facts(NINE));
        f.step(Input::Busy(true), busy(NINE));
        f.step(Input::Checked(Err("offline".into())), busy(NINE));
        assert!(!f.window_visible());
        assert_eq!(
            f.state(),
            &UpdateState::WaitingForQuit {
                version: NEW.into()
            }
        );
    }
}
