---
title: About Longhand
description: "The About pane tells what Longhand is, with its version, author and repository link"
area: interface
added: 2026-09-28
source: "a conversation on 2026-09-28"
related: [clickable-links-in-notes]
---

# About Longhand

## Idea
The About pane (Settings → About) starts with a section about Longhand itself, above the acknowledgements and licences:

- a short text that tells what Longhand is, with the same description as the first lines of `README.md`;
- the version and commit;
- the author;
- a link to the repository.

## Why
The About pane now tells only about the things that Longhand uses. It says almost nothing about Longhand.

## Notes / Current state
- The README part of this idea is done: the repository has `README.md`, `CONTRIBUTING.md` and an MIT `LICENSE` (public repository plan, 2026-10-11). Use the README's description and author ("Copyright (c) 2026 Erv Walter") so the two texts agree.
- `src/settings/About.tsx` already shows "Longhand" with the version and commit in its first group (`BuildVersion`, `versionText`). The new section can extend that group.
- The pane opens links through tauri-plugin-opener. The Settings capability already allows `https://*` (`src-tauri/capabilities/settings.json`), so a repository link needs no new permission.
- There are two About surfaces, and they are different. The app menu has "About Longhand", the standard macOS About panel (`PredefinedMenuItem::about` with no metadata, `src-tauri/src/menu.rs`). Settings has the About pane. Decide in the brainstorm how they relate: the menu item opens the Settings pane, the macOS panel gets the same metadata (`AboutMetadata`: version, authors, website, copyright), or each shows a different part.
- The repository is `github.com/twistymaze/longhand`.
- Open questions:
  - the author text (a name, a name and a site, or a copyright line);
  - whether the About section has a link to report a problem (GitHub issues are limited to collaborators for now).
- Invoke `apple-design` before the About pane design. Use `STRATEGY.md` and `CONCEPTS.md` for the description, so that the README, the About pane and the strategy use the same terms.
