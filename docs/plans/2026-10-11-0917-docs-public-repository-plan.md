---
title: Public Repository - Plan
type: docs
date: 2026-10-11
topic: public-repository
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
---

# Public Repository - Plan

## Goal Capsule

- **Objective:** Visitors can follow Longhand on GitHub. They understand what it is and its licence, and they know that the project does not accept outside contributions. The current files are general and show no secrets or private files. The history shows no secrets and no exposure that the user did not accept.
- **Product authority:** This Product Contract. The audit items in `docs/ideas/someday/pre-public-repo-audit.md` are input. The About-pane part of `docs/ideas/someday/about-longhand-and-readme.md` is not active scope.
- **Open blockers:** None.

---

## Product Contract

### Summary

Add a README, an MIT licence and a contributing document. Make the docs general and correct for a public repository. Check the history, the refs and the Actions logs for private data. Then make the repository public with a checklist that the user follows, and verify the lockdown settings.

### Problem Frame

The repository is private, and the user wants other people to follow its development. A public repository cannot usefully be made private again, because anyone can clone it while it is public. So everything that becomes visible must be correct before the switch.

Today a visitor would see a list of files with no README, no licence and no statement about contributions. `STRATEGY.md` names the user's employer and says "Not a public project". `docs/preview-builds.md` says "The source repository stays private".

On 2026-10-11, the history of `main` was written again to remove home infrastructure details: the Proxmox and NAS host names, the VLAN number, router rules, storage names and hardware. GitHub still keeps the old commits. Old Actions logs show their IDs, and so does the force-push entry in the PR #54 timeline. The host names of the self-hosted runners are generic and are not sensitive.

### Key Decisions

- **Keep this repository and do a light cleanup.** Nobody else has ever had access, and the user accepts that a person who follows the PR #54 timeline can read the old infrastructure details. Governs R10, R11. (session-settled: user-directed — chosen over a new clean repository, a GitHub Support request, and a one-commit restart, after the review showed that the old commits hold infrastructure host names and that the PR #54 timeline links to them)
- **Issues stay on, with creation limited to collaborators.** The user wants to use Renovate's dependency dashboard issue later. The "Issue permissions: Collaborators only" setting is already on. Governs R15. (session-settled: user-directed — chosen over turning issues off: Renovate's dashboard needs issues)
- **Make the docs general, and accept the employer name in history.** The aim is general docs, not to hide the employer: the user's GitHub profile already names it. Governs R7, R8. (session-settled: user-directed — chosen over a history rewrite that removes the employer name from every commit)
- **The README says that official releases are coming soon, and gives build-from-source steps.** Preview builds are public in `twistymaze/longhand-builds`, but they are not official releases, so the README does not link them. Governs R2. (session-settled: user-directed — chosen over "no downloads yet", which is false, and over a link to the preview builds)
- **MIT licence, copyright holder Erv Walter.** All borrowed code is MIT or Apache-2.0, and the dependency gate allows only permissive licences. The MPL-2.0 exceptions are file-level and come through Tauri. Governs R5. (session-settled: user-directed — chosen over "twistymaze" or "Erv Walter and Longhand contributors" as the holder)

### Requirements

**Front door**

- R1. A `README.md` at the repository root introduces Longhand to a visitor. It describes the concept from `STRATEGY.md`: a lazy notes scratchpad, a local transcript as the system of record, and data that stays on the user's machine. It uses the terms in `CONCEPTS.md`.
- R2. The README says that official releases are coming soon. It then gives short build-from-source steps for people who want to try Longhand.
- R3. The README links to the licence and to the contributing document. It includes no screenshot.
- R4. The README is not complete until the user approves it. The implementation presents the draft README to the user for manual review and applies the user's changes before the work is called complete.
- R5. A `LICENSE` file at the repository root holds the MIT licence text with the line "Copyright (c) 2026 Erv Walter".
- R6. A `CONTRIBUTING.md` says warmly that Longhand is a young project and is not looking for outside contributions yet. It thanks the visitor for their interest and invites them to follow along. It does not sound like a refusal.

**General docs**

