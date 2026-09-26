# Longhand

Local-first desktop app (Tauri 2, Rust workspace, React + Vite + Tailwind, bun) that records an in-person session on one microphone and produces a transcript and summary. The working plan is `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`; its units (U1–U23) and checkpoint log are the source of truth for scope and sequencing.

## UI design

Before designing, building, or reviewing any UI surface (the probe window, settings, the recording rail and scratchpad, the processing and session views), invoke the `apple-design` skill and follow it. Pair it as the work calls for:

- `emil-design-eng` for component polish and the small details.
- `animate` when adding motion; `review-animations` or `improve-animations` to critique existing motion; `find-animation-opportunities` to look for gaps.
- `animation-vocabulary` to name an effect.

`animate-expo`, `mobile-native` and `write-swift` do not apply: Longhand is a desktop webview, not React Native, a phone web app, or Swift.

Apple's restraint principle matches a product rule already in the plan: the recording surface must not pull a participant's attention back to the screen (U7's two-tier rail). When the two seem to conflict, the plan's attention budget wins over added motion or ornament.

## Language

Code identifiers, comments, log text, docs, plans and agent guidance follow ASD-STE100:

- Give each word one meaning, and use one word for each concept. `CONCEPTS.md` holds the terms. One meaning is one sense, not one use: "save" means to keep something in a place, whether that place is a folder or Keychain.
- Write short sentences in the active voice.
- Use no metaphors. For example, a process does not "land": it saves, finishes or completes. `CONCEPTS.md` lists the terms to avoid.

UI text is plain language, not STE:

- Keep it short and direct.
- Use one term for each concept on every screen.
- Use no metaphors and no jargon. STE's controlled vocabulary does not apply.

## Working in this repo

- Python only through `uv`. `bun` is the only JavaScript package manager. Rust is pinned to 1.94.0.
- Microphone and Keychain behaviour must be tested from the signed bundle (`bun tauri build --bundles app`), never `tauri dev`: an unsigned dev binary's microphone request is attributed to the launching terminal.
- `reference-source/` holds the anarlog and Handy clones: read-only, never edited, never tracked. Borrowed code needs a `PROVENANCE.md` record; `scripts/check-provenance.sh` enforces it.
- CI runs only on pull requests into main, on pushes to main that change more than docs, and on `gh workflow run ci.yml --ref <branch>`: runner minutes are billed. The runs on main save the cache that each new pull request restores. For this rule, docs are every Markdown file except `PROVENANCE.md`, plus `docs/`, `.claude/` and `.compound-engineering/`. Local checks are the gate, including `scripts/check-windows.sh`, which lints for Windows from macOS in seconds except the crates that build C or C++. Commit per unit and push in batches. While implementing, keep working after a push instead of waiting for CI. Once a PR is open and the work is done, watch it until CI passes. A change that touches only docs (plans, ideas, solutions, `CONCEPTS.md`, agent guidance) needs no PR: commit it on main and push it directly, because the CI run costs more than it checks.
- Parallel work has two parts:
  - Planning documents (`ce-ideate`, `ce-brainstorm`, `ce-plan`, `docs/ideas/`) are written, committed and pushed directly on `main` in the main checkout. Other sessions can work in the same checkout at the same time. Commit only this session's files, by path (`git commit -- <paths>`), and run `git pull --rebase origin main` before each push.
  - Implementation (`ce-work`) runs in its own worktree: use `EnterWorktree` with the plan's topic as the name, and never implement in the main checkout. A new worktree branches from `origin/main`. Before you create it, make sure the plan is pushed to `origin/main`. If it is not, commit and push it first. `.worktreeinclude` lists the gitignored files that are copied into each worktree.
  - When a PR merges, offer the cleanup as the next step, and ask before you start it, as `ce-brainstorm` asks before it starts `ce-plan`. Use the blocking question tool, with "Clean up now" as the recommended option. After a yes: confirm that the branch head is in `origin/main`, remove the worktree with `ExitWorktree`, switch the main checkout to `main`, pull `main`, run `ce-compound` for the work that the PR shipped, then commit the resulting docs on `main` and push them. The compound-engineering skills stop at the merge report and do not offer this step.
- `docs/solutions/` holds documented solutions to past problems (bugs, CI and tooling lessons), organized by category with YAML frontmatter (`module`, `tags`, `problem_type`); relevant when changing or debugging a documented area.
- `docs/ideas/` is the parking lot for feature ideas: one file per idea, indexed in `docs/ideas/README.md`, which also says how to maintain it. When a feature idea comes up in any session, add or update its file and index row there rather than leaving it only in a plan, brainstorm or conversation. The folder is a local convention layered on the compound-engineering skills; if a plugin update ever makes the two disagree, raise it and resolve it rather than ignoring it.
- `CONCEPTS.md` holds shared domain vocabulary (entities, named processes, status concepts); relevant when orienting to the codebase or discussing domain concepts.
- `sample.m4a`, `sample.txt`, `fixtures/`, `manual-tests/` (the destination for manual test sessions), `docs/ideas/*-screenshots/` (reference screenshots showing real names) and `.env` are private and gitignored. Never commit, print, or upload them.
