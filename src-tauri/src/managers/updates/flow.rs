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
    /// The result of a manual check in a build that is not a release build.
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
    /// A click on the update item of the tray menu (R4).
    TrayOpen,
}

/// The facts at the time of an input.
#[derive(Debug, Clone, Copy)]
pub struct Facts {
    /// UTC seconds since 1970.
    pub now: i64,
    pub busy: bool,
    pub automatic_checks: bool,
    /// True for a release build. Only a release build gets updates (KTD8).
    pub is_release_build: bool,
}

/// What the update manager does after a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Check,
    /// A check with the short timeout, for an Install click (KTD7).
    QuickCheck,
    Download {
        version: String,
    },
    /// Install the saved archive and restart.
    Restart,
    /// Write the delays to the store.
    SaveDelays,
}

/// The purposes that wait for the one check in flight. A result serves
/// each of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Pending {
    /// The timer started the check, or a recording start turned a manual
    /// check into one. Its result can show an offer.
    background: bool,
    /// "Check for Updates…" waits for the result.
    manual: bool,
    /// An Install click waits for the result to choose the version that
    /// downloads (R7).
    install: bool,
}

/// A download of `version` that has not started.
fn downloading(version: String) -> UpdateState {
    UpdateState::Downloading {
        version,
        downloaded: 0,
        total: None,
    }
}

