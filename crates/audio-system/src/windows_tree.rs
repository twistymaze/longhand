//! The Windows process tree, kept free of Windows calls so that it is
//! tested on every platform (KTD4, KTD5).
//!
//! A per-process loopback client records a root process and all the
//! processes that it starts. [`ProcessTable::roots`] gives one root for
//! each process tree of the apps in the app set. [`ProcessTable::in_chain`]
//! tells whether a process or one of its parents belongs to an app, for
//! example a Teams helper process that plays the call.

use std::collections::{HashMap, HashSet};

/// One running process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    /// The PID of the process that started it. 0 when there is none.
    pub parent: u32,
    /// The app ID of its executable ([`crate::apps::windows_app_id`]).
    pub app_id: String,
}

/// The running processes at one time.
#[derive(Debug, Clone, Default)]
pub struct ProcessTable {
    by_pid: HashMap<u32, ProcessEntry>,
}

impl ProcessTable {
    pub fn new(entries: Vec<ProcessEntry>) -> Self {
        Self {
            by_pid: entries.into_iter().map(|e| (e.pid, e)).collect(),
        }
    }

    /// The process with `pid`, then its parent, then the parent of that
    /// process, and so on. The chain stops at a PID that is not in the
    /// table and at a PID that is already in the chain (Windows can give a
    /// PID again to a new process).
    pub fn chain(&self, pid: u32) -> Vec<&ProcessEntry> {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut next = pid;
        while seen.insert(next) {
            let Some(entry) = self.by_pid.get(&next) else {
                break;
            };
            chain.push(entry);
            next = entry.parent;
        }
        chain
    }

    /// Whether the process with `pid`, or a parent in its chain, belongs
    /// to the app with `app_id`.
    pub fn in_chain(&self, pid: u32, app_id: &str) -> bool {
        self.chain(pid).iter().any(|e| e.app_id == app_id)
    }

    /// The root processes of the apps in `apps`, sorted by PID, with their
    /// app IDs. A root is a process of an app in `apps` whose parents
    /// belong to no app in `apps`. The process with `own_pid` (Longhand)
    /// is never a root.
    pub fn roots(&self, apps: &[String], own_pid: u32) -> Vec<(u32, String)> {
        let mut roots: Vec<(u32, String)> = self
            .by_pid
            .values()
            .filter(|e| e.pid != own_pid && apps.contains(&e.app_id))
            .filter(|e| {
                !self.chain(e.pid)[1..]
                    .iter()
                    .any(|parent| apps.contains(&parent.app_id))
            })
            .map(|e| (e.pid, e.app_id.clone()))
            .collect();
        roots.sort_unstable();
        roots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pid: u32, parent: u32, app_id: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent,
            app_id: app_id.to_owned(),
        }
    }

    fn ids(apps: &[&str]) -> Vec<String> {
        apps.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn a_helper_whose_parent_chain_includes_teams_counts_as_teams() {
        let table = ProcessTable::new(vec![
            entry(4, 0, "system"),
            entry(100, 4, "explorer"),
            entry(200, 100, "ms-teams"),
            entry(300, 200, "msedgewebview2"),
            entry(400, 300, "msedgewebview2"),
            entry(500, 100, "msedgewebview2"),
        ]);
        assert!(table.in_chain(200, "ms-teams"), "Teams itself");
        assert!(table.in_chain(400, "ms-teams"), "a helper of a helper");
        assert!(
            !table.in_chain(500, "ms-teams"),
            "the same executable started by another app"
        );
        assert!(!table.in_chain(999, "ms-teams"), "an unknown process");
    }

    #[test]
    fn a_parent_cycle_ends_the_chain() {
        let table = ProcessTable::new(vec![entry(10, 20, "a"), entry(20, 10, "b")]);
        let chain: Vec<u32> = table.chain(10).iter().map(|e| e.pid).collect();
        assert_eq!(chain, vec![10, 20]);
        assert!(!table.in_chain(10, "ms-teams"));
    }

    #[test]
    fn each_app_has_one_root_for_each_process_tree() {
        let table = ProcessTable::new(vec![
            entry(100, 1, "explorer"),
            entry(200, 100, "chrome"),
            entry(201, 200, "chrome"),
            entry(202, 200, "chrome"),
            entry(300, 100, "zoom"),
            entry(301, 300, "zoom"),
            entry(400, 100, "chrome"),
            entry(500, 100, "discord"),
        ]);
        assert_eq!(
            table.roots(&ids(&["chrome", "zoom"]), 1),
            vec![
                (200, "chrome".to_owned()),
                (300, "zoom".to_owned()),
                (400, "chrome".to_owned()),
            ]
        );
        assert!(table.roots(&[], 1).is_empty());
        assert!(table.roots(&ids(&["slack"]), 1).is_empty());
    }

    #[test]
    fn an_app_started_by_another_listed_app_is_in_that_tree() {
        // A loopback client of Teams also records the processes that Teams
        // starts, so a second client for Zoom would record Zoom twice.
        let table = ProcessTable::new(vec![entry(200, 1, "ms-teams"), entry(300, 200, "zoom")]);
        assert_eq!(
            table.roots(&ids(&["ms-teams", "zoom"]), 1),
            vec![(200, "ms-teams".to_owned())]
        );
        assert_eq!(
            table.roots(&ids(&["zoom"]), 1),
            vec![(300, "zoom".to_owned())]
        );
    }

    #[test]
    fn longhand_is_never_a_root() {
        let table = ProcessTable::new(vec![entry(42, 1, "longhand"), entry(43, 1, "zoom")]);
        assert_eq!(
            table.roots(&ids(&["longhand", "zoom"]), 42),
            vec![(43, "zoom".to_owned())]
        );
    }
}
