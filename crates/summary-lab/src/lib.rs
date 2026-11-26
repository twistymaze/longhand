//! The summary lab: a dev harness that compares agent summaries with
//! the summaries of the app's summary pipeline (R16). The app does not depend on this crate.

pub mod config;
pub mod cross;
pub mod hosts;
pub mod judge;
pub mod report;
pub mod run;
pub mod run_config;
pub mod snapshot;
pub mod versus;
