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
- On a remote Mac: zip the bundle with `ditto -c -k --keepParent`. Provide the `scp`, unzip, move-to-Applications, and `open` steps for that build. A remote Mac needs a notarized build or Gatekeeper stops the app. Explain this requirement and notarize when the user tests remotely.

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
4. Run `ce-compound` for the work that the PR shipped.
5. Commit the resulting docs on `main` and push them. Commit only this session's files by path, and run `git pull --rebase origin main` before the push.
