---
title: Signed Windows Releases and In-App Updates - Plan
type: feat
date: 2026-10-28
topic: windows-signed-releases-and-updates
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Signed Windows Releases and In-App Updates - Plan

## Goal Capsule

- **Objective:** A Windows user installs Longhand once from a signed installer that shows the publisher's name. After that, each new release comes to the Windows app as an in-app update, with the same rules as on macOS.
- **Product authority:** This plan. It covers the x64 Windows build of each release only. The Windows download button on longhand.md is a separate step after a real release proves this path (R16).
- **Open blockers:** The Azure Artifact Signing setup in Appendix A must be complete before a release can sign. Individual identity validation needs an Azure billing account of type Individual in the United States or Canada (A3). The implementation can start before the setup is complete, but no signed release can publish until it is.

---

## Product Contract

### Summary

Each release's Windows installer is signed through Azure Artifact Signing with the validated name of the maintainer. The Windows app of a release checks for updates and installs them with the macOS rules. The release order does not change: the release publishes after the macOS build, and the Windows workflow then attaches the signed installer and adds Windows to the update manifest.

### Problem Frame

Several colleagues of the maintainer now use Longhand on Windows at work. The Windows installer of each release is not signed. When a browser downloads it, SmartScreen shows an "unknown publisher" warning, and a managed work PC with a policy that blocks unsigned apps can refuse it. The installed Windows app never checks for updates, because the Windows build does not set the release flag. Each new release therefore needs a manual download and install on each Windows PC, and colleagues fall behind the macOS users.

`STRATEGY.md` says that installers and update paths come "only when a real person wants one". Real people now want one. The strategy text is out of date and gets a separate update (Scope Boundaries).

### Key Decisions

- **Sign with Azure Artifact Signing, as an individual.** Microsoft validates the maintainer's identity, and the publisher name is the validated legal name. No key file to keep, and about USD 10 a month for the Basic tier. (session-settled: user-directed — chosen over an OV certificate from a CA, SignPath Foundation and no signing: a CA certificate costs more and needs a cloud HSM; SignPath shows "SignPath Foundation" as publisher; no signing leaves managed PCs blocked.) Governs R1, R2, R13.
- **macOS first, Windows later.** The release publishes after the macOS build, as now. The Windows workflow adds Windows to the release after its build. (session-settled: user-directed — chosen over a release that waits for both builds: a Windows build failure must not block macOS users.) Governs R6, R7, R11.
- **x64 only.** (session-settled: user-directed — chosen over x64 and ARM64: no known ARM64 user, and a second Windows build adds CI time.) ARM64 PCs run the x64 build through emulation.
- **The website download waits for a real release.** (session-settled: user-directed — chosen over enabling the button in this change: the path must work on a real release first.) Governs R16.
- **Never ship an unsigned Windows build after signing starts.** An unsigned file would bring back the warning and the policy block that this work removes. Governs R3.
- **Per-user install.** A work PC user often has no admin rights, and an update must install without a UAC prompt. Governs R9.
- **No long-lived signing secret in GitHub.** GitHub Actions signs in to Azure through a federated credential that trusts the `release` environment of `twistymaze/longhand`, so no client secret expires or leaks. Governs R13.

### Requirements

**Signing**

- R1. Each Windows installer that a release offers is Authenticode-signed with the Artifact Signing certificate profile and timestamped, and the Longhand executable that it installs is signed too.
- R2. Windows shows the validated name as the publisher in the install prompt and in the file properties of the installer and the installed executable.
- R3. When signing fails, that release gets no Windows installer and no Windows update entry; the Windows workflow fails visibly.

**Updates**

- R4. The Windows app of a release checks for updates and shows the update window with the same schedule, the same What's new content and the same rule as macOS: it never offers or installs an update while a note records or processes.
- R5. When the user chooses to install, the Windows app closes, installs the update with no admin prompt, and opens again.
- R6. The update manifest of a release names the Windows update only after the signed installer of that release is attached. Until then, the Windows app finds no update and shows no error.
- R7. When the Windows build of a release fails or GitHub replaces its waiting run, Windows apps stay on their version until a later release has a Windows update.
- R8. The Windows app accepts only an update signed with the same updater key as macOS.

