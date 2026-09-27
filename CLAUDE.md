# Longhand

Local-first desktop app (Tauri 2, Rust workspace, React + Vite + Tailwind, bun) that records an in-person session on one microphone and produces a transcript and summary. The working plan is `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`; its units (U1–U23) and checkpoint log are the source of truth for scope and sequencing.

## Shared project memory

The user alternates between Claude Code and Codex. `AGENTS.md` links to this file so both tools use the same project guidance.

- At the start of a session, read `~/.claude/projects/-Users-erv-Development-transcription/memory/MEMORY.md` when it exists. Read the linked entries that apply to the task before planning, reviewing, or implementing. Use this same project memory directory when working in a worktree.
- If that index is absent, check `~/.claude/projects/` for this repository's memory directory. Verify that it belongs to Longhand before using it. If no matching directory exists, continue with the repository documentation and state that the local Claude memories were unavailable.
- Keep important Longhand knowledge in shared repository documentation. Put required project rules in `CLAUDE.md`, domain terms in `CONCEPTS.md`, and verified lessons and solutions in `docs/solutions/`. Search the relevant documentation before starting work. Read the memory index first; do not load every memory file into each session.
- Codex: do not deliberately save important project-specific knowledge for this repository in your normal Codex-only memory store (`~/.codex/memories/`), including its update-note directory. Claude Code cannot discover those entries through its normal memory loading. Save the shared lesson in the appropriate repository document instead. Background-generated Codex memories are supplementary, not the authoritative project record.
- When an existing Claude memory contains a lasting project lesson, verify it against current evidence before adding it to shared documentation. Keep machine-specific details in local memory, and keep secrets and private session data out of shared documentation. Do not merge or symlink the tools' internal memory stores.

## Models and host tools

- Use the current host's native models for normal work, including brainstorm and plan authoring: Codex models in Codex, Claude models in Claude Code. Do not send authoring to another provider unless the user explicitly requests it.
- For an independent cross-model review, use a different serving model family: prefer Claude from Codex and Codex from Claude Code. Follow the skill's route checks and disclosure rules. If no independent route is available, report that the cross-model pass did not run; do not label a same-family review as independent.
- Leave host-specific model overrides commented out in the shared Compound Engineering config. A model choice the user makes for the current task takes precedence.
- Use the current host's managed worktree tools. In Codex, use `create_worktree` and use its returned directory explicitly for subsequent work; creating it does not change the chat's default directory. In Claude Code, use `EnterWorktree`. Prefer a suitable existing worktree when the host provides one. Use the host's managed archive or removal tool for cleanup.

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
- When work reaches a manual test step, make the signed build at once, without a request from the user. Then give two ways to run it, each with the exact path of the build just made:
  - on this Mac: `open <checkout>/target/release/bundle/macos/Longhand.app`;
  - on a remote Mac: zip the bundle with `ditto -c -k --keepParent`, then give the `scp`, unzip, move-to-Applications and `open` steps. A remote Mac needs a notarized build, or Gatekeeper stops the app. Say so, and notarize when the user tests remotely.
- `reference-source/` holds the anarlog and Handy clones: read-only, never edited, never tracked. Borrowed code needs a `PROVENANCE.md` record; `scripts/check-provenance.sh` enforces it.
- CI runs only on pull requests into main, on pushes to main that change more than docs, and on `gh workflow run ci.yml --ref <branch>`: runner minutes are billed. The runs on main save the cache that each new pull request restores. For this rule, docs are every Markdown file except `PROVENANCE.md`, plus `docs/`, `.claude/` and `.compound-engineering/`. Local checks are the gate, including `scripts/check-windows.sh`, which lints for Windows from macOS in seconds except the crates that build C or C++, and `scripts/check-licences.sh`, which fails when the stored full licence list in `licences/generated/` does not match the dependencies (it needs cargo-about 0.9.2). Commit per unit and push in batches. While implementing, keep working after a push instead of waiting for CI. Once a PR is open and the work is done, watch it until CI passes. A change that touches only docs (plans, ideas, solutions, `CONCEPTS.md`, agent guidance) needs no PR: commit it on main and push it directly, because the CI run costs more than it checks.
- Parallel work has two parts:
  - Planning documents (`ce-ideate`, `ce-brainstorm`, `ce-plan`, `docs/ideas/`) are written, committed and pushed directly on `main` in the main checkout. Other sessions can work in the same checkout at the same time. Commit only this session's files, by path (`git commit -- <paths>`), and run `git pull --rebase origin main` before each push.
  - Implementation (`ce-work`) runs in its own worktree: use the host's managed worktree tool with the plan's topic as the name, and never implement in the main checkout. A new worktree branches from `origin/main`. Before you create it, make sure the plan is pushed to `origin/main`. If it is not, commit and push it first. `.worktreeinclude` lists the gitignored files needed in each worktree; verify they are present because host tools can differ in how they copy them.
  - When a PR merges, offer the cleanup as the next step, and ask before you start it, as `ce-brainstorm` asks before it starts `ce-plan`. Use the blocking question tool when available, with "Clean up now" as the recommended option; otherwise ask in chat. After a yes: confirm that the branch head is in `origin/main`, archive or remove the worktree with the host's managed tool, return to the main checkout on `main`, pull `main`, run `ce-compound` for the work that the PR shipped, then commit the resulting docs on `main` and push them. The compound-engineering skills stop at the merge report and do not offer this step.
- `docs/solutions/` holds documented solutions to past problems (bugs, CI and tooling lessons), organized by category with YAML frontmatter (`module`, `tags`, `problem_type`); relevant when changing or debugging a documented area.
- `docs/ideas/` is the parking lot for feature ideas: one file per idea, indexed in `docs/ideas/README.md`, which also says how to maintain it. When a feature idea comes up in any session, add or update its file and index row there rather than leaving it only in a plan, brainstorm or conversation. The folder is a local convention layered on the compound-engineering skills; if a plugin update ever makes the two disagree, raise it and resolve it rather than ignoring it.
- `CONCEPTS.md` holds shared domain vocabulary (entities, named processes, status concepts); relevant when orienting to the codebase or discussing domain concepts.
- `sample.m4a`, `sample.txt`, `fixtures/`, `manual-tests/` (the destination for manual test sessions), `docs/ideas/*-screenshots/` (reference screenshots showing real names) and `.env` are private and gitignored. Never commit, print, or upload them.
