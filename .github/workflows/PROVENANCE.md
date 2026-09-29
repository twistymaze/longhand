# Provenance

Handy-derived steps in the CI workflow, one record per file (R27).

## `ci.yml`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `.github/workflows/build.yml` (the steps "import Apple Developer Certificate" and "verify certificate")
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — used in the `preview` job only; the keychain is a temporary file with a random password, it is added to the search list and not made the default, and a later step deletes it
