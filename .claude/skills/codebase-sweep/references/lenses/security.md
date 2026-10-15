# Lens: security

Find security defects for the threat model in the profile. For a local desktop app, the main sources of untrusted data are files on disk (also files that another computer or an agent wrote), downloads, model output, and text that the user pastes.

## Look for

- Paths built from data: a name or id from a file or from model output that can contain `..`, an absolute path, or a separator, and so reads or writes outside the intended folder.
- Untrusted text that becomes markup, a command, a URL that the app opens, or a shell argument.
- Secrets (API keys, tokens) written to logs, error messages, files outside the secret store, or crash reports.
- Downloads and updates without a verified signature or checksum, or with a check that a failure skips.
- Commands that the backend exposes to the web view with more power than the interface needs.
- Temporary files with predictable names in shared folders.

## Do not report

A deliberate choice that the user makes, for example a setting that sends data to a service that the user selected. Follow the profile's threat model.
