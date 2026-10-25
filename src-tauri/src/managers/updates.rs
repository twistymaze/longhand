//! The update manager (KTD6 to KTD10 of
//! `docs/plans/2026-09-28-1240-feat-preview-builds-and-updates-plan.md`). In
//! a release build, it checks the update manifest of the latest release,
//! holds an offer while a note records or processes, keeps the remind and
//! ready delays, downloads after the user's click, and installs at the
//! click, at quit or at the next launch.
//!
//! [`flow::Flow`] makes each decision. This module does the effects: the
//! network, the saved archives, the store, the window and the restart.

mod archive;
pub mod flow;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;
use tauri_plugin_updater::{Update, UpdaterExt};
use tauri_specta::Event as _;
use tokio::sync::mpsc;

use crate::commands::build_info;
use crate::managers::capture::CaptureManager;
pub use flow::{Choice, UpdateState};
use flow::{Delays, Effect, Facts, Flow, Input};

const STORE_PATH: &str = "updates.json";
const DELAYS_KEY: &str = "delays";
const INSTALL_FAILURE_KEY: &str = "install_failure";
/// The first automatic check comes 30 seconds after launch (KTD8).
const LAUNCH_DELAY: Duration = Duration::from_secs(30);
/// The flow starts a check at a tick when one is due. This timer stops
/// while the computer sleeps, so the flow uses UTC time (R9).
const TICK: Duration = Duration::from_secs(60);
/// A held offer shows at most this long after busy ends (KTD7).
const BUSY_POLL: Duration = Duration::from_secs(15);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// The check before an install at launch (KTD10).
const LAUNCH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
/// The check at an Install click: the progress already shows (KTD7).
const QUICK_CHECK_TIMEOUT: Duration = LAUNCH_CHECK_TIMEOUT;
/// The least download progress between two state events.
const PROGRESS_STEP: u64 = 256 * 1024;

/// The update manifest: `latest.json` of the latest release (KTD5 of the
/// releases plan).
pub const MANIFEST_URL: &str =
    "https://github.com/twistymaze/longhand/releases/latest/download/latest.json";

/// The update state changed. The update window follows it.
#[derive(Serialize, Type, Debug, Clone, tauri_specta::Event)]
pub struct UpdateStateChanged {
    pub state: UpdateState,
    /// The user asked for the window, so it has the focus and Return
    /// chooses the default button (KTD6).
    pub with_key: bool,
}

enum Message {
    Timer,
    ManualCheck,
    Choice(Choice),
    /// A recording started or stopped, or the poll is due.
    Busy,
    Checked(Result<Option<Box<Update>>, String>),
    Progress {
        downloaded: u64,
        total: Option<u64>,
    },
    Downloaded(Result<(), String>),
    /// A click on the update item of the tray menu (R4).
    TrayOpen,
    /// A Longhand window became the key window, so a waiting notice can
    /// show (R1).
    Activated,
    /// The restart claim failed, because a note is open.
    RestartRefused,
}

/// A failed install at quit or at launch, in `updates.json`. The error shows
/// once (`shown`). Until the next install or download starts, the launch
/// does not try that install again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct InstallFailure {
    version: String,
    error: String,
    shown: bool,
}

impl InstallFailure {
    fn new(version: &str, error: impl Into<String>) -> Self {
        Self {
            version: version.to_string(),
            error: error.into(),
            shown: false,
        }
    }
}

/// True when the launch must not install: a failure names a build newer
/// than the running one. The saved archive can be gone, because a check that
/// fails removes it.
fn launch_skips_install(failure: Option<&InstallFailure>, current: &str) -> bool {
    failure.is_some_and(|f| flow::newer(&f.version, current))
}

/// At the start of the manager: the record to keep. A failure of a build
/// that is not newer than the running one is old, so it goes.
fn failure_at_start(failure: Option<InstallFailure>, current: &str) -> Option<InstallFailure> {
    failure.filter(|f| flow::newer(&f.version, current))
}

/// True when the window now shows the error of a failure that it did not
/// show before, so the record can say that it showed. A launch with no
/// window, or a quit before the error shows, keeps it for the next launch.
fn shows_failure(failure: &InstallFailure, state: &UpdateState, visible: bool) -> bool {
    !failure.shown
        && visible
        && matches!(state, UpdateState::Failed { version, .. } if *version == failure.version)
}

