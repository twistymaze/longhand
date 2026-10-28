---
title: Landing page at longhand.md - Plan
type: feat
date: 2026-10-28
topic: landing-page
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Landing page at longhand.md - Plan

## Goal Capsule

- **Objective:** A newcomer who opens https://longhand.md understands in about a minute what Longhand is, why it is different, and how a meeting goes with it, and can download it. The page does not look like a generic AI-made product page.
- **Product authority:** the user (maintainer). The page copy, the look and the screenshots are approved by the user at the review stops in R21 and R22 before the PR opens.
- **Open blockers:** none.

---

## Product Contract

### Summary

A single landing page at the root of `longhand.md`, ordered what → why → how, with three screenshots that a repeatable script renders from the real app screens with invented meeting content, each framed as a real macOS window. The site also deploys when the page changes on `main`. The same work fixes two rendering faults in the changelog lists.

### Problem Frame

The site at `longhand.md` holds only the changelog; its root sends the visitor to `/changelog/`. A newcomer who hears about Longhand lands on the GitHub README, which is written for developers and shows no screenshot of the app. There is no page that shows the app and explains the idea in plain words.

Pages made with AI help often share the same look: purple or indigo gradients, Inter for all text, a centered hero with a vague tagline, rows of rounded cards with emoji, glass effects. Readers now recognize that look and read it as low effort. The page must not have it.

In the changelog lists (Recent changes, the update window and the changelog page), the group headings are gray and look washed out, and the bullet of a wrapped line sits at its last line instead of its first.

### Key Decisions

