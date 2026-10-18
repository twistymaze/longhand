//! The single-instance lock (U19).

use session_core::lock::{Acquired, Holder, LOCK_FILE, acquire};

#[test]
fn a_second_instance_is_refused_while_the_first_lives() {
    let dir = tempfile::tempdir().unwrap();
    let Acquired::Exclusive(first) = acquire(dir.path(), "run-a") else {
        panic!("the first instance should take the lock");
    };
    assert_eq!(first.displaced, None);

    match acquire(dir.path(), "run-b") {
        // Outside Windows, the refusal names the holder.
        #[cfg(not(windows))]
        Acquired::HeldBy(holder) => {
            assert_eq!(
                holder,
                Some(Holder {
                    pid: std::process::id(),
                    instance: "run-a".into()
                })
            );
        }
        // Windows bars reading a locked file, so the holder can be unknown.
        #[cfg(windows)]
        Acquired::HeldBy(holder) => {
            if let Some(holder) = holder {
                assert_eq!(
                    holder,
                    Holder {
                        pid: std::process::id(),
                        instance: "run-a".into()
                    }
                );
            }
        }
        other => panic!("expected the lock to be held, got {other:?}"),
    }
}

#[test]
fn a_dead_instances_lock_is_taken_over_and_names_it() {
    let dir = tempfile::tempdir().unwrap();
    // What a killed process leaves: the file, with no lock on it.
    std::fs::write(dir.path().join(LOCK_FILE), "999999 dead-run\n").unwrap();

    let Acquired::Exclusive(lock) = acquire(dir.path(), "new-run") else {
        panic!("a lock nobody holds should be taken over");
    };
    assert_eq!(
        lock.displaced,
        Some(Holder {
            pid: 999_999,
            instance: "dead-run".into()
        })
    );
    drop(lock);
    let text = std::fs::read_to_string(dir.path().join(LOCK_FILE)).unwrap();
    assert_eq!(text, format!("{} new-run\n", std::process::id()));
}

#[test]
fn a_lock_released_by_its_holder_is_free_again() {
    let dir = tempfile::tempdir().unwrap();
    let Acquired::Exclusive(first) = acquire(dir.path(), "run-a") else {
        panic!();
    };
    drop(first);
    assert!(matches!(
        acquire(dir.path(), "run-b"),
        Acquired::Exclusive(_)
    ));
}
