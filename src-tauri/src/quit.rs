//! Save the notes before a quit.
//!
//! The note editor waits a short time before it sends a change. A quit that
//! ends the process at once loses the change that waits. Each quit path
//! therefore calls `save_notes_then`: it asks the main window to send each
//! waiting change, and it waits for the answer on the async runtime, for at
//! most `QUIT_SAVE_LIMIT`. Then it runs the quit. The main thread does not
//! wait, so the webview and its commands continue to run.
//!
//! `RunEvent::ExitRequested` + `prevent_exit()` is not used
//! (tauri-apps/tauri#13511, see `window.rs`).
//!
//! On macOS, the app menu Quit, Cmd+Q, the Dock Quit and a logout send
//! `terminate:` to the application. `applicationShouldTerminate:`, which
//! this module adds to tao's app delegate, starts the save and answers
//! `NSTerminateLater`. AppKit ends the app when the save replies.
//!
//! The quick save at a quit is anarlog's (commit 0a9b725, #8238); see
//! src-tauri/PROVENANCE.md.

use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;

use crate::window::MAIN_WINDOW;

/// The longest time that a quit waits for the main window to save the
/// notes. A webview that does not answer must not stop the quit.
pub const QUIT_SAVE_LIMIT: Duration = Duration::from_secs(3);

/// The app quits. The main window sends each waiting note change, waits for
/// the saves, and then calls `notes_saved_for_quit`.
#[derive(serde::Serialize, Clone, specta::Type, tauri_specta::Event)]
pub struct QuitRequested;

/// The state of the quit: whether one started, and the sender of the main
/// window's answer while the quit waits for it.
struct Quit(Mutex<QuitState>);

struct QuitState {
    started: bool,
    answer: Option<oneshot::Sender<()>>,
}

impl Quit {
    const fn new() -> Self {
        Self(Mutex::new(QuitState {
            started: false,
            answer: None,
        }))
    }

    /// Start the quit. Returns the receiver of the answer for the first
    /// call, and `None` when a quit already started.
    fn begin(&self) -> Option<oneshot::Receiver<()>> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.started {
            return None;
        }
        state.started = true;
        let (sender, receiver) = oneshot::channel();
        state.answer = Some(sender);
        Some(receiver)
    }

    /// True when a quit started.
    fn started(&self) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).started
    }

    /// The main window answered: the quit continues.
    fn answer(&self) {
        let sender = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .answer
            .take();
        if let Some(sender) = sender {
            let _ = sender.send(());
        }
    }
}

static QUIT: Quit = Quit::new();

/// How the wait for the main window ended.
#[derive(Debug, PartialEq, Eq)]
enum Waited {
    Answered,
    Limit,
}

/// Wait for the answer, for at most `limit`. An answer sender that is gone
/// counts as an answer: nothing can answer later.
async fn wait_for_answer(answer: oneshot::Receiver<()>, limit: Duration) -> Waited {
    match tokio::time::timeout(limit, answer).await {
        Ok(_) => Waited::Answered,
        Err(_) => Waited::Limit,
    }
}

/// Ask the main window to save the notes, then run `then`. `then` runs after
/// the answer or after `QUIT_SAVE_LIMIT`. When a quit already started, or
/// when no main window can answer, `then` runs at once.
pub fn save_notes_then<F>(app: &AppHandle, then: F)
where
    F: FnOnce(&AppHandle) + Send + 'static,
{
    let Some(answer) = QUIT.begin() else {
        log::info!("quit: a second quit request; the app quits now");
        then(app);
        return;
    };
    if app.get_webview_window(MAIN_WINDOW).is_none() {
        then(app);
        return;
    }
    if let Err(e) = tauri_specta::Event::emit_to(&QuitRequested, app, MAIN_WINDOW) {
        log::error!("quit: the save request could not be sent: {e}");
        then(app);
        return;
    }
    log::info!("quit: the main window saves the notes");
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match wait_for_answer(answer, QUIT_SAVE_LIMIT).await {
            Waited::Answered => log::info!("quit: the notes save is complete"),
            Waited::Limit => log::warn!(
                "quit: the notes save did not complete in {} s; the app quits",
                QUIT_SAVE_LIMIT.as_secs()
            ),
        }
        then(&app);
    });
}

/// Save the notes, then quit.
pub fn quit(app: &AppHandle) {
    save_notes_then(app, |app| app.exit(0));
}

/// The main window sent each waiting note change and the saves completed.
#[tauri::command]
#[specta::specta]
pub async fn notes_saved_for_quit() {
    QUIT.answer();
}

#[cfg(target_os = "macos")]
pub use terminate::install_terminate_handler;

#[cfg(target_os = "macos")]
mod terminate {
    use std::ffi::CString;
    use std::sync::OnceLock;

    use objc2::encode::Encode;
    use objc2::ffi::{NSUInteger, class_addMethod};
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::{class, msg_send, sel};
    use tauri::AppHandle;

    /// `NSApplicationTerminateReply` values.
    const TERMINATE_NOW: NSUInteger = 1;
    const TERMINATE_LATER: NSUInteger = 2;

    static APP: OnceLock<AppHandle> = OnceLock::new();

