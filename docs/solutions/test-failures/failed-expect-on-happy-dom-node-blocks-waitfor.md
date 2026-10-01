---
title: A failed expect on a happy-dom node blocked waitFor and timed out a test
date: 2026-10-01
category: test-failures
module: Frontend test environment (bun test, happy-dom, Testing Library)
problem_type: test_failure
component: testing_framework
severity: medium
symptoms:
  - "One test failed CI on main twice with \"this test timed out after 5000ms\" and passed on rerun both times"
  - "After the timeout, an \"Unhandled error between tests\" showed a failed text lookup on a page that belonged to the next test"
  - A waitFor with a 1 s limit took more than 2 s and did not fail with its own timeout
  - The test took 2.6 s locally and 3.0–4.0 s on CI while the tests next to it took 5–40 ms
root_cause: config_error
resolution_type: test_fix
framework_version: bun test with happy-dom and @testing-library/react
related_components:
  - frontend
tags:
  - bun-test
  - happy-dom
  - testing-library
  - waitfor
  - flaky-test
  - test-timeout
---

# A failed expect on a happy-dom node blocked waitFor and timed out a test

## Problem

`a read-only state makes the recognition choices read-only` (`src/settings/Models.test.tsx`) timed out on CI on `main` after the PR #29 and PR #30 merges. The cause was not a slow runner. Each time a `waitFor` check failed, bun used 2.3 s to make the failure message.

## Symptoms

- `(fail) a read-only state … [6251.48ms]` and `[5747.91ms]`, each with `this test timed out after 5000ms`.
- An `Unhandled error between tests` followed: `Unable to find an element with the text: The storage folder is not available…`. The DOM dump in that error showed the next test's page ("Downloading…" on Whisper), not the read-only page.
- "Re-run failed jobs" passed. The passing runs took 3.0–4.0 s, close to the limit.

## What Didn't Work

- **Calling it a runner stall.** After the first failure, the diagnosis was "the event loop stopped for 5–6 s on a loaded runner". A second failure in the same test disproved this. A random stall does not choose the same test twice.
- **Repeated local runs.** 30 runs of the file all passed, because a fast machine stays under 5 s. The pass/fail result hid the problem. The duration of each test showed it.

## Solution

Find the slow test and the slow step:

1. Get the duration of each test. Bun prints durations only for failures, unless CI is set. A JUnit report gives all of them:
   ```sh
   bun test ./src/ --reporter=junit --reporter-outfile=$TMPDIR/all.xml
   ```
   The read-only test took 2.6 s. The tests next to it took 4–15 ms.
2. Time the query and the assertion separately inside the `waitFor` callback. The query took 1 ms. The failed `expect(element).toBeNull()` took 2,290 ms.

The fix is in the test preload, `src/test/setup.ts:29`. It gives each DOM node a `Symbol.for("nodejs.util.inspect.custom")` method that returns `prettyDOM` markup. Bun uses this method when it prints a received value.

```ts
Object.defineProperty(Node.prototype, Symbol.for("nodejs.util.inspect.custom"), {
  configurable: true,
  value(this: Node) {
    if (this instanceof Element || this instanceof Document) {
      return prettyDOM(this, 2000, { highlight: false }) || this.nodeName;
    }
    return `${this.nodeName} ${JSON.stringify(this.textContent)}`;
  },
});
```

After the fix, the test takes 11 ms. `src/test/smoke.test.tsx:65` checks that a failed `expect` on an element prints `<p>` markup and no `Symbol(` internals.

## Why This Works

When `expect(received).toBeNull()` fails, bun prints `received`. For a happy-dom node, bun prints each property, including `[Symbol(listeners)]` maps, and follows references through `ownerDocument` to the whole page. The time increases with the size of the page: 56 ms for one paragraph, 2.3 s for the Models settings page.

This failure is the normal first step of `waitFor(() => expect(screen.queryByText("Checking…")).toBeNull())`, because the text is still on the page at the first check. The printing is synchronous, so no timers run during it. The 1 s timeout timer of `waitFor` cannot fire. Bun's 5 s test timer fires when the printing ends.

The second error comes from the same cause. After bun times out a test, the test's async function keeps running. `cleanup()` removes the page and the next test renders its own page. The `waitFor` check then passes, because the "Checking…" text is gone, and the next line searches the next test's page.

## Prevention

- **A `waitFor` that takes longer than its own timeout means that the thread was busy.** Look for synchronous work in the callback, such as printing a failure message, before you blame the runner.
- **Read the duration of each test, not only pass or fail.** A test that takes much more time than the tests next to it is near the timeout on a slower runner. Use `--reporter=junit` to see durations locally.
- **Keep the node printer in `src/test/setup.ts`.** It covers all 15 `waitFor(() => expect(query…).toBeNull())` checks in the suite and new ones. Do not change those checks to compare booleans to avoid the printing.
- **After a timeout, ignore a second error that comes after it.** It can come from the timed-out test's code running against the next test's page.
