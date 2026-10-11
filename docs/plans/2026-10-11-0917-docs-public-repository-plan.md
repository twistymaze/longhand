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

- **Objective:** Visitors can follow Longhand on GitHub. They understand what it is and its licence, and they know that the project does not accept outside contributions. The public repository shows no secrets, private files or personal details.
- **Product authority:** This Product Contract. The audit items in `docs/ideas/someday/pre-public-repo-audit.md` are input. The About-pane part of `docs/ideas/someday/about-longhand-and-readme.md` is not active scope.
- **Open blockers:** None.

---

## Product Contract

### Summary

Add a README, an MIT licence and a contributing document. Remove employer details from the docs. Check the history, the refs and the Actions logs for private data. Then make the repository public with a checklist that the user follows, and verify the lockdown settings.

### Problem Frame

The repository is private, and the user wants other people to follow its development. A public repository cannot usefully be made private again, because anyone can clone it while it is public. So everything that becomes visible must be correct before the switch.

Today a visitor would see a list of files with no README, no licence and no statement about contributions. `STRATEGY.md` names the user's employer and says "Not a public project". On 2026-10-11, the history of `main` was written again to remove home network details. Those details are only slightly sensitive (IP addresses). GitHub still keeps the old commits, and old Actions logs show their IDs.

### Key Decisions

- **Keep this repository and do a light cleanup.** The old details are only slightly sensitive, and nobody else has ever had access. A new repository or a GitHub Support request would cost more than the risk is worth. Governs R8, R9. (session-settled: user-directed — chosen over a new clean repository, a GitHub Support request, and a one-commit restart: the old data is low-sensitivity IP addresses)
- **Issues stay on, with creation limited to collaborators.** The user wants to use Renovate's dependency dashboard issue later. The "Issue permissions: Collaborators only" setting is already on. Governs R13. (session-settled: user-directed — chosen over turning issues off: Renovate's dashboard needs issues)
- **Remove employer details from the docs.** Docs describe the user and peers in general terms, not by employer. Governs R6, R7. (session-settled: user-approved — chosen over keeping the employer named, or no change)
- **The README tells people to build from source.** There is no public release, and `STRATEGY.md` defers distribution until someone asks. Governs R2. (session-settled: user-approved — chosen over no install section, or public preview builds)
- **MIT licence, copyright holder Erv Walter.** All borrowed code is MIT or Apache-2.0, and the dependency gate allows only permissive licences. The MPL-2.0 exceptions are file-level and come through Tauri. Governs R4. (session-settled: user-directed — chosen over "twistymaze" or "Erv Walter and Longhand contributors" as the holder)

### Requirements

**Front door**

- R1. A `README.md` at the repository root introduces Longhand to a visitor. It describes the concept from `STRATEGY.md`: a lazy notes scratchpad, a local transcript as the system of record, and data that stays on the user's machine. It uses the terms in `CONCEPTS.md`.
- R2. The README says plainly that there are no downloads yet. It then gives short build-from-source steps for people who want to try Longhand.
- R3. The README links to the licence and to the contributing document. It includes no screenshot.
- R4. A `LICENSE` file at the repository root holds the MIT licence text with the line "Copyright (c) 2026 Erv Walter".
- R5. A `CONTRIBUTING.md` says warmly that Longhand is a young project and is not looking for outside contributions yet. It thanks the visitor for their interest and invites them to follow along. It does not sound like a refusal.

**Personal details**

- R6. `STRATEGY.md` describes its users without naming an employer. Its "Not a public project" line changes to say that the repository is public to follow but closed to contributions.
- R7. No tracked file names the user's employer. The UX-redesign plan is the other known file.

**History and logs**

- R8. A scan of the full history and all refs on GitHub, including `refs/pull/*`, finds no secrets and no committed private paths from the `CLAUDE.md` list. Any match is removed or found harmless before the switch.
- R9. The Actions run logs of `worktree-self-hosted-ci-runners` from 2026-10-10 and 2026-10-11 are deleted.

**The switch**

- R10. A checklist lists the steps before and after the switch in order. The user changes the visibility in the GitHub settings. An agent does not.
- R11. Before the switch, the self-hosted runner settings for a public repository, from the README of the private repository `twistymaze/homelab-ci`, are applied. CI selects these runners through `vars.MACOS_RUNNER` and `vars.WINDOWS_RUNNER` in `.github/workflows/ci.yml`. Code from people outside the repository must not run on the home runners.
- R12. After the switch, a check confirms that pull request creation is still limited to collaborators.
- R13. After the switch, a check confirms that issue creation is still limited to collaborators. Wiki, Discussions and Projects stay off.
- R14. The idea `pre-public-repo-audit` moves to `docs/ideas/done/`. The idea `about-longhand-and-readme` stays in `someday`, with only its About-pane part.

### Scope Boundaries

- The About section in Settings and the macOS About panel are out of scope. They stay in the `about-longhand-and-readme` idea.
- Renovate setup is deferred. When Renovate is set up, confirm that the collaborators-only issue setting does not stop its app from opening the dashboard issue.
- A README screenshot is deferred. It must use a note with no real names.
- No `SECURITY.md`, issue templates or PR templates. Outside users cannot file issues or PRs.
- Forking cannot be turned off for a public repository. MIT allows forks.
- No GitHub Support request to remove the unreachable commits.

### Dependencies / Assumptions

- The REST API does not show the issue creation setting (`gh api repos/twistymaze/longhand` returns only `pull_request_creation_policy`). Checks for R13 use the settings page.

### Outstanding Questions

**Deferred to Planning**

- Which tool scans the history for secrets (R8), and how to fetch all `refs/pull/*` for the scan.
- Which build steps the README gives (toolchain versions, macOS signing note), taken from `docs/development-workflow.md`.
- Whether the repository description and topics on GitHub get set at the switch.

### Sources / Research

- `docs/ideas/someday/pre-public-repo-audit.md`: the audit items and the old-commit context.
- `docs/ideas/someday/about-longhand-and-readme.md`: the README idea and its About-pane part.
- `deny.toml`, `about.toml`, `licences/`, and each `PROVENANCE.md`: licence policy and borrowed-code records (anarlog MIT, none from its `enterprise/` folder; Handy MIT; DTLN-aec MIT; NeMo Apache-2.0).
- `STRATEGY.md` and `CONCEPTS.md`: the description and terms for the README.