**Install**

- R9. The installer installs Longhand for the current user and needs no admin rights.
- R10. An install of the first signed release over an existing unsigned install keeps the notes, the settings, the saved API keys and the storage folder choice, with no uninstall.
- R11. The Windows build never delays or blocks the publication of a release or its macOS update.

**Setup and documentation**

- R12. `docs/releases.md` gets a step-by-step one-time setup for Artifact Signing and GitHub, at the same level of detail as its Apple steps. Each step says where to click or what command to run, and how to check the result. Appendix A is the draft.
- R13. The Windows release workflow signs with a federated Azure login from the `release` environment only, so pull request runs cannot sign.
- R14. The Expiry dates table in `docs/releases.md` lists the Artifact Signing credentials and how to renew each.
- R15. `docs/releases.md`, the Release entry in `CONCEPTS.md` and the Windows idea file describe the signed Windows installer and Windows updates in place of the unsigned installer.

**Validation**

- R16. After the PR merges, the first real release is checked on a Windows PC before the website download is turned on: a browser download of the installer shows the publisher name, the install works without admin rights over an older install, and the next release comes as an in-app update.

### Key Flows

- F1. First signed install
  - **Trigger:** A colleague downloads the signed installer of a release from its GitHub release page.
  - **Steps:** Windows shows the publisher name (SmartScreen can still warn until reputation builds). The installer runs for the current user and replaces the older unsigned install. Longhand opens with the existing notes and settings.
  - **Covered by:** R1, R2, R9, R10
- F2. Release with a Windows update
  - **Trigger:** A push to `main` makes a release.
  - **Steps:** The macOS build publishes the release. The Windows workflow builds and signs the installer, attaches it, and adds Windows to the update manifest. A Windows app finds the update on its next check and offers it when no note records or processes. The user installs; the app closes, installs and opens again.
  - **Covered by:** R4, R5, R6, R11
- F3. Windows build fails
  - **Trigger:** Signing or the Windows build of a release fails.
  - **Steps:** The release keeps its macOS files and update. It has no Windows installer and no Windows entry. Windows apps stay on their version. The next release with a successful Windows build brings them up to date.
  - **Covered by:** R3, R7

### Acceptance Examples

- AE1. **Covers R6.** Given release 1.9.0 publishes and its Windows build still runs, when a Windows app on 1.8.0 checks for updates, then it finds no update and shows nothing. When the Windows build attaches the signed installer, the next check offers 1.9.0.
- AE2. **Covers R4.** Given a Windows app records a note and an update is available, when the check runs, then the app holds the offer and shows it after the note stops and processing ends.
- AE3. **Covers R3, R7.** Given signing of release 1.9.0 fails, then 1.9.0 has no Windows installer, and a Windows app on 1.8.0 stays on 1.8.0. When release 1.9.1 builds and signs, the app is offered 1.9.1.
- AE4. **Covers R10.** Given a colleague runs an unsigned 1.8.0 install with notes, a storage folder and a saved API key, when they run the signed 1.9.0 installer, then Longhand opens on the same notes and folder, and the summary still works with the saved key.

### Success Criteria

- A colleague installs the signed release once, and each later release with a Windows build reaches them as an update offer with no manual download.
- The install prompt and the file properties name the maintainer as publisher. A SmartScreen warning on early downloads does not fail this criterion: reputation builds with downloads.
- No macOS release is later or blocked because of the Windows path.

### Scope Boundaries

- The Windows download button on longhand.md. It is a separate change after R16.
- A native ARM64 Windows build.
- Microsoft Store, winget and MSI distribution.
- A change to the release order (a release that waits for both builds) or immutable releases.
- Signing for local or manual test builds of Windows.
- The update of the out-of-date line in `STRATEGY.md`. It is a separate docs change on `main`.

### Dependencies / Assumptions

