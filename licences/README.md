# Licence files

- `sources/` contains upstream licence texts and third-party notices. Preserve their contents. Keep each component name in its filename.
- `manifest.json` records sources that the dependency tools cannot collect. It also supplies replacement texts when a package has only SPDX metadata. Paths are relative to the repository root.
- `generated/` contains the JavaScript package data and the full licence lists for macOS and Windows. The app embeds the platform lists. Do not edit these files by hand.

## Add or update a source

1. Put the upstream licence text and any required notices in `sources/`.
2. Add or update the manifest entry with the component version, platforms, source and file paths. Borrowed code must also have a `PROVENANCE.md` record that names the source file.
3. Run `scripts/check-provenance.sh` and `scripts/check-licences.sh` from the repository root. The licence check requires npm 12 and cargo-about 0.9.2. It installs the JavaScript packages with `npm ci`, builds the frontend and regenerates the lists. If the lists change, inspect and commit them with the source change.

Dependency tools do not detect every bundled file. When a workspace crate adds third-party models or other embedded data, add the required manifest entry. See [licence generator limitations](../docs/solutions/workflow-issues/full-licence-list-blind-spots.md).
