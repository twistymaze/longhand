//! The library watcher (U6, KTD7, KTD8): while Longhand runs, a change that
//! a sync makes in a retained Destination reaches the store and the UI.
//!
//! Each retained Destination that is present gets one recursive watch with
//! a 900 ms debounce, as anarlog's notify plugin (`plugins/notify/src/ext.rs`)
//! watches its vault. An event marks only its top-level note folder for a
//! targeted scan. A rescan event, an event with no usable path and an
//! unpaired rename run a full scan of that Destination. A full scan of every
//! Destination runs each minute, because FSEvents can drop events and File
//! Provider folders do not always report their changes. That scan also adds
//! a watch for a Destination that is present again and drops the watch of
//! one that went away.
//!
//! One thread owns the watches, the timers and the [`Reconciler`] with its
//! pending deletes, and runs every reconcile off the UI thread. Each
//! reconcile that changes rows emits [`LibraryChanged`] (KTD10). Longhand's
//! own writes cause scans too, but the refresh changes nothing for them
//! (KTD3), so they emit nothing.
//!
//! An event on the shared settings file or in the prompts folder of the
//! storage folder (`.longhand/settings.json`, `.longhand/prompts/`) starts
//! no note scan. It asks the shared settings manager to read the shared
//! files again. The full scan each minute asks for that read too, because a
//! sync can deliver a change with no event (KTD13).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use library::reconcile::{Changes, Reconciler, Target};
use notify::event::{EventKind, ModifyKind, RenameMode};
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use serde::Serialize;
use shared_settings::file::{LONGHAND_FOLDER, SETTINGS_FILE};
use shared_settings::prompts::PROMPTS_FOLDER;
use specta::Type;

/// The rows that a library reconcile changed (KTD10). `changed` holds
/// adopted notes, refreshed notes and notes in a new folder. `removed`
/// holds notes whose folder is gone.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Type, tauri_specta::Event)]
pub struct LibraryChanged {
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

impl From<Changes> for LibraryChanged {
    fn from(changes: Changes) -> Self {
        Self {
            changed: changes.changed,
            removed: changes.removed,
        }
    }
}

/// The times that the watcher uses.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// The debounce of each watch (anarlog's `DEBOUNCE_DELAY_MS`).
    pub debounce: Duration,
    /// The time between two full scans of every Destination.
    pub full_scan_every: Duration,
    /// The shortest time between two scans for a pending delete. A row
    /// that is busy stays pending after its wait, and it must not start a
    /// scan in each loop.
    pub retry_wait: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(900),
            full_scan_every: Duration::from_secs(60),
            retry_wait: Duration::from_secs(5),
        }
    }
}

/// How long [`LibraryWatcher::stop`] waits for a scan in progress.
const STOP_WAIT: Duration = Duration::from_secs(2);

/// Scan one target and apply it to the store.
pub type ReconcileFn =
    Box<dyn FnMut(&Target, &mut Reconciler) -> Result<Changes, String> + Send + 'static>;
/// Send one [`LibraryChanged`] to the UI.
pub type EmitFn = Box<dyn Fn(LibraryChanged) + Send + 'static>;
/// Read the shared settings and the prompts of the storage folder again
/// (KTD13).
pub type SharedFn = Box<dyn FnMut() + Send + 'static>;

/// Whether `relative`, a path in the storage folder, is a shared settings
/// path: the `.longhand/` folder itself, its `settings.json`, or a path in
/// its `prompts/` folder. The library marker and other `.longhand/` paths
/// are not.
fn is_shared_path(relative: &Path) -> bool {
    let mut components = relative.components();
    if components.next() != Some(Component::Normal(LONGHAND_FOLDER.as_ref())) {
        return false;
    }
    match components.next() {
        None => true,
        Some(Component::Normal(name)) if name == SETTINGS_FILE => components.next().is_none(),
        Some(Component::Normal(name)) => name == PROMPTS_FOLDER,
        Some(_) => false,
    }
}

/// Whether the watcher ignores a change at `path`, whose path in its
/// Destination is `relative`. As anarlog's `should_skip_path` (`path.rs`):
/// `.DS_Store`, tempfile's `.tmp` names and the `.tmp` extension. Also
/// every name in `relative` that starts with a dot: Longhand's staging,
/// rename and temporary folders, the library marker and the old result
/// copies. Audio files are not skipped: a note waits for its audio (R6).
/// Removal markers are not skipped: a marker makes a note complete (R15).
/// Their names have no leading dot for this reason (KTD1).
pub fn should_skip_path(relative: &Path, path: &Path) -> bool {
    is_temporary(path)
        || relative.components().any(|c| match c {
            Component::Normal(name) => name.to_string_lossy().starts_with('.'),
            _ => false,
        })
}

/// `.DS_Store`, tempfile's `.tmp` names and the `.tmp` extension.
fn is_temporary(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|n| n.to_str())
        && (name == ".DS_Store" || name.starts_with(".tmp"))
    {
        return true;
    }
    path.extension().is_some_and(|ext| ext == "tmp")
}

