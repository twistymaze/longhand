# Preview builds

Each push to `main` that passes CI publishes a preview build (`CONCEPTS.md`, Updates). The `preview` job in `.github/workflows/ci.yml` signs, notarizes and staples the app and the DMG. Then `scripts/publish-preview.ts` publishes them to the public repository `twistymaze/longhand-builds`:

- A pre-release tagged `preview-<YYYYMMDDHHMMSS>` holds the DMG, `Longhand.app.tar.gz` and `Longhand.app.tar.gz.sig`.
- `preview/latest.json` on `main` is the update manifest. The app reads it from `https://raw.githubusercontent.com/twistymaze/longhand-builds/main/preview/latest.json`.
- CI keeps the 20 newest preview releases and deletes the others.

The source repository stays private. The builds repository holds only built files.

## One-time setup

Do these steps once, in this order.

1. **Create the builds repository.** Create `twistymaze/longhand-builds` as a public repository with a README, so that the `main` branch exists.

2. **Make the updater key pair.** On your Mac:

   ```sh
   bun tauri signer generate -w ~/.tauri/longhand-updater.key
   ```

   Set a password when it asks. Keep the private key file and its password in your password manager. The public key is in `~/.tauri/longhand-updater.key.pub`. Put its contents in `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`. The public key is not secret.

3. **Export the Developer ID certificate.** In Keychain Access, find "Developer ID Application: Ervin Walter (7GLQLN8HXK)", with its private key. Export both as a `.p12` file with a password.

4. **Create the `preview` environment.** In the source repository, go to Settings → Environments → New environment. Name it `preview`. Under "Deployment branches and tags", select "Selected branches and tags" and add the rule `main`. Pull request runs then cannot read its secrets.

5. **Make the publish token.** In GitHub, go to Settings → Developer settings → Fine-grained tokens. Make a token for the `twistymaze` owner with access to `twistymaze/longhand-builds` only. Give it the repository permission "Contents: Read and write". Set an expiry date and a reminder to renew it.

6. **Add the secrets to the environment.** Run each command from the source checkout. `gh secret set` asks for each value, so no value goes into the shell history.

   | Secret | Value |
   |---|---|
   | `APPLE_CERTIFICATE` | The `.p12` file, base64: `base64 -i DeveloperID.p12 \| gh secret set APPLE_CERTIFICATE --env preview` |
   | `APPLE_CERTIFICATE_PASSWORD` | The `.p12` password |
   | `APPLE_API_KEY` | The App Store Connect API key ID |
   | `APPLE_API_ISSUER` | The App Store Connect issuer ID |
   | `APPLE_API_PRIVATE_KEY` | The `.p8` file: `gh secret set APPLE_API_PRIVATE_KEY --env preview < <path to .p8>` |
   | `TAURI_SIGNING_PRIVATE_KEY` | The updater private key: `gh secret set TAURI_SIGNING_PRIVATE_KEY --env preview < ~/.tauri/longhand-updater.key` |
   | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The updater key password |
   | `BUILDS_TOKEN` | The token from step 5 |

   For a secret that is not a file, run `gh secret set <name> --env preview` and paste the value at the prompt. Add `-R twistymaze/longhand` if `gh` does not find the repository.

7. **Delete the local copies** of the `.p12` file. Keep the updater key only in your password manager.

## Install the first build

An installed local build gets no updates. Install the DMG of the newest preview build once:

1. Open the releases of `twistymaze/longhand-builds` and download the DMG of the newest `preview-` release.
2. Open the DMG and drag Longhand to Applications. Replace the old app.

Later preview builds then come as updates.

## When a build does not publish

- **The checks failed.** The preview job does not run. The manifest continues to name the previous build.
- **Signing or notarization failed.** The job fails before it publishes. At most a draft release stays, and the next publish deletes it.
- **"is not higher than".** The manifest already names this commit or a newer one. The job ends without an error. This occurs for a re-run, and for an older commit that finished after a newer one.
- **"more than 10 minutes after the job start".** The commit time is in the future. Correct the clock of the computer that made the commit, then push a new commit.

## A lost or leaked updater key

Installed builds accept only updates signed with the private key whose public key they contain.

- **Lost key or password:** installed builds cannot accept any later update.
- **Leaked key:** another person can sign an update that installed builds accept.

In both cases:

1. Make a new key pair (step 2) and put the new public key in `tauri.conf.json`.
2. Replace `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in the `preview` environment.
3. Push to `main`, so that CI publishes a build with the new key.
4. Tell each user to install the DMG of that build by hand. Installed builds cannot move to a new key through an update.
