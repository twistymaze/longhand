//! The changes that an offered update gives (U6 of
//! `docs/plans/2026-10-28-0833-feat-monthly-changelog-plan.md`; R13, R14,
//! KTD5). The app downloads the offered release's `changelog.json` once
//! for each version. It lists the offered entries that are later than the
//! bundled head commit and that the bundled changelog does not have. A
//! failure gives "not available" (None), and the window then shows the link
//! to the full changelog.

use std::future::Future;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::commands::changelog::{self, Changelog, ChangelogEntry};

/// The largest `changelog.json` that the app reads.
pub(super) const MAX_BYTES: usize = 5 * 1024 * 1024;

/// The download timeout, the same as the timeout of an update check.
const TIMEOUT: Duration = super::CHECK_TIMEOUT;

/// The offered changes, or None when they are not available.
pub type OfferedChanges = Option<Vec<ChangelogEntry>>;

/// The address of the changelog of a release (KTD5).
fn address(version: &str) -> String {
    format!("https://github.com/twistymaze/longhand/releases/download/v{version}/changelog.json")
}

/// The entries of `offered` that KTD5 selects, in the order of `offered`:
/// the commit time is later than `head_time`, and `bundled` does not have
/// the commit id. The line text has no effect (AE9). The result is None
/// when either changelog is not available.
pub(super) fn select(
    offered: &Changelog,
    head_time: Option<i64>,
    bundled: impl Fn(&str) -> bool,
) -> OfferedChanges {
    let head_time = head_time?;
    if !offered.available {
        return None;
    }
    Some(
        offered
            .entries
            .iter()
            .filter(|entry| entry.time > head_time && !bundled(&entry.id))
            .cloned()
            .collect(),
    )
}

/// Read a download result. A failed request, a status other than success,
/// a body larger than [`MAX_BYTES`] or a body that `changelog::parse` does
/// not accept gives "not available".
pub(super) fn read_response(response: Result<(u16, Vec<u8>), String>) -> Changelog {
    let body = match response {
        Ok((status, body)) if (200..300).contains(&status) && body.len() <= MAX_BYTES => body,
        Ok((status, body)) => {
            log::info!(
                "updates: the changelog download gave status {status} and {} bytes",
                body.len()
            );
            return Changelog::not_available();
        }
        Err(e) => {
            log::info!("updates: the changelog download failed: {e}");
            return Changelog::not_available();
        }
    };
    match std::str::from_utf8(&body) {
        Ok(text) => changelog::parse(text),
        Err(_) => Changelog::not_available(),
    }
}

/// Download the body of `url`. The download stops after one byte more than
/// [`MAX_BYTES`], so [`read_response`] rejects the body.
async fn download(url: &str) -> Result<(u16, Vec<u8>), String> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client.get(url).send().await.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_BYTES {
            break;
        }
    }
    Ok((status, body))
}

/// The offered changes of one version. The lock stays held during the
/// download, so a second call for the same version waits and does not
/// download again.
#[derive(Default)]
pub(super) struct Cache {
    slot: Mutex<Option<(String, OfferedChanges)>>,
}

impl Cache {
    /// The changes of `version`. Only a call for another version calls
    /// `fetch`.
    pub(super) async fn get<F, Fut>(&self, version: &str, fetch: F) -> OfferedChanges
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = OfferedChanges>,
    {
        let mut slot = self.slot.lock().await;
        if let Some((kept, changes)) = slot.as_ref()
            && kept == version
        {
            return changes.clone();
        }
        let changes = fetch().await;
        *slot = Some((version.to_string(), changes.clone()));
        changes
    }
}