- A1. Signing works only after the maintainer completes Appendix A. Billing for the Artifact Signing account starts when the account is created.
- A2. Identity validation for an individual can take minutes or, when Microsoft asks for documents, up to 20 business days. Plan the setup before the implementation is ready to merge.
- A3. Individual validation needs an Azure billing account of type Individual whose legal name and sold-to address match the government ID. If the existing subscription bills to an organization account, the maintainer needs an Individual billing account first. If individual onboarding is not available, this plan returns to brainstorming to choose a CA certificate.
- A4. In-app updates download without the Mark of the Web, so SmartScreen does not check them. Signing matters for the first install and for policy checks such as Smart App Control.
- A5. App data lives in the per-user Tauri app data folder for the identifier `net.ewal.longhand`, and saved keys are in the Windows credential store under that identifier. A per-user reinstall keeps both. Planning confirms where the pointer to the storage folder is saved.

### Outstanding Questions

**Resolve Before Planning**

None.

**Deferred to Planning**

- The updater signature covers the installer bytes, so Authenticode signing must happen before Tauri writes the updater signature. Choose how the Tauri build calls Artifact Signing for each binary.
- How the Windows workflow adds the Windows entry to `latest.json` of a release that is already published, and what happens when a newer release publishes while the Windows build of the older one runs.
- The Windows build job holds the Azure login during a build. Decide how this keeps the rule that the job that can write to the repository runs no build code.
- Map the macOS install moments (at the click, at quit, at the next launch) to Windows, where the installer must run while the app is closed.
- Whether the Windows release build stays on `windows-latest` or uses the 8-core larger runner.
- Whether the update window or its text has macOS-only wording.
- The exact GitHub variable names and the federated credential subject, for Appendix A.

### Sources / Research