    /// Add `applicationShouldTerminate:` to the class of the application's
    /// delegate (tao's), so that each `terminate:` saves the notes first.
    /// Call it on the main thread, once.
    pub fn install_terminate_handler(app: &AppHandle) {
        if APP.set(app.clone()).is_err() {
            return;
        }
        // SAFETY: this runs on the main thread. `delegate` is the object
        // that the application keeps as its delegate.
        unsafe {
            let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let delegate: *mut AnyObject = msg_send![ns_app, delegate];
            if delegate.is_null() {
                log::warn!("quit: the application has no delegate; a quit does not save the notes");
                return;
            }
            let class: *const AnyClass = msg_send![delegate, class];
            if !add_should_terminate(&*class) {
                log::warn!("quit: the app delegate already answers applicationShouldTerminate:");
                return;
            }
            // tao set the delegate before the method existed, and AppKit can
            // read a delegate's methods when it gets the delegate. Set it
            // again, so AppKit reads the new method.
            let _: () = msg_send![ns_app, setDelegate: std::ptr::null_mut::<AnyObject>()];
            let _: () = msg_send![ns_app, setDelegate: delegate];
        }
    }

    /// Add `applicationShouldTerminate:` to `class`. False when `class`
    /// already has the method; the method then does not change.
    pub(super) fn add_should_terminate(class: &AnyClass) -> bool {
        let types = CString::new(format!(
            "{}{}{}{}",
            NSUInteger::ENCODING,
            <*mut AnyObject>::ENCODING,
            Sel::ENCODING,
            <*mut AnyObject>::ENCODING,
        ))
        .expect("an encoding has no NUL byte");
        type ShouldTerminate =
            unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> NSUInteger;
        let imp: ShouldTerminate = should_terminate;
        // SAFETY: `imp` has the signature of `applicationShouldTerminate:`,
        // and `types` is its encoding. The runtime copies `types`.
        unsafe {
            class_addMethod(
                (class as *const AnyClass).cast_mut(),
                sel!(applicationShouldTerminate:),
                std::mem::transmute::<ShouldTerminate, Imp>(imp),
                types.as_ptr(),
            )
        }
        .as_bool()
    }

    /// `applicationShouldTerminate:`. The first request saves the notes and
    /// answers later. A request while a quit runs, or with no app, quits
    /// now. The function cannot panic: a panic here would unwind into
    /// AppKit.
    unsafe extern "C-unwind" fn should_terminate(
        _this: *mut AnyObject,
        _cmd: Sel,
        _sender: *mut AnyObject,
    ) -> NSUInteger {
        let Some(app) = APP.get() else {
            return TERMINATE_NOW;
        };
        if super::QUIT.started() {
            return TERMINATE_NOW;
        }
        super::save_notes_then(app, |app| {
            let queued = app.run_on_main_thread(|| {
                // SAFETY: this runs on the main thread, after AppKit got
                // `NSTerminateLater`.
                unsafe {
                    let ns_app: *mut AnyObject =
                        msg_send![class!(NSApplication), sharedApplication];
                    let _: () = msg_send![ns_app, replyToApplicationShouldTerminate: Bool::YES];
                }
            });
            if let Err(e) = queued {
                log::error!("quit: the terminate reply could not be queued: {e}");
                app.exit(0);
            }
        });
        TERMINATE_LATER
    }

    #[cfg(test)]
    mod tests {
        use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, NSObject};
        use objc2::{ClassType, msg_send, sel};

        use super::*;

        #[test]
        fn the_method_is_added_once() {
            let class: &AnyClass =
                ClassBuilder::new(c"LonghandQuitTestDelegate", NSObject::class())
                    .expect("the test class name is free")
                    .register();
            assert!(!class.responds_to(sel!(applicationShouldTerminate:)));
            assert!(add_should_terminate(class));
            assert!(class.responds_to(sel!(applicationShouldTerminate:)));
            assert!(!add_should_terminate(class));
        }

        #[test]
        fn with_no_app_the_method_quits_now() {
            let class: &AnyClass =
                ClassBuilder::new(c"LonghandQuitTestDelegateNoApp", NSObject::class())
                    .expect("the test class name is free")
                    .register();
            assert!(add_should_terminate(class));
            // SAFETY: `delegate` is a new instance of a registered class,
            // and the method takes one object argument.
            let reply: NSUInteger = unsafe {
                let delegate: *mut AnyObject = msg_send![class, new];
                let reply = msg_send![delegate, applicationShouldTerminate: std::ptr::null_mut::<AnyObject>()];
                let _: () = msg_send![delegate, release];
                reply
            };
            assert_eq!(reply, TERMINATE_NOW);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_request_starts_a_quit() {
        let quit = Quit::new();
        assert!(!quit.started());
        assert!(quit.begin().is_some());
        assert!(quit.started());
        assert!(quit.begin().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn the_answer_ends_the_wait() {
        let quit = Quit::new();
        let answer = quit.begin().expect("the first request");
        quit.answer();
        assert_eq!(
            wait_for_answer(answer, QUIT_SAVE_LIMIT).await,
            Waited::Answered
        );
    }

    #[tokio::test(start_paused = true)]
    async fn with_no_answer_the_wait_ends_at_the_limit() {
        let quit = Quit::new();
        let answer = quit.begin().expect("the first request");
        let start = tokio::time::Instant::now();
        assert_eq!(
            wait_for_answer(answer, QUIT_SAVE_LIMIT).await,
            Waited::Limit
        );
        assert_eq!(start.elapsed(), QUIT_SAVE_LIMIT);
        // An answer after the limit changes nothing.
        quit.answer();
    }

    #[test]
    fn an_answer_with_no_quit_does_nothing() {
        let quit = Quit::new();
        quit.answer();
        assert!(!quit.started());
    }
}