/// True when version `a` is newer than version `b`. Two versions that do
/// not parse count as newer when they differ.
fn newer(a: &str, b: &str) -> bool {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(a), Ok(b)) => a > b,
        _ => a != b,
    }
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
    /// The release that the last successful check named. A check that
    /// names none clears it, and a failed check keeps it.
    newest: Option<String>,
    /// The check in flight, if any. Only one check runs at a time.
    pending: Option<Pending>,
    /// The user asked for the window ("Check for Updates…" or the tray
    /// item), so it shows with the focus and needs no Longhand window in
    /// front. It ends when the window closes.
    user_asked: bool,
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
            newest: None,
            pending: None,
            user_asked: false,
        }
    }

    /// True while the window shows a state that the user asked for.
    pub fn user_asked(&self) -> bool {
        self.user_asked
    }

    /// The version for the update item of the tray menu: a newer release
    /// that is not installing (R4, R5).
    pub fn tray_version(&self) -> Option<&str> {
        if let UpdateState::Installing { .. } = self.state {
            return None;
        }
        self.newest
            .as_deref()
            .filter(|newest| newer(newest, &self.current))
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

    /// True when a check found a release newer than `version` (R9).
    fn superseded(&self, version: &str) -> bool {
        self.newest.as_deref().is_some_and(|n| newer(n, version))
    }

    /// The build that a quit must install (KTD10). A ready build that a
    /// newer release replaces does not install (R9). A build that the user
    /// confirmed with "Relaunch Now" or a download installs (KTD8).
    pub fn install_at_quit(&self) -> Option<&str> {
        match &self.state {
            UpdateState::ReadyHeld { version } | UpdateState::Installing { version } => {
                Some(version)
            }
            UpdateState::Ready { version } | UpdateState::WaitingForQuit { version } => {
                self.unless_superseded(version)
            }
            // A check and its result keep the build that waits for a quit.
            UpdateState::Checking { .. }
            | UpdateState::UpToDate { .. }
            | UpdateState::CheckFailed { .. } => match &self.before_check {
                UpdateState::WaitingForQuit { version } => self.unless_superseded(version),
                _ => None,
            },
            _ => None,
        }
    }

    fn unless_superseded<'a>(&self, version: &'a str) -> Option<&'a str> {
        (!self.superseded(version)).then_some(version)
    }

    /// The state to return to after a manual check or its result: the
    /// notice that was there before, or Idle.
    fn rest(&self) -> UpdateState {
        match &self.state {
            UpdateState::Offered { .. }
            | UpdateState::Held { .. }
            | UpdateState::WaitingForQuit { .. } => self.state.clone(),
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
        let asked = matches!(input, Input::ManualCheck | Input::TrayOpen);
        let effects = self.apply(input, facts);
        if asked {
            self.user_asked = true;
        }
        if !self.window_visible() {
            self.user_asked = false;
        }
        effects
    }

    fn apply(&mut self, input: Input, facts: Facts) -> Vec<Effect> {
        match input {
            Input::Timer => self.timer(facts),
            Input::ManualCheck => self.manual_check(facts),
            Input::TrayOpen => self.tray_open(),
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

    /// The tray item shows the notice for the newest release, also while
    /// busy and during a delay, as "Check for Updates…" does (R4).
    fn tray_open(&mut self) -> Vec<Effect> {
        match self.state.clone() {
            UpdateState::Idle => {
                if let Some(newest) = self.newest.clone() {
                    self.state = self.notice(newest, false);
                }
            }
            UpdateState::Held { version } => {
                self.state = UpdateState::Offered {
                    version,
                    current: self.current.clone(),
                }
            }
            UpdateState::ReadyHeld { version } => self.state = UpdateState::Ready { version },
            UpdateState::WaitingForQuit { version } => {
                self.state = match self.newest.clone() {
                    Some(newest) if newer(&newest, &version) => self.notice(newest, false),
                    _ => UpdateState::Ready { version },
                }
            }
            UpdateState::Downloading { .. } => self.download_hidden = false,
            _ => {}
        }
        vec![]
    }

    /// An automatic check (R9, R10). It runs also during the delays, and
    /// the delays decide only whether its result shows a notice (R6).
    fn timer(&mut self, facts: Facts) -> Vec<Effect> {
        if !facts.automatic_checks || !facts.is_release_build || self.pending.is_some() {
            return vec![];
        }
        if let UpdateState::Downloading { .. } | UpdateState::Installing { .. } = self.state {
            return vec![];
        }
        self.pending = Some(Pending {
            background: true,
            ..Pending::default()
        });
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
                return vec![];
            }
            _ => {}
        }
        if !facts.is_release_build {
            self.state = UpdateState::LocalBuild;
            return vec![];
        }
        self.before_check = self.rest();
        self.state = UpdateState::Checking { manual: true };
        match &mut self.pending {
            // The check in flight serves the user too.
            Some(pending) => {
                pending.manual = true;
                vec![]
            }
            None => {
                self.pending = Some(Pending {
                    manual: true,
                    ..Pending::default()
                });
                vec![Effect::Check]
            }
        }
    }

    fn checked(&mut self, result: Result<Option<String>, String>, facts: Facts) -> Vec<Effect> {
        let Some(pending) = self.pending.take() else {
            return vec![];
        };
        match &result {
            Ok(Some(version)) => self.newest = Some(version.clone()),
            Ok(None) => self.newest = None,
            Err(_) => {}
        }
        if pending.install {
            return self.install_checked(result.ok().flatten(), facts);
        }
        if pending.manual && self.state == (UpdateState::Checking { manual: true }) {
            self.manual_result(result);
        } else if pending.background && matches!(result, Ok(Some(_))) {
            // An automatic check that fails or finds nothing shows nothing (R12).
            self.offer_newest(facts);
        }
        vec![]
    }

    /// The result of "Check for Updates…" shows at once.
    fn manual_result(&mut self, result: Result<Option<String>, String>) {
        match result {
            Ok(Some(version)) => {
                self.before_check = UpdateState::Idle;
                self.state = if self.saved.as_deref() == Some(version.as_str()) {
                    UpdateState::Ready { version }
                } else {
                    UpdateState::Offered {
                        version,
                        current: self.current.clone(),
                    }
                };
            }
            Ok(None) => {
                self.state = UpdateState::UpToDate {
                    current: self.current.clone(),
                }
            }
            Err(error) => self.state = UpdateState::CheckFailed { error },
        }
    }

    /// After an automatic check: offer the newest release when no delay
    /// holds it (R6, R15, R17). A shown ready question keeps its version
    /// (KTD9).
    fn offer_newest(&mut self, facts: Facts) {
        let Some(newest) = self.newest.clone() else {
            return;
        };
        let remind = Self::active(self.delays.remind_until, facts.now);
        let later = Self::active(self.delays.later_until, facts.now);
        let saved = self.saved.as_deref() == Some(newest.as_str());
        match &self.state {
            UpdateState::Idle if saved && later => {
                self.state = UpdateState::WaitingForQuit { version: newest }
            }
            UpdateState::Idle if !remind && !later => self.state = self.notice(newest, facts.busy),
            UpdateState::WaitingForQuit { .. } if !later => {
                self.state = self.notice(newest, facts.busy)
            }
            UpdateState::Offered { version, .. } | UpdateState::Held { version }
                if *version != newest =>
            {
                self.state = self.notice(newest, facts.busy)
            }
            _ => {}
        }
    }

    /// The notice for `version`: the ready question for the saved build,
    /// else the offer. While busy, it is held (R14, R17).
    fn notice(&self, version: String, busy: bool) -> UpdateState {
        match (self.saved.as_deref() == Some(version.as_str()), busy) {
            (true, true) => UpdateState::ReadyHeld { version },
            (true, false) => UpdateState::Ready { version },
            (false, true) => UpdateState::Held { version },
            (false, false) => UpdateState::Offered {
                version,
                current: self.current.clone(),
            },
        }
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
                    if let Some(pending) = &mut self.pending {
                        pending.manual = false;
                        pending.background = true;
                    }
                    self.settle();
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
            // A "Relaunch Now" that a newer release replaced the question
            // under arrives at the offer, and installs the newest (KTD9).
            (Choice::Install | Choice::RelaunchNow, UpdateState::Offered { version, .. })
            | (Choice::TryAgain, UpdateState::Failed { version, .. }) => {
                let version = version.clone();
                self.install(version)
            }
            (Choice::RelaunchNow, UpdateState::Ready { version }) if self.superseded(version) => {
                let newest = self.newest.clone().unwrap_or_else(|| version.clone());
                self.install(newest)
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
                // After a cancel, the user no longer waits for the check,
                // so its result changes only the newest release.
                if let Some(pending) = &mut self.pending {
                    pending.manual = false;
                }
                self.settle();
                vec![]
            }
            _ => vec![],
        }
    }

    /// The install path (R7, KTD7): the progress shows at once, and a check
    /// chooses the version that downloads. An Install click while a check
    /// runs waits for that check.
    fn install(&mut self, version: String) -> Vec<Effect> {
        self.delays.remind_until = None;
        self.download_hidden = false;
        self.state = downloading(version);
        match &mut self.pending {
            Some(pending) => {
                pending.install = true;
                vec![Effect::SaveDelays]
            }
            None => {
                self.pending = Some(Pending {
                    install: true,
                    ..Pending::default()
                });
                vec![Effect::QuickCheck, Effect::SaveDelays]
            }
        }
    }

    /// The check of an Install click ended. A check that fails or names no
    /// release downloads the shown version.
    fn install_checked(&mut self, named: Option<String>, facts: Facts) -> Vec<Effect> {
        let UpdateState::Downloading { version: shown, .. } = &self.state else {
            return vec![];
        };
        let version = named.unwrap_or_else(|| shown.clone());
        if self.saved.as_deref() == Some(version.as_str()) {
            return self.install_saved(version, facts);
        }
        self.state = downloading(version.clone());
        vec![Effect::Download { version }]
    }

    /// The archive of `version` is saved: restart, or wait while busy.
    fn install_saved(&mut self, version: String, facts: Facts) -> Vec<Effect> {
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

    fn downloaded(&mut self, result: Result<(), String>, facts: Facts) -> Vec<Effect> {
        let UpdateState::Downloading { version, .. } = &self.state else {
            return vec![];
        };
        let version = version.clone();
        self.download_hidden = false;
        match result {
            Ok(()) => self.install_saved(version, facts),
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

    const CURRENT: &str = "1.0.3";
    const NEW: &str = "1.0.4";
    const NEWER: &str = "1.0.5";
    const HOUR: i64 = 60 * 60;
    /// 2026-09-29 09:00 UTC.
    const NINE: i64 = 1_790_672_400;

    fn facts(now: i64) -> Facts {
        Facts {
            now,
            busy: false,
            automatic_checks: true,
            is_release_build: true,
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
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE));
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
    fn a_build_that_is_not_a_release_build_never_checks_and_says_so_at_a_manual_check() {
        let mut f = flow();
        let local = Facts {
            is_release_build: false,
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
        // The checks continue during the delay, and their results show
        // nothing (R6).
        for later in [NINE + 3 * HOUR, NINE + 24 * HOUR - 1] {
            assert_eq!(f.step(Input::Timer, facts(later)), vec![Effect::Check]);
            f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(later));
            assert_eq!(f.state(), &UpdateState::Idle, "at {later}");
            assert_eq!(f.tray_version(), Some(NEWER));
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
        f.step(Input::Timer, facts(NINE));
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE));
        assert_eq!(f.state(), &UpdateState::Idle);
        let mut f = Flow::new(CURRENT, stored, None);
        f.step(Input::Timer, facts(NINE + HOUR));
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE + HOUR));
        assert_eq!(f.state(), &offered(NEW));
    }

    #[test]
    fn install_downloads_and_a_download_that_ends_when_not_busy_restarts() {
        let mut f = offering(NEW);
        assert_eq!(
            f.step(Input::Choice(Choice::Install), facts(NINE)),
            vec![Effect::QuickCheck, Effect::SaveDelays]
        );
        assert_eq!(
            f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE)),
            vec![Effect::Download {
                version: NEW.into()
            }]
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
        f.step(Input::Checked(Ok(Some(NEW.into()))), busy(NINE));
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
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE));
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
        assert_eq!(
            f.step(Input::Timer, facts(NINE + 23 * HOUR)),
            vec![Effect::Check]
        );
        f.step(
            Input::Checked(Ok(Some(NEW.into()))),
            facts(NINE + 23 * HOUR),
        );
        assert_eq!(
            f.state(),
            &UpdateState::WaitingForQuit {
                version: NEW.into()
            }
        );
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
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE));
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
            vec![Effect::QuickCheck, Effect::SaveDelays]
        );
        assert_eq!(
            f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE)),
            vec![Effect::Download {
                version: NEW.into()
            }]
        );
    }

    #[test]
    fn covers_ae2_install_downloads_the_release_that_a_check_at_the_click_names() {
        let mut f = offering(NEW);
        f.step(Input::Choice(Choice::Install), facts(NINE));
        assert_eq!(
            f.state(),
            &downloading(NEW.into()),
            "the progress shows at once"
        );
        assert_eq!(
            f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(NINE)),
            vec![Effect::Download {
                version: NEWER.into()
            }]
        );
        assert_eq!(f.state(), &downloading(NEWER.into()));
    }

    #[test]
    fn install_downloads_the_shown_version_when_its_check_fails_or_names_none() {
        for result in [Err("offline".to_string()), Ok(None)] {
            let mut f = offering(NEW);
            f.step(Input::Choice(Choice::Install), facts(NINE));
            assert_eq!(
                f.step(Input::Checked(result), facts(NINE)),
                vec![Effect::Download {
                    version: NEW.into()
                }]
            );
        }
    }

    #[test]
    fn install_of_the_saved_version_restarts_without_a_download() {
        let mut f = Flow::new(CURRENT, Delays::default(), Some(NEWER.into()));
        f.step(Input::Timer, facts(NINE));
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE));
        f.step(Input::Choice(Choice::Install), facts(NINE));
        let effects = f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(NINE));
        assert_eq!(effects, vec![Effect::SaveDelays, Effect::Restart]);
        assert_eq!(
            f.state(),
            &UpdateState::Installing {
                version: NEWER.into()
            }
        );
    }

    #[test]
    fn install_while_a_check_runs_waits_for_that_check() {
        let mut f = offering(NEW);
        f.step(Input::Timer, facts(NINE + HOUR));
        assert_eq!(
            f.step(Input::Choice(Choice::Install), facts(NINE + HOUR)),
            vec![Effect::SaveDelays],
            "no second check"
        );
        assert_eq!(
            f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(NINE + HOUR)),
            vec![Effect::Download {
                version: NEWER.into()
            }]
        );
    }

    #[test]
    fn busy_and_close_during_the_install_check_hide_the_progress() {
        let mut f = offering(NEW);
        f.step(Input::Choice(Choice::Install), facts(NINE));
        f.step(Input::Busy(true), busy(NINE));
        assert!(!f.window_visible());
        let mut f = offering(NEW);
        f.step(Input::Choice(Choice::Install), facts(NINE));
        f.step(Input::Choice(Choice::Close), facts(NINE));
        assert!(!f.window_visible());
        assert_eq!(
            f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE)),
            vec![Effect::Download {
                version: NEW.into()
            }],
            "the download continues"
        );
    }

    #[test]
    fn try_again_downloads_the_newest_release() {
        let mut f = offering(NEW);
        f.step(Input::Choice(Choice::Install), facts(NINE));
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE));
        f.step(Input::Downloaded(Err("offline".into())), facts(NINE));
        f.step(Input::Timer, facts(NINE + HOUR));
        f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(NINE + HOUR));
        f.step(Input::Choice(Choice::TryAgain), facts(NINE + HOUR));
        assert_eq!(
            f.step(Input::Checked(Err("offline".into())), facts(NINE + HOUR)),
            vec![Effect::Download {
                version: NEW.into()
            }],
            "the shown version when the check fails"
        );
        let mut f = offering(NEW);
        f.step(Input::Choice(Choice::Install), facts(NINE));
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE));
        f.step(Input::Downloaded(Err("offline".into())), facts(NINE));
        f.step(Input::Choice(Choice::TryAgain), facts(NINE));
        assert_eq!(
            f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(NINE)),
            vec![Effect::Download {
                version: NEWER.into()
            }]
        );
    }

    #[test]
    fn relaunch_now_with_a_newer_release_installs_the_newest() {
        let mut f = ready();
        f.step(Input::Timer, facts(NINE + HOUR));
        f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(NINE + HOUR));
        assert_eq!(
            f.step(Input::Choice(Choice::RelaunchNow), facts(NINE + HOUR)),
            vec![Effect::QuickCheck, Effect::SaveDelays]
        );
        assert_eq!(f.state(), &downloading(NEWER.into()));
    }

    #[test]
    fn relaunch_now_that_arrives_at_the_offer_installs() {
        let mut f = offering(NEW);
        assert_eq!(
            f.step(Input::Choice(Choice::RelaunchNow), facts(NINE)),
            vec![Effect::QuickCheck, Effect::SaveDelays]
        );
    }

    #[test]
    fn covers_ae3_the_tray_item_opens_the_offer_during_the_remind_delay() {
        let mut f = offering(NEW);
        f.step(Input::Choice(Choice::RemindLater), facts(NINE));
        assert_eq!(f.tray_version(), Some(NEW));
        f.step(Input::TrayOpen, facts(NINE + HOUR));
        assert_eq!(f.state(), &offered(NEW));
        assert!(f.user_asked());
        assert_eq!(f.delays().remind_until, Some(NINE + DELAY_SECONDS));
    }

    #[test]
    fn covers_ae5_a_manual_check_counts_as_asked() {
        let mut f = flow();
        f.step(Input::ManualCheck, facts(NINE));
        assert!(f.user_asked());
    }

    #[test]
    fn a_close_ends_the_ask() {
        let mut f = offering(NEW);
        f.step(Input::TrayOpen, facts(NINE));
        assert!(f.user_asked());
        f.step(Input::Choice(Choice::Close), facts(NINE));
        assert!(!f.user_asked());
    }

    #[test]
    fn an_automatic_offer_is_not_asked_for() {
        let f = offering(NEW);
        assert!(!f.user_asked());
    }

    #[test]
    fn the_tray_item_shows_held_notices_while_busy() {
        let mut f = offering(NEW);
        f.step(Input::Busy(true), busy(NINE));
        f.step(Input::TrayOpen, busy(NINE));
        assert_eq!(f.state(), &offered(NEW));
        let mut f = ready();
        f.step(Input::Busy(true), busy(NINE));
        f.step(Input::TrayOpen, busy(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::Ready {
                version: NEW.into()
            }
        );
        let mut f = offering(NEW);
        f.step(Input::Choice(Choice::Install), facts(NINE));
        f.step(Input::Choice(Choice::Close), facts(NINE));
        f.step(Input::TrayOpen, facts(NINE));
        assert!(f.window_visible(), "a hidden download shows again");
    }

    #[test]
    fn the_tray_item_while_waiting_for_quit_asks_or_offers_the_newer_release() {
        let mut f = waiting_for_quit();
        f.step(Input::TrayOpen, facts(NINE + HOUR));
        assert_eq!(
            f.state(),
            &UpdateState::Ready {
                version: NEW.into()
            }
        );
        let mut f = waiting_for_quit();
        f.step(Input::Timer, facts(NINE + HOUR));
        f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(NINE + HOUR));
        f.step(Input::TrayOpen, facts(NINE + HOUR));
        assert_eq!(f.state(), &offered(NEWER));
    }

    #[test]
    fn the_tray_item_does_nothing_while_installing() {
        let mut f = ready();
        f.step(Input::Choice(Choice::RelaunchNow), facts(NINE));
        f.step(Input::TrayOpen, facts(NINE));
        assert_eq!(
            f.state(),
            &UpdateState::Installing {
                version: NEW.into()
            }
        );
        assert_eq!(f.tray_version(), None);
    }

    #[test]
    fn the_tray_version_is_the_newest_release_or_none() {
        let mut f = flow();
        assert_eq!(f.tray_version(), None);
        f.step(Input::Timer, facts(NINE));
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE));
        assert_eq!(f.tray_version(), Some(NEW));
        f.step(Input::Choice(Choice::RemindLater), facts(NINE));
        let mut later = waiting_for_quit();
        assert_eq!(later.tray_version(), Some(NEW));
        later.step(Input::Timer, facts(NINE + HOUR));
        later.step(Input::Checked(Ok(None)), facts(NINE + HOUR));
        assert_eq!(later.tray_version(), None);
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
        assert_eq!(
            f.step(Input::Timer, facts(NINE + 25 * HOUR)),
            vec![Effect::Check]
        );
        assert_eq!(
            f.state(),
            &UpdateState::WaitingForQuit {
                version: NEW.into()
            }
        );
        assert_eq!(f.install_at_quit(), Some(NEW));
    }

    #[test]
    fn covers_ae2_a_newer_release_changes_the_offer_in_place() {
        let mut f = offering(NEW);
        assert_eq!(
            f.step(Input::Timer, facts(NINE + HOUR)),
            vec![Effect::Check]
        );
        assert_eq!(
            f.state(),
            &offered(NEW),
            "the offer stays while the check runs"
        );
        f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(NINE + HOUR));
        assert_eq!(f.state(), &offered(NEWER));
        assert!(f.window_visible());
    }

    #[test]
    fn covers_ae7_a_newer_release_found_after_later_stops_the_install_at_quit() {
        let mut f = waiting_for_quit();
        f.step(Input::Timer, facts(NINE + HOUR));
        f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(NINE + HOUR));
        assert_eq!(
            f.state(),
            &UpdateState::WaitingForQuit {
                version: NEW.into()
            },
            "the Later delay holds the offer"
        );
        assert_eq!(f.install_at_quit(), None);
        let next_day = NINE + 24 * HOUR;
        f.step(Input::Timer, facts(next_day));
        f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(next_day));
        assert_eq!(f.state(), &offered(NEWER));
    }

    #[test]
    fn a_shown_ready_question_keeps_its_version_when_a_newer_release_comes_out() {
        let mut f = ready();
        f.step(Input::Timer, facts(NINE + HOUR));
        f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(NINE + HOUR));
        assert_eq!(
            f.state(),
            &UpdateState::Ready {
                version: NEW.into()
            }
        );
        assert_eq!(f.tray_version(), Some(NEWER));
        assert_eq!(f.install_at_quit(), None);
    }

    #[test]
    fn a_confirmed_install_still_installs_when_a_newer_release_comes_out() {
        let mut f = ready();
        f.step(Input::Timer, facts(NINE));
        f.step(Input::Choice(Choice::RelaunchNow), facts(NINE));
        f.step(Input::Checked(Ok(Some(NEWER.into()))), facts(NINE));
        assert_eq!(f.install_at_quit(), Some(NEW));
    }

    #[test]
    fn the_newest_release_follows_the_last_successful_check() {
        let mut f = flow();
        f.step(Input::Timer, facts(NINE));
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE));
        f.step(Input::Choice(Choice::RemindLater), facts(NINE));
        f.step(Input::Timer, facts(NINE + HOUR));
        f.step(Input::Checked(Err("offline".into())), facts(NINE + HOUR));
        assert_eq!(f.tray_version(), Some(NEW), "a failed check keeps it");
        f.step(Input::Timer, facts(NINE + 2 * HOUR));
        f.step(Input::Checked(Ok(None)), facts(NINE + 2 * HOUR));
        assert_eq!(
            f.tray_version(),
            None,
            "a check that names no release clears it"
        );
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert!(newer("1.0.10", "1.0.9"));
        assert!(!newer("1.0.9", "1.0.10"));
        assert!(!newer("1.0.4", "1.0.4"));
    }

    #[test]
    fn a_timer_while_a_check_runs_starts_no_second_check() {
        let mut f = flow();
        f.step(Input::ManualCheck, facts(NINE));
        assert_eq!(f.step(Input::Timer, facts(NINE)), vec![]);
    }

    #[test]
    fn the_checks_continue_while_a_manual_result_shows() {
        let mut f = flow();
        f.step(Input::ManualCheck, facts(NINE));
        f.step(Input::Checked(Ok(None)), facts(NINE));
        assert_eq!(
            f.step(Input::Timer, facts(NINE + HOUR)),
            vec![Effect::Check]
        );
        f.step(Input::Checked(Ok(Some(NEW.into()))), facts(NINE + HOUR));
        assert_eq!(
            f.state(),
            &UpdateState::UpToDate {
                current: CURRENT.into()
            }
        );
        assert_eq!(f.tray_version(), Some(NEW));
    }

    #[test]
    fn a_failed_manual_check_from_the_offer_returns_to_the_offer() {
        let mut f = offering(NEW);
        f.step(Input::ManualCheck, facts(NINE));
        f.step(Input::Checked(Err("offline".into())), facts(NINE));
        f.step(Input::Choice(Choice::Close), facts(NINE));
        assert_eq!(f.state(), &offered(NEW));
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
