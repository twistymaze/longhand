---
title: A new permissive licence goes in the allow-list, not in a per-crate exception
date: 2026-09-26
category: workflow-issues
module: Licence gate (deny.toml, about.toml, scripts/check-licences.sh)
problem_type: convention
component: development_workflow
applies_when:
  - "cargo deny check licenses or scripts/check-licences.sh fails because a new dependency has a licence that is not on the list"
  - "A dependency update changes a crate's licence"
tags: [licences, cargo-deny, cargo-about, allow-list, cc0, public-domain, dependencies]
---

# A new permissive licence goes in the allow-list, not in a per-crate exception

## Context

Two tools check the licences of the code that ships in the binary. `cargo deny` reads `deny.toml`, and cargo-about reads `about.toml` to make the full licence list. Each file has an allow-list: MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0 and CC0-1.0. `about.toml` also accepts `blessing` for SQLite's notice.

PR #14 added the `notify` crate, which is CC0-1.0 (a public-domain dedication). CC0 was not on the lists, so the first fix added a per-crate exception for `notify` in both files. The user then decided that CC0 belongs in the allow-list itself. Public domain asks for less than the licences already on the list, which ask for a notice at most. So the exception only made the lists harder to read, and the next CC0 crate would need a new exception.

## Guidance

When a new dependency brings a licence that the gate rejects, first compare that licence with the ones on the list:

- **It is consistent with the list:** it is permissive and asks at most for a notice (examples: public domain, CC0-1.0, 0BSD, Unlicense). Add it to `allow` in `deny.toml` and to `accepted` in `about.toml`. Keep the two lists the same.
- **It asks for more:** for example, file-level copyleft (MPL-2.0), or a licence with conditions that the list does not hold. Add a per-crate exception in both files, with a comment that says why that crate cannot be avoided. The MPL-2.0 exceptions for the crates that Tauri needs are the model.
- **It is copyleft that reaches the binary** (LGPL, GPL): the gate fails the build by design. Find another crate.

After a change, run `cargo deny check licenses` and `scripts/check-licences.sh`. The second one needs cargo-about 0.9.2, and the JavaScript dependencies installed (`bun install`), because it runs `bun run build`.

The comment at the top of `deny.toml` states this rule.

## Why This Matters

A per-crate exception says that the licence was a special decision for that crate. That is right for MPL-2.0, which the project accepts only where it cannot avoid it. For a licence that is at least as permissive as the list, an exception is noise. It hides the real exceptions among routine ones, and each new crate with the same licence needs a new review and a new entry.

## When to Apply

- Adding or updating a Rust dependency that the licence gate rejects.
- Reviewing `deny.toml` or `about.toml` for an exception whose licence is permissive. A permissive licence that is rare can stay a per-crate exception: the allow-list is for licences that many crates use. For this reason, the `webpki-root-certs` exception (CDLA-Permissive-2.0, which reaches the graph only as a build dependency) stays an exception. The user decided this when CC0 moved.

## Examples

Before (PR #14; shortened, the comments and the other exceptions are left out):

```toml
# deny.toml
exceptions = [
  { crate = "notify", allow = ["CC0-1.0"] },
]
# about.toml
notify = { accepted = ["CC0-1.0"] }
```

After:

```toml
# deny.toml
allow = [ "MIT", …, "Unicode-3.0", "CC0-1.0" ]
# about.toml
accepted = [ "MIT", …, "Unicode-3.0", "CC0-1.0", "blessing" ]
```

## Related

- [full-licence-list-blind-spots](full-licence-list-blind-spots.md): what the full licence list's tools cannot see.