/// True when a step starts an install or a download, so a recorded failure
/// goes: the user chose to try again, or chose another build.
fn starts_install_or_download(before: &UpdateState, after: &UpdateState) -> bool {
    let installs_or_downloads = |s: &UpdateState| {
        matches!(
            s,
            UpdateState::Installing { .. } | UpdateState::Downloading { .. }
        )
    };
    installs_or_downloads(after) && !installs_or_downloads(before)
}

/// True when the update window may be visible (KTD2): the flow wants it,
/// and it shows already, a Longhand window is active, or the user asked.
fn may_show(wanted: bool, shown: bool, asked: bool, active: impl FnOnce() -> bool) -> bool {
    wanted && (shown || asked || active())
}

/// True when the launch installs the saved archive: the manifest names
/// exactly that version (R8, KTD11).
fn installs_at_launch(named: &str, saved: &str) -> bool {
    matches!(
        (semver::Version::parse(named), semver::Version::parse(saved)),
        (Ok(named), Ok(saved)) if named == saved
    )
}

pub struct UpdateManager {
    app: AppHandle,
    flow: Mutex<Flow>,
    /// The `Update` of the last check that found a build. Only a check makes
    /// one, and each download needs it.
    update: Mutex<Option<Update>>,
    /// The `Update` of the build that a quit installs. A later check does
    /// not replace it, so a confirmed install stays (KTD8).
    install_update: Mutex<Option<Update>>,
    /// The recorded install failure, if any.
    failure: Mutex<Option<InstallFailure>>,
    busy: Box<dyn Fn() -> bool + Send + Sync>,
    was_busy: AtomicBool,
    messages: mpsc::UnboundedSender<Message>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<Message>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn updates_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok().map(|p| p.join("updates"))
}

/// The updater public key from `tauri.conf.json` (`plugins.updater.pubkey`).
fn pubkey(app: &AppHandle) -> String {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(|k| k.as_str())
        .unwrap_or_default()
        .to_string()
}

fn current_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

fn read_delays(app: &AppHandle) -> Delays {
    app.store(STORE_PATH)
        .ok()
        .and_then(|store| store.get(DELAYS_KEY))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn write_delays(app: &AppHandle, delays: &Delays) {
    let result = app
        .store(STORE_PATH)
        .map_err(|e| e.to_string())
        .and_then(|store| {
            store.set(
                DELAYS_KEY,
                serde_json::to_value(delays).map_err(|e| e.to_string())?,
            );
            store.save().map_err(|e| e.to_string())
        });
    if let Err(e) = result {
        log::warn!("updates: cannot save the delays: {e}");
    }
}

fn read_failure(app: &AppHandle) -> Option<InstallFailure> {
    app.store(STORE_PATH)
        .ok()
        .and_then(|store| store.get(INSTALL_FAILURE_KEY))
        .and_then(|value| serde_json::from_value(value).ok())
}

/// Write the failure, or remove it with none.
fn write_failure(app: &AppHandle, failure: Option<&InstallFailure>) {
    let result = app
        .store(STORE_PATH)
        .map_err(|e| e.to_string())
        .and_then(|store| {
            match failure {
                Some(f) => store.set(
                    INSTALL_FAILURE_KEY,
                    serde_json::to_value(f).map_err(|e| e.to_string())?,
                ),
                // No record: nothing to save.
                None if !store.delete(INSTALL_FAILURE_KEY) => return Ok(()),
                None => {}
            }
            store.save().map_err(|e| e.to_string())
        });
    if let Err(e) = result {
        log::warn!("updates: cannot save the install failure: {e}");
    }
}

async fn check(app: &AppHandle, timeout: Duration) -> Result<Option<Update>, String> {
    let url = tauri::Url::parse(MANIFEST_URL).map_err(|e| e.to_string())?;
    app.updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())
}

