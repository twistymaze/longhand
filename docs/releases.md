# Releases

Each push to `main` that changes the shipped app goes out in a numbered release on the [Releases page](https://github.com/twistymaze/longhand/releases) of `twistymaze/longhand` (`CONCEPTS.md`, Updates). Each release is a patch release by default. A minor release occurs only when the user chooses one, and a major release occurs only when the user asks for one.

## Release flow

One CI run on `main` does the checks, the build and the publication. `.github/workflows/ci.yml` runs these jobs in sequence:

1. **The checks** (`checks` and `cargo deny`). When a check fails, no later job runs.
2. **`release-build`** runs on `macos-latest` in the `release` environment. It can only read the repository.
   1. `scripts/release-version.ts` uses git-cliff and `cliff.toml` to decide whether a release is due, to get the next patch version and to write the release notes. The script then raises the version when a commit subject has a `+` or `!` marker (see [When a release is due](#when-a-release-is-due)).
   2. When a release is due, the job builds the app with that version, and signs, notarizes and staples the app and the DMG. Tauri also makes the updater archive `Longhand.app.tar.gz` and its signature.
   3. The job uploads the DMG, the archive, the signature and the notes as a workflow artifact.
3. **`release-publish`** runs on `ubuntu-latest` only when a release is due. It is the only job that can write to the repository. It has no secrets and runs no build code. `scripts/publish-release.ts publish` creates a draft release with the tag `v<version>` on the built commit, uploads the 4 files and `latest.json`, and then publishes the draft as the latest release.

The tag and the release come into existence only after the macOS build succeeds. A failed build leaves no tag.

After a CI run on `main` succeeds, `.github/workflows/release-windows.yml` starts. It finds the release whose target is the commit that CI tested. When no release has that commit, it ends without an error. Otherwise, it builds the unsigned Windows NSIS installer on a standard GitHub-hosted Windows runner and attaches it to the release. This workflow has its own concurrency group, so the long Windows build does not hold back the next CI run on `main`.

CI runs on `main` one at a time. When more runs wait, GitHub keeps only the newest waiting run. The commits of a replaced run go out in the release of the next run. When GitHub replaces a waiting Windows run, that release gets no installer.

When a newer CI run on `main` is queued or running at the time of publication, the publish step publishes nothing. It logs "a newer CI run on main is queued or running" and ends without an error. The newer run publishes these commits in its release. Without this rule, publication can fail: when a newer commit changed a file in `.github/workflows/`, GitHub refuses to let the workflow token create a tag on the older commit (403, "Resource not accessible by integration").

A push of only docs does not start CI (see [Checks and CI](development-workflow.md#checks-and-ci)). Its changes go out in the next release.

### When a release is due

A release is due when no `v<major>.<minor>.<patch>` tag exists, or when the commits since the latest release tag change the version. Other tags, such as old preview tags, do not count. The first release is 1.0.0.

| Commit subject | Effect on the version |
|---|---|
| `feat`, `fix`, `perf`, `refactor`, `revert`, `build` | patch (1.0.3 to 1.0.4) |
| `feat+: …` or `feat(scope)+: …` | minor (1.0.4 to 1.1.0) |
| a release type with `!`, for example `feat!: …` | major (1.4.2 to 2.0.0) |
| `docs`, `ci`, `test`, `chore`, `style` | no release; the change goes out in the next release. `!` on these types has no effect. |

When the commits since the latest release tag have different effects, the highest effect applies. With no release tag, the first release is 1.0.0, whatever the markers.

git-cliff gives the next patch version (`bump_type = "patch"` in `cliff.toml`). `scripts/release-version.ts` reads the original commit subjects since the latest release tag and raises that version for a `+` or `!` marker. The script fails when git-cliff does not give the next patch. git-cliff does not understand `+`, so `cliff.toml` removes it before parsing. The notes show a `feat+` commit in the same way as a `feat` commit.

The release notes list only the commits that make a release, under these headings:

| Heading | Commit types |
|---|---|
| New features and improvements | `feat` |
| Fixes | `fix`, `perf` |
| Other changes | `refactor`, `revert`, `build` |

A heading with no commits does not appear.

### Minor releases

A minor release occurs only when the user chooses one. Before an agent opens a `feat` PR while the user is present, it asks "patch or minor?" and gives a recommendation. When the user chooses minor, the agent writes `+` after the type or scope, for example `feat(capture)+: record system audio`. A hands-off run, such as lfg, never writes `+`. The user can add `+` to the title of any `feat` PR before merge. `+` is not valid on other types, so a release with only fixes is never a minor release.

### Major releases

A major release occurs only when the user tells an agent in chat to make one. The agent then writes `!` after the type in the PR title, for example `feat!: change the storage format`, and the version goes from 1.4.2 to 2.0.0. Agents never write `!` or `BREAKING CHANGE` in a PR title otherwise. A `BREAKING CHANGE` footer has no effect, because the release script reads only the commit subject.

### PR titles

The repository allows only squash merges. The squash commit title is the PR title, and the squash commit body is empty. So each commit on `main` is one PR title, and the PR title decides the release.

The workflow `.github/workflows/pr-title.yml` checks that each PR title is a Conventional Commit (`type: description` or `type(scope): description`) with one of the types in the table. `scripts/check-pr-title.sh` then checks the markers: it rejects `+` on types other than `feat`, `+` together with `!`, and `!` on types that make no release. The job, `PR title`, is a required check of the `main` ruleset. A commit pushed directly to `main` (docs-only work) uses only `docs`, `chore` or `ci`. `CLAUDE.md` gives the rules for how agents choose the type and the markers.

If someone edits the squash commit title in the merge dialog, the edited title becomes the commit on `main`, and no check sees it.

## One-time setup

Do these steps once, in this order. Do a step again only to replace a credential (see [Expiry dates](#expiry-dates)).

1. **Make the updater key pair.** On your Mac:

   ```sh
   npm run tauri -- signer generate -w ~/.tauri/longhand-updater.key
   ```

   Do not set a password. In GitHub, a password and the key are secrets in the same environment, so a password protects only copies of the key file outside GitHub. CI does not set `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Keep the private key file in your password manager. The public key is in `~/.tauri/longhand-updater.key.pub`. Put its contents in `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`. The public key is not secret.

2. **Make a Developer ID Application certificate.** CI runs on a new Mac with an empty keychain, so the certificate and its private key go to CI as a `.p12` file. Only the Account Holder of the Apple Developer account can make this certificate.

   1. In Keychain Access, choose Keychain Access → Certificate Assistant → Request a Certificate From a Certificate Authority. Enter your Apple Account email address and your name, leave "CA Email Address" empty, select "Saved to disk", and save the file. Keychain Access also makes the private key in the login keychain.
   2. Open [Certificates](https://developer.apple.com/account/resources/certificates/list) in the Apple Developer account. Click "+", select "Developer ID Application", and click Continue.
   3. Select the profile type "G2 Sub-CA (Xcode 11.4.1 or later)". Upload the request file from step 1, click Continue, and download the `.cer` file.
   4. Open the `.cer` file. Keychain Access adds it to the login keychain, with the private key from step 1.
   5. In Keychain Access, select the login keychain and the "My Certificates" tab. Find "Developer ID Application: Ervin Walter (7GLQLN8HXK)" whose expiry date is 5 years from today. Its private key shows under it.
   6. If an older certificate with the same name is in the keychain, delete it (the certificate and its private key) after the new one works. `codesign` refuses a name that matches two identities, so local builds fail until then. Do not revoke the older certificate: apps signed with it continue to work until it expires.

3. **Create the `release` environment.** In `twistymaze/longhand`, go to Settings → Environments → New environment. Name it `release`. Under "Deployment branches and tags", select "Selected branches and tags" and add the rule `main`. Pull request runs then cannot read its secrets.

4. **Add the 6 secrets to the environment.** Each `gh secret set` command asks for the value or reads it from a file, so no value goes into the shell history or a chat.

   1. The App Store Connect API key is the one `.p8` file in `~/.appstoreconnect/private_keys`. Its file name is `AuthKey_<key ID>.p8`. The Issuer ID is at the top of App Store Connect → Users and Access → Integrations → Team Keys.
   2. Developer ID certificate. In Keychain Access, open the login keychain and the My Certificates tab. Right-click "Developer ID Application: Ervin Walter (7GLQLN8HXK)", choose Export, select "Personal Information Exchange (.p12)", save it as `~/Desktop/DeveloperID.p12`, and set a new password. Then run:

      ```sh
      base64 -i ~/Desktop/DeveloperID.p12 | gh secret set APPLE_CERTIFICATE --env release -R twistymaze/longhand
      ```

   3. Certificate password. Run this command and paste the password from step 2:

      ```sh
      gh secret set APPLE_CERTIFICATE_PASSWORD --env release -R twistymaze/longhand
      ```

   4. API key ID. This command takes the key ID from the `.p8` file name:

      ```sh
      basename ~/.appstoreconnect/private_keys/AuthKey_*.p8 .p8 | sed 's/^AuthKey_//' | tr -d '\n' | gh secret set APPLE_API_KEY --env release -R twistymaze/longhand
      ```

   5. Issuer ID. Run this command and paste the Issuer ID from step 1:

      ```sh
      gh secret set APPLE_API_ISSUER --env release -R twistymaze/longhand
      ```

   6. API private key:

      ```sh
      gh secret set APPLE_API_PRIVATE_KEY --env release -R twistymaze/longhand < ~/.appstoreconnect/private_keys/AuthKey_*.p8
      ```

   7. Updater key. If the file is gone, run the command without `< …` and paste the key from the password manager:

      ```sh
      gh secret set TAURI_SIGNING_PRIVATE_KEY --env release -R twistymaze/longhand < ~/.tauri/longhand-updater.key
      ```

   8. Delete `~/Desktop/DeveloperID.p12`. Then check that this command lists the 6 names:

      ```sh
      gh secret list --env release -R twistymaze/longhand
      ```

A wrong secret makes `release-build` fail at the certificate import or at notarization, and nothing publishes.

## Expiry dates

| Credential | Expiry | Renewal |
|---|---|---|
| Developer ID Application certificate | 5 years after it is made | Do setup step 2 again, and replace `APPLE_CERTIFICATE` and `APPLE_CERTIFICATE_PASSWORD` (steps 4.2 and 4.3). The team ID stays the same, so installed builds accept updates, and the microphone and Keychain access stay. |
| App Store Connect API key (`.p8`) | None. It works until it is revoked. | Make a new key and replace `APPLE_API_KEY`, `APPLE_API_ISSUER` and `APPLE_API_PRIVATE_KEY` (steps 4.4 to 4.6). |
| Updater key pair | None | See [A lost or leaked updater key](#a-lost-or-leaked-updater-key). |

Publication uses the workflow's own `GITHUB_TOKEN`, which needs no renewal. When a credential expires, `release-build` fails and nothing publishes. Installed builds continue to work.

## Install the first build

A local build gets no updates. Install the DMG of the latest release once:

1. Open the [latest release](https://github.com/twistymaze/longhand/releases/latest) and download its DMG.
2. Open the DMG and drag Longhand to Applications. Replace the old app.

Later releases then come as updates.

Installed preview builds from before 1.0.0 read their update manifest from `twistymaze/longhand-builds`. A one-time step after the first release writes the `latest.json` of 1.0.0 to that repository, so that each preview build updates to 1.0.0 once. After that update, the app reads the update manifest of the latest release in `twistymaze/longhand`. Then the old repository is archived.

## When a release does not publish

- **No release is due.** The commits since the latest release tag have only the types `docs`, `ci`, `test`, `chore` or `style`. `release-build` ends after the version step, and `release-publish` does not run. Their changes go out in the next release.
- **The checks failed.** `release-build` does not run.
- **The build failed.** For example, signing or notarization failed. The run publishes nothing and creates no tag. At most a draft release stays, and the next publication of that version deletes it. To publish the release, merge a fix PR with a `fix:` or `ci:` title. Its run on `main` computes the version from all commits since the latest release tag, so it publishes the release that the failed run did not publish.
- **A re-run.** A re-run uses the workflow and the scripts of the original commit. So a re-run helps only for a fault that needs no code change, for example an outage of the notarization service. A re-run of a run that published its release finds no release due. When a newer release exists, the publish step logs "is not higher than" and ends with "skipped", without an error.
- **The Windows installer is missing.** The Windows build failed, or GitHub replaced its waiting run. The release keeps its macOS files. To build and attach the installer again, for example after a fix to `release-windows.yml`, run:

  ```sh
  gh workflow run release-windows.yml -f tag=v<version> -R twistymaze/longhand
  ```

  The run replaces an installer of the same name.

## A lost or leaked updater key

Installed builds accept only updates signed with the private key whose public key they contain.

- **Lost key:** installed builds cannot accept any later update.
- **Leaked key:** another person can sign an update that installed builds accept.

In both cases:

1. Make a new key pair (setup step 1) and put the new public key in `tauri.conf.json`.
2. Replace `TAURI_SIGNING_PRIVATE_KEY` in the `release` environment (setup step 4.7).
3. Merge the `tauri.conf.json` change in a PR with a `fix:` title, so that CI publishes a release with the new key.
4. Tell each user to install the DMG of that release by hand. Installed builds cannot move to a new key through an update.
