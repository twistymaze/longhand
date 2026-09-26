//! Every writer follows the folder (KTD3, KD4, R9). Before a writer changes
//! a saved note (rename, notes, Regenerate Summary, Transcribe Again,
//! Resume, the save of a run), it brings its row up to date from the note
//! folder. So a title, notes or a transcript that another computer wrote is
//! in the row, and the writer does not undo it.

use std::path::Path;

use library::refresh::{self, Refresh, RefreshError, Scope};
use library::scan::{MARKER, READ_TIMEOUT};
use session_core::Store;
use session_core::jobs::busy;

use crate::StageError;

/// Bring the row `session_id` up to date from its home folder. A row with
/// no home folder does not change. `claimed` tells whether other work (a
/// summary regeneration or a delete) holds a claim on the session. A row
/// that is busy takes only its title and notes, because a run owns its
/// recognition results ([`Scope::TitleAndNotes`]). The caller holds the
/// store lock.
pub fn follow_folder(
    store: &mut Store,
    session_id: &str,
    claimed: bool,
) -> Result<Refresh, StageError> {
    let record = store
        .session(session_id)?
        .ok_or_else(|| StageError::Failed(format!("no session {session_id}")))?;
    let Some(home) = record.home_path.clone() else {
        return Ok(Refresh::default());
    };
    let scope = if busy(&record, claimed) {
        Scope::TitleAndNotes
    } else {
        Scope::All
    };
    follow_scoped(store, session_id, Path::new(&home), scope)
}

/// Bring the parts of the row `session_id` that `scope` names up to date
/// from the note folder `folder`. A folder that cannot be read leaves the
/// row as it is: the writer then reports what it cannot do. Only a store
/// error fails. The caller holds the store lock.
pub fn follow_scoped(
    store: &mut Store,
    session_id: &str,
    folder: &Path,
    scope: Scope,
) -> Result<Refresh, StageError> {
    match refresh::refresh_scoped(store, session_id, folder, READ_TIMEOUT, scope) {
        Ok(refreshed) => Ok(refreshed),
        Err(e @ RefreshError::Store(_)) => Err(e.into()),
        Err(e) => {
            log::warn!(
                "could not update {session_id} from {}: {e}",
                folder.display()
            );
            Ok(Refresh::default())
        }
    }
}

/// Write the library marker (KTD5) in the Destination `root` when it has
/// none: Longhand saves to the root now. A marker that cannot be written
/// does not stop the save; the next library scan writes it.
pub fn mark_destination(root: &Path) {
    if !root.join(MARKER).exists() {
        library::scan::write_marker(root);
    }
}
