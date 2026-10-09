# Development workflow

Read the applicable section before manual tests, validation, pushes, or cleanup. `CLAUDE.md` defines the project rules.

## Manual tests

At a manual test step, make a signed build immediately:

```sh
bun tauri build --bundles app
```

Test microphone and Keychain behaviour from this bundle. An unsigned `tauri dev` binary attributes microphone requests to the launching terminal.

Give both ways to run the build, using the exact checkout path:

- On this Mac: `open <checkout>/target/release/bundle/macos/Longhand.app`.
- On a remote Mac: zip the bundle with `ditto -c -k --keepParent`. Provide the `scp`, unzip, `spctl` and `open` steps for that build, and the cleanup step. Start the steps with the cleanup step too: an old build in the folder makes the unzip fail. A remote Mac needs a notarized build or Gatekeeper stops the app. Explain this requirement and notarize when the user tests remotely.

Never put a test build in `/Applications`. The app there is the installed preview build, and it gets updates. Unzip a test build into its own folder, such as `~/Downloads/longhand-test/`, and open it from there. Quit the preview build first: the test build has the same bundle ID, so the preview build can receive the launch instead. The test build uses the same app data as the preview build.

After the test, quit the test build and delete its folder:

```sh
rm -rf ~/Downloads/longhand-test
```

A local build has no track, so it gets no updates. The preview build in `/Applications` keeps its updates.

## Checks and CI

Run local checks, including:

- `scripts/check-windows.sh`: lints for Windows from macOS in seconds, except crates that build C or C++.
- `scripts/check-licences.sh`: requires cargo-about 0.9.2. Fails if the stored full licence list in `licences/generated/` does not match dependencies.

CI runs only on:

- Pull requests into `main`.
- Pushes to `main` that change more than docs.
- Explicit runs with `gh workflow run ci.yml --ref <branch>`.

Runner minutes are billed. Runs on `main` save the cache that new pull requests restore. For CI, docs are all Markdown files except `PROVENANCE.md`, plus `docs/`, `.claude/`, and `.compound-engineering/`.

Docs-only changes need no PR. Commit and push them directly on `main`; CI costs more than it checks for these changes. This includes plans, ideas, solutions, `CONCEPTS.md`, and agent guidance.

Commit per unit and push in batches. Continue implementation after a push. Once a PR is open and work is done, watch it until CI passes.

### Before a PR

Other worktrees often merge into `main` while a branch is in progress. Immediately before you create a PR:

1. Run `git fetch origin main`, then `git log --stat HEAD..origin/main`.
2. If all new commits change only docs (the definition above), do not rebase.
3. Otherwise, rebase onto `origin/main` and resolve the conflicts.
4. Use `git grep` to find each identifier that the branch removed or renamed. New code on `main` can use a removed name without a conflict.
5. Read the new code on `main` in the areas that the branch changed. Find text or logic that the change of the branch made incorrect.
6. Run the local checks again, then create the PR. In the PR, identify the manual and real-audio results that come from before the rebase.

[Rebase before a PR when main moved](solutions/workflow-issues/rebase-before-pr-when-main-moved.md) records the example that caused this rule.

### Local recognition checks

Keep real-model route matrices, paced recording tests, and long-audio tests local and opt-in. Do not add them to normal CI. Run the applicable local checks when capture, preprocessing, transcription, speaker recognition, routing, result reuse, or publication code changes. Keep audio, model paths, and raw results outside tracked files.

Use a short excerpt to diagnose a failure. Before completing a processing change, run the affected model and route combinations on at least five minutes of real audio. For changes to live processing, also use input paced at recording speed. For changes to checkpoints, restarts, or long-session state, add a test that crosses the affected duration boundary. Record the source duration, input pace, models, runtime, coverage, and result. A compressed-input stress test does not establish real-time performance.

Normal CI runs fast synthetic tests and a small public-audio test of the installed helper. That test checks package dependencies and native execution; it does not run the full model matrix or establish recognition quality. The recorder soak remains an explicit workflow-dispatch option.

The [recognition timing reference](reference/recognition-timings-2026-09-27.md) records the initial route measurements and their limits. Add a dated result when later processing changes need comparison. Keep earlier results identifiable by their build and test conditions.

## After a merge

Offer cleanup as the next step and ask before starting. Use the blocking question tool when available, with "Clean up now" recommended; otherwise ask in chat. Compound Engineering skills stop at the merge report, so make this offer explicitly.

After approval:

1. Confirm that the branch head is in `origin/main`.
2. Archive or remove the worktree with the host's managed tool.
3. Return to the main checkout on `main` and pull `main`.
4. Delete the local branch of the PR. If `git branch -d` refuses because the merge rewrote the commits, confirm that `gh pr view <number> -R twistymaze/longhand` shows `MERGED`, then use `git branch -D`.
5. Run `git fetch --prune origin`. GitHub deletes the head branch when the PR merges. If `git branch -r` still shows it, delete it with `git push origin --delete <branch>`.
6. Remove the subagent worktrees of this work, in `.claude/worktrees/agent-*`. Subagents that run with worktree isolation create them, and the managed tool of the session does not remove them. For each one, confirm that `git status --short` shows no changes and that its commits are in `main`: `git cherry main <branch>` shows only `-` lines, or the files of the commit are in `main` in a later form. Then run `git worktree remove <path>` and delete its branch. If a subagent worktree has changes or commits that are not in `main`, keep it and tell the user.
7. Run `ce-compound` for the work that the PR shipped.
8. Move the idea of the work to `done/` with `bun scripts/ideas.ts move <slug> done`, and write the PR in its file ([Feature ideas](ideas/README.md)). Also move any other idea that the PR made unnecessary. Then run `bun scripts/ideas.ts check`.
9. Commit the resulting docs on `main` and push them. Commit only this session's files by path, and run `git pull --rebase origin main` before the push.
10. Make sure that nothing is left. `git worktree list` shows only the main checkout and the worktrees of open work. `git branch` and `git branch -r` show only `main` and the branches of open PRs. Report each remaining item and its reason.

The Claude Code auto mode classifier blocks `git branch -D` and `git push origin --delete`. If it blocks a step, do not try another way. Give the user the exact commands and continue with the other steps.
