---
title: "The opener plugin's URL scope is a glob where * also matches /"
date: 2026-09-26
category: tooling-decisions
module: Settings window opener scope (src-tauri/capabilities/settings.json)
problem_type: tooling_decision
component: frontend
severity: medium
applies_when:
  - "A capability file grants opener:allow-open-url to a window, or changes its scope"
  - "A wildcard inside a host, such as https://*.github.com/*, looks like a safe scope entry"
  - "An exact scope entry does not match an address that looks the same"
  - "Work starts on clickable links in notes, which needs an opener permission in the main window"
tags: [tauri, tauri-plugin-opener, glob, capabilities, url-scope, settings-window, security]
---

# The opener plugin's URL scope is a glob where * also matches /

Paths that start with `tauri-plugin-opener/` or `glob/` are in the Cargo registry copies of tauri-plugin-opener 2.5.5 and glob 0.3.4, not in this repository.

## Context

Archived PR 13 made the addresses in Settings > About clickable. The pane opens them with `openUrl` from `@tauri-apps/plugin-opener` (`src/settings/About.tsx:6`, `:20`, `:63`). The Settings capability must allow each address, or the plugin refuses the call with `ForbiddenUrl`.

The allow-list in `src-tauri/capabilities/settings.json` had three shapes during archived PR 13:

1. 42 exact addresses.
2. 11 `https://<host>/*` patterns.
3. One `https://*` entry, chosen by the user (`src-tauri/capabilities/settings.json:11-14`).

The work needed a clear model of how the plugin matches an address against a scope entry.

## Guidance

### How the plugin matches

- The plugin compiles each `url` scope entry into a `glob::Pattern` (`tauri-plugin-opener/src/scope.rs:39-42`; the field type is at `:19-22`).
- `open_url` passes the address string from the frontend to the scope without change (`tauri-plugin-opener/src/commands.rs:19`, `:36-39`).
- `matches_url` calls `Pattern::matches` on that string (`tauri-plugin-opener/src/scope.rs:80-85`).
- `is_url_allowed` checks the deny entries first. A deny match refuses the address. Otherwise one allow match is enough (`tauri-plugin-opener/src/scope.rs:117-124`).
- `Pattern::matches` uses `MatchOptions::new()` (`glob/src/lib.rs:770-772`). Those options are `case_sensitive: true`, `require_literal_separator: false`, `require_literal_leading_dot: false` (`glob/src/lib.rs:1102-1107`).

Consequences:

- `*` matches any characters, including `/`, because `require_literal_separator` is false.
- The match covers the whole string. It is not a prefix or a substring test.
- The match is case-sensitive. `HTTPS://x.com/` does not match `https://*`.
- The address is not parsed or normalized. An exact entry must equal the address byte for byte, including a trailing slash.
- A pattern is bound to one host only when a literal `/` follows the host. The literal `/` stops `*` from taking part of the host.

### Pattern table

A scratch Rust program against glob 0.3.4 (offline, outside the repo) gave these results:

| Pattern | Address | Match | Reason |
|---|---|---|---|
| `https://*` | `https://github.com/a/b` | yes | `*` takes `/` |
| `https://*` | `file:///etc` | no | scheme differs |
| `https://*` | `HTTPS://x.com/` | no | case-sensitive |
| `https://github.com/*` | `https://github.com/a/b` | yes | host, then literal `/` |
| `https://github.com/*` | `https://github.com.example.net/x` | no | `.` is not the literal `/` |
| `https://github.com/*` | `https://github.com` | no | no `/` after the host |
| `https://github.com/*` | `https://GitHub.com/a` | no | case-sensitive |
| `https://*.github.com/*` | `https://docs.github.com/a` | yes | intended match |
| `https://*.github.com/*` | `https://evil.example/x.github.com/y` | yes | leading `*` takes `evil.example/x` — not host-bound |
| `https://tauri.app` | `https://tauri.app/` | no | exact entry, trailing slash |
| `https://tauri.app/` | `https://tauri.app/?q` | no | exact entry, whole string |

