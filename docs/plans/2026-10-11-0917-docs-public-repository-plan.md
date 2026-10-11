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

- **Objective:** Visitors can follow Longhand on GitHub at `twistymaze/longhand`. They understand what it is and its licence, and they know that the project does not accept outside contributions. The public repository shows no secrets, no private files, no home infrastructure details and no exposure that the user did not accept.
- **Product authority:** This Product Contract. The audit items in `docs/ideas/someday/pre-public-repo-audit.md` are input. The About-pane part of `docs/ideas/someday/about-longhand-and-readme.md` is not active scope.
- **Open blockers:** None.

---

## Product Contract

### Summary

Add a README, an MIT licence and a contributing document, and make the docs general and correct for a public repository. Rename the current repository to a private archive. Create a fresh repository with the same name from the current `main` history, set it up by script, and make it public when it works.

### Problem Frame

The repository is private, and the user wants other people to follow its development. A public repository cannot usefully be made private again, because anyone can clone it while it is public. So everything that becomes visible must be correct before the switch.

Today a visitor would see a list of files with no README, no licence and no statement about contributions. `STRATEGY.md` names the user's employer and says "Not a public project". `docs/preview-builds.md` says "The source repository stays private".

On 2026-10-11, the history of `main` was written again to remove home infrastructure details: the Proxmox and NAS host names, the VLAN number, router rules, storage names and hardware. The current repository still keeps the old commits. Its PR refs, its Actions logs and the force-push entry in the PR #54 timeline point to them, and no cleanup inside that repository can remove them. The host names of the self-hosted runners are generic and are not sensitive.

### Key Decisions

- **Publish a fresh repository, and keep the current one as a private archive.** The old commits stay only in a repository that never becomes public. All 7 `preview` secrets can be recovered on this machine or from Apple, so the setup costs no irreplaceable credential. Governs R18, R19, R20, R21, R22. (session-settled: user-directed — chosen over keeping the repository with a light cleanup that accepts the old commits as exposure, after the user confirmed that each secret is recoverable)
- **The archive is named `twistymaze/longhand-pre-public`.** The name tells what period it holds. Governs R18. (session-settled: user-directed — chosen over `longhand-archive`, which the user found unclear, and over `longhand-private-history`, `longhand-old-private` and `longhand-2026-private`)
- **PR references in tracked files become plain text.** The fresh repository numbers its PRs from 1, so "PR #53" would point at an unrelated PR, and links to the private archive show a 404 to visitors. Governs R17. (session-settled: user-directed — chosen over links to the archive and over no change)
- **Issues stay on, with creation limited to collaborators.** The user wants to use Renovate's dependency dashboard issue later. Governs R15, R21. (session-settled: user-directed — chosen over turning issues off: Renovate's dashboard needs issues)
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
- R17. Each reference to a PR of the current repository in a tracked file, such as "PR #53", becomes plain text that names it as an archived PR, such as "archived PR 53".

**History check**

- R10. Before the push to the new repository, a scan of the `main` history looks for secrets, committed private paths from the `CLAUDE.md` list, and home infrastructure details. The scan for infrastructure details uses the pattern in `docs/solutions/workflow-issues/keep-home-network-details-out-of-the-repository.md` and the known infrastructure host names. The host names are never written into a tracked file. A real credential that matches is revoked or rotated. A match in the current files is removed. A match only in history is shown to the user, who decides before the push.

**Fresh repository**

- R18. The current repository is renamed to `twistymaze/longhand-pre-public`. It stays private and keeps its PRs, its Actions history and its old commits.
- R19. A new private repository `twistymaze/longhand` receives the `main` history. No other branch moves.
- R20. A script copies the settings of the archive to the new repository, then reads both back and shows that they match: the repository variables, the ruleset, the merge and feature settings, the PR creation policy, the `preview` environment without its secret values, and access to the `longhand-checks` runner group.
- R21. The user enters the 7 `preview` secret values and the issue creation setting. The implementation gives one command for each secret, which reads the value from a file or the password manager. The implementation never reads or prints a secret value.
- R22. Before the switch, one CI run and one preview publish pass on the new repository.
- R23. The local checkout uses the new repository as its remote.