impl UpdateManager {
    /// `busy` is true while a note records or processes (KTD7). It must not
    /// need a lock that a caller of [`recording_changed`] holds.
    pub fn new(app: &AppHandle, busy: impl Fn() -> bool + Send + Sync + 'static) -> Arc<Self> {
        let current = current_version(app);
        let saved = updates_dir(app).and_then(|dir| archive::newest_saved(&dir, &current));
        let stored = read_failure(app);
        let failure = failure_at_start(stored.clone(), &current);
        if failure != stored {
            write_failure(app, failure.as_ref());
        }
        let mut flow = Flow::new(current, read_delays(app), saved);
        if let Some(f) = &failure {
            let error = (!f.shown).then(|| f.error.clone());
            flow.install_failed(f.version.clone(), error);
        }
        let (messages, receiver) = mpsc::unbounded_channel();
        Arc::new(Self {
            app: app.clone(),
            flow: Mutex::new(flow),
            update: Mutex::new(None),
            install_update: Mutex::new(None),
            failure: Mutex::new(failure),
            busy: Box::new(busy),
            was_busy: AtomicBool::new(false),
            messages,
            receiver: Mutex::new(Some(receiver)),
        })
    }

    /// Start the schedule: a tick 30 s after launch, then each minute. The
    /// first tick checks, and the flow decides when the next check is due
    /// (KTD8).
    pub fn start(self: &Arc<Self>) {
        let Some(mut receiver) = lock(&self.receiver).take() else {
            return;
        };
        let manager = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let start = tokio::time::Instant::now() + LAUNCH_DELAY;
            let mut timer = tokio::time::interval_at(start, TICK);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut poll = tokio::time::interval(BUSY_POLL);
            loop {
                let message = tokio::select! {
                    _ = timer.tick() => Message::Timer,
                    _ = poll.tick() => Message::Busy,
                    message = receiver.recv() => match message {
                        Some(message) => message,
                        None => return,
                    },
                };
                // The busy check can lock the capture state, so it runs on a
                // blocking thread, apart from the async workers.
                let worker = Arc::clone(&manager);
                let _ = tauri::async_runtime::spawn_blocking(move || worker.handle(message)).await;
            }
        });
    }

    fn send(&self, message: Message) {
        let _ = self.messages.send(message);
    }

    /// "Check for Updates…" (R11).
    pub fn check_now(&self) {
        self.send(Message::ManualCheck);
    }

    pub fn choose(&self, choice: Choice) {
        self.send(Message::Choice(choice));
    }

    /// The update item of the tray menu (R4).
    pub fn tray_open(&self) {
        self.send(Message::TrayOpen);
    }

    /// The state and whether the window has the focus, read together.
    pub fn snapshot(&self) -> UpdateStateChanged {
        let flow = lock(&self.flow);
        UpdateStateChanged {
            state: flow.state().clone(),
            with_key: flow.user_asked(),
        }
    }

    fn facts(&self, busy: bool) -> Facts {
        Facts {
            now: now(),
            busy,
            automatic_checks: crate::managers::settings::get_settings(&self.app)
                .check_updates_automatically,
            is_release_build: build_info::is_release_build(),
        }
    }

    fn handle(self: &Arc<Self>, message: Message) {
        let busy = (self.busy)();
        let facts = self.facts(busy);
        let mut inputs = Vec::new();
        if self.was_busy.swap(busy, Ordering::SeqCst) != busy {
            inputs.push(Input::Busy(busy));
        }
        // A request for the window brings it to the front with the focus.
        let asked_now = matches!(message, Message::ManualCheck | Message::TrayOpen);
        match message {
            Message::Timer => inputs.push(Input::Timer),
            Message::ManualCheck => inputs.push(Input::ManualCheck),
            Message::TrayOpen => inputs.push(Input::TrayOpen),
            Message::Choice(choice) => inputs.push(Input::Choice(choice)),
            Message::Busy | Message::Activated => {}
            Message::RestartRefused => inputs.push(Input::RestartRefused),
            Message::Checked(result) => {
                let result = result.map(|update| {
                    let version = update.as_ref().map(|u| u.version.clone());
                    if update.is_some() {
                        *lock(&self.update) = update.map(|u| *u);
                    }
                    version
                });
                if let Err(e) = &result {
                    log::info!("updates: the check failed: {e}");
                }
                inputs.push(Input::Checked(result));
            }
            Message::Progress { downloaded, total } => {
                inputs.push(Input::Progress { downloaded, total })
            }
            Message::Downloaded(result) => {
                if let Err(e) = &result {
                    log::warn!("updates: the download failed: {e}");
                }
                inputs.push(Input::Downloaded(result));
            }
        }

        let (before, with_key_before, effects, state, wanted, with_key, install, tray, delays) = {
            let mut flow = lock(&self.flow);
            let before = flow.state().clone();
            let with_key_before = flow.user_asked();
            let effects: Vec<Effect> = inputs
                .into_iter()
                .flat_map(|input| flow.step(input, facts))
                .collect();
            (
                before,
                with_key_before,
                effects,
                flow.state().clone(),
                flow.window_visible(),
                flow.user_asked(),
                flow.install_at_quit().map(str::to_string),
                flow.tray_version().map(str::to_string),
                flow.delays().clone(),
            )
        };
        self.keep_install_update(install.as_deref());
        if starts_install_or_download(&before, &state) && lock(&self.failure).take().is_some() {
            write_failure(&self.app, None);
        }
        // A request on a shown notice changes only with_key, and the page
        // must get it too.
        if state != before || with_key != with_key_before {
            if state != before {
                log::info!("updates: {before:?} -> {state:?}");
            }
            let _ = UpdateStateChanged {
                state: state.clone(),
                with_key,
            }
            .emit(&self.app);
        }
        let shown = self
            .app
            .get_webview_window(crate::window::UPDATE_WINDOW)
            .is_some();
        // The key-window query waits on the main thread, so it runs only
        // when it decides the result.
        let visible = may_show(wanted, shown, with_key, || {
            crate::window::longhand_window_active(&self.app)
        });
        crate::window::show_update_window(&self.app, visible, with_key, asked_now);
        self.note_failure_shown(&state, visible);
        crate::tray::set_update_item(&self.app, tray);
        for effect in effects {
            match effect {
                Effect::Check => self.start_check(CHECK_TIMEOUT),
                Effect::QuickCheck => self.start_check(QUICK_CHECK_TIMEOUT),
                Effect::Download { version } => self.start_download(version),
                Effect::SaveDelays => write_delays(&self.app, &delays),
                // The `RunEvent::Exit` arm stops processing and installs
                // (`install_at_exit`), then tauri starts the new build. A
                // recording can start after the busy check above, so the
                // claim checks again. If the claim fails, the flow holds the
                // build or asks again (KD6).
                Effect::Restart => {
                    if self.app.state::<Arc<CaptureManager>>().claim_restart() {
                        self.app.request_restart();
                    } else {
                        log::info!("updates: a note is open, so the restart waits");
                        self.send(Message::RestartRefused);
                    }
                }
            }
        }
    }

    /// Record that the window showed the install error, so a later launch
    /// does not show it again.
    fn note_failure_shown(&self, state: &UpdateState, visible: bool) {
        let mut failure = lock(&self.failure);
        if let Some(f) = failure
            .as_mut()
            .filter(|f| shows_failure(f, state, visible))
        {
            f.shown = true;
            write_failure(&self.app, Some(f));
        }
    }

    fn start_check(self: &Arc<Self>, timeout: Duration) {
        let manager = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let result = if build_info::is_release_build() {
                check(&manager.app, timeout).await.map(|u| u.map(Box::new))
            } else {
                Ok(None)
            };
            manager.send(Message::Checked(result));
        });
    }

    fn start_download(self: &Arc<Self>, version: String) {
        let manager = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let result = manager.download(&version).await;
            manager.send(Message::Downloaded(result));
        });
    }

    /// Download and verify the archive (the plugin checks the signature),
    /// then save it with its signature (KTD10).
    async fn download(&self, version: &str) -> Result<(), String> {
        let update = lock(&self.update)
            .clone()
            .filter(|u| u.version == version)
            .ok_or_else(|| format!("no update to {version} is known; check again"))?;
        let dir = updates_dir(&self.app).ok_or("the app has no cache folder")?;
        let mut downloaded = 0u64;
        let mut sent = 0u64;
        let bytes = update
            .download(
                |chunk, total| {
                    downloaded += chunk as u64;
                    if downloaded - sent >= PROGRESS_STEP {
                        sent = downloaded;
                        self.send(Message::Progress { downloaded, total });
                    }
                },
                || {},
            )
            .await
            .map_err(|e| e.to_string())?;
        archive::save(&dir, version, &bytes, &update.signature)
    }

    /// Keep the `Update` of the build that a quit installs, taken from the
    /// check that named it. A later check replaces only `update` (KTD8).
    fn keep_install_update(&self, version: Option<&str>) {
        let Some(version) = version else {
            return;
        };
        let mut install = lock(&self.install_update);
        if install.as_ref().is_some_and(|u| u.version == version) {
            return;
        }
        if let Some(update) = lock(&self.update).as_ref().filter(|u| u.version == version) {
            *install = Some(update.clone());
        }
    }

    /// At `RunEvent::Exit`: install the build that the user asked for (R17,
    /// AE7). Processing must already be stopped.
    pub fn install_at_exit(&self) {
        let Some(version) = lock(&self.flow).install_at_quit().map(str::to_string) else {
            return;
        };
        let Some(update) = lock(&self.install_update)
            .clone()
            .filter(|u| u.version == version)
        else {
            log::warn!("updates: {version} waits for install, but no update is known");
            let failure = InstallFailure::new(
                &version,
                "Longhand could not check for the update before it quit.",
            );
            return write_failure(&self.app, Some(&failure));
        };
        let Some(dir) = updates_dir(&self.app) else {
            return;
        };
        let result = archive::load_verified(&dir, &version, &pubkey(&self.app))
            .and_then(|bytes| update.install(bytes).map_err(|e| e.to_string()));
        match result {
            Ok(()) => log::info!("updates: installed {version} at exit"),
            Err(e) => {
                log::error!("updates: cannot install {version} at exit: {e}");
                write_failure(&self.app, Some(&InstallFailure::new(&version, e)));
            }
        }
    }
}

