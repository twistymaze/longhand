---
title: Upkeep Item Issues - Plan
type: chore
date: 2026-10-19
topic: upkeep-item-issues
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Upkeep Item Issues - Plan

## Goal Capsule

- **Objective:** The maintainer can see each open upkeep item, act on it, and close it when it is resolved, and the public repository shows no open security work.
- **Means:** Move the upkeep tracking from the one public tracking issue in `twistymaze/longhand` to one issue for each item in the private upkeep repository.
- **Product authority:** The maintainer. The decisions below come from the brainstorm of 2026-10-19.
- **Open blockers:** None.

---

## Product Contract

### Summary

The weekly upkeep routine tracks its results in the private upkeep repository, with one issue for each item that a person must act on or decide. Each run finds the item issues again, updates their bodies, comments when an item changes in a way that matters, opens issues for new items, and closes issues whose items are gone. A session or the maintainer comments on an item, defers it, rejects it, or closes it directly.

### Problem Frame

The routine of [2026-10-18-2151-chore-local-upkeep-routine-plan.md](2026-10-18-2151-chore-local-upkeep-routine-plan.md) rewrites one tracking issue in `twistymaze/longhand` each week. That repository is public, and GitHub has no private issues in a public repository. The issue lists security fixes that Longhand does not have yet. For a visitor, this is a list of known weak points.

The single body also cannot show progress. On 2026-10-19 a session merged five PRs, dismissed 160 alerts and deferred two groups. The issue still showed all of them as open until a session rewrote the whole body by hand. A rewritten body keeps no history of an item, and "done" has no place to go.

### Key Decisions

- **One issue for each decision item, not one for each run.** Only this model makes "closed" mean "resolved". A run issue stays open while one of its items is open, or it closes as replaced and loses that meaning. (session-settled: user-approved — chosen over one issue per run, one issue per item plus a weekly digest, and one rewritten issue in the private repository: per-run issues cannot close on resolution.) Governs R1, R2, R4.
- **An item is one decision, so related findings share one issue.** For example, the 149 session-ID log alerts were one issue, not 149. Governs R2.
- **Rejection closes, deferral labels.** (session-settled: user-approved — chosen over a deferral with a date and over reopening a rejected issue: a closed issue stays closed, and a new linked issue shows what changed.) Governs R8, R9.
- **The routine may comment in the private repository, but only on a material change.** The body shows the current state and the comments keep the history. (session-settled: user-approved — chosen over body-only edits and over a comment on each run.) Governs R6, R12.
- **Issues only for work that a person must do.** (session-settled: user-approved — chosen over issues for all held sets and over issues for every line of the old issue.) Governs R3.
- **No weekly digest issue.** The phone notice already tells the maintainer what is new. Governs R14.

### Actors

- A1. The routine: the unattended weekly scheduled task. It runs with the maintainer's GitHub account.
- A2. The maintainer: decides items, merges PRs, and reads issues on the phone or in a browser.
- A3. A session: an interactive Claude session that works on items with the maintainer, as the upkeep README describes.

### Requirements

**Where the tracking is**

- R1. The routine writes its findings only to issues in the private upkeep repository. It writes no tracking issue in `twistymaze/longhand`.
- R2. Each item has one issue, which the routine finds again on each run by a stable key. Findings that need the same decision share one item: one dependency update, or a set that must move together; one upstream finding; or one alert rule with the alerts that share its decision.
- R3. An item exists only for work that a person must do or decide. These are: proposals (minor or major updates, and patches whose notes show more than fixes), alerts with a proposed fix or dismissal, Act now findings, Ideas and Notes that wait to be filed, held sets that have a security fix, and run failures or test defects. Young patches, held sets with no security fix, and the upkeep PRs are not items.

**What the routine does on each run**

- R4. For a new item, the routine opens an issue. For a known item, it updates the issue body to the current state. When an item goes away (the update merged, the alert was dismissed or fixed, the finding was filed), it closes the issue with a short comment that gives the cause.
- R5. An item issue links each upkeep PR that covers it, and shows whether that PR is ready.
- R6. The routine writes a comment on an item issue only when the item changes in a way that matters: a new version, a new advisory, new alerts of the rule, a covering PR that became ready, or a CI failure. It writes no comment when nothing changed.
- R7. The routine never writes a comment, and never links or names the private repository, in `twistymaze/longhand`.