/// Download, read and select the changes of `version`. A version that is
/// not a semantic version gives "not available", so the address contains
/// only a version.
pub(super) async fn fetch(version: &str) -> OfferedChanges {
    if semver::Version::parse(version).is_err() {
        return None;
    }
    let offered = read_response(download(&address(version)).await);
    select(
        &offered,
        changelog::bundled_head_time(),
        changelog::bundled_contains,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::commands::changelog::ChangelogHead;

    fn entry(id: &str, time: i64, line: &str) -> ChangelogEntry {
        ChangelogEntry {
            id: id.to_string(),
            time,
            month: "2026-10".into(),
            group: "Fixes".into(),
            rank: 1,
            line: line.to_string(),
            pr: None,
        }
    }

    fn available(entries: Vec<ChangelogEntry>) -> Changelog {
        Changelog {
            available: true,
            head: Some(ChangelogHead {
                id: "head".into(),
                time: 1000,
            }),
            entries,
        }
    }

    fn ids(changes: &OfferedChanges) -> Vec<String> {
        changes
            .as_ref()
            .expect("available")
            .iter()
            .map(|e| e.id.clone())
            .collect()
    }

    /// Entries A to F at times 10 to 60.
    fn a_to_f() -> Changelog {
        available(
            ["A", "B", "C", "D", "E", "F"]
                .iter()
                .zip(1..)
                .map(|(id, n)| entry(id, n * 10, &format!("Line {id}")))
                .collect(),
        )
    }

    fn bundled(ids: &[&str]) -> impl Fn(&str) -> bool {
        let ids: HashSet<String> = ids.iter().map(|s| s.to_string()).collect();
        move |id: &str| ids.contains(id)
    }

    #[test]
    fn covers_ae4_the_changes_after_the_bundled_build_show_in_offered_order() {
        // The bundled build has A and B, and its head commit is B.
        let changes = select(&a_to_f(), Some(20), bundled(&["A", "B"]));
        assert_eq!(ids(&changes), ["C", "D", "E", "F"]);
    }

    #[test]
    fn covers_ae9_an_entry_with_a_bundled_id_and_other_text_does_not_show() {
        let offered = available(vec![
            entry("A", 10, "New text of A"),
            entry("B", 30, "New text of B"),
            entry("C", 40, "Line C"),
        ]);
        // B is later than the head time here, so only the id check removes it.
        let changes = select(&offered, Some(20), bundled(&["A", "B"]));
        assert_eq!(ids(&changes), ["C"]);
    }

    #[test]
    fn an_old_commit_that_a_new_rule_includes_does_not_show() {
        // X is a commit before the bundled head that the bundled build did
        // not list, because its type was not included then.
        let offered = available(vec![entry("X", 15, "Old commit"), entry("C", 30, "Line C")]);
        let changes = select(&offered, Some(20), bundled(&["A", "B"]));
        assert_eq!(ids(&changes), ["C"]);
    }

    #[test]
    fn an_offered_file_with_nothing_new_gives_an_empty_list() {
        let changes = select(
            &a_to_f(),
            Some(60),
            bundled(&["A", "B", "C", "D", "E", "F"]),
        );
        assert_eq!(changes, Some(Vec::new()));
    }

    #[test]
    fn a_bundled_changelog_that_is_not_available_gives_not_available() {
        assert_eq!(select(&a_to_f(), None, bundled(&[])), None);
    }

    #[test]
    fn an_offered_changelog_that_is_not_available_gives_not_available() {
        assert_eq!(select(&changelog::parse(""), Some(20), bundled(&[])), None);
    }

    const VALID: &str = r#"{"format":1,"available":true,
        "head":{"id":"cccc","time":100},
        "entries":[{"id":"bbbb","time":90,"month":"2026-10","group":"Fixes",
                    "rank":1,"line":"Fix the timer (#12)","pr":12}]}"#;

    #[test]
    fn a_valid_response_gives_its_changelog() {
        let changelog = read_response(Ok((200, VALID.as_bytes().to_vec())));
        assert!(changelog.available);
        assert_eq!(changelog.entries.len(), 1);
    }

    #[test]
    fn a_failed_request_gives_not_available() {
        assert!(!read_response(Err("no connection".into())).available);
        assert!(!read_response(Ok((404, b"Not Found".to_vec()))).available);
        assert!(!read_response(Ok((500, VALID.as_bytes().to_vec()))).available);
    }

    #[test]
    fn a_body_larger_than_the_limit_gives_not_available() {
        // A valid file with padding after it: only the size makes it fail.
        let mut body = VALID.as_bytes().to_vec();
        body.resize(MAX_BYTES + 1, b' ');
        assert!(!read_response(Ok((200, body.clone()))).available);
        body.truncate(MAX_BYTES);
        assert!(read_response(Ok((200, body))).available);
    }

    #[test]
    fn an_unknown_format_or_text_that_is_not_utf8_gives_not_available() {
        let format_2 = VALID.replacen(r#""format":1"#, r#""format":2"#, 1);
        assert!(!read_response(Ok((200, format_2.into_bytes()))).available);
        assert!(!read_response(Ok((200, vec![0xff, 0xfe]))).available);
    }

    #[test]
    fn the_address_names_the_release_of_the_version() {
        assert_eq!(
            address("1.0.25"),
            "https://github.com/twistymaze/longhand/releases/download/v1.0.25/changelog.json"
        );
    }

    #[tokio::test]
    async fn a_version_that_is_not_a_semantic_version_gives_not_available() {
        assert_eq!(fetch("../../evil").await, None);
    }

    #[tokio::test]
    async fn a_second_call_for_the_same_version_does_not_download_again() {
        let cache = Cache::default();
        let downloads = AtomicUsize::new(0);
        let fetch = || async {
            downloads.fetch_add(1, Ordering::SeqCst);
            Some(vec![entry("C", 30, "Line C")])
        };
        let first = cache.get("1.0.25", fetch).await;
        let second = cache.get("1.0.25", fetch).await;
        assert_eq!(downloads.load(Ordering::SeqCst), 1);
        assert_eq!(first, second);
        // A failed result also stays for that version.
        let failed = || async {
            downloads.fetch_add(1, Ordering::SeqCst);
            None
        };
        assert_eq!(cache.get("1.0.26", failed).await, None);
        assert_eq!(cache.get("1.0.26", failed).await, None);
        assert_eq!(downloads.load(Ordering::SeqCst), 2);
    }
}
