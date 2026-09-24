# Longhand

Local-first desktop app (Tauri 2, Rust workspace, React + Vite + Tailwind, bun) that records an in-person session on one microphone and produces a transcript and summary. The working plan is `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`; its units (U1–U23) and checkpoint log are the source of truth for scope and sequencing.

## UI design

Before designing, building, or reviewing any UI surface (the probe window, settings, the recording rail and scratchpad, the processing and session views), invoke the `apple-design` skill and follow it. Pair it as the work calls for:

- `emil-design-eng` for component polish and the small details.
- `animate` when adding motion; `review-animations` or `improve-animations` to critique existing motion; `find-animation-opportunities` to look for gaps.
- `animation-vocabulary` to name an effect.

`animate-expo`, `mobile-native` and `write-swift` do not apply: Longhand is a desktop webview, not React Native, a phone web app, or Swift.

Apple's restraint principle matches a product rule already in the plan: the recording surface must not pull a participant's attention back to the screen (U7's two-tier rail). When the two seem to conflict, the plan's attention budget wins over added motion or ornament.

## Working in this repo

- Python only through `uv`. `bun` is the only JavaScript package manager. Rust is pinned to 1.94.0.
- Microphone and Keychain behaviour must be tested from the signed bundle (`bun tauri build --bundles app`), never `tauri dev`: an unsigned dev binary's microphone request is attributed to the launching terminal.
- `reference-source/` holds the anarlog and Handy clones: read-only, never edited, never tracked. Borrowed code needs a `PROVENANCE.md` record; `scripts/check-provenance.sh` enforces it.
- CI runs only on pull requests into main and on `gh workflow run ci.yml --ref <branch>`: runner minutes are billed. Local checks are the gate, including `scripts/check-windows.sh`, which lints for Windows from macOS in seconds except the crates that build C or C++. Commit per unit, push in batches, and don't wait on CI.
- `docs/solutions/` holds documented solutions to past problems (bugs, CI and tooling lessons), organized by category with YAML frontmatter (`module`, `tags`, `problem_type`); relevant when changing or debugging a documented area.
- `docs/ideas/` is the parking lot for feature ideas: one file per idea, indexed in `docs/ideas/README.md`, which also says how to maintain it. When a feature idea comes up in any session, add or update its file and index row there rather than leaving it only in a plan, brainstorm or conversation. The folder is a local convention layered on the compound-engineering skills; if a plugin update ever makes the two disagree, raise it and resolve it rather than ignoring it.
- `CONCEPTS.md` holds shared domain vocabulary (entities, named processes, status concepts); relevant when orienting to the codebase or discussing domain concepts.
- `sample.m4a`, `sample.txt`, `fixtures/`, `manual-tests/` (the destination for manual test sessions), `docs/ideas/*-screenshots/` (reference screenshots showing real names) and `.env` are private and gitignored. Never commit, print, or upload them.
