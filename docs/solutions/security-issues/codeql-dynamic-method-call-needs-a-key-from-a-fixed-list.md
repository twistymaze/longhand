---
title: CodeQL did not accept an own-key check before a call by a name from the URL
date: 2026-10-28
category: security-issues
module: website screenshots (src/screenshots/main.tsx)
problem_type: security_issue
component: frontend
severity: medium
symptoms:
  - "The CodeQL check failed on PR #75 with the rule `js/unvalidated-dynamic-method-call` (Unvalidated dynamic method call)"
  - "The alert was on `scenes[name]()`, where `name` came from `new URLSearchParams(window.location.search).get(\"scene\")`"
  - "After a fix with `Object.prototype.hasOwnProperty.call(scenes, requested)`, CodeQL opened a second alert on the same call"
root_cause: missing_validation
resolution_type: code_fix
tags:
  - codeql
  - code-scanning
  - dynamic-method-call
  - typescript
  - url-query
---

## Problem

The screenshot preview page (`src/screenshots/main.tsx`) calls one scene function. The `?scene=` query names it. The page called `scenes[name]()` with that name. CodeQL reports this as an unvalidated dynamic method call, because a query value can name an inherited property such as `constructor` or `toString`. The alert failed the CodeQL check, so the PR could not merge.

## What did not work

An own-key check before the call did not clear the alert:

```ts
const requested = new URLSearchParams(window.location.search).get("scene") ?? "library";
if (!Object.prototype.hasOwnProperty.call(scenes, requested)) throw new Error(`no scene named ${requested}`);
const scene = scenes[requested as SceneName]();
```

The check is correct at run time, but CodeQL does not use it as a sanitizer for this rule. The value in the call is still the query value, so CodeQL opened a second alert on the same line. `Object.hasOwn` is not an option either: the app's TypeScript `lib` is older than ES2022.

## Solution

Take the key from the object's own list of keys, and call the function with that key, not with the query value:

```ts
const requested = new URLSearchParams(window.location.search).get("scene") ?? "library";
const name = (Object.keys(scenes) as SceneName[]).find((key) => key === requested);
if (!name) throw new Error(`screenshots: no scene named ${requested}`);
const scene = scenes[name]();
```

After this change, CodeQL marked both alerts (179 and 180) fixed.

## Prevention

When a URL value, a message or another outside value selects a function, find the key in a fixed list (`Object.keys`, an array of names, or a `switch`) and use the found key in the call. A guard that only tests the outside value is not enough for CodeQL, even when it is correct.

Code scanning also reopens a resolved review thread until all analyses of the new commit finish. A reopened thread on a fixed alert needs only another resolve.
