//! The database check at launch (U4) and the startup failure alert (R9).
//!
//! The check runs in `setup`, after `take_instance_lock` and
//! `install_at_launch`, before any reader opens `longhand.db` (KTD5). It
//! inspects the database without a change to it. A database that is not
//! this build's own is moved aside as one backup, and normal startup then
//! makes a new database. When the database holds unfinished work, or the
//! inspection cannot read it, the user chooses Quit or Rebuild in an alert
//! (R6, R7, R8).

use std::path::{Path, PathBuf};

use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use session_core::backup::{self, BackupKind};
use session_core::inspect::{self, Inspection, NotOwnReason, UnfinishedWork};
use session_core::jobs::Liveness;
use tauri::Manager;

/// The button that closes the app and changes no file (R8).
pub const QUIT: &str = "Quit";
/// The button that moves the database aside and makes a new one (R8).
pub const REBUILD: &str = "Rebuild";

/// What the launch does after the inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Start normally with the database as it is.
    Continue,
    /// Move the database aside with no alert (R2).
    Rebuild,
    /// Show an alert and let the user choose Quit or Rebuild (R6, R7).
    Ask(Question),
}

/// The alert that [`Decision::Ask`] shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    /// The database holds unfinished work (R6). `reason` tells if the
    /// database is from a newer version or from another version.
    UnfinishedWork {
        reason: NotOwnReason,
        work: UnfinishedWork,
    },
    /// The inspection could not read the database (R7). The text is the
    /// error.
    CheckFailed(String),
}

/// The choice that the user made in a Quit or Rebuild alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Quit,
    Rebuild,
}

