//! Against the real operating system. Ignored by default: CI runners do not
//! expose power assertions reliably, and a unit test cannot say whether the
//! OS honoured one. Run with `--ignored --test-threads=1`.

use std::time::Duration;

#[cfg(target_os = "macos")]
fn our_assertion_is_held() -> bool {
    let out = std::process::Command::new("pmset")
        .args(["-g", "assertions"])
        .output()
        .expect("pmset runs");
    let text = String::from_utf8_lossy(&out.stdout);
    let pid = format!("pid {}(", std::process::id());
    text.lines().any(|l| {
        l.contains(&pid) && l.contains("PreventUserIdleSystemSleep") && l.contains(power::REASON)
    })
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "queries the real power manager"]
fn the_assertion_is_held_while_capturing_and_released_within_a_second() {
    assert!(!our_assertion_is_held(), "nothing held before capture");
    let guard = power::IdleGuard::acquire().expect("assertion granted");
    assert!(our_assertion_is_held());
    std::thread::sleep(Duration::from_secs(2));
    assert!(our_assertion_is_held(), "still held later in capture");
    drop(guard);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!our_assertion_is_held(), "released after stop");
}

/// `SetThreadExecutionState` is per thread: a helper thread that ends must
/// not take the capture thread's assertion with it. Check with
/// `powercfg /requests` (elevated) while this runs.
#[cfg(windows)]
#[test]
#[ignore = "verify with powercfg /requests"]
fn a_helper_thread_ending_leaves_the_capture_threads_assertion_held() {
    let guard = power::IdleGuard::acquire().expect("assertion granted");
    std::thread::spawn(|| {
        let _helper = power::IdleGuard::acquire();
    })
    .join()
    .unwrap();
    println!("capture thread still holds the assertion; check powercfg /requests now");
    std::thread::sleep(Duration::from_secs(20));
    drop(guard);
}
