#![cfg(feature = "test-helper")]
use nemo_runtime::Helper;
use serde_json::json;
use std::{
    path::Path,
    time::{Duration, Instant},
};
fn helper(mode: &str) -> Helper {
    let mut child = Helper::spawn(Path::new(env!("CARGO_BIN_EXE_nemo-test-helper")), 7, 3).unwrap();
    child.send(json!({"mode":mode}), vec![]).unwrap();
    child
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
        assert!(child.receive(Duration::from_secs(5)).is_err(), "{mode}");
        child.cancel().unwrap();
    }
}
#[test]
fn drains_stderr_without_blocking_results() {
    let mut child = helper("full_stderr");
    assert_eq!(
        child.receive(Duration::from_secs(5)).unwrap().metadata["event"],
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
    let start = Instant::now();
    child.cancel().unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
    child.cancel().unwrap();
}
#[test]
fn full_result_pipe_does_not_prevent_cancellation() {
    let mut child = helper("full_stdout");
    std::thread::sleep(Duration::from_millis(100));
    let start = Instant::now();
    child.cancel().unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
}