/// A watched Destination: its path as the settings hold it, and the same
/// path with links resolved, as FSEvents reports it (`/var` is
/// `/private/var` on macOS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watched {
    pub root: PathBuf,
    pub resolved: PathBuf,
    /// The Destination is the storage folder, so its `.longhand/` folder
    /// holds the shared settings that this computer uses (KTD13). The old
    /// folder of a Move is not.
    pub shared: bool,
}

impl Watched {
    pub fn new(root: &Path, shared: bool) -> Self {
        Self {
            root: root.to_path_buf(),
            resolved: std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()),
            shared,
        }
    }

    /// Where `path` is in this Destination.
    fn place(&self, path: &Path) -> Place {
        let Some(relative) = path
            .strip_prefix(&self.root)
            .or_else(|_| path.strip_prefix(&self.resolved))
            .ok()
        else {
            return Place::Outside;
        };
        let Some(Component::Normal(first)) = relative.components().next() else {
            return Place::Root;
        };
        if self.shared && !is_temporary(path) && is_shared_path(relative) {
            return Place::Shared;
        }
        if should_skip_path(relative, path) {
            return Place::Skipped;
        }
        Place::Folder(self.root.join(first))
    }
}

enum Place {
    /// In the top-level folder at this path, under the settings' path.
    Folder(PathBuf),
    /// The Destination itself.
    Root,
    /// A path that is not in the Destination.
    Outside,
    /// A shared settings path of the storage folder (KTD13).
    Shared,
    Skipped,
}

/// The scans that a set of events asks for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Batch {
    /// The shared settings get a read (KTD13). This starts no note scan.
    pub shared: bool,
    /// Destinations that get a full scan.
    pub full: BTreeSet<PathBuf>,
    /// The note folders that get a targeted scan, by Destination.
    pub folders: BTreeMap<PathBuf, HashSet<PathBuf>>,
}

impl Batch {
    /// Add the debounced events of the watch on `watched`. An error of the
    /// watch runs a full scan.
    pub fn add(&mut self, watched: &Watched, result: &DebounceEventResult) {
        match result {
            Ok(events) => {
                for event in events {
                    self.add_event(watched, event);
                }
            }
            Err(errors) => {
                for e in errors {
                    log::warn!(
                        "library: the watch of {} failed: {e}",
                        watched.root.display()
                    );
                }
                self.scan_all(watched);
            }
        }
    }

    pub fn add_event(&mut self, watched: &Watched, event: &notify::Event) {
        let root = &watched.root;
        if event.need_rescan() {
            self.scan_all(watched);
            return;
        }
        match event.kind {
            // The debouncer joins the two halves of a rename into `Both`.
            // One half alone is a move into or out of the Destination.
            // FSEvents gives each path of a rename as `Any`, and those
            // paths are marked as usual.
            EventKind::Modify(ModifyKind::Name(RenameMode::From | RenameMode::To)) => {
                self.scan_all(watched);
                return;
            }
            // As anarlog: creates, removes, data changes and renames.
            EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(ModifyKind::Any | ModifyKind::Data(_) | ModifyKind::Name(_)) => {}
            _ => return,
        }
        if event.paths.is_empty() {
            self.scan_all(watched);
            return;
        }
        for path in &event.paths {
            match watched.place(path) {
                Place::Folder(folder) => {
                    self.folders.entry(root.clone()).or_default().insert(folder);
                }
                Place::Root | Place::Outside => self.scan_all(watched),
                Place::Shared => self.shared = true,
                Place::Skipped => {}
            }
        }
    }

    /// A full scan of `watched`. The events can have lost a change of the
    /// shared settings too, so a full scan of the storage folder also reads
    /// them.
    fn scan_all(&mut self, watched: &Watched) {
        self.full.insert(watched.root.clone());
        self.shared |= watched.shared;
    }

    /// The targets to reconcile. A full scan of a Destination includes
    /// its targeted folders.
    pub fn targets(self) -> Vec<Target> {
        let mut targets = Vec::new();
        if !self.full.is_empty() {
            targets.push(Target::Full(self.full.iter().cloned().collect()));
        }
        for (root, folders) in self.folders {
            if !self.full.contains(&root) {
                targets.push(Target::Folders { root, folders });
            }
        }
        targets
    }
}

enum Command {
    Events {
        watched: Watched,
        result: DebounceEventResult,
    },
    Roots {
        roots: Vec<PathBuf>,
        storage: Option<PathBuf>,
    },
    Stop,
}