**The switch**

- R12. A checklist lists the steps before and after the switch in order. The user changes the visibility in the GitHub settings. An agent does not.
- R13. Before the switch, the self-hosted runner settings for a public repository, from the README of the private repository `twistymaze/homelab-ci`, are applied. CI selects these runners through `vars.MACOS_RUNNER` and `vars.WINDOWS_RUNNER` in `.github/workflows/ci.yml`. Code from people outside the repository must not run on the home runners. A setting that GitHub allows only on a public repository is set immediately after the switch.
- R14. After the switch, a check confirms that pull request creation is still limited to collaborators.
- R15. After the switch, a check confirms that issue creation is still limited to collaborators. Wiki, Discussions and Projects stay off.
- R16. The idea `pre-public-repo-audit` moves to `docs/ideas/done/`. The idea `about-longhand-and-readme` stays in `someday`, with only its About-pane part.

### Scope Boundaries

- The About section in Settings and the macOS About panel are out of scope. They stay in the `about-longhand-and-readme` idea.
- PRs, issues, Actions history and stars do not move to the new repository. They stay in the archive.
- Merge commit messages in the pushed history keep text such as "Merge pull request #54". GitHub links them to the new PR with that number. Changing them needs a history rewrite.
- The employer name stays in the pushed history.
- Renovate setup is deferred. When Renovate is set up, confirm that the collaborators-only issue setting does not stop its app from opening the dashboard issue.
- A README screenshot is deferred. It must use a note with no real names.
- No `SECURITY.md`, issue templates or PR templates. Outside users cannot file issues or PRs.
- Forking cannot be turned off for a public repository. MIT allows forks.

### Dependencies / Assumptions

- The REST API does not show the issue creation setting (`gh api repos/twistymaze/longhand` returns only `pull_request_creation_policy`). Checks for R15 use the settings page.
- The `homelab-ci` README covers the approval setting for workflows from outside contributors. If it does not, R13 adds that setting.
- The 7 `preview` secrets are recoverable: the updater key at `~/.tauri/longhand-updater.key` matches the public key in `src-tauri/tauri.conf.json`, the Developer ID identity is in the keychain, the `.p8` file is in `~/.appstoreconnect/private_keys/`, the API key ID and issuer ID show in App Store Connect, and a new `BUILDS_TOKEN` takes a minute to make.
- The update manifest URL is in `twistymaze/longhand-builds`, so installed preview builds do not depend on the source repository.

### Outstanding Questions

**Deferred to Planning**

- Which tool scans the history (R10).
- Which build steps the README gives (toolchain versions, macOS signing note), taken from `docs/development-workflow.md`.
- How the CI path filter in `.github/workflows/ci.yml` stops a `LICENSE` change from starting CI and a preview publish. The filter now excludes only Markdown and the docs folders.
- Whether the repository description and topics on GitHub get set at the switch.
- Whether the checklist turns on GitHub secret scanning and push protection after the switch.

### Sources / Research

- `docs/ideas/someday/pre-public-repo-audit.md`: the audit items and the old-commit context.
- `docs/ideas/someday/about-longhand-and-readme.md`: the README idea and its About-pane part.
- `docs/solutions/workflow-issues/keep-home-network-details-out-of-the-repository.md`: what the history rewrite removed, and the search pattern.
- `docs/preview-builds.md`: the public preview builds, the `preview` environment and the source of each secret.
- `deny.toml`, `about.toml`, `licences/`, and each `PROVENANCE.md`: licence policy and borrowed-code records (anarlog MIT, none from its `enterprise/` folder; Handy MIT; DTLN-aec MIT; NeMo Apache-2.0).
- `STRATEGY.md` and `CONCEPTS.md`: the description and terms for the README.
