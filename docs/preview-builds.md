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

   A password is optional. In GitHub, the password and the key are secrets in the same environment, so the password protects only copies of the key file outside GitHub. Keep the private key file (and its password, if you set one) in your password manager. The public key is in `~/.tauri/longhand-updater.key.pub`. Put its contents in `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`. The public key is not secret.

3. **Make a Developer ID Application certificate and export it.** CI runs on a new Mac with an empty keychain, so the certificate and its private key go to CI as a `.p12` file. Only the Account Holder of the Apple Developer account can make this certificate.

   1. In Keychain Access, choose Keychain Access → Certificate Assistant → Request a Certificate From a Certificate Authority. Enter your Apple Account email address and your name, leave "CA Email Address" empty, select "Saved to disk", and save the file. Keychain Access also makes the private key in the login keychain.
   2. Open [Certificates](https://developer.apple.com/account/resources/certificates/list) in the Apple Developer account. Click "+", select "Developer ID Application", and click Continue.
   3. Select the profile type "G2 Sub-CA (Xcode 11.4.1 or later)". Upload the request file from step 1, click Continue, and download the `.cer` file.
   4. Open the `.cer` file. Keychain Access adds it to the login keychain, with the private key from step 1.
   5. In Keychain Access, select the login keychain and the "My Certificates" tab. Find "Developer ID Application: Ervin Walter (7GLQLN8HXK)" whose expiry date is 5 years from today. Its private key shows under it.
   6. Right-click that certificate, choose Export, select the format "Personal Information Exchange (.p12)", and set a password.
   7. If an older certificate with the same name is in the keychain, delete it (the certificate and its private key) after the new one works. `codesign` refuses a name that matches two identities, so local builds fail until then. Do not revoke the older certificate: apps signed with it continue to work until it expires.

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
   | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The updater key password. Do not set it for a key with no password. |
   | `BUILDS_TOKEN` | The token from step 5 |

   For a secret that is not a file, run `gh secret set <name> --env preview` and paste the value at the prompt. Add `-R twistymaze/longhand` if `gh` does not find the repository.

7. **Delete the local copies** of the `.p12` file. Keep the updater key only in your password manager.

## Expiry dates

| Credential | Expiry | Renewal |
|---|---|---|
| Developer ID Application certificate | 5 years after it is made | Do step 3 again and replace `APPLE_CERTIFICATE` and `APPLE_CERTIFICATE_PASSWORD`. The team ID stays the same, so installed builds accept updates, and the microphone and Keychain access stay. |
| App Store Connect API key (`.p8`) | None. It works until it is revoked. | Make a new key and replace `APPLE_API_KEY`, `APPLE_API_ISSUER` and `APPLE_API_PRIVATE_KEY`. |
| Updater key pair | None | See "A lost or leaked updater key". |
| `BUILDS_TOKEN` | The date set in step 5 | Make a new token and replace `BUILDS_TOKEN`. |

When a credential expires, the preview job fails and publishes nothing. Installed builds continue to work.

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
