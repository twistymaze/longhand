# Lens: test-fidelity

The shard holds test code. Do not look for defects in the app here. Find tests that can pass while the behaviour that they check is broken, because that hides defects.

## Look for

- Assertions that cannot fail: a check of a value that the test itself set, an `assert` on a mock's return value, a comparison that is always true, or a test with no assertion.
- A test whose name or comment says it checks a behaviour that its assertions do not check.
- Mocks and fakes that behave differently from the real component in a way that matters for the test, for example a fake device that never fails, never delivers late data, or uses another sample format.
- Tests that do not run: marked to skip, behind a feature flag or an environment variable that CI does not set, in a file that the test runner does not find, or with an early `return` that skips the assertions.
- Tests that depend on the time zone, the current date, the locale, the order of a hash map, the file system's case rules, or a timing window, so they pass on one computer and fail on another, or pass when they should fail.
- Error paths that a test calls "tested" but that the test never reaches.
- Important behaviour in the app code next to the shard that no test checks. Report this only for data loss, recovery, or a state machine, and cite the app line.

## For each finding

`trigger` gives the defect in the app that the test would miss. `failure` explains why the test still passes.
