# Longhand

Local-first desktop app (Tauri 2, Rust workspace, React + Vite + Tailwind, npm). Records an in-person meeting on one microphone into a note with a transcript and a summary. The units (U1–U23) and checkpoint log in `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md` define scope and sequencing.

## Shared project memory

`AGENTS.md` links here so Claude Code and Codex use the same guidance.

- At session start, find this checkout's project directory under `~/.claude/projects/`. For linked worktrees, identify the main checkout with `git worktree list` and use its memory directory. Match the actual checkout path; assume no fixed username or location. Verify the match, then read `memory/MEMORY.md` and only task-relevant linked entries before work. If unavailable, say so and use repository docs.
- Store lasting project knowledge in shared docs: rules here, terms in `CONCEPTS.md`, and verified lessons in `docs/solutions/`. Search relevant docs before work. Verify memory-derived lessons before adding them.
- Keep machine-specific details in local memory and private data out of shared docs. Do not deliberately save project knowledge in Codex-only memory (including update notes), or merge or symlink the tools' memory stores. Background-generated Codex memories are supplementary.

## Skills for informal requests

The user often asks for work in informal words and does not name a skill. When a Compound Engineering skill matches the task, use it, also when the user does not name it. For example:

- A defect or a failure: `ce-debug`.
- A new idea or scope: `ce-brainstorm`. A plan: `ce-plan`. Implementation of a plan or a clear request: `ce-work`.
- A commit: `ce-commit`. A PR: `ce-commit-push-pr`. An open PR: `ce-babysit-pr`.
- A review of code: `ce-code-review`. A review of a document: `ce-doc-review`.
- A solved problem with a lesson: `ce-compound`.
- An in-depth question about how or why something works, or about what happened: `ce-explain`. A request for a judgment or a recommendation: `ce-pov`.

Do the work directly when no skill matches, or when the task is small enough that a skill adds only steps (a simple question, one edit, one command). The rules in this file have priority over the skill when they are different.

## Models and host tools

- Use the host's native models for normal work, including brainstorms and plans: Codex in Codex, Claude in Claude Code. Use another provider only at the user's request.
- The user gives standing approval for all types of cross-model review in this project, including cloud review of requirements, plans, documents, code, and diffs. Review tools may send the relevant project content to the selected model provider without asking for approval again. This permission applies to reviews; it does not authorize unrelated uploads or override the private-file restrictions below. Keep the required provider disclosure and read-only review controls.
- Independent cross-model reviews require a different serving model family: prefer Claude from Codex and Codex from Claude Code. Follow the skill's route checks and disclosure rules. If unavailable, report that the pass did not run. Never label a same-family review independent.
- When Codex is the cross-model reviewer, request the Sol model `gpt-6-sol`: pass `CROSS_MODEL_MODEL_OVERRIDE_TARGET=codex CROSS_MODEL_MODEL_OVERRIDE=gpt-6-sol` in the peer start command. The plugin default is a Luna model; do not use it for review. Do not set `cross_model_model` in the shared CE config: from Codex the peer is Claude, and the worker refuses a GPT model for that peer.
- Keep host-specific model overrides commented out in shared Compound Engineering config. The user's model choice takes precedence.
- Use managed worktree tools: `create_worktree` in Codex, `EnterWorktree` in Claude Code. Prefer a suitable existing worktree. In Codex, use the returned directory explicitly; the chat's default directory does not change. Use managed tools for cleanup.

## UI design

Invoke `apple-design` before any UI design, implementation, or review. Add skills as needed:

- `emil-design-eng` for component polish.
- `animate` for new motion; `review-animations` or `improve-animations` for existing motion; `find-animation-opportunities` for gaps.
- `animation-vocabulary` to name an effect.

The recording surface must not draw participants' attention back to the screen. U7's two-tier rail and attention budget take precedence over motion or ornament.

Show a notice, warning, or error message only when one of these conditions is true:

- The user must do something.
- Something abnormal or unexpected occurred, and it is likely to be important to the user.

Do not show a message for normal behaviour, for example a folder move that the app does after a recording. Do not show a message for a problem that the app corrects without help. Show it only if the app tries to correct the problem and does not succeed.

## Language

Code identifiers, comments, logs, docs, plans, and agent guidance follow ASD-STE100. Use short, active sentences, one meaning per word (one sense, not one use), and one term per concept. Use the terms in `CONCEPTS.md`. Use no metaphors.

UI text uses short, direct plain language with consistent terms, no metaphors, and no jargon. STE's controlled vocabulary does not apply.

## Working in this repo

