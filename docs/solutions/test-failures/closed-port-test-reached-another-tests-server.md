---
title: A closed-port test reached another test's server, because the helper gave its port back
date: 2026-10-25
category: test-failures
module: summarize and pipeline test helpers (crates/summarize/tests/common/mod.rs, crates/summarize/tests/client.rs, crates/pipeline/tests/runner.rs)
problem_type: test_failure
component: testing_framework
severity: medium
symptoms:
  - "Windows CI failed a_request_with_no_retry_waits_fails_at_once_without_the_network with: called `Result::unwrap_err()` on an `Ok` value: Completion { text: \"slow\", finish_reason: Some(\"stop\") }"
  - "The same test passed on macOS and on other Windows runs"
  - "The failing PR did not change the summarize crate"
root_cause: concurrency
resolution_type: test_fix
tags:
  - test-race
  - ephemeral-port
  - mock-server
  - windows-ci
  - closed-port
  - parallel-tests
---

# A closed-port test reached another test's server, because the helper gave its port back

## Problem

`closed_base()` was to give a URL on a port that nothing listens on. It bound `127.0.0.1:0`, read the port, and closed the listener. Tests in the same binary run in parallel, so another test's mock server could bind the same port before the request. On Windows the "closed" URL then reached that server, and the request succeeded.

## Symptoms

- On the Windows runner of PR #70: `a_request_with_no_retry_waits_fails_at_once_without_the_network ... FAILED`, at `crates\summarize\tests\client.rs`, with `Ok(Completion { text: "slow", ... })`. "slow" is the reply of another test's mock server.
- Two tests that started a server "later" on a port from `free_port()` had the same gap: another test could bind the port between the pick and the bind.

## What Didn't Work

- **A bound socket that does not listen, as the closed port for every test.** It keeps the port, so no other test can take it. But on macOS a connection to it is not refused. The connection request is dropped, so the client waits for its connect timeout. Each closed-port test took 7.8 s instead of 0 s, and it passed only because the timeout was also a connect error. That hides the "refused at once" case that the tests are for.

## Solution

PR #70 changes the helpers in `crates/summarize/tests/common/mod.rs` (the pipeline tests include the same file as `mod http`):

- `closed_base()` returns `http://127.0.0.1:0/v1` (`crates/summarize/tests/common/mod.rs:93-96`). A connection to port 0 fails with no wait on macOS (0.00 s, against 7.8 s for the dropped request), the Windows CI job passed with it, and no server can listen on port 0, so no other test can answer.
- `closed_socket()` returns a tokio `TcpSocket` that is bound but does not listen, and its port (`crates/summarize/tests/common/mod.rs:84-89`). It replaces `free_port()`. A test that starts its server later calls `socket.listen(1024)` on that socket (`crates/summarize/tests/client.rs:62-69`, `crates/pipeline/tests/runner.rs:5655-5680`). The port stays held from the start, and no connection succeeds until the test listens.

## Why This Works

The race needed a port number that was free for a time. Port 0 is never a listening port, so a request to it can only fail. The held socket keeps its port from the first line of the test, so no other test can bind it. For these two tests it does not matter whether the OS refuses or drops the connection request before `listen`: the client retries, and the request succeeds when the test listens.

## Prevention

- Never close a socket to "reserve" a port and then use the port number. Either use an address that cannot listen (port 0 for a closed URL), or keep the socket bound until you listen on it.
- On macOS, a bound socket that does not listen drops connection requests. It does not refuse them, so it is not a substitute for a closed port in a test that expects a fast failure.
- When a test fails only on Windows CI, look for state that parallel tests share (ports, files, environment). The project rule applies: fix the race in the same PR, also when the PR did not cause it.

## Related Issues

- `docs/solutions/test-failures/runner-test-stopped-before-the-title-job-that-the-save-queued.md`: another race that failed only on Windows CI, in `crates/pipeline/tests/runner.rs`.
