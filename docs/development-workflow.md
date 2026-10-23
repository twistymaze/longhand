# Development workflow

Read the applicable section before manual tests, validation, pushes, or cleanup. `CLAUDE.md` defines the project rules.

## Manual tests

At a manual test step, make a signed build immediately:

```sh
npm run tauri -- build --bundles app
```

Test microphone and Keychain behaviour from this bundle. An unsigned `tauri dev` binary attributes microphone requests to the launching terminal.

This bundle gets no updates. To test the updater, build with `LONGHAND_RELEASE=1` ([Manual test of the updater](solutions/workflow-issues/manual-test-of-the-updater-needs-a-release-flag-build.md)).

Give both ways to run the build, using the exact checkout path:

- On this Mac: `open <checkout>/target/release/bundle/macos/Longhand.app`.
- On a remote Mac: zip the bundle with `ditto -c -k --norsrc --noextattr --keepParent`. Without `--norsrc --noextattr`, the zip keeps extended attributes as `._*` files inside the bundle. `unzip` writes them as real files, which breaks the signature, and macOS then says that the app is damaged. Provide the `scp`, unzip, `spctl` and `open` steps for that build, and the cleanup step. Start the steps with the cleanup step too: an old build in the folder makes the unzip fail. A remote Mac needs a notarized build or Gatekeeper stops the app. Explain this requirement and notarize when the user tests remotely.

Never put a test build in `/Applications`. The app there is the installed release, and it gets updates. Unzip a test build into its own folder, such as `~/Downloads/longhand-test/`, and open it from there. Quit the installed release first: the test build has the same bundle ID, so the installed release can receive the launch instead. The test build uses the same app data as the installed release.

After the test, quit the test build and delete its folder:

```sh
rm -rf ~/Downloads/longhand-test
```

A local build is not a release build, so it gets no updates. The release in `/Applications` keeps its updates.

### Windows

A Mac cannot build the Windows installer. Each release has the Windows installer ([Releases](releases.md)). To test code on `main`, download the installer of the latest release into the gitignored `.context/windows-test/` folder:

```sh
gh release download -R twistymaze/longhand -p '*.exe' -D .context/windows-test
```