- Use Python only through `uv`, JavaScript packages only through npm, and Rust 1.95.0.
- Test microphone and Keychain behaviour only in a signed bundle, never `tauri dev`. At each manual test step, build without waiting for a request. First read [Manual tests](docs/development-workflow.md#manual-tests); provide exact local and remote launch steps, and notarize for remote tests.
- Keep the anarlog, Handy, and Shorthand clones in `reference-source/` read-only and untracked. Shorthand (https://github.com/mshish/shorthand, MIT) is a Handy fork that adds system audio and live meeting notes. Record borrowed code in `PROVENANCE.md`; `scripts/check-provenance.sh` enforces this.
- Local checks are required. Before validation or a push, read [Checks and CI](docs/development-workflow.md#checks-and-ci). Commit per unit, push in batches, and continue implementation while CI runs. Before you create a PR, do the steps in [Before a PR](docs/development-workflow.md#before-a-pr): rebase when `main` has new code changes. Once the PR is open and work is done, run `ce-babysit-pr` on it until the PR is ready to merge. Do this for each PR that you open.
- A test that passes or fails because of timing is a defect in the test. Do not run CI again to get a pass, and do not call the failure "flaky" or "not caused by this change". Find the race and fix the test in the same PR, also when the PR did not cause it. A test must wait for the event that it checks, not for a delay, a poll count or a lucky order of threads. Run a CI job again only for an infrastructure fault, for example a runner that did not start or a network download that failed.
- Write, commit, and push planning documents and docs-only changes directly on `main` in the main checkout. A change to `plugins/` (the agent skill, also its Markdown files) is not a docs-only change: it goes through a PR, because the plugin reaches users from `main` with no release. Other sessions can use that checkout. Commit only this session's files by path (`git commit -- <paths>`), and run `git pull --rebase origin main` before each push.
  - Push a branch other than `main` only when the branch is for a pull request. This rule also applies when a session tells you to push its branch. A pushed branch that has no pull request stays on GitHub, and the user must delete it.
  - A cloud session (claude.ai/code) starts on its own `claude/*` branch and has no main checkout. For docs-only changes, commit on that branch, but do not push it. Put the commits on `main` (`git fetch origin main`, `git checkout -B main origin/main`, `git cherry-pick <commits>`), run `node scripts/ideas.ts check` when `docs/ideas/` changed, and push `main`. This rule is the user's permission to push docs-only changes to `main` from a cloud session. The push to `main` keeps the work, so a push of the session branch is not necessary. The session's git proxy does not delete remote branches, so a session branch that was pushed stays on GitHub. If that occurs, tell the user the branch name, so that they can delete it.
- Implement (`ce-work`) in a separate worktree, never the main checkout. Push the plan to `origin/main` first. Create new worktrees from `origin/main`, named for the plan's topic. Verify that files listed in `.worktreeinclude` are present.
- Do not add Compound Engineering branding or badges to pull request descriptions or commit messages.
- Each merged PR that changes the app makes a release and an update notice. Ship one feature in one PR, also when its plan has phases: the phases are units and commits on one branch. Open more than one PR only when each PR gives users value by itself, or when the user asks. If a plan's Phased Delivery proposes more than one PR for one feature, raise it before implementation.
- Each PR title is a Conventional Commit (`type: description` or `type(scope): description`). PRs merge by squash, and the PR title becomes the commit on `main`, so the title decides the release. [Releases](docs/releases.md) gives the version rules. Choose the type by the effect on users, not by the kind of work. The type does not decide the size of the release: each release type makes a patch release, and only the user chooses a minor release.
  - `feat` is for any gain that the user can see: a new capability, an option, information, or a clear improvement to how something works.
  - `fix` is for a defect, or for small polish (wording, spacing). `perf`, `refactor`, `revert` and `build` also make a patch release.
  - A `+` after the type or scope of a `feat` title (`feat+: …`, `feat(capture)+: …`) makes a minor release. Write `+` only when the user chooses a minor release. `+` is not valid on other types.
  - When the user is present, ask "patch or minor?" before you open a PR whose type is `feat`, and before you change a PR title to `feat`. Use the host's blocking question tool, and give your recommendation and one reason. Then stop and wait for the answer; do not open or retitle the PR in the same turn. Do not ask again when the user already said patch or minor for this change. Do not ask for other types.
  - When you write the title of a PR with `+` again, keep the `+` unless the user says patch. Ask the user before you change the type of a `feat+` title.
  - A hands-off run (lfg, an unattended babysit, or another flow that runs while the user is not present) does not ask and never adds `+`, so its release is a patch. It keeps a `+` that the user already chose, and it does not change the type of a `feat+` title; it reports the change it would make instead. In its final report, say that the PR makes a patch release and that the user can add `+` to the title before merge.
  - A dependency update is `build(deps)`. Use `fix(deps)` only when the update fixes a fault that users see.
  - `style` is formatting only. A CSS or visual change is a `feat` or a `fix`.
  - A change to build tools or CI that does not change the shipped app is a `chore` or a `ci`. `docs`, `ci`, `test`, `chore` and `style` make no release.
  - A change only to `plugins/` (the agent skill) makes no app release, because the plugin updates from `main`, not from a release. Use `docs` for a change to the skill text, and `chore` for other changes, such as a change to the skill scripts. Do not use `feat` or `fix`. A PR that also changes the app uses the rules above for the app change.
  - Never write `!` or `BREAKING CHANGE` in a PR title unless the user asks for a major release. `!` on a release type makes a major release.
  - A commit pushed directly to `main` (docs-only work) uses only `docs`, `chore` or `ci`.
- After a PR merges, offer cleanup and wait for approval. Before cleanup, read [After a merge](docs/development-workflow.md#after-a-merge).
- Read relevant `docs/solutions/` entries before changing a documented area. Solutions use category folders and YAML fields `module`, `tags`, and `problem_type`.
- Record each feature idea in its own file in `docs/ideas/`, even when it also appears in a plan or conversation. The folder is the status. List ideas with `node scripts/ideas.ts`, and move them only with `node scripts/ideas.ts move`. Follow `docs/ideas/README.md`. `ce-ideate` does not read or write this folder: before a run, give it the known ideas; after a run, file each survivor that is not brainstormed. The README gives the steps. Raise and resolve conflicts between this convention and plugin updates.
- Private, gitignored paths: `sample.m4a`, `sample.txt`, `fixtures/`, `manual-tests/` (manual test sessions), `docs/ideas/**/*-screenshots/` (real names), and `.env`. Never commit, print, or upload them.
