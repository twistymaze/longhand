---
title: About Longhand, and a README
description: "The About pane tells what Longhand is, with its version, author and repository link; the repository gets a real README.md"
area: interface
added: 2026-09-28
source: "a conversation on 2026-09-28"
related: [clickable-links-in-notes]
---

# About Longhand, and a README

## Idea
The About pane (Settings → About) starts with a section about Longhand itself, above the acknowledgements and licences:

- a short text that tells what Longhand is: it records an in-person conversation on one microphone and makes a transcript and summary on the computer;
- the version and commit;
- the author;
- a link to the repository.

At the same time, the repository gets a real `README.md` for people who visit it on GitHub. It tells what Longhand is, what it does, and how to install and build it. The two texts use the same short description.

## Why
The About pane now tells only about the things that Longhand uses. It says almost nothing about Longhand. The repository has no README, so a visitor sees only a list of files.

## Notes / Current state
- `src/settings/About.tsx` already shows "Longhand" with the version and commit in its first group (`BuildVersion`, `versionText`). The new section can extend that group.
- The pane opens links through tauri-plugin-opener. The Settings capability already allows `https://*` (`src-tauri/capabilities/settings.json`), so a repository link needs no new permission.
- There are two About surfaces, and they are different. The app menu has "About Longhand", the standard macOS About panel (`PredefinedMenuItem::about` with no metadata, `src-tauri/src/menu.rs`). Settings has the About pane. Decide in the brainstorm how they relate: the menu item opens the Settings pane, the macOS panel gets the same metadata (`AboutMetadata`: version, authors, website, copyright), or each shows a different part.
- The repository is `github.com/twistymaze/longhand`.
- The repository has no `LICENSE` file. Decide the licence before the README and the About pane state one.
- Open questions:
  - the author text (a name, a name and a site, or a copyright line);
  - whether the About section has a link to report a problem (GitHub issues);
  - what the README says about installation while there is no public release (see `preview-builds-and-updates`);
  - whether the README has a screenshot. A screenshot must show no real names.
- Invoke `apple-design` before the About pane design. Use `STRATEGY.md` and `CONCEPTS.md` for the description, so that the README, the About pane and the strategy use the same terms.