/// A Longhand window became the key window (R1). The caller is the main
/// thread, so this only sends a message.
pub fn window_activated(app: &AppHandle) {
    if let Some(manager) = app.try_state::<Arc<UpdateManager>>() {
        manager.send(Message::Activated);
    }
}

/// A recording started or stopped. The caller can hold the capture state
/// lock, so this only sends a message (AE2).
pub fn recording_changed(app: &AppHandle) {
    if let Some(manager) = app.try_state::<Arc<UpdateManager>>() {
        manager.send(Message::Busy);
    }
}

/// Before the main window opens: install a saved archive that no quit
/// installed (KTD10). This runs only with automatic checks on, and only
/// when a check within 5 s still names exactly that build. On an
/// install, the app restarts and this does not return.
pub fn install_at_launch(app: &AppHandle) {
    if !build_info::is_release_build() {
        return;
    }
    let Some(dir) = updates_dir(app) else {
        return;
    };
    let current = current_version(app);
    let Some(saved) = archive::newest_saved(&dir, &current) else {
        return;
    };
    // The same install failed before. The manager shows the error, and
    // "Try Again" installs (R18).
    if launch_skips_install(read_failure(app).as_ref(), &current) {
        return log::info!("updates: {saved} failed to install before; no install at launch");
    }
    if !crate::managers::settings::get_settings(app).check_updates_automatically {
        return;
    }
    // The launch waits for this check. The client timeout applies to the
    // request only, so the outer timeout also limits the client build and
    // any other step that does not end.
    let checked = tauri::async_runtime::block_on(async {
        tokio::time::timeout(LAUNCH_CHECK_TIMEOUT, check(app, LAUNCH_CHECK_TIMEOUT)).await
    });
    let update = match checked {
        Ok(Ok(Some(update))) => update,
        Ok(Ok(None)) => return,
        Ok(Err(e)) => return log::info!("updates: the launch check failed: {e}"),
        Err(_) => return log::info!("updates: the launch check timed out"),
    };
    // A newer release replaces the saved build: the manager offers it after
    // the launch, and no second update follows this one (R8).
    if !installs_at_launch(&update.version, &saved) {
        return;
    }
    let result = archive::load_verified(&dir, &saved, &pubkey(app))
        .and_then(|bytes| update.install(bytes).map_err(|e| e.to_string()));
    match result {
        Ok(()) => {
            log::info!("updates: installed {saved} at launch; restarting");
            // The new copy must not find this copy's single-instance socket.
            tauri_plugin_single_instance::destroy(app);
            app.restart();
        }
        Err(e) => {
            log::error!("updates: cannot install {saved} at launch: {e}");
            // The manager starts later in this launch and shows the error.
            write_failure(app, Some(&InstallFailure::new(&saved, e)));
        }
    }
}

