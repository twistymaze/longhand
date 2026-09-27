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

## After a merge

Offer cleanup as the next step and ask before starting. Use the blocking question tool when available, with "Clean up now" recommended; otherwise ask in chat. Compound Engineering skills stop at the merge report, so make this offer explicitly.

After approval:

1. Confirm that the branch head is in `origin/main`.
2. Archive or remove the worktree with the host's managed tool.
3. Return to the main checkout on `main` and pull `main`.
4. Run `ce-compound` for the work that the PR shipped.
5. Commit the resulting docs on `main` and push them. Commit only this session's files by path, and run `git pull --rebase origin main` before the push.
