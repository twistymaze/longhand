//! Files that another app promises in a drag (macOS).
//!
//! Some apps, for example Voice Memos, do not put a file on the drag
//! pasteboard. They put a file promise: the app writes the file only when
//! the drop target asks for it. wry's drop handler reads only file URLs, so
//! a drop of a promise gives Home no paths, and nothing imports.
//!
//! This module adds to wry's `WryWebView` class. When a drag over the main
//! window has promises and no file URLs, the drag start sends
//! [`PromisedFiles::Over`], so Home shows the drop target. The drop asks the
//! other app to write the files into a new folder in the app cache, then
//! sends [`PromisedFiles::Received`] with their paths. Home then opens the
//! import sheet, as for a drop of files. wry's own handlers run first in
//! each case, so a drop of files does not change.
//!
//! The import copies a file later, in its worker, so a received file must
//! stay. Each start removes the folders that are older than
//! [`KEEP_RECEIVED`].

use serde::Serialize;
use specta::Type;

/// A drag of promised files over the main window.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Type, tauri_specta::Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PromisedFiles {
    /// A drag with promised files and no file URLs entered the window.
    Over,
    /// The other app wrote the promised files of a drop. `paths` are the
    /// files that it wrote. `failed` counts the files that it did not
    /// write.
    Received { paths: Vec<String>, failed: u32 },
}

#[cfg(target_os = "macos")]
pub use native::accept_promised_files;

/// The age after which a start removes a folder of received files.
#[cfg(target_os = "macos")]
const KEEP_RECEIVED: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// The folder in the app cache that holds the received files.
#[cfg(target_os = "macos")]
const RECEIVED_DIR: &str = "received-files";

/// The files of one drop: the event goes when the last file is written or
/// failed.
#[cfg(target_os = "macos")]
#[derive(Debug)]
struct Batch {
    left: usize,
    paths: Vec<String>,
    failed: u32,
}

#[cfg(target_os = "macos")]
impl Batch {
    fn new(files: usize) -> Self {
        Self {
            left: files,
            paths: Vec::new(),
            failed: 0,
        }
    }

    /// One file is written (`Some` with its path) or failed (`None`).
    /// Returns the event when it was the last file of the drop.
    fn file_done(&mut self, path: Option<String>) -> Option<PromisedFiles> {
        match path {
            Some(path) => self.paths.push(path),
            None => self.failed += 1,
        }
        self.left = self.left.saturating_sub(1);
        (self.left == 0).then(|| PromisedFiles::Received {
            paths: std::mem::take(&mut self.paths),
            failed: self.failed,
        })
    }
}

/// Remove each folder in `dir` whose last change is older than `max_age`
/// at `now`. A folder that cannot be read or removed stays.
#[cfg(target_os = "macos")]
fn remove_old_folders(
    dir: &std::path::Path,
    now: std::time::SystemTime,
    max_age: std::time::Duration,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified| now.duration_since(modified).is_ok_and(|age| age > max_age));
        if old
            && entry.file_type().is_ok_and(|t| t.is_dir())
            && let Err(e) = std::fs::remove_dir_all(entry.path())
        {
            log::warn!(
                "cannot remove the received files {}: {e}",
                entry.path().display()
            );
        }
    }
}

/// The class that has the method `sel` itself: `class` or one of its
/// superclasses below WKWebView. `None` when WKWebView or a class above it
/// has the method, because a change there would change each webview class.
#[cfg(target_os = "macos")]
fn method_owner(class: &AnyClass, sel: Sel) -> Option<&AnyClass> {
    let mut current = Some(class);
    while let Some(class) = current {
        if class.name() == c"WKWebView" {
            return None;
        }
        if class.instance_methods().iter().any(|m| m.name() == sel) {
            return Some(class);
        }
        current = class.superclass();
    }
    None
}

#[cfg(target_os = "macos")]
use objc2::runtime::{AnyClass, Sel};