/// "Check for Updates…" in the app menu and the tray menu (R11, KTD11).
pub fn check_for_updates(app: &AppHandle) {
    if let Some(manager) = app.try_state::<Arc<UpdateManager>>() {
        manager.check_now();
    }
}

/// The update item of the tray menu (R4).
pub fn tray_open(app: &AppHandle) {
    if let Some(manager) = app.try_state::<Arc<UpdateManager>>() {
        manager.tray_open();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_ae4_the_launch_installs_only_the_version_that_the_manifest_names() {
        assert!(installs_at_launch("1.0.4", "1.0.4"));
        assert!(!installs_at_launch("1.0.5", "1.0.4"), "a newer release");
        assert!(!installs_at_launch("1.0.3", "1.0.4"), "an older release");
        assert!(!installs_at_launch("not a version", "1.0.4"));
    }

    #[test]
    fn the_update_window_shows_when_wanted_and_shown_active_or_asked() {
        assert!(
            may_show(true, false, false, || true),
            "a Longhand window is key"
        );
        assert!(
            !may_show(true, false, false, || false),
            "the notice waits for a Longhand window"
        );
        assert!(may_show(true, true, false, || false), "it shows already");
        assert!(may_show(true, false, true, || false), "the user asked");
        assert!(
            !may_show(false, true, true, || true),
            "the flow does not want it"
        );
    }

    #[test]
    fn the_manifest_is_the_latest_json_of_the_latest_release_of_the_source_repository() {
        assert_eq!(
            MANIFEST_URL,
            "https://github.com/twistymaze/longhand/releases/latest/download/latest.json"
        );
        assert!(tauri::Url::parse(MANIFEST_URL).is_ok());
    }

    #[test]
    fn a_failed_install_shows_once_and_stops_the_install_at_launch_until_the_next_install() {
        // A failure at quit.
        let failure = Some(InstallFailure::new("1.0.4", "read-only"));
        assert!(launch_skips_install(failure.as_ref(), "1.0.3"));
        // The next launch keeps it, and the error shows only when the window
        // shows it.
        let mut failure = failure_at_start(failure, "1.0.3").unwrap();
        let failed = UpdateState::Failed {
            version: "1.0.4".into(),
            error: "read-only".into(),
        };
        assert!(!shows_failure(&failure, &failed, false), "no window");
        assert!(
            !shows_failure(&failure, &UpdateState::Idle, true),
            "another state"
        );
        assert!(shows_failure(&failure, &failed, true));
        failure.shown = true;
        assert!(!shows_failure(&failure, &failed, true), "once");
        // The launch after that still does not install.
        let failure = failure_at_start(Some(failure), "1.0.3");
        assert!(launch_skips_install(failure.as_ref(), "1.0.3"));
        assert!(failure.is_some(), "the record stays until the next install");
        // "Try Again" or "Relaunch Now" removes it.
        let downloading = UpdateState::Downloading {
            version: "1.0.4".into(),
            downloaded: 0,
            total: None,
        };
        let ready = UpdateState::Ready {
            version: "1.0.4".into(),
        };
        let installing = UpdateState::Installing {
            version: "1.0.4".into(),
        };
        assert!(starts_install_or_download(&failed, &downloading));
        assert!(starts_install_or_download(&ready, &installing));
        assert!(
            !starts_install_or_download(&downloading, &downloading),
            "progress"
        );
        assert!(
            !starts_install_or_download(&installing, &ready),
            "a refused restart"
        );
    }

    #[test]
    fn an_install_failure_of_an_installed_build_goes() {
        for current in ["1.0.4", "1.0.5"] {
            let failure = Some(InstallFailure::new("1.0.4", "read-only"));
            assert!(!launch_skips_install(failure.as_ref(), current));
            assert_eq!(failure_at_start(failure, current), None);
        }
        assert_eq!(failure_at_start(None, "1.0.3"), None);
        assert!(!launch_skips_install(None, "1.0.3"));
    }
}