- Existing idea: `docs/ideas/next/windows-preview-builds-and-updates.md`.
- Current flow: `docs/releases.md`, `.github/workflows/release-windows.yml`, `scripts/publish-release.ts` (writes only `darwin-aarch64`), `src-tauri/src/commands/build_info.rs` (the `LONGHAND_RELEASE` flag), `src-tauri/src/managers/updates.rs` and `src-tauri/src/managers/updates/flow.rs` (the busy rule and the install moments).
- Earlier plans that deferred Windows updates: `docs/plans/2026-09-28-1240-feat-preview-builds-and-updates-plan.md`, `docs/plans/2026-10-11-1323-ci-releases-in-source-repository-plan.md`.
- Microsoft: [Artifact Signing quickstart](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart), [FAQ](https://learn.microsoft.com/en-us/azure/artifact-signing/faq), [signing integrations](https://learn.microsoft.com/en-us/azure/artifact-signing/how-to-signing-integrations), [GitHub action](https://github.com/azure/artifact-signing-action).
- Tauri: [Windows code signing](https://v2.tauri.app/distribute/sign/windows/).

---

## Appendix

### Appendix A. Draft setup guide: Artifact Signing and GitHub

This draft is from the Microsoft documentation of October 2026. The implementation checks it against the current portal and moves the final version into `docs/releases.md` (R12). Do the steps in this order. The names in `code` are suggestions; write down the names that you use.

**A. Check the billing account (5 minutes)**

1. Sign in to the [Azure portal](https://portal.azure.com/) with the account that owns your subscription.
2. Search for **Cost Management + Billing** and open it. Select **Billing scopes**, then your billing account, then **Properties**.
3. Check that **Account type** is **Individual**. If it is **Organization** or the page shows a company, stop and tell the agent: the plan needs another billing account (A3).
4. Check that the legal name and the sold-to address are exactly as on your driver's license or passport. Edit them there if they are not. The certificate shows your name, city, state and country.

**B. Register the resource provider (2 minutes)**

1. Search for **Subscriptions** and select your subscription.
2. Under **Settings**, select **Resource providers**.
3. Find **Microsoft.CodeSigning**, select it, and select **Register**. Wait until the status is **Registered**.

**C. Create the signing account (5 minutes; billing starts here)**

1. Search for **Artifact Signing Accounts** and select **Create**.
2. **Subscription:** your subscription. **Resource group:** **Create new**, `longhand-signing`.
3. **Account name:** for example `longhandsigning` (3 to 24 letters and digits, unique in Azure, starts with a letter).
4. **Region:** for example **East US**. Write down the endpoint of that region: East US is `https://eus.codesigning.azure.net`.
5. **Pricing:** **Basic**. Select **Review + Create**, then **Create**, then **Go to resource**.

**D. Give yourself the verifier role (2 minutes)**

1. In the signing account, select **Access control (IAM)** > **Add** > **Add role assignment**.
2. Select the role **Artifact Signing Identity Verifier**, select **Next**, add your own user, and select **Review + assign**.

**E. Validate your identity (15 minutes; you need your phone and your ID)**

1. Install **Microsoft Authenticator** on your phone if it is not installed.
2. In the signing account, under **Objects**, select **Identity validations**.
3. Change **Organization** to **Individual**, select **New identity**, then **Public**.
4. Select the billing account. The form fills in from it. Check the **Certificate subject preview**, then select **Create**. The status becomes **In Progress**.
5. When the status is **Action Required**, select your name, and open the link under "Please complete your verification here". Sign in with the same email address as in the request.
6. Select **Get verified here through our trusted ID-verifiers** and follow the AU10TIX steps: email PIN, phone number, then scan the QR code with your phone. Photograph your ID on a flat surface with no flash, and do the face check in even light.
7. On the phone, select **Open Authenticator** and add the Verified ID. Scan the next QR code in the browser, select the credential, and select **Share**.
8. The browser shows **Verification Successful**. After a few minutes the portal status is **Completed**. If Microsoft asks for documents (for example a utility bill), the email says which, and the wait can be up to 20 business days.

**F. Create the certificate profile (2 minutes)**

1. In the signing account, under **Objects**, select **Certificate profiles** > **Create** > **Public Trust**.
2. **Certificate Profile Name:** for example `longhand`. Keep **Program Type** at **None**.
3. **Verified CN and O:** select your completed validation. Leave **Include street address** and **Include postal code** clear. Select **Create**.

**G. Let GitHub sign with no secret (10 minutes)**

1. Search for **Microsoft Entra ID** > **App registrations** > **New registration**. Name it `longhand-release-signing`, keep the defaults, and select **Register**. Write down the **Application (client) ID** and the **Directory (tenant) ID**.
2. In the app, select **Certificates & secrets** > **Federated credentials** > **Add credential**. Choose the scenario **GitHub Actions deploying Azure resources**. **Organization:** `twistymaze`. **Repository:** `longhand`. **Entity type:** **Environment**. **GitHub environment name:** `release`. Name it `longhand-release` and select **Add**.
3. Go back to the signing account > **Access control (IAM)** > **Add role assignment**. Select the role **Artifact Signing Certificate Profile Signer**, then **Next** > **Select members**, search for `longhand-release-signing`, and select **Review + assign**.
4. Write down your **Subscription ID** (Subscriptions page).

**H. Put the values in GitHub (5 minutes)**

These values are not secret, so they go in as environment variables of the `release` environment. Run each command and paste the value when it asks. The final names come from the implementation.

```bash
gh variable set AZURE_CLIENT_ID --env release -R twistymaze/longhand
```

```bash
gh variable set AZURE_TENANT_ID --env release -R twistymaze/longhand
```

```bash
gh variable set AZURE_SUBSCRIPTION_ID --env release -R twistymaze/longhand
```

```bash
gh variable set ARTIFACT_SIGNING_ENDPOINT --env release -R twistymaze/longhand
```

```bash
gh variable set ARTIFACT_SIGNING_ACCOUNT --env release -R twistymaze/longhand
```

```bash
gh variable set ARTIFACT_SIGNING_PROFILE --env release -R twistymaze/longhand
```

Check that this command lists the 6 names:

```bash
gh variable list --env release -R twistymaze/longhand
```

**I. Renewal**

- The signing certificates last 3 days, and Azure renews them by itself. The timestamp keeps old signatures valid.
- Microsoft sends reminders from 60 days before the identity validation expires. Renew it then, or signing stops.
- The federated credential does not expire.
- Delete the signing account to stop billing; signed files stay valid.