#[cfg(target_os = "macos")]
mod native {
    use std::ffi::c_void;
    use std::panic::AssertUnwindSafe;
    use std::path::PathBuf;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicPtr, Ordering};
    use std::sync::{Arc, Mutex, OnceLock, PoisonError};

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, ProtocolObject, Sel};
    use objc2::{ClassType, msg_send, sel};
    #[allow(deprecated)]
    use objc2_app_kit::NSFilenamesPboardType;
    use objc2_app_kit::{NSDragOperation, NSDraggingInfo, NSFilePromiseReceiver};
    use objc2_foundation::{NSArray, NSDictionary, NSError, NSOperationQueue, NSString, NSURL};
    use tauri::{AppHandle, Manager, WebviewWindow};
    use tauri_specta::Event;

    use super::{Batch, KEEP_RECEIVED, PromisedFiles, RECEIVED_DIR, remove_old_folders};
    use crate::window::MAIN_WINDOW;

    type Entered =
        unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> NSDragOperation;
    type Perform = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool;

    /// wry's `draggingEntered:` and `performDragOperation:`.
    static ENTERED: OnceLock<Imp> = OnceLock::new();
    static PERFORM: OnceLock<Imp> = OnceLock::new();
    /// The WKWebView of the main window. Only a drop on it receives files.
    static MAIN_VIEW: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    static APP: OnceLock<AppHandle> = OnceLock::new();

    thread_local! {
        /// The queue of the reader blocks. AppKit calls the drop methods on
        /// the main thread, so one queue stays for the life of the app.
        static QUEUE: Retained<NSOperationQueue> = NSOperationQueue::new();
    }

    /// Accept promised files on the main window `window`. Call it once,
    /// after the build of the main window.
    pub fn accept_promised_files(app: &AppHandle, window: &WebviewWindow) {
        let _ = APP.set(app.clone());
        if let Ok(cache) = app.path().app_cache_dir() {
            let dir = cache.join(RECEIVED_DIR);
            std::thread::spawn(move || {
                remove_old_folders(&dir, std::time::SystemTime::now(), KEEP_RECEIVED)
            });
        }
        let added = window.with_webview(|webview| {
            let view = webview.inner();
            MAIN_VIEW.store(view, Ordering::SeqCst);
            // SAFETY: `view` is the live WKWebView of the main window.
            let class = unsafe { &*view.cast::<AnyObject>() }.class();
            add_methods(class);
        });
        if let Err(e) = added {
            log::warn!("cannot accept promised files on the main window: {e}");
        }
    }

    /// Put this module's drag methods on `class`, once.
    fn add_methods(class: &AnyClass) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let entered: Entered = entered;
            let perform: Perform = perform;
            // SAFETY: each function has the signature of the method that
            // it replaces.
            unsafe {
                replace(
                    class,
                    sel!(draggingEntered:),
                    std::mem::transmute::<Entered, Imp>(entered),
                    &ENTERED,
                );
                replace(
                    class,
                    sel!(performDragOperation:),
                    std::mem::transmute::<Perform, Imp>(perform),
                    &PERFORM,
                );
            }
        });
    }

    /// Replace the method `sel` of wry's class with `imp`, and keep the old
    /// implementation in `slot`. `class` is the class of the webview
    /// object. macOS can put a key-value observing subclass on top of wry's
    /// class, so the search goes up from `class` to the class that has the
    /// method itself.
    ///
    /// # Safety
    ///
    /// `imp` has the signature of `sel`.
    unsafe fn replace(class: &AnyClass, sel: Sel, imp: Imp, slot: &OnceLock<Imp>) {
        let Some(owner) = super::method_owner(class, sel) else {
            log::warn!(
                "{} has no {sel}; promised files are not accepted",
                class.name().to_string_lossy()
            );
            return;
        };
        let methods = owner.instance_methods();
        let Some(method) = methods.iter().find(|m| m.name() == sel) else {
            return;
        };
        if slot.set(method.implementation()).is_ok() {
            // SAFETY: the caller gives an `imp` with the signature of `sel`,
            // and `slot` holds the old implementation before the change.
            unsafe { method.set_implementation(imp) };
            log::info!(
                "promised files: {sel} of {} is replaced",
                owner.name().to_string_lossy()
            );
        }
    }

    fn is_main(this: *mut AnyObject) -> bool {
        !this.is_null() && this.cast::<c_void>() == MAIN_VIEW.load(Ordering::SeqCst)
    }

    /// `draggingEntered:`: wry's method, then [`PromisedFiles::Over`] for a
    /// drag of promised files. The function cannot panic: a panic here
    /// would unwind into AppKit.
    unsafe extern "C-unwind" fn entered(
        this: *mut AnyObject,
        cmd: Sel,
        info: *mut AnyObject,
    ) -> NSDragOperation {
        let Some(original) = ENTERED.get() else {
            return NSDragOperation::None;
        };
        // SAFETY: `original` is wry's `draggingEntered:`.
        let original = unsafe { std::mem::transmute::<Imp, Entered>(*original) };
        // SAFETY: the arguments are the ones that AppKit gave.
        let operation = unsafe { original(this, cmd, info) };
        if is_main(this) {
            guarded("the drag start", || {
                // SAFETY: AppKit gives an `NSDraggingInfo`.
                unsafe { promises_only(info) }?;
                let app = APP.get()?;
                if let Err(e) = PromisedFiles::Over.emit_to(app, MAIN_WINDOW) {
                    log::warn!("cannot send the promised files drag: {e}");
                }
                Some(())
            });
        }
        operation
    }

    /// Run `work`, a step of this module in an AppKit callback. A Rust
    /// panic or an Objective-C exception in `work` gives `None` and a log
    /// line: the drop then goes on as wry handles it, and the app does not
    /// stop.
    fn guarded<R>(step: &str, work: impl FnOnce() -> Option<R>) -> Option<R> {
        let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
            objc2::exception::catch(AssertUnwindSafe(work))
        }));
        match caught {
            Ok(Ok(value)) => value,
            Ok(Err(exception)) => {
                log::error!("promised files: an Objective-C exception in {step}: {exception:?}");
                None
            }
            Err(panic) => {
                let message = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_default();
                log::error!("promised files: a panic in {step}: {message}");
                None
            }
        }
    }

    /// `performDragOperation:`: wry's method, then the receive of the
    /// promised files. The function cannot panic.
    unsafe extern "C-unwind" fn perform(
        this: *mut AnyObject,
        cmd: Sel,
        info: *mut AnyObject,
    ) -> Bool {
        let Some(original) = PERFORM.get() else {
            return Bool::NO;
        };
        // SAFETY: `original` is wry's `performDragOperation:`.
        let original = unsafe { std::mem::transmute::<Imp, Perform>(*original) };
        // SAFETY: the arguments are the ones that AppKit gave.
        let done = unsafe { original(this, cmd, info) };
        if !is_main(this) {
            return done;
        }
        let received = guarded("the drop", || {
            // SAFETY: AppKit gives an `NSDraggingInfo`.
            let receivers = unsafe { promises_only(info) }?;
            receive(&receivers);
            Some(())
        });
        if received.is_some() { Bool::YES } else { done }
    }

    /// The promise receivers of the drag `info`, or `None` when the drag
    /// has file URLs (wry gives those) or no promises.
    ///
    /// # Safety
    ///
    /// `info` is null or an object that conforms to `NSDraggingInfo`.
    unsafe fn promises_only(info: *mut AnyObject) -> Option<Vec<Retained<NSFilePromiseReceiver>>> {
        // SAFETY: AppKit gives an `NSDraggingInfo` to the drag methods.
        let info = unsafe { info.cast::<ProtocolObject<dyn NSDraggingInfo>>().as_ref() }?;
        let pasteboard = info.draggingPasteboard();
        let url_class = NSArray::from_slice(&[NSURL::class()]);
        // SAFETY: the class list holds a class that reads from a pasteboard.
        let urls = unsafe { pasteboard.readObjectsForClasses_options(&url_class, None) };
        let has_file_url = urls.is_some_and(|items| {
            items.iter().any(|item| {
                item.downcast_ref::<NSURL>()
                    .is_some_and(|url| url.isFileURL())
            })
        });
        #[allow(deprecated)]
        // SAFETY: `NSFilenamesPboardType` is an AppKit constant.
        let has_file_names = pasteboard
            .availableTypeFromArray(&NSArray::from_slice(&[unsafe { NSFilenamesPboardType }]))
            .is_some();
        if has_file_url || has_file_names {
            return None;
        }
        let promise_class = NSArray::from_slice(&[NSFilePromiseReceiver::class()]);
        // SAFETY: as above.
        let items = unsafe { pasteboard.readObjectsForClasses_options(&promise_class, None) }?;
        let receivers: Vec<_> = items
            .iter()
            .filter_map(|item| item.downcast::<NSFilePromiseReceiver>().ok())
            .collect();
        (!receivers.is_empty()).then_some(receivers)
    }

    /// Ask the other app to write the files of `receivers` into a new
    /// folder. The reader blocks send [`PromisedFiles::Received`] after the
    /// last file.
    fn receive(receivers: &[Retained<NSFilePromiseReceiver>]) {
        let Some(app) = APP.get() else {
            return;
        };
        let files: usize = receivers.iter().map(|r| promised_count(r)).sum();
        let batch = Arc::new(Mutex::new(Batch::new(files)));
        let folder = match new_folder(app) {
            Ok(folder) => folder,
            Err(e) => {
                log::error!("cannot make a folder for the dropped files: {e}");
                for _ in 0..files {
                    finish(app, &batch, None);
                }
                return;
            }
        };
        let destination = NSURL::fileURLWithPath_isDirectory(
            &NSString::from_str(&folder.to_string_lossy()),
            true,
        );
        let options = NSDictionary::new();
        for receiver in receivers {
            let app = app.clone();
            let batch = Arc::clone(&batch);
            // The block runs on the queue's thread: a panic must not unwind
            // into Foundation, and each call must count its file.
            let reader = RcBlock::new(move |url: NonNull<NSURL>, error: *mut NSError| {
                let path = guarded("a received file", || {
                    if error.is_null() {
                        // SAFETY: AppKit gives the URL of the written file.
                        unsafe { url.as_ref() }.path().map(|p| p.to_string())
                    } else {
                        // SAFETY: a non-null `error` is the NSError of the
                        // file.
                        let error = unsafe { &*error };
                        log::warn!(
                            "the other app did not give a dropped file: {}",
                            error.localizedDescription()
                        );
                        None
                    }
                });
                guarded("the received files", || {
                    finish(&app, &batch, path);
                    Some(())
                });
            });
            QUEUE.with(|queue| {
                // SAFETY: `options` is an empty dictionary, and `queue` is a
                // queue that stays for the life of the app.
                unsafe {
                    receiver.receivePromisedFilesAtDestination_options_operationQueue_reader(
                        &destination,
                        &options,
                        queue,
                        &reader,
                    )
                }
            });
        }
    }

    /// The number of files that `receiver` promises: one reader block call
    /// comes for each. `fileNames` is nil until the receive starts, and
    /// objc2's binding panics on nil, so the count comes from `fileTypes`,
    /// read with a call that accepts nil. At least one.
    fn promised_count(receiver: &NSFilePromiseReceiver) -> usize {
        // SAFETY: `fileTypes` takes no arguments and returns an NSArray or
        // nil.
        let types: Option<Retained<NSArray>> = unsafe { msg_send![receiver, fileTypes] };
        types.map_or(1, |types| types.count().max(1))
    }

    /// One file of `batch` is written or failed; after the last file, send
    /// the event to the main window.
    fn finish(app: &AppHandle, batch: &Mutex<Batch>, path: Option<String>) {
        let event = batch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .file_done(path);
        if let Some(event) = event
            && let Err(e) = event.emit_to(app, MAIN_WINDOW)
        {
            log::warn!("cannot send the received files: {e}");
        }
    }

    /// A new folder for the files of one drop.
    fn new_folder(app: &AppHandle) -> std::io::Result<PathBuf> {
        let cache = app.path().app_cache_dir().map_err(std::io::Error::other)?;
        let folder = cache
            .join(RECEIVED_DIR)
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&folder)?;
        Ok(folder)
    }

    #[cfg(test)]
    mod tests {
        use objc2_foundation::{NSException, NSString};

        use super::guarded;

        #[test]
        fn a_step_with_no_failure_gives_its_value() {
            assert_eq!(guarded("a test", || Some(3)), Some(3));
        }

        #[test]
        fn a_panic_gives_none() {
            assert_eq!(guarded::<()>("a test", || panic!("test panic")), None);
        }

        #[test]
        fn an_objective_c_exception_gives_none() {
            let exception = NSException::new(&NSString::from_str("TestException"), None, None)
                .expect("an exception");
            assert_eq!(guarded::<()>("a test", || exception.raise()), None);
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    #[test]
    fn the_event_goes_after_the_last_file() {
        let mut batch = Batch::new(3);
        assert_eq!(batch.file_done(Some("/a.m4a".into())), None);
        assert_eq!(batch.file_done(None), None);
        assert_eq!(
            batch.file_done(Some("/b.m4a".into())),
            Some(PromisedFiles::Received {
                paths: vec!["/a.m4a".into(), "/b.m4a".into()],
                failed: 1,
            })
        );
    }

    #[test]
    fn a_drop_where_each_file_failed_sends_no_paths() {
        let mut batch = Batch::new(1);
        assert_eq!(
            batch.file_done(None),
            Some(PromisedFiles::Received {
                paths: Vec::new(),
                failed: 1,
            })
        );
    }

    // macOS can put a key-value observing subclass with no drag methods on
    // top of wry's class.
    #[test]
    fn the_method_owner_is_found_above_a_subclass() {
        use objc2::ClassType;
        use objc2::runtime::{AnyObject, ClassBuilder, NSObject};
        use objc2::sel;

        extern "C-unwind" fn method(_this: *mut AnyObject, _cmd: Sel) {}
        let mut builder =
            ClassBuilder::new(c"LonghandPromiseTestOwner", NSObject::class()).expect("free name");
        // SAFETY: the signature matches a method with no arguments.
        unsafe {
            builder.add_method(
                sel!(longhandPromiseTest),
                method as extern "C-unwind" fn(*mut AnyObject, Sel),
            )
        };
        let owner = builder.register();
        let observer = ClassBuilder::new(c"LonghandPromiseTestObserver", owner)
            .expect("free name")
            .register();

        assert_eq!(
            method_owner(observer, sel!(longhandPromiseTest)).map(|c| c.name()),
            Some(c"LonghandPromiseTestOwner")
        );
        assert!(method_owner(observer, sel!(longhandPromiseTestMissing)).is_none());
    }

    #[test]
    fn only_folders_older_than_the_limit_are_removed() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir(dir.path().join("drop")).expect("folder");
        std::fs::write(dir.path().join("drop/memo.m4a"), b"audio").expect("file");
        std::fs::write(dir.path().join("file.m4a"), b"audio").expect("file");
        let day = Duration::from_secs(24 * 60 * 60);

        remove_old_folders(dir.path(), SystemTime::now(), day);
        assert!(dir.path().join("drop/memo.m4a").exists());

        remove_old_folders(dir.path(), SystemTime::now() + 2 * day, day);
        assert!(!dir.path().join("drop").exists());
        // A file at the top level is not a drop folder.
        assert!(dir.path().join("file.m4a").exists());
    }
}
