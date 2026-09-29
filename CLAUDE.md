# Longhand

Local-first desktop app (Tauri 2, Rust workspace, React + Vite + Tailwind, bun). Records an in-person session on one microphone and produces a transcript and summary. The units (U1–U23) and checkpoint log in `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md` define scope and sequencing.

## Shared project memory

`AGENTS.md` links here so Claude Code and Codex use the same guidance.

- At session start, find this checkout's project directory under `~/.claude/projects/`. For linked worktrees, identify the main checkout with `git worktree list` and use its memory directory. Match the actual checkout path; assume no fixed username or location. Verify the match, then read `memory/MEMORY.md` and only task-relevant linked entries before work. If unavailable, say so and use repository docs.
- Store lasting project knowledge in shared docs: rules here, terms in `CONCEPTS.md`, and verified lessons in `docs/solutions/`. Search relevant docs before work. Verify memory-derived lessons before adding them.
- Keep machine-specific details in local memory and private data out of shared docs. Do not deliberately save project knowledge in Codex-only memory (including update notes), or merge or symlink the tools' memory stores. Background-generated Codex memories are supplementary.

## Models and host tools

- Use the host's native models for normal work, including brainstorms and plans: Codex in Codex, Claude in Claude Code. Use another provider only at the user's request.
- The user gives standing approval for all types of cross-model review in this project, including cloud review of requirements, plans, documents, code, and diffs. Review tools may send the relevant project content to the selected model provider without asking for approval again. This permission applies to reviews; it does not authorize unrelated uploads or override the private-file restrictions below. Keep the required provider disclosure and read-only review controls.
- Independent cross-model reviews require a different serving model family: prefer Claude from Codex and Codex from Claude Code. Follow the skill's route checks and disclosure rules. If unavailable, report that the pass did not run. Never label a same-family review independent.
- Keep host-specific model overrides commented out in shared Compound Engineering config. The user's model choice takes precedence.
- Use managed worktree tools: `create_worktree` in Codex, `EnterWorktree` in Claude Code. Prefer a suitable existing worktree. In Codex, use the returned directory explicitly; the chat's default directory does not change. Use managed tools for cleanup.

## UI design

Invoke `apple-design` before any UI design, implementation, or review. Add skills as needed:

- `emil-design-eng` for component polish.
- `animate` for new motion; `review-animations` or `improve-animations` for existing motion; `find-animation-opportunities` for gaps.
- `animation-vocabulary` to name an effect.

The recording surface must not draw participants' attention back to the screen. U7's two-tier rail and attention budget take precedence over motion or ornament.

## Language

Code identifiers, comments, logs, docs, plans, and agent guidance follow ASD-STE100. Use short, active sentences, one meaning per word (one sense, not one use), and one term per concept. Use the terms in `CONCEPTS.md`. Use no metaphors.

UI text uses short, direct plain language with consistent terms, no metaphors, and no jargon. STE's controlled vocabulary does not apply.

## Working in this repo

- Use Python only through `uv`, JavaScript packages only through `bun`, and Rust 1.94.0.
- Test microphone and Keychain behaviour only in a signed bundle, never `tauri dev`. At each manual test step, build without waiting for a request. First read [Manual tests](docs/development-workflow.md#manual-tests); provide exact local and remote launch steps, and notarize for remote tests.
- Keep the anarlog and Handy clones in `reference-source/` read-only and untracked. Record borrowed code in `PROVENANCE.md`; `scripts/check-provenance.sh` enforces this.
- Local checks are required. Before validation or a push, read [Checks and CI](docs/development-workflow.md#checks-and-ci). Commit per unit, push in batches, and continue implementation while CI runs. Before you create a PR, do the steps in [Before a PR](docs/development-workflow.md#before-a-pr): rebase when `main` has new code changes. Once the PR is open and work is done, watch CI until it passes.
- Write, commit, and push planning documents and docs-only changes directly on `main` in the main checkout. Other sessions can use that checkout. Commit only this session's files by path (`git commit -- <paths>`), and run `git pull --rebase origin main` before each push.
- Implement (`ce-work`) in a separate worktree, never the main checkout. Push the plan to `origin/main` first. Create new worktrees from `origin/main`, named for the plan's topic. Verify that files listed in `.worktreeinclude` are present.
- After a PR merges, offer cleanup and wait for approval. Before cleanup, read [After a merge](docs/development-workflow.md#after-a-merge).
- Read relevant `docs/solutions/` entries before changing a documented area. Solutions use category folders and YAML fields `module`, `tags`, and `problem_type`.
- Record each feature idea in its own file in `docs/ideas/`, even when it also appears in a plan or conversation. The folder is the status. List ideas with `bun scripts/ideas.ts`, and move them only with `bun scripts/ideas.ts move`. Follow `docs/ideas/README.md`. Raise and resolve conflicts between this convention and plugin updates.
- Private, gitignored paths: `sample.m4a`, `sample.txt`, `fixtures/`, `manual-tests/` (manual test sessions), `docs/ideas/**/*-screenshots/` (real names), and `.env`. Never commit, print, or upload them.