/// A startup error with a plain description for the R9 alert.
#[derive(Debug)]
pub struct Failure {
    pub description: &'static str,
    pub source: Box<dyn std::error::Error>,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// The R9 description when the error has no specific description.
pub const DEFAULT_FAILURE: &str = "An error stopped Longhand from starting.";
const REBUILD_FAILURE: &str = "Longhand could not make a new notes list.";
const BACKUP_UNDO_FAILURE: &str =
    "Longhand could not put back the files of a backup that it did not finish.";

/// Map the inspection to the launch action. `only_instance` is true when
/// this build holds the instance lock. Without the lock, the launch skips
/// the check and continues: another instance can be live, and a database
/// error then stops the start with the R9 alert.
pub fn decide(inspection: &Inspection, only_instance: bool) -> Decision {
    if !only_instance {
        return Decision::Continue;
    }
    match inspection {
        Inspection::Own => Decision::Continue,
        Inspection::NotOwn { work, .. } if work.is_empty() => Decision::Rebuild,
        Inspection::NotOwn { reason, work } => Decision::Ask(Question::UnfinishedWork {
            reason: *reason,
            work: *work,
        }),
        Inspection::Unreadable { error } => Decision::Ask(Question::CheckFailed(error.clone())),
    }
}

/// One item of the work list: "a {one}" for a count of 1, else
/// "{count} {many}".
fn counted(count: u64, one: &str, many: &str) -> Option<String> {
    match count {
        0 => None,
        1 => Some(one.to_string()),
        n => Some(format!("{n} {many}")),
    }
}

/// The kinds of unfinished work, as plain text for the R6 alert.
pub fn work_items(work: &UnfinishedWork) -> Vec<String> {
    let notes = match work.notes {
        0 => None,
        1 => Some("1 note that is not saved yet".to_string()),
        n => Some(format!("{n} notes that are not saved yet")),
    };
    [
        notes,
        counted(
            work.jobs,
            "a transcription or summary that is running",
            "transcriptions or summaries that are running",
        ),
        work.open_move
            .then(|| "a move of your storage folder".to_string()),
        counted(work.renames, "a folder rename", "folder renames"),
        counted(
            work.publications,
            "a summary that is being written",
            "summaries that are being written",
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// "a", "a and b", "a, b and c".
fn join_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The title and the body of a Quit or Rebuild alert.
pub fn question_text(question: &Question, backups: &Path) -> (String, String) {
    let backups = backups.display();
    match question {
        Question::UnfinishedWork { reason, work } => {
            // A different migration hash does not tell which version is
            // newer, so that text says "another version".
            let (from, open) = match reason {
                NotOwnReason::NewerVersion { .. } => ("a newer version", "the newer version"),
                NotOwnReason::DifferentMigration { .. } => ("another version", "that version"),
            };
            (
                format!("Your notes list is from {from} of Longhand"),
                format!(
                    "It has work that is not finished: {}.\n\n\
                     Rebuild makes a new notes list from your saved notes. The unfinished \
                     work will not be in it. A copy is kept in {backups}. To get the work \
                     back, put the copy back and open {open}.",
                    join_list(&work_items(work))
                ),
            )
        }
        Question::CheckFailed(error) => (
            "Longhand can't check your notes list".to_string(),
            format!(
                "It may be from a newer version of Longhand and may have work that is \
                 not finished.\n\n\
                 Rebuild makes a new notes list from your saved notes. Any unfinished \
                 work will not be in it. A copy is kept in {backups}.\n\n\
                 Details: {error}"
            ),
        ),
    }
}

/// The title and the body of the R9 alert.
pub fn failure_text(description: &str, log_file: Option<&Path>, error: &str) -> (String, String) {
    let next_step = match log_file {
        Some(path) => format!(
            "Open Longhand again. If the problem continues, send the log file: {}",
            path.display()
        ),
        None => "Open Longhand again. If the problem continues, send the log file.".to_string(),
    };
    (
        "Longhand can't start".to_string(),
        format!("{description}\n\n{next_step}\n\nDetails: {error}"),
    )
}

/// The choice that an alert result means. Every result other than the
/// Rebuild button, such as a closed alert, is Quit.
pub fn choice_from(result: &MessageDialogResult) -> Choice {
    match result {
        MessageDialogResult::Custom(text) if text == REBUILD => Choice::Rebuild,
        _ => Choice::Quit,
    }
}

/// Show a Quit or Rebuild alert and wait for the choice. Call this on the
/// main thread. The first button is the default button, so Quit is first.
fn ask(question: &Question, backups: &Path) -> Choice {
    let (title, body) = question_text(question, backups);
    let result = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(title)
        .set_description(body)
        .set_buttons(MessageButtons::OkCancelCustom(
            QUIT.to_string(),
            REBUILD.to_string(),
        ))
        .show();
    choice_from(&result)
}

/// The log file of the `tauri-plugin-log` `LogDir` target, for a build
/// error, when no app handle exists. The path is the same as
/// `app_log_dir()` joined with `{product_name}.log`.
pub fn log_file_for(identifier: &str, product_name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let dir = dirs::home_dir().map(|home| home.join("Library/Logs").join(identifier));
    #[cfg(not(target_os = "macos"))]
    let dir = dirs::data_local_dir().map(|data| data.join(identifier).join("logs"));
    dir.map(|dir| dir.join(format!("{product_name}.log")))
}

/// The log file of the `tauri-plugin-log` `LogDir` target.
pub fn log_file(app: &tauri::AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_log_dir().ok()?;
    Some(dir.join(format!("{}.log", app.package_info().name)))
}

/// Log a startup error, show the R9 alert and exit with code 1. The app
/// does not panic (KTD7).
pub fn fail(error: &(dyn std::error::Error + 'static), log_file: Option<&Path>) -> ! {
    let description = error
        .downcast_ref::<Failure>()
        .map_or(DEFAULT_FAILURE, |failure| failure.description);
    log::error!("startup failed: {description} {error}");
    log::logger().flush();
    eprintln!("Longhand can't start: {error}");
    let (title, body) = failure_text(description, log_file, &error.to_string());
    MessageDialog::new()
        .set_level(MessageLevel::Error)
        .set_title(title)
        .set_description(body)
        .set_buttons(MessageButtons::OkCustom(QUIT.to_string()))
        .show();
    std::process::exit(1);
}

/// The old database version for the R10 log line.
fn old_version(inspection: &Inspection) -> String {
    match inspection {
        Inspection::Own => "this build's own version".to_string(),
        Inspection::NotOwn {
            reason: NotOwnReason::NewerVersion { found, .. },
            ..
        } => format!("version {found}"),
        Inspection::NotOwn {
            reason: NotOwnReason::DifferentMigration { version },
            ..
        } => format!("a different migration at version {version}"),
        Inspection::Unreadable { error } => format!("a database that could not be read ({error})"),
    }
}

/// Move the database and the working folder aside, apply the retention
/// and write the R10 log line.
fn rebuild(app_data: &Path, kind: BackupKind, inspection: &Inspection) -> Result<(), Failure> {
    let to_failure = |e: backup::BackupError| Failure {
        description: REBUILD_FAILURE,
        source: Box::new(e),
    };
    let backup_path = backup::move_aside(app_data, kind).map_err(to_failure)?;
    // The database is aside, so normal startup can make a new one. A
    // failure to delete old backups does not stop the start.
    match backup::apply_retention(app_data) {
        Ok(removed) => {
            for folder in removed {
                log::info!("deleted the old database backup {}", folder.display());
            }
        }
        Err(e) => log::warn!("could not delete old database backups: {e}"),
    }
    log::info!(
        "rebuilt the notes database: old {}, new version {}, backup {}",
        old_version(inspection),
        session_core::schema::MIGRATION_SQL.len(),
        backup_path.display()
    );
    Ok(())
}

/// Check `longhand.db` before any reader opens it, and rebuild it when it
/// is not this build's own (KTD5). Quit in an alert exits with code 0 and
/// changes no file (R8).
pub fn check_database(app: &tauri::AppHandle, liveness: &Liveness) -> Result<(), Failure> {
    // The inspection deletes a -shm or -wal file that it made. With another
    // instance possibly live, that file can be the other instance's, so the
    // check runs only when this build holds the instance lock.
    if !matches!(liveness, Liveness::OnlyInstance) {
        log::info!("database check skipped: the instance lock is not held");
        return Ok(());
    }
    let app_data = app.path().app_data_dir().map_err(|e| Failure {
        description: DEFAULT_FAILURE,
        source: Box::new(e),
    })?;
    for undone in backup::undo_incomplete(&app_data).map_err(|e| Failure {
        description: BACKUP_UNDO_FAILURE,
        source: Box::new(e),
    })? {
        log::info!(
            "moved the files of an incomplete backup back from {}",
            undone.display()
        );
    }
    if let Some(moved) = backup::move_orphan_wal(&app_data).map_err(|e| Failure {
        description: REBUILD_FAILURE,
        source: Box::new(e),
    })? {
        log::info!(
            "moved a -wal or -shm file with no database into {}",
            moved.display()
        );
    }
    let inspection = inspect::inspect(&app_data);
    match decide(&inspection, true) {
        Decision::Continue => Ok(()),
        Decision::Rebuild => rebuild(&app_data, BackupKind::Automatic, &inspection),
        Decision::Ask(question) => {
            log::warn!("the notes database needs a choice: {inspection:?}");
            match ask(&question, &app_data.join(backup::BACKUPS_DIR)) {
                Choice::Quit => {
                    log::info!("the user chose Quit; no file changed");
                    log::logger().flush();
                    std::process::exit(0);
                }
                Choice::Rebuild => {
                    log::info!("the user chose Rebuild");
                    rebuild(&app_data, BackupKind::ChosenInAlert, &inspection)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn not_own(work: UnfinishedWork) -> Inspection {
        Inspection::NotOwn {
            reason: NotOwnReason::NewerVersion { found: 12, own: 10 },
            work,
        }
    }

    fn unreadable() -> Inspection {
        Inspection::Unreadable {
            error: "file is not a database".into(),
        }
    }

    fn one_of_each() -> UnfinishedWork {
        UnfinishedWork {
            notes: 1,
            jobs: 1,
            open_move: true,
            renames: 1,
            publications: 1,
        }
    }

    #[test]
    fn own_continues_with_any_liveness() {
        assert_eq!(decide(&Inspection::Own, true), Decision::Continue);
        assert_eq!(decide(&Inspection::Own, false), Decision::Continue);
    }

    #[test]
    fn not_own_with_no_work_rebuilds_as_the_only_instance() {
        assert_eq!(
            decide(&not_own(UnfinishedWork::default()), true),
            Decision::Rebuild
        );
        let other = Inspection::NotOwn {
            reason: NotOwnReason::DifferentMigration { version: 3 },
            work: UnfinishedWork::default(),
        };
        assert_eq!(decide(&other, true), Decision::Rebuild);
    }

    #[test]
    fn not_own_with_work_asks_as_the_only_instance() {
        let work = UnfinishedWork {
            notes: 1,
            ..Default::default()
        };
        assert_eq!(
            decide(&not_own(work), true),
            Decision::Ask(Question::UnfinishedWork {
                reason: NotOwnReason::NewerVersion { found: 12, own: 10 },
                work
            })
        );
        let other = Inspection::NotOwn {
            reason: NotOwnReason::DifferentMigration { version: 3 },
            work,
        };
        assert_eq!(
            decide(&other, true),
            Decision::Ask(Question::UnfinishedWork {
                reason: NotOwnReason::DifferentMigration { version: 3 },
                work
            })
        );
    }

    #[test]
    fn unreadable_asks_with_the_check_failed_question_as_the_only_instance() {
        assert_eq!(
            decide(&unreadable(), true),
            Decision::Ask(Question::CheckFailed("file is not a database".into()))
        );
    }

    #[test]
    fn not_own_or_unreadable_continues_without_the_instance_lock() {
        // The check is skipped: normal startup reaches the R9 path if the
        // database cannot be opened.
        assert_eq!(decide(&not_own(one_of_each()), false), Decision::Continue);
        assert_eq!(
            decide(&not_own(UnfinishedWork::default()), false),
            Decision::Continue
        );
        assert_eq!(decide(&unreadable(), false), Decision::Continue);
    }

    #[test]
    fn work_items_name_each_kind_that_is_present() {
        assert_eq!(
            work_items(&one_of_each()),
            vec![
                "1 note that is not saved yet",
                "a transcription or summary that is running",
                "a move of your storage folder",
                "a folder rename",
                "a summary that is being written",
            ]
        );
        let many = UnfinishedWork {
            notes: 3,
            jobs: 2,
            open_move: false,
            renames: 4,
            publications: 2,
        };
        assert_eq!(
            work_items(&many),
            vec![
                "3 notes that are not saved yet",
                "2 transcriptions or summaries that are running",
                "4 folder renames",
                "2 summaries that are being written",
            ]
        );
        let move_only = UnfinishedWork {
            open_move: true,
            ..Default::default()
        };
        assert_eq!(
            work_items(&move_only),
            vec!["a move of your storage folder"]
        );
    }

    #[test]
    fn the_unfinished_work_alert_names_the_work_and_the_backups_folder() {
        let work = UnfinishedWork {
            notes: 2,
            open_move: true,
            ..Default::default()
        };
        let backups = Path::new("/data/database-backups");
        let newer = NotOwnReason::NewerVersion { found: 12, own: 10 };
        let (title, body) = question_text(
            &Question::UnfinishedWork {
                reason: newer,
                work,
            },
            backups,
        );
        assert_eq!(title, "Your notes list is from a newer version of Longhand");
        assert_eq!(
            body,
            "It has work that is not finished: 2 notes that are not saved yet \
             and a move of your storage folder.\n\n\
             Rebuild makes a new notes list from your saved notes. The unfinished \
             work will not be in it. A copy is kept in /data/database-backups. To get \
             the work back, put the copy back and open the newer version."
        );
        let (_, body) = question_text(
            &Question::UnfinishedWork {
                reason: newer,
                work: one_of_each(),
            },
            backups,
        );
        assert!(body.starts_with(
            "It has work that is not finished: 1 note that is not saved yet, \
             a transcription or summary that is running, a move of your storage \
             folder, a folder rename and a summary that is being written.\n\n"
        ));
    }

    #[test]
    fn the_unfinished_work_alert_for_a_different_migration_says_another_version() {
        let work = UnfinishedWork {
            notes: 1,
            ..Default::default()
        };
        let backups = Path::new("/data/database-backups");
        let (title, body) = question_text(
            &Question::UnfinishedWork {
                reason: NotOwnReason::DifferentMigration { version: 3 },
                work,
            },
            backups,
        );
        assert_eq!(title, "Your notes list is from another version of Longhand");
        assert_eq!(
            body,
            "It has work that is not finished: 1 note that is not saved yet.\n\n\
             Rebuild makes a new notes list from your saved notes. The unfinished \
             work will not be in it. A copy is kept in /data/database-backups. To get \
             the work back, put the copy back and open that version."
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_log_file_on_macos_is_in_library_logs() {
        let home = dirs::home_dir().expect("a home folder");
        assert_eq!(
            log_file_for("net.ewal.longhand", "Longhand"),
            Some(home.join("Library/Logs/net.ewal.longhand/Longhand.log"))
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_log_file_on_other_systems_is_in_the_local_data_folder() {
        let data = dirs::data_local_dir().expect("a local data folder");
        assert_eq!(
            log_file_for("net.ewal.longhand", "Longhand"),
            Some(
                data.join("net.ewal.longhand")
                    .join("logs")
                    .join("Longhand.log")
            )
        );
    }

    #[test]
    fn the_check_failed_alert_says_the_check_failed() {
        let backups = Path::new("/data/database-backups");
        let (title, body) = question_text(&Question::CheckFailed("disk I/O error".into()), backups);
        assert_eq!(title, "Longhand can't check your notes list");
        assert_eq!(
            body,
            "It may be from a newer version of Longhand and may have work that is \
             not finished.\n\n\
             Rebuild makes a new notes list from your saved notes. Any unfinished \
             work will not be in it. A copy is kept in /data/database-backups.\n\n\
             Details: disk I/O error"
        );
    }

    #[test]
    fn the_failure_alert_has_the_description_the_next_step_the_log_and_the_error() {
        let log = Path::new("/Users/a/Library/Logs/net.ewal.longhand/Longhand.log");
        let (title, body) = failure_text(DEFAULT_FAILURE, Some(log), "permission denied");
        assert_eq!(title, "Longhand can't start");
        assert_eq!(
            body,
            "An error stopped Longhand from starting.\n\n\
             Open Longhand again. If the problem continues, send the log file: \
             /Users/a/Library/Logs/net.ewal.longhand/Longhand.log\n\n\
             Details: permission denied"
        );
        let (_, body) = failure_text(DEFAULT_FAILURE, None, "permission denied");
        assert!(
            body.contains("Open Longhand again. If the problem continues, send the log file.\n\n")
        );
    }

    #[test]
    fn only_the_rebuild_button_rebuilds() {
        assert_eq!(
            choice_from(&MessageDialogResult::Custom(REBUILD.into())),
            Choice::Rebuild
        );
        assert_eq!(
            choice_from(&MessageDialogResult::Custom(QUIT.into())),
            Choice::Quit
        );
        assert_eq!(choice_from(&MessageDialogResult::Cancel), Choice::Quit);
        assert_eq!(choice_from(&MessageDialogResult::Ok), Choice::Quit);
    }
}