### How to choose a scope

- `opener:default` already allows `mailto:*`, `tel:*`, `http://*` and `https://*` (`tauri-plugin-opener/permissions/default.toml`, `tauri-plugin-opener/permissions/allow-default-urls.toml`). Use `opener:allow-open-url` with your own `allow` list when you want less.
- Capabilities apply per window (`"windows": [...]`). The Settings window has the opener. The main window (`src-tauri/capabilities/default.json`) and the licences window (`src-tauri/capabilities/licences.json`) do not.
- The risk is a scheme that is not a web page: `file:`, `smb:`, or a custom app scheme that starts a program. An `https://` address only opens a page in the browser.
- The allow-list is defence in depth. It limits what script we did not write can do if it runs in the window, for example text injected through a rendering bug.
- For a host allow-list, write `https://<host>/*`. Never put `*` before a literal host part, as in `https://*.<host>/*`.
- Pin the scope with a test that reads the capability file.

## Why This Matters

- A wrong mental model gives a false sense of safety. `https://*.github.com/*` looks host-bound, but it allows any host.
- Exact entries fail without warning on a small difference. `https://tauri.app` did not match `https://tauri.app/`.
- Exact entries and host lists need a capability edit for each new catalog model or new curated row. `https://*` does not, and it still blocks every non-web scheme.

## When to Apply

- The clickable-links-in-notes work (`docs/ideas/someday/clickable-links-in-notes.md`). The main window has no opener permission. That work must add one to `src-tauri/capabilities/default.json`, choose a scope, and add a test. Addresses in notes come from user text and transcripts, so the case for "https only" is stronger there.
- Any new window that opens addresses. Give it its own capability entry; a permission in one window does not apply to another.
- Any change to an opener scope, or any use of a `glob`-based scope in another Tauri plugin.
- An agent tool may refuse a scope change such as `https://*` as a security reduction. In archived PR 13, Claude Code's auto mode refused the first `https://*` edit. It accepted the edit after the user asked for it in their own words.

## Examples

Shape 1, exact addresses (first version in archived PR 13; entries shown are illustrative):

```json
{ "identifier": "opener:allow-open-url",
  "allow": [{ "url": "https://tauri.app/" }, { "url": "https://github.com/ggml-org/ggml" }, "..."] }
```

42 entries. `"https://tauri.app"` had to become `"https://tauri.app/"` to match. Each new catalog model or curated row needed an edit.

Shape 2, one pattern per host (entries shown are illustrative):

```json
"allow": [{ "url": "https://github.com/*" }, { "url": "https://huggingface.co/*" }, "..."]
```

11 entries. The test checked that each pattern names one host, that every address the pane can open matches one pattern, and that no pattern is unused.

Shape 3, https only (merged, `src-tauri/capabilities/settings.json:11-14`):

```json
{ "identifier": "opener:allow-open-url", "allow": [{ "url": "https://*" }] }
```

The test (`src/settings/About.test.tsx:139-168`) reads the capability file. It collects every address the pane can open: the acknowledgement links, and the leading address of each licence detail in the components list and the model catalog (through `leadingAddress`, `src/settings/About.tsx:48-52`). Then it asserts:

```ts
expect(allowed).toEqual(["https://*"]);
expect([...expected].filter((url) => !url.startsWith("https://"))).toEqual([]);
```

The first assertion stops a silent widening of the scope. The second stops an `http://` or other address that the opener would refuse.

## Related

- `docs/ideas/someday/clickable-links-in-notes.md`: the open follow-up. It needs a scope for the main window.
- `docs/solutions/ui-bugs/tauri-csp-img-src-blocks-inlined-app-icon.md`: another Tauri security setting whose real behaviour differs from how it reads.
- `docs/solutions/workflow-issues/full-licence-list-blind-spots.md`: a different gap with the same plugin, from the same PR.