- **Audience: curious newcomers.** People who heard of Longhand and want to know what it is; the page ends at Download. Governs R1-R7. (session-settled: user-directed — chosen over a personal page for friends and over a page for developers and agent users)
- **Information order: what → why → how.** The idea and privacy come before the screenshots. Governs R1-R4. (session-settled: user-directed — chosen over a layout that follows the meeting first, over one large screenshot first, and over a page styled as a Markdown note)
- **Look: a well-typeset page, between the Mac app's look and almost no design.** Use the Mac app's look only where it helps; no writing-paper skeuomorphism. Governs R8-R10. (session-settled: user-directed — chosen over "looks like the Mac app", "Longhand as writing" and "almost no design")
- **Screenshots: a repeatable script with invented content, framed as macOS windows.** Governs R12-R16. (session-settled: user-directed — chosen over a one-time capture; the window frame was the user's addition)
- **Windows: "coming soon", with no download link.** Governs R5. (session-settled: user-directed — chosen over a preview link to the unsigned installer)
- **Deploy also when the page changes.** Governs R17-R18. (session-settled: user-approved — chosen over deploys only at releases)
- **Changelog group headings in the normal text color.** Governs R19. (session-settled: user-approved — chosen over keeping the gray and over a darker gray)
- **Download goes to the latest release page.** The DMG file name changes with each version, so a stable link points at the release page. Governs R4.
- **One PR.** The changelog-list fix changes the app, so the PR makes a patch release, and its title names both the landing page and the list fix. That release also deploys the new page.

### Requirements

**Page content and order**

- R1. The page opens with **what**: the name Longhand, one plain sentence that says what it is, and the download action.
- R2. Next comes **why**: the idea (take part in the meeting; type a few words; the transcript holds the details; afterwards Longhand writes a summary from your notes and the transcript) and the privacy points (audio and transcription stay on the computer; you choose the AI service; no account, no Longhand cloud, no usage data; notes are plain files you own).
- R3. Then comes **how**, a meeting in order, each step with its screenshot: during the meeting (a note while recording), after it (the finished note with its summary), and over time (the notes library).
- R4. A footer links to the changelog (`/changelog/`), the GitHub repository and the latest release page. The download action also goes to the latest release page.
- R5. The page names Windows as a platform that is coming soon, with no Windows download link, and does not read as Mac-only.
- R6. The page copy is short, specific plain language taken from what Longhand does (the README is the source of facts). No generic marketing phrases.
- R7. The page works and reads well at phone width as well as desktop width.

**Look**

- R8. The page avoids the known AI-design tells: no purple or indigo gradients or gradient text, no Inter-by-default, no centered-hero-plus-card-grid, no emoji or generic icon grids, no glass effects, no stock illustrations.
- R9. The page follows the visitor's light or dark setting.
- R10. The typography is chosen deliberately, and the page borrows the Mac app's look (colors, controls) only where it helps. No writing-paper or handwriting imitation.

**Privacy of the page itself**

- R11. The page loads nothing from other sites: no analytics, no trackers, no web fonts or scripts from a CDN.

**Screenshots**

- R12. Three screenshots: a note while recording, the finished note with its summary, and the notes library.
- R13. A script renders the screenshots from the real app UI, so a change to the app's look is one command away from new screenshots.
- R14. All content in the screenshots is invented: people, meeting topics, notes, transcript text and summaries. No real names, and nothing from private paths (`fixtures/`, `manual-tests/`, `sample.*`). The invented content is checked into the repo and reviewed in the PR.
- R15. Each screenshot looks like a full macOS window capture: a title bar with the window buttons and the window title, rounded corners and the window shadow, in the proportions of the app's real windows. It is not a borderless render of the page.
- R16. The screenshots are light mode.

**Deploy**

- R17. The site deploys when a release is published (as now) and also when a push to `main` changes the landing page or its screenshots.
- R18. A deploy that is not from a release keeps the changelog page of the latest release.

**Changelog lists**

- R19. In Recent changes, the update window and the changelog page, the group headings use the normal text color and stay smaller than the month heading.
- R20. A list item that wraps to more than one line has its bullet at the first line.

**Review stops**

- R21. Before the page is built out, the user reviews sample screenshots, and the work continues only after the user approves them.
- R22. Before the PR opens, the user reviews the finished page served locally, and the PR opens only after the user approves it.

### Acceptance Examples

- AE1. **Covers R17, R18.** **Given** the latest release is v1.2.10, **when** a push to `main` changes only the landing page text, **then** `longhand.md` shows the new text and `longhand.md/changelog/` still shows the v1.2.10 changelog.
- AE2. **Covers R20.** **Given** a changelog line too long for one line, **when** it wraps, **then** its bullet aligns with the first line of the text, in the app and on the page.
- AE3. **Covers R11.** **Given** the page is open, **when** the browser's network log is read, **then** every request goes to `longhand.md` (or the GitHub Pages host behind it).

### Scope Boundaries

- A Windows download on the page (it waits for a signed installer).
- More pages (blog, docs, FAQ), more screenshots, other languages, dark-mode screenshots.
- Changes to the README's Windows text: the README keeps its link to the unsigned installer.
- A change to the app's `PAGES_URL`: it stays the `github.io` address, which GitHub forwards to the custom domain.

### Dependencies / Assumptions

- The custom domain `longhand.md` is set in the Pages settings and HTTPS is on; `docs/releases.md` setup step 5 records the setup.
- The changelog page and its styles come from `scripts/lib/changelog.ts` (`html()`), and the site is assembled by the `changelog-pages` job in `.github/workflows/ci.yml`.
- Assumption: a page-only deploy can get the latest release's changelog files from the release assets (`changelog.json`, and the page made from it).

### Outstanding Questions

**Deferred to Planning**

- How the screenshot script renders the app UI with invented data (an existing mock or preview path, if one exists, or a new one), and how it draws the macOS window frame so that it matches the app's real title bar style.
- Which typeface the page uses, within R10 and R11 (system fonts or one self-hosted font).
- Where the landing page source lives in the repo, and how the deploy decides that the page changed.

### Sources / Research

- Common tells of AI-made pages: [Why Your AI Keeps Building the Same Purple Gradient Website](https://prg.sh/ramblings/Why-Your-AI-Keeps-Building-the-Same-Purple-Gradient-Website), [Scoring 500 Show HN pages for AI design patterns](https://adriankrebs.ch/blog/design-slop/), [The visible tells of AI design](https://impeccable.style/slop).
- `README.md` for the product facts and the privacy points.
- `docs/plans/2026-10-28-0833-feat-monthly-changelog-plan.md` for the changelog page and the Pages deploy.