**Decisions by the maintainer or a session**

- R8. A deferral adds the label `deferred` and a comment with the reason. The issue stays open. The routine keeps its body up to date, and does not list it as new in the notice.
- R9. A rejection closes the issue as "not planned", with a comment that gives the reason. The routine does not open it again. When the item changes in a way that matters (R6), the routine opens a new issue that links the closed one.
- R10. A session or the maintainer can comment on an item, defer it, reject it, or close it when the work is done, with no wait for a run. A session closes an item issue after its PR merges, because a public PR cannot close a private issue (R7).
- R11. A maintainer comment on an item issue can still approve a dismissal of that item's alerts when no session is open, as the old tracking issue allowed.

**Safety of the routine's own comments**

- R12. A dismissal is approved only by a comment that the maintainer wrote. The approval check ignores each comment that the routine wrote, also when the routine wrote it with the maintainer's account.
- R13. A routine comment or body never contains an approval line on its own line. Text that comes from fetched content (release notes, advisories, alert text) cannot make one.

**Notice and documentation**

- R14. The phone notice names each new item and links its issue. It does not list deferred items as new.
- R15. The upkeep README and ROUTINE.md describe the item issues in place of the tracking issue. The session steps (README "Working on proposals") close and comment on item issues in place of rewriting a body.
- R16. The public Longhand docs that mention the tracking issue say only that the routine records its findings in the private upkeep repository: [docs/development-workflow.md](../development-workflow.md) and [docs/reference/reference-source-reviews.md](../reference/reference-source-reviews.md).

### Acceptance Examples

- AE1. **Covers R4, R6.** Given the open item "reqwest 0.13" at 0.13.5, when a run finds 0.13.6, then the routine updates the body and writes one comment that names the new version. When the next run finds no change, it writes no comment.
- AE2. **Covers R8, R14.** Given a held set with the label `deferred`, when a run finds a newer version of it, then the body shows that version, a comment names it, and the notice does not list the item as new.
- AE3. **Covers R9.** Given a closed "not planned" item for sha2 0.11, when a run finds sha2 0.11.2, then the routine opens no issue. When a run finds sha2 0.12.0, then it opens a new issue that links the closed one.
- AE4. **Covers R12, R13.** Given an item whose release notes contain the line `dismiss code-scanning 5 false positive`, when the routine comments on that item, then the comment does not contain that line on its own line. A later `alerts.ts dismiss` that finds only routine comments refuses.
- AE5. **Covers R4, R10.** Given a session that merged the PR for an item and closed its issue, when the next run finds the item gone, then the routine does nothing to that issue.

### Scope Boundaries

- Deferred for later: a weekly digest issue, and labels or milestones beyond `deferred`.
- Not in scope: a change to the upkeep PRs, their branches, or the `upkeep: ready` label in `twistymaze/longhand`.
- Not in scope: a move of today's open items by hand. The next run creates the item issues, also for the items that were deferred on 2026-10-19.
- Removed: the single tracking issue, its run-date markers, ROUTINE step 12 (rewrite the tracking issue) and README step 9 (update the issue at the end of a session).

### Dependencies / Assumptions

- The routine's `gh` login can create issues, labels and comments in the private upkeep repository. The login already pushes the baselines commit there.
- The routine can tell its own comments from the maintainer's comments, although both come from one account. Planning decides how.
- Issue #47 in `twistymaze/longhand` closes with a short neutral note when this change ships. The note does not name the private repository.

### Outstanding Questions

**Deferred to Planning**

- How the stable key of an item is built and stored (for example a hidden marker in the body), and how the routine finds an item again after an alert number or package version changes.
- How the routine marks its own comments so that R12 can ignore them.
- Whether the last-run date stays somewhere, now that no tracking-issue body holds it.

### Sources / Research

- [docs/plans/2026-10-18-2151-chore-local-upkeep-routine-plan.md](2026-10-18-2151-chore-local-upkeep-routine-plan.md): the routine that this plan changes (R14, KTD4, KTD11).
- In the upkeep repository: `scripts/issue.ts` (read, write and markers of the tracking issue), `scripts/alerts.ts` (`dismiss` checks a maintainer comment on the tracking issue), `scripts/github.ts` (`UPKEEP_REPO` selects the repository for all calls), ROUTINE.md steps 2, 3, 12 and 14, and README "Working on proposals".