struct Watch {
    watched: Watched,
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

/// The state of the watcher thread.
struct Worker {
    roots: Vec<PathBuf>,
    /// The storage folder: the root whose `.longhand/` folder holds the
    /// shared settings (KTD13).
    storage: Option<PathBuf>,
    watches: BTreeMap<PathBuf, Watch>,
    reconciler: Reconciler,
    reconcile: ReconcileFn,
    shared: SharedFn,
    emit: EmitFn,
    /// Each watch sends its events here.
    commands: Sender<Command>,
    timing: Timing,
    next_full: Instant,
    last_run: Instant,
}

impl Worker {
    /// Watch each Destination in `roots` that is present, and drop the
    /// watches of the others.
    fn sync_watches(&mut self) {
        let roots = &self.roots;
        self.watches.retain(|root, _| {
            let keep = roots.contains(root) && root.is_dir();
            if !keep {
                log::info!("library: stopped the watch of {}", root.display());
            }
            keep
        });
        for root in self.roots.clone() {
            if self.watches.contains_key(&root) || !root.is_dir() {
                continue;
            }
            match self.watch(&root) {
                Ok(watch) => {
                    log::info!("library: watching {}", root.display());
                    self.watches.insert(root, watch);
                }
                Err(e) => log::warn!("library: cannot watch {}: {e}", root.display()),
            }
        }
    }

    fn watch(&self, root: &Path) -> notify::Result<Watch> {
        let watched = Watched::new(root, self.storage.as_deref() == Some(root));
        let commands = self.commands.clone();
        let sent = watched.clone();
        let mut debouncer = new_debouncer(
            self.timing.debounce,
            None,
            move |result: DebounceEventResult| {
                let _ = commands.send(Command::Events {
                    watched: sent.clone(),
                    result,
                });
            },
        )?;
        debouncer.watch(root, RecursiveMode::Recursive)?;
        Ok(Watch {
            watched,
            _debouncer: debouncer,
        })
    }

    /// A new set of Destinations: start the watches again on it, and scan
    /// every Destination. The change of folder already read the shared
    /// settings of the new storage folder.
    fn set_roots(&mut self, roots: Vec<PathBuf>, storage: Option<PathBuf>) {
        self.roots = roots;
        self.storage = storage;
        self.watches.clear();
        self.sync_watches();
        self.full_scan();
    }

    fn events(&mut self, watched: &Watched, result: &DebounceEventResult) {
        // A watch that a restart dropped can still send events.
        if self.watches.get(&watched.root).map(|w| &w.watched) != Some(watched) {
            return;
        }
        let mut batch = Batch::default();
        batch.add(watched, result);
        let shared = batch.shared;
        self.apply(batch.targets());
        if shared {
            (self.shared)();
        }
    }

    fn full_scan(&mut self) {
        let target = Target::Full(self.roots.clone());
        self.apply(vec![target]);
    }

    /// Reconcile each target, and emit the rows that changed.
    fn apply(&mut self, targets: Vec<Target>) {
        if targets.is_empty() {
            return;
        }
        let mut changed = BTreeSet::new();
        let mut removed = BTreeSet::new();
        for target in &targets {
            match (self.reconcile)(target, &mut self.reconciler) {
                Ok(changes) => {
                    changed.extend(changes.changed);
                    removed.extend(changes.removed);
                }
                Err(e) => log::error!("library: a reconcile failed: {e}"),
            }
        }
        self.last_run = Instant::now();
        let changes = Changes {
            changed: changed.into_iter().collect(),
            removed: removed.into_iter().collect(),
        };
        if !changes.is_empty() {
            (self.emit)(changes.into());
        }
    }

    /// When the thread must wake with no event: for the next full scan,
    /// or for the next pending delete.
    fn deadline(&self) -> Instant {
        let due = self
            .reconciler
            .next_due()
            .map(|due| due.max(self.last_run + self.timing.retry_wait));
        due.map_or(self.next_full, |due| due.min(self.next_full))
    }

    /// No event came before [`Worker::deadline`].
    fn timeout(&mut self, now: Instant) {
        let minute = now >= self.next_full;
        if minute {
            self.sync_watches();
            self.next_full = now + self.timing.full_scan_every;
        }
        // A full scan, for the minute or for a pending delete: only a full
        // scan can tell that a folder is still gone.
        self.full_scan();
        // A sync can change the shared files with no event (KTD13).
        if minute && self.storage.is_some() {
            (self.shared)();
        }
    }

    fn run(mut self, commands: Receiver<Command>) {
        loop {
            let wait = self.deadline().saturating_duration_since(Instant::now());
            match commands.recv_timeout(wait) {
                Ok(Command::Events { watched, result }) => self.events(&watched, &result),
                Ok(Command::Roots { roots, storage }) => self.set_roots(roots, storage),
                Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => self.timeout(Instant::now()),
            }
        }
        self.watches.clear();
    }
}

/// Watches the retained Destinations until it is stopped or dropped.
pub struct LibraryWatcher {
    commands: Sender<Command>,
    /// The thread, and the channel on which it says that it stopped.
    thread: Mutex<Option<(Receiver<()>, JoinHandle<()>)>>,
}

impl LibraryWatcher {
    /// Start watching `roots`. `storage`, one of the roots, is the storage
    /// folder: `shared` reads its shared settings again (KTD13).
    /// `reconciler` holds the pending deletes of the launch reconcile. No
    /// scan runs at the start: the launch scan ran just before.
    pub fn start(
        roots: Vec<PathBuf>,
        storage: Option<PathBuf>,
        reconciler: Reconciler,
        reconcile: ReconcileFn,
        shared: SharedFn,
        emit: EmitFn,
        timing: Timing,
    ) -> std::io::Result<Self> {
        let (commands, received) = mpsc::channel();
        let now = Instant::now();
        let mut worker = Worker {
            roots,
            storage,
            watches: BTreeMap::new(),
            reconciler,
            reconcile,
            shared,
            emit,
            commands: commands.clone(),
            timing,
            next_full: now + timing.full_scan_every,
            last_run: now,
        };
        let (done, stopped) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("library-watcher".into())
            .spawn(move || {
                worker.sync_watches();
                worker.run(received);
                let _ = done.send(());
            })?;
        Ok(Self {
            commands,
            thread: Mutex::new(Some((stopped, thread))),
        })
    }