- R7. `STRATEGY.md` describes its users without naming an employer. Its "Not a public project" line changes to say that the repository is public to follow but closed to contributions.
- R8. No tracked file names the user's employer. A search of all tracked files finds every match, including `docs/plans/2026-09-24-1956-feat-ux-redesign-plan.md`.
- R9. `docs/preview-builds.md` no longer says that the source repository stays private.

**History and logs**

- R10. A scan of the full history and all refs on GitHub, including `refs/pull/*`, looks for secrets, committed private paths from the `CLAUDE.md` list, and home infrastructure details. The scan for infrastructure details uses the pattern in `docs/solutions/workflow-issues/keep-home-network-details-out-of-the-repository.md` and the known infrastructure host names. Before R11 deletes the old Actions logs, the commit IDs that they show are recorded, and the scan includes those commits. A real credential that matches is revoked or rotated before the switch, because `refs/pull/*` and unreachable commits stay readable. A match in the current files is removed. A match only in history is shown to the user, who decides before the switch; the known old commits are the accepted exposure.
- R11. The Actions run logs of `worktree-self-hosted-ci-runners` from 2026-10-10 and 2026-10-11 are deleted.

**The switch**

- R12. A checklist lists the steps before and after the switch in order. The user changes the visibility in the GitHub settings. An agent does not.
- R13. Before the switch, the self-hosted runner settings for a public repository, from the README of the private repository `twistymaze/homelab-ci`, are applied. CI selects these runners through `vars.MACOS_RUNNER` and `vars.WINDOWS_RUNNER` in `.github/workflows/ci.yml`. Code from people outside the repository must not run on the home runners.
- R14. After the switch, a check confirms that pull request creation is still limited to collaborators.
- R15. After the switch, a check confirms that issue creation is still limited to collaborators. Wiki, Discussions and Projects stay off.
- R16. The idea `pre-public-repo-audit` moves to `docs/ideas/done/`. The idea `about-longhand-and-readme` stays in `someday`, with only its About-pane part.

### Scope Boundaries

- The About section in Settings and the macOS About panel are out of scope. They stay in the `about-longhand-and-readme` idea.
- Renovate setup is deferred. When Renovate is set up, confirm that the collaborators-only issue setting does not stop its app from opening the dashboard issue.
- A README screenshot is deferred. It must use a note with no real names.
- No `SECURITY.md`, issue templates or PR templates. Outside users cannot file issues or PRs.
- Forking cannot be turned off for a public repository. MIT allows forks.
- No GitHub Support request to remove the unreachable commits, and no history rewrite for the employer name.
- The logs of other runs on the home runners are not checked: they show only the generic runner host names.

### Dependencies / Assumptions

- The REST API does not show the issue creation setting (`gh api repos/twistymaze/longhand` returns only `pull_request_creation_policy`). Checks for R15 use the settings page.
- The `homelab-ci` README covers the approval setting for workflows from external contributors. If it does not, R13 adds that setting.

### Outstanding Questions

**Deferred to Planning**

- Which tool scans the history (R10), and how to fetch all `refs/pull/*` for the scan.
- Which build steps the README gives (toolchain versions, macOS signing note), taken from `docs/development-workflow.md`.
- Whether the repository description and topics on GitHub get set at the switch.
- Whether the checklist turns on GitHub secret scanning and push protection after the switch.

### Sources / Research

- `docs/ideas/someday/pre-public-repo-audit.md`: the audit items and the old-commit context.
- `docs/ideas/someday/about-longhand-and-readme.md`: the README idea and its About-pane part.
- `docs/solutions/workflow-issues/keep-home-network-details-out-of-the-repository.md`: what the history rewrite removed, and the search pattern.
- `docs/preview-builds.md`: the public preview builds in `twistymaze/longhand-builds`.
- `deny.toml`, `about.toml`, `licences/`, and each `PROVENANCE.md`: licence policy and borrowed-code records (anarlog MIT, none from its `enterprise/` folder; Handy MIT; DTLN-aec MIT; NeMo Apache-2.0).
- `STRATEGY.md` and `CONCEPTS.md`: the description and terms for the README.
