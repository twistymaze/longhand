//! Longhand's library: the notes in every retained Destination, whichever
//! computer saved them (KTD8). The scan reads each note folder's
//! `session.md`, reconcile applies a scan to the store, and the refresh
//! brings one row up to date from its folder. Nothing here depends on
//! Tauri.

pub mod conflict;
pub mod reconcile;
pub mod refresh;
pub mod scan;