If the latest release has no installer yet, `release-windows.yml` is still building it, or it failed. Download the `release-windows` artifact of a successful run of that workflow in place of the release file: `gh run download <run-id> -R twistymaze/longhand -n release-windows -D .context/windows-test`. Code that is not on `main` has no Windows installer from CI. Give the PowerShell steps to copy it with `scp` into `$HOME\Downloads\longhand-test\` and run it. Start the steps with the removal of that folder, and give the cleanup steps: uninstall Longhand in Settings, delete the folder, and delete `.context/windows-test/` on the Mac.

The installer is not signed. A file copied with `scp` has no Mark of the Web, so SmartScreen does not check it. A browser download shows a SmartScreen warning.

## Checks and CI

The JavaScript tools are Node (`^24.15.0` or `^26`) and npm 12.2 or later. `devEngines` in `package.json` sets these versions. An older npm stops with `EBADDEVENGINES`. No Node release includes npm 12, so install it with `npm install --global npm@12`. In a new worktree, run `npm ci`. Do not copy `node_modules` from another checkout.

Run local checks, including:

- `npm run typecheck`: type-checks the app and the scripts (`tsconfig.scripts.json`).
- `npm test`: runs the frontend tests with Vitest. To run one file, use `npx vitest run src/<path>.test.ts`.
- `npm run test:scripts`: runs the script tests with `node --test`. To run one file, use `node --test scripts/<name>.test.ts`. Run each script with `node scripts/<name>.ts`.
- `scripts/check-windows.sh`: lints for Windows from macOS in seconds, except crates that build C or C++.
- `scripts/check-licences.sh`: requires cargo-about 0.9.2. Fails if the stored full licence list in `licences/generated/` does not match dependencies. Run it after each change that adds or removes a dependency, and commit the files that it writes. It runs `npm ci` first, so `js-packages.json` gets the versions in `package-lock.json`. CI runs the full check only when a licence input changed (`scripts/licence-inputs-changed.sh` names them); otherwise it compares only `js-packages.json`.

Dependency rules that apply to each change:

- A PR fails the `cargo deny` check when `Cargo.lock` or `package-lock.json` gains a version that is less than 7 days old, or a `package-lock.json` entry that does not come from the npm registry (`scripts/check-dependency-age.ts`). [Dependency age](#dependency-age) gives the steps for a younger version.
- `.npmrc` sets `allow-git=none` and `allow-remote=none`, so npm takes packages only from the registry.
- `.npmrc` sets `strict-allow-scripts=true`. [Install scripts](#install-scripts) gives the rules.
- A PR fails the `cargo deny` check when it adds an advisory that `main` does not have (`scripts/check-new-advisories.ts`). An advisory that is already on `main` does not fail a PR.
- `deny.toml` checks known vulnerabilities for the shipped targets only. Only the maintainer adds an entry to `[advisories] ignore`, and each entry has a `reason`. Run the check with `cargo deny check advisories`.

### Dependency age

`.npmrc` sets `min-release-age=7`, so `npm install` and `npm update` do not take a JavaScript package version that is less than 7 days old. `npm ci` accepts a version that is already in `package-lock.json`.

When you must have a younger version, for example for a security fix:

1. For a direct dependency, run `npm install <name>@<version> --package-lock-only --ignore-scripts --min-release-age-exclude=<name>`.
2. For a transitive dependency, first add an `overrides` entry for that exact version to `package.json`. Then run the same command.
3. The exclusion removes the age limit for that package completely. Accept the result only when `package-lock.json` has the exact fixing version. If it has a different version, do not use the result.
4. Add the version to `min-age-exceptions.toml` with a reason, and remove the entry again after 7 days.

### Install scripts

With `strict-allow-scripts=true`, `npm ci` stops before any install script runs when a package has an install script that `allowScripts` in `package.json` does not name. A PR that adds such a package must add an `allowScripts` entry, and its reason to this list:

- `esbuild: false`: esbuild takes its binary from an optional platform package and needs no `postinstall`.
- `fsevents: false`: fsevents includes a prebuilt `fsevents.node` and needs no install script.

### CI

CI runs only on:

- Pull requests into `main`.
- Pushes to `main` that change more than docs.
- Explicit runs with `gh workflow run ci.yml --ref <branch>`.

The repository variables `MACOS_RUNNER` and `WINDOWS_RUNNER` select the runner of each checks job, and `MACOS_RELEASE_RUNNER` selects the runner of the release build: a self-hosted runner, or a GitHub label such as the larger runner `macos-latest-xlarge`. With no variable, the job runs on GitHub's standard runner. To change the runner, use `gh variable set MACOS_RUNNER -R twistymaze/longhand -b <label>`. The self-hosted macOS runners are ephemeral: each job gets a new VM, and the VM is deleted after the job. Their environment file sets `LONGHAND_RUNNER_EPHEMERAL=1`, so the jobs use the GitHub Actions cache (rust-cache and sccache), the same as on GitHub's runners. The self-hosted Windows runner is persistent: it keeps the build output of the previous job and uses a local sccache folder. On 2026-10-20 the warm macOS checks took about 6 minutes on the self-hosted Mac Studio, 13.4 minutes on `macos-latest-xlarge` and 22.1 minutes on `macos-latest`; the CodeQL Rust analysis took about 13 minutes. The runner setup is in a private repository, because it contains details of the home network. To run the checks on GitHub's standard runners instead, use `gh workflow run ci.yml --ref <branch> -f hosted=true`. To see whether a runner is offline, use `gh api orgs/twistymaze/actions/runners --jq '.runners[] | [.name, .status] | @tsv'`. A job for an offline self-hosted runner waits in the queue. `cargo deny` and the Windows release installer build run on GitHub's runners. The macOS release build runs on GitHub's runners when `MACOS_RELEASE_RUNNER` is not set. On GitHub's runners, runs on `main` save the cache that new pull requests restore. For CI, docs are all Markdown files except `PROVENANCE.md`, plus `docs/`, `.claude/`, `.compound-engineering/` and the root `LICENSE`.

The Markdown files of the agent skill in `plugins/` match the docs pattern of the `main` push filter, so a push to `main` that changes only them runs no CI. They are not docs for the PR rule: each change to `plugins/` goes through a PR, and CI runs on each PR. Agents install the plugin from `main`, so a change there reaches users with no release.

Docs-only changes need no PR. Commit and push them directly on `main`; CI costs more than it checks for these changes. This includes plans, ideas, solutions, `CONCEPTS.md`, and agent guidance.

Commit per unit and push in batches. Continue implementation after a push. Once a PR is open and work is done, run `ce-babysit-pr` on it until the PR is ready to merge. Do this for each PR that you open, also for a small fix.

### Dependency and upstream upkeep

A weekly Claude routine on the maintainer's machine keeps the dependencies and the borrowed code current. Its instructions and scripts are in a private repository. It opens a `build(deps)` PR only for patch versions with fix-only release notes and for high or critical security fixes, and `fix(upstream)` and `fix` (code scanning) PRs on `upkeep/` branches. It marks each PR ready with the label `upkeep: ready`, and records all other items, such as proposed updates and alerts, in a private repository. The maintainer merges each ready PR. Routine PRs pass the same checks as all other PRs.

### Before a PR

Other worktrees often merge into `main` while a branch is in progress. Immediately before you create a PR:

1. Run `git fetch origin main`, then `git log --stat HEAD..origin/main`.
2. If all new commits change only docs (the definition above), do not rebase.
3. Otherwise, rebase onto `origin/main` and resolve the conflicts.
4. Use `git grep` to find each identifier that the branch removed or renamed. New code on `main` can use a removed name without a conflict.
5. Read the new code on `main` in the areas that the branch changed. Find text or logic that the change of the branch made incorrect.
6. Run the local checks again, then create the PR. In the PR, identify the manual and real-audio results that come from before the rebase.

[Rebase before a PR when main moved](solutions/workflow-issues/rebase-before-pr-when-main-moved.md) records the example that caused this rule.

### Skill script tests and the test storage folder

The scripts of the `longhand-notes` skill support Python 3.9 and later. Run their tests on Python 3.9 and on the current Python. uv downloads each Python:

```sh
uv run --no-project --python 3.9 python -m unittest discover -s plugins/longhand/skills/longhand-notes/scripts/tests -t plugins/longhand/skills/longhand-notes/scripts
uv run --no-project python -m unittest discover -s plugins/longhand/skills/longhand-notes/scripts/tests -t plugins/longhand/skills/longhand-notes/scripts
```

CI runs the same tests on Python 3.9 and 3.15, on macOS and Windows.

The Rust tests and the script tests read one checked-in test storage folder, `testdata/storage-folder/`. It is synthetic, and its files have LF line ends (`.gitattributes`). The real writers make it: `crates/library/tests/test_storage_folder.rs` writes the folder into a temporary folder and fails when the checked-in folder has other files or other bytes. After a change to a writer, write the folder again, and then read the diff of `testdata/`:

```sh
LONGHAND_UPDATE_TESTDATA=1 cargo test -p library --test test_storage_folder
```

`crates/library/tests/skill_drift.rs` runs the skill scripts on a folder that the real writers make, so a change to the format of a note or of the shared settings fails when the scripts do not know it. These tests use `uv run --no-project python`, or `python3` when uv is not installed. With neither, each test prints why it does not run and passes, except when `CI` is set: then it fails.

### Local recognition checks

Keep real-model route matrices, paced recording tests, and long-audio tests local and opt-in. Do not add them to normal CI. Run the applicable local checks when capture, preprocessing, transcription, speaker recognition, routing, result reuse, or publication code changes. Keep audio, model paths, and raw results outside tracked files.

Use a short excerpt to diagnose a failure. Before completing a processing change, run the affected model and route combinations on at least five minutes of real audio. For changes to live processing, also use input paced at recording speed. For changes to checkpoints, restarts, or long-recording state, add a test that crosses the affected duration boundary. Record the source duration, input pace, models, runtime, coverage, and result. A compressed-input stress test does not establish real-time performance.

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
8. Move the idea of the work to `done/` with `node scripts/ideas.ts move <slug> done`, and write the PR in its file ([Feature ideas](ideas/README.md)). Also move any other idea that the PR made unnecessary. Then run `node scripts/ideas.ts check`.
9. Commit the resulting docs on `main` and push them. Commit only this session's files by path, and run `git pull --rebase origin main` before the push.
10. Make sure that nothing is left. `git worktree list` shows only the main checkout and the worktrees of open work. `git branch` and `git branch -r` show only `main` and the branches of open PRs. Report each remaining item and its reason. Also report whether `ce-babysit-pr` watched the PR until it was ready to merge ([Checks and CI](#checks-and-ci)).

The Claude Code auto mode classifier blocks `git branch -D` and `git push origin --delete`. If it blocks a step, do not try another way. Give the user the exact commands and continue with the other steps.
