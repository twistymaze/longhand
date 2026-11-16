#![cfg(feature = "test-helper")]
use nemo_runtime::Helper;
use serde_json::json;
use std::{path::Path, sync::mpsc, time::Duration};
fn helper(mode: &str) -> Helper {
    let mut child = Helper::spawn(Path::new(env!("CARGO_BIN_EXE_nemo-test-helper")), 7, 3).unwrap();
    child.send(json!({"mode":mode}), vec![]).unwrap();
    child
}
/// A hang limit: no step in these tests may take this long.
const HANG_LIMIT: Duration = Duration::from_secs(60);
/// Runs `step` on its own thread and fails the test if it does not end
/// within the hang limit.
fn within_hang_limit<T: Send + 'static>(step: impl FnOnce() -> T + Send + 'static) -> T {
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(step());
    });
    finished
        .recv_timeout(HANG_LIMIT)
        .expect("the step did not end within the hang limit")
}
#[test]
fn rejects_version_generation_revision_and_truncated_output() {
    for mode in [
        "version",
        "generation",
        "revision",
        "sequence",
        "truncated",
        "exit",
    ] {
        let mut child = helper(mode);
        assert!(child.receive(HANG_LIMIT).is_err(), "{mode}");
        child.cancel().unwrap();
    }
}
#[test]
fn drains_stderr_without_blocking_results() {
    let mut child = helper("full_stderr");
    assert_eq!(
        child.receive(HANG_LIMIT).unwrap().metadata["event"],
        "hello"
    );
    child.cancel().unwrap();
    assert_eq!(child.diagnostics().len(), 16384);
}
#[test]
fn cancellation_reaps_a_child_with_full_input_pipe() {
    let mut child = helper("blocked_input");
    for _ in 0..8 {
        let _ = child.send(json!({"command":"push"}), vec![0.0; 16000]);
    }
    let mut child = within_hang_limit(move || {
        child.cancel().unwrap();
        child
    });
    child.cancel().unwrap();
}
#[test]
fn full_result_pipe_does_not_prevent_cancellation() {
    let mut child = helper("full_stdout");
    within_hang_limit(move || {
        // The helper writes results until the result queue is full. Then
        // the reader stops, the pipe fills, and the helper blocks. The next
        // send sees the full queue and cancels the helper before it returns
        // the overflow error.
        loop {
            match child.send(json!({"command":"push"}), vec![]) {
                Err(error) if error.to_string() == "helper output queue overflow" => break,
                _ => std::thread::yield_now(),
            }
        }
        child.cancel().unwrap();
    });
}