    /// The retained Destinations changed: the watches start again on
    /// `roots`, with `storage` as the storage folder, and a full scan runs.
    pub fn set_roots(&self, roots: Vec<PathBuf>, storage: Option<PathBuf>) {
        let _ = self.commands.send(Command::Roots { roots, storage });
    }

    /// Stop the watches and the thread. Waits for a scan in progress, but
    /// not longer than [`STOP_WAIT`], so a read that does not finish does
    /// not stop the quit.
    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
        let taken = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some((stopped, thread)) = taken {
            match stopped.recv_timeout(STOP_WAIT) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                    let _ = thread.join();
                }
                Err(RecvTimeoutError::Timeout) => {
                    log::warn!("library: the watcher did not stop in {STOP_WAIT:?}");
                }
            }
        }
    }
}

impl Drop for LibraryWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, DataChange, Flag, MetadataKind};
    use notify_debouncer_full::DebouncedEvent;
    use session_core::folder::{SessionDoc, fingerprint};
    use session_core::{Segment, Store};
    use std::sync::Arc;

    const TRANSCRIPT: &str = "# Transcript\n\n- [00:00:01] Speaker 1: Hello.\n";
    const SUMMARY: &str = "---\nstatus: stub\nfailed_stage: summarize\n---\n\nNo summary.\n";

    /// The storage folder at `root`.
    fn watched(root: &str) -> Watched {
        Watched {
            root: PathBuf::from(root),
            resolved: PathBuf::from(format!("/private{root}")),
            shared: true,
        }
    }

    #[test]
    fn a_shared_settings_event_asks_for_a_read_and_no_note_scan() {
        let w = watched("/var/notes");
        for path in [
            "/var/notes/.longhand/settings.json",
            "/private/var/notes/.longhand/prompts/Recap.md",
            "/var/notes/.longhand/prompts/Recap 2.md",
            "/var/notes/.longhand/prompts",
            "/var/notes/.longhand",
        ] {
            let b = batch(&w, vec![event(data(), &[path])]);
            assert!(b.shared, "{path}");
            assert!(b.targets().is_empty(), "{path}");
        }
    }

    #[test]
    fn other_longhand_paths_ask_for_no_read_and_no_note_scan() {
        let w = watched("/var/notes");
        for path in [
            "/var/notes/.longhand/library",
            "/var/notes/.longhand/settings 2.json",
            "/var/notes/.longhand/settings.json/x",
            "/var/notes/.longhand/.tmpA1B2",
            "/var/notes/.longhand/prompts/.tmpA1B2",
            "/var/notes/.longhand/prompts/.DS_Store",
            "/var/notes/.longhand-moving",
        ] {
            let b = batch(&w, vec![event(data(), &[path])]);
            assert_eq!(b, Batch::default(), "{path}");
        }
        // The old folder of a Move does not hold the shared settings.
        let old = Watched {
            shared: false,
            ..watched("/var/old")
        };
        let b = batch(
            &old,
            vec![event(data(), &["/var/old/.longhand/settings.json"])],
        );
        assert_eq!(b, Batch::default());
    }

    #[test]
    fn a_full_scan_of_the_storage_folder_also_reads_the_shared_settings() {
        // The lost events can hold a change of the shared settings too.
        let rescan = event(EventKind::Other, &[]).set_flag(Flag::Rescan);
        assert!(batch(&watched("/var/notes"), vec![rescan.clone()]).shared);
        let old = Watched {
            shared: false,
            ..watched("/var/old")
        };
        assert!(!batch(&old, vec![rescan]).shared);
    }

    fn event(kind: EventKind, paths: &[&str]) -> notify::Event {
        paths.iter().fold(notify::Event::new(kind), |e, p| {
            e.add_path(PathBuf::from(p))
        })
    }

    fn data() -> EventKind {
        EventKind::Modify(ModifyKind::Data(DataChange::Content))
    }

    fn batch(watched: &Watched, events: Vec<notify::Event>) -> Batch {
        let mut batch = Batch::default();
        for e in &events {
            batch.add_event(watched, e);
        }
        batch
    }

    fn folders(batch: &Batch, root: &str) -> BTreeSet<PathBuf> {
        batch
            .folders
            .get(Path::new(root))
            .map(|f| f.iter().cloned().collect())
            .unwrap_or_default()
    }

    #[test]
    fn the_path_filter_skips_hidden_names_and_temporary_files() {
        let skip =
            |relative: &str| should_skip_path(Path::new(relative), &Path::new("/d").join(relative));
        assert!(skip(".DS_Store"));
        assert!(skip("Planning/.DS_Store"));
        assert!(skip(".tmp6s1cca"));
        assert!(skip("Planning/.tmpABC123"));
        assert!(skip("Planning/notes.md.tmp"));
        assert!(skip(".longhand/library"));
        assert!(skip(".staging-A1/transcript.md"));
        assert!(skip(".rename-A1"));
        assert!(skip("Planning/.notes.md.partial"));
        // Old result copies are not note files: their change is not a
        // change of the note.
        assert!(skip("Planning/.longhand-results/generation-1/summary.md"));
        assert!(skip("Planning/.longhand-result-transition.json"));
        assert!(!skip("Planning"));
        assert!(!skip("Planning/session.md"));
        assert!(!skip("Planning/notes (1).md"));
        assert!(!skip("Planning/audio/segment-01.opus"));
        assert!(!skip("Planning/audio/segment-01.wav"));
        // A removal marker makes a note complete, so its delivery is a
        // change (R15).
        let marker = session_core::folder::removal_marker("audio/segment-01.opus");
        assert!(!skip(&format!("Planning/{marker}")));
    }

    #[test]
    fn a_change_of_old_result_copies_marks_no_note_folder() {
        let w = watched("/var/notes");
        let b = batch(
            &w,
            vec![
                event(
                    EventKind::Create(CreateKind::File),
                    &["/var/notes/Planning/.longhand-results/generation-1/summary.md"],
                ),
                event(
                    data(),
                    &["/var/notes/Planning/.longhand-result-transition.json"],
                ),
            ],
        );
        assert_eq!(b, Batch::default());
    }

    #[test]
    fn a_removal_marker_event_marks_its_note_folder() {
        let w = watched("/var/notes");
        let marker = session_core::folder::removal_marker("audio/segment-01.opus");
        let b = batch(
            &w,
            vec![event(
                EventKind::Create(CreateKind::File),
                &[&format!("/private/var/notes/Planning/{marker}")],
            )],
        );
        assert!(b.full.is_empty());
        assert_eq!(
            folders(&b, "/var/notes"),
            BTreeSet::from([PathBuf::from("/var/notes/Planning")])
        );
    }

    #[test]
    fn an_event_marks_only_its_top_level_note_folder() {
        let w = watched("/var/notes");
        let b = batch(
            &w,
            vec![
                event(data(), &["/var/notes/Planning/summary.md"]),
                event(
                    EventKind::Create(CreateKind::File),
                    &["/private/var/notes/Review/audio/segment-01.opus"],
                ),
                event(data(), &["/var/notes/.staging-A1/notes.md"]),
            ],
        );
        assert!(b.full.is_empty());
        assert_eq!(
            folders(&b, "/var/notes"),
            BTreeSet::from([
                PathBuf::from("/var/notes/Planning"),
                PathBuf::from("/var/notes/Review"),
            ])
        );
        assert_eq!(
            b.targets(),
            vec![Target::Folders {
                root: PathBuf::from("/var/notes"),
                folders: HashSet::from([
                    PathBuf::from("/var/notes/Planning"),
                    PathBuf::from("/var/notes/Review"),
                ]),
            }]
        );
    }

    #[test]
    fn a_renamed_folder_marks_both_names() {
        let w = watched("/var/notes");
        let both = EventKind::Modify(ModifyKind::Name(RenameMode::Both));
        let any = EventKind::Modify(ModifyKind::Name(RenameMode::Any));
        let b = batch(
            &w,
            vec![
                event(both, &["/var/notes/Old", "/var/notes/New"]),
                event(any, &["/var/notes/Other"]),
            ],
        );
        assert!(b.full.is_empty());
        assert_eq!(folders(&b, "/var/notes").len(), 3);
    }

    #[test]
    fn a_rescan_an_unpaired_rename_or_no_path_runs_a_full_scan() {
        let w = watched("/var/notes");
        let rescan = event(EventKind::Other, &[]).set_flag(Flag::Rescan);
        let from = event(
            EventKind::Modify(ModifyKind::Name(RenameMode::From)),
            &["/var/notes/Planning"],
        );
        let no_path = event(data(), &[]);
        let root = event(
            EventKind::Remove(notify::event::RemoveKind::Folder),
            &["/var/notes"],
        );
        let outside = event(data(), &["/elsewhere/file.md"]);
        for e in [rescan, from, no_path, root, outside] {
            let b = batch(&w, vec![e.clone()]);
            assert_eq!(
                b.full,
                BTreeSet::from([PathBuf::from("/var/notes")]),
                "{e:?}"
            );
            assert_eq!(
                b.targets(),
                vec![Target::Full(vec![PathBuf::from("/var/notes")])]
            );
        }
        let mut errors = Batch::default();
        errors.add(&w, &Err(vec![notify::Error::generic("lost")]));
        assert_eq!(errors.full, BTreeSet::from([PathBuf::from("/var/notes")]));
    }

    #[test]
    fn access_and_metadata_events_ask_for_no_scan() {
        let w = watched("/var/notes");
        let b = batch(
            &w,
            vec![
                event(
                    EventKind::Access(notify::event::AccessKind::Any),
                    &["/var/notes/Planning/session.md"],
                ),
                event(
                    EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
                    &["/var/notes/Planning/session.md"],
                ),
            ],
        );
        assert_eq!(b, Batch::default());
        assert!(b.targets().is_empty());
    }

    #[test]
    fn a_full_scan_of_a_destination_includes_its_targeted_folders() {
        let w = watched("/var/notes");
        let b = batch(
            &w,
            vec![
                event(data(), &["/var/notes/Planning/summary.md"]),
                event(EventKind::Other, &[]).set_flag(Flag::Rescan),
            ],
        );
        assert_eq!(
            b.targets(),
            vec![Target::Full(vec![PathBuf::from("/var/notes")])]
        );
    }

    fn doc(id: &str) -> SessionDoc {
        let start = chrono::DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap();
        let segment = Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 60_000,
            wall_start: start,
            wall_end: start + chrono::TimeDelta::minutes(1),
            audio_file: "audio/segment-01.opus".into(),
            channels: 1,
        };
        let mut doc = SessionDoc::new(id, None, &[segment]).unwrap();
        doc.title = Some("Planning".into());
        doc.transcript_fingerprint = Some(fingerprint(TRANSCRIPT.as_bytes()));
        doc.summary_fingerprint = Some(fingerprint(SUMMARY.as_bytes()));
        doc
    }

    /// A complete note folder, as another computer's save leaves it. The
    /// documents come first and `session.md` last, as a sync can deliver
    /// them.
    fn note(root: &Path, name: &str, id: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("audio")).unwrap();
        std::fs::write(dir.join("transcript.md"), TRANSCRIPT).unwrap();
        std::fs::write(dir.join("summary.md"), SUMMARY).unwrap();
        std::fs::write(dir.join("notes.md"), "My notes\n").unwrap();
        std::fs::write(dir.join("audio/segment-01.opus"), b"opus").unwrap();
        std::fs::write(dir.join("session.md"), doc(id).render()).unwrap();
        dir
    }

    fn marked_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        library::scan::write_marker(root.path());
        root
    }

    type Targets = Arc<Mutex<Vec<Target>>>;

    /// A reconcile over a real store, which also records each target.
    fn reconcile_into(store: Arc<Mutex<Store>>, targets: Targets) -> ReconcileFn {
        Box::new(move |target, reconciler| {
            targets.lock().unwrap().push(target.clone());
            reconciler
                .run(&store, target, &|_| false)
                .map_err(|e| e.to_string())
        })
    }

    fn fast() -> Timing {
        Timing {
            debounce: Duration::from_millis(100),
            full_scan_every: Duration::from_secs(3600),
            retry_wait: Duration::from_millis(50),
        }
    }

    /// A worker with no thread, for the tests that call its steps.
    fn worker(roots: Vec<PathBuf>, timing: Timing) -> (Worker, Targets, Receiver<Command>) {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let targets = Targets::default();
        let (commands, received) = mpsc::channel();
        let now = Instant::now();
        let worker = Worker {
            storage: roots.first().cloned(),
            roots,
            watches: BTreeMap::new(),
            reconciler: Reconciler::default(),
            reconcile: reconcile_into(store, Arc::clone(&targets)),
            shared: Box::new(|| {}),
            emit: Box::new(|_| {}),
            commands,
            timing,
            next_full: now + timing.full_scan_every,
            last_run: now,
        };
        (worker, targets, received)
    }

    fn watched_roots(worker: &Worker) -> Vec<PathBuf> {
        worker.watches.keys().cloned().collect()
    }

    #[test]
    fn a_folder_added_under_a_watched_root_emits_its_id() {
        let root = marked_root();
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let targets = Targets::default();
        let (sent, emitted) = mpsc::channel();
        let watcher = LibraryWatcher::start(
            vec![root.path().to_path_buf()],
            Some(root.path().to_path_buf()),
            Reconciler::default(),
            reconcile_into(Arc::clone(&store), Arc::clone(&targets)),
            Box::new(|| {}),
            Box::new(move |changes| {
                let _ = sent.send(changes);
            }),
            fast(),
        )
        .unwrap();
        // FSEvents can start the stream after the first change, and then
        // it reports nothing. Write `session.md` again until a change
        // arrives, but never run a full scan.
        std::thread::sleep(Duration::from_millis(500));
        let dir = note(root.path(), "2026-09-25 0900 Planning", "A1");
        let mut changes = None;
        for _ in 0..5 {
            match emitted.recv_timeout(Duration::from_secs(2)) {
                Ok(c) if !c.changed.is_empty() => {
                    changes = Some(c);
                    break;
                }
                Ok(_) => {}
                Err(_) => std::fs::write(dir.join("session.md"), doc("A1").render()).unwrap(),
            }
        }
        let changes = changes.expect("no LibraryChanged in 10 seconds");
        assert_eq!(changes.changed, ["A1"]);
        assert!(changes.removed.is_empty());
        assert!(store.lock().unwrap().session("A1").unwrap().is_some());
        watcher.stop();
        assert!(
            targets
                .lock()
                .unwrap()
                .iter()
                .all(|t| matches!(t, Target::Folders { .. })),
            "a change in one folder asks for a targeted scan: {:?}",
            targets.lock().unwrap()
        );
    }

    #[test]
    fn an_unchanged_folder_emits_nothing() {
        let root = marked_root();
        let (mut worker, _, _) = worker(vec![root.path().to_path_buf()], fast());
        let (sent, emitted) = mpsc::channel();
        worker.emit = Box::new(move |changes| {
            let _ = sent.send(changes);
        });
        note(root.path(), "Planning", "A1");
        worker.full_scan();
        assert_eq!(emitted.try_recv().unwrap().changed, ["A1"]);
        worker.full_scan();
        assert!(emitted.try_recv().is_err());
    }

    #[test]
    fn a_rescan_event_runs_a_full_scan() {
        let root = marked_root();
        let (mut worker, targets, _) = worker(vec![root.path().to_path_buf()], fast());
        worker.sync_watches();
        let watched = worker.watches[root.path()].watched.clone();
        let rescan = event(EventKind::Other, &[]).set_flag(Flag::Rescan);
        worker.events(
            &watched,
            &Ok(vec![DebouncedEvent::new(rescan, Instant::now())]),
        );
        assert_eq!(
            *targets.lock().unwrap(),
            [Target::Full(vec![root.path().to_path_buf()])]
        );
    }

    #[test]
    fn a_roots_change_restarts_the_watches_on_the_new_set() {
        let first = marked_root();
        let second = marked_root();
        let (mut worker, targets, _) = worker(vec![first.path().to_path_buf()], fast());
        worker.sync_watches();
        assert_eq!(watched_roots(&worker), [first.path()]);
        let old = worker.watches[first.path()].watched.clone();

        let roots = vec![second.path().to_path_buf(), first.path().to_path_buf()];
        worker.set_roots(roots.clone(), Some(second.path().to_path_buf()));
        let mut expected = [first.path(), second.path()];
        expected.sort();
        assert_eq!(watched_roots(&worker), expected);
        assert!(worker.watches[second.path()].watched.shared);
        assert!(!worker.watches[first.path()].watched.shared);
        assert_eq!(*targets.lock().unwrap(), [Target::Full(roots)]);
        // An event from the old watch asks for no scan.
        worker.events(
            &old,
            &Ok(vec![DebouncedEvent::new(
                event(EventKind::Other, &[]),
                Instant::now(),
            )]),
        );
        assert_eq!(targets.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_root_that_appears_later_gets_a_watch_at_the_next_full_scan() {
        let parent = tempfile::tempdir().unwrap();
        let later = parent.path().join("Notes");
        let (mut worker, targets, _) = worker(vec![later.clone()], fast());
        worker.sync_watches();
        assert!(worker.watches.is_empty());

        std::fs::create_dir(&later).unwrap();
        // Before the minute, a timeout for a pending delete adds no watch.
        worker.timeout(Instant::now());
        assert!(worker.watches.is_empty());
        let minute = worker.next_full;
        worker.timeout(minute);
        assert_eq!(watched_roots(&worker), [later.as_path()]);
        assert_eq!(targets.lock().unwrap().len(), 2);
        assert_eq!(worker.next_full, minute + worker.timing.full_scan_every);

        // A root that goes away loses its watch at the next full scan.
        std::fs::remove_dir_all(&later).unwrap();
        worker.timeout(worker.next_full);
        assert!(worker.watches.is_empty());
    }

    #[test]
    fn a_pending_delete_wakes_the_watcher_when_it_is_due() {
        let root = marked_root();
        let (mut worker, _, _) = worker(vec![root.path().to_path_buf()], fast());
        let dir = note(root.path(), "Planning", "A1");
        worker.full_scan();
        assert_eq!(worker.deadline(), worker.next_full);

        std::fs::remove_dir_all(&dir).unwrap();
        worker.full_scan();
        let due = worker.reconciler.next_due().expect("a pending delete");
        assert_eq!(worker.deadline(), due);
        assert!(worker.deadline() < worker.next_full);
    }

    #[test]
    fn a_pending_delete_that_stays_due_waits_between_scans() {
        let root = marked_root();
        let (mut worker, _, _) = worker(vec![root.path().to_path_buf()], fast());
        let dir = note(root.path(), "Planning", "A1");
        worker.full_scan();
        std::fs::remove_dir_all(&dir).unwrap();
        worker.full_scan();
        // As if the wait had passed while a stage held the row.
        worker.last_run = Instant::now();
        let due = worker.reconciler.next_due().unwrap();
        assert!(worker.deadline() >= due);
        assert!(worker.deadline() >= worker.last_run + worker.timing.retry_wait);
    }

    use crate::managers::shared::SharedSettingsManager;
    use shared_settings::file::SharedValue;

    /// Whether each read of the shared settings changed the view. The app
    /// emits `SettingsChanged` for each `true`.
    type Reads = Arc<Mutex<Vec<bool>>>;

    /// One computer that uses the storage folder `root`: its shared settings
    /// manager and its local cache.
    struct Computer {
        manager: Arc<SharedSettingsManager>,
        _cache: tempfile::TempDir,
    }

    fn computer(root: &Path) -> Computer {
        let cache = tempfile::tempdir().unwrap();
        let manager = Arc::new(SharedSettingsManager::new(
            cache.path().to_path_buf(),
            Some(root.to_path_buf()),
        ));
        manager.join_at_launch();
        Computer {
            manager,
            _cache: cache,
        }
    }

    /// A worker on `root` whose shared read goes to `b`, as in the app.
    fn worker_of(root: &Path, b: &Computer) -> (Worker, Targets, Reads) {
        let (mut worker, targets, _) = worker(vec![root.to_path_buf()], fast());
        let reads = Reads::default();
        let (manager, record) = (Arc::clone(&b.manager), Arc::clone(&reads));
        worker.shared = Box::new(move || record.lock().unwrap().push(manager.reread()));
        worker.sync_watches();
        (worker, targets, reads)
    }

    fn deliver(worker: &mut Worker, root: &Path, relative: &str) {
        let watched = worker.watches[root].watched.clone();
        let path = root.join(relative);
        let path = path.to_str().unwrap();
        worker.events(
            &watched,
            &Ok(vec![DebouncedEvent::new(
                event(data(), &[path]),
                Instant::now(),
            )]),
        );
    }

    #[test]
    fn an_event_on_the_settings_file_reads_the_shared_settings_and_starts_no_note_scan() {
        let root = marked_root();
        let (a, b) = (computer(root.path()), computer(root.path()));
        let (mut worker, targets, reads) = worker_of(root.path(), &b);

        a.manager
            .save(&[SharedValue::LiveTranscript(false)])
            .unwrap();
        deliver(&mut worker, root.path(), ".longhand/settings.json");
        assert!(targets.lock().unwrap().is_empty(), "no note scan");
        assert_eq!(*reads.lock().unwrap(), [true]);
        assert!(
            !b.manager
                .snapshot()
                .view
                .fields
                .live_transcript
                .known()
                .unwrap()
        );

        // The library marker starts neither a read nor a scan.
        deliver(&mut worker, root.path(), ".longhand/library");
        assert!(targets.lock().unwrap().is_empty());
        assert_eq!(reads.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_new_prompt_text_from_another_computer_changes_the_settings() {
        // AE2: B shows A's change with no restart.
        let root = marked_root();
        let (a, b) = (computer(root.path()), computer(root.path()));
        let id = a.manager.create_prompt().unwrap();
        a.manager.rename_prompt(&id, "Recap").unwrap();
        let (mut worker, _, reads) = worker_of(root.path(), &b);
        deliver(&mut worker, root.path(), ".longhand/prompts/Recap.md");
        assert_eq!(*reads.lock().unwrap(), [true]);

        a.manager.set_prompt_text(&id, "New recap text.").unwrap();
        deliver(&mut worker, root.path(), ".longhand/prompts/Recap.md");
        assert_eq!(*reads.lock().unwrap(), [true, true]);
        let prompts = b.manager.snapshot().view.prompts;
        assert_eq!(prompts[0].text, "New recap text.");
    }

    #[test]
    fn the_event_of_this_computers_own_write_emits_no_second_change() {
        let root = marked_root();
        let b = computer(root.path());
        let (mut worker, _, reads) = worker_of(root.path(), &b);
        // The command emits the change of the save itself.
        b.manager
            .save(&[SharedValue::LiveTranscript(false)])
            .unwrap();
        deliver(&mut worker, root.path(), ".longhand/settings.json");
        let id = b.manager.create_prompt().unwrap();
        b.manager.set_prompt_text(&id, "Mine.").unwrap();
        deliver(&mut worker, root.path(), ".longhand/prompts/New prompt.md");
        assert_eq!(*reads.lock().unwrap(), [false, false]);
    }

    #[test]
    fn a_change_with_no_file_event_is_read_at_the_next_full_scan() {
        let root = marked_root();
        let (a, b) = (computer(root.path()), computer(root.path()));
        let (mut worker, _, reads) = worker_of(root.path(), &b);
        a.manager
            .save(&[SharedValue::CheckUpdatesAutomatically(false)])
            .unwrap();
        // A scan for a pending delete, before the minute, does not read.
        worker.timeout(Instant::now());
        assert!(reads.lock().unwrap().is_empty());
        worker.timeout(worker.next_full);
        assert_eq!(*reads.lock().unwrap(), [true]);
        assert!(
            !b.manager
                .snapshot()
                .view
                .fields
                .check_updates_automatically
                .known()
                .unwrap()
        );
    }
}
