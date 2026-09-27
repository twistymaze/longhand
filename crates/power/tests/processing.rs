use std::marker::PhantomData;
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};

struct Assertion {
    events: Arc<Mutex<Vec<(bool, ThreadId)>>>,
    _not_send: PhantomData<*const ()>,
}

impl Drop for Assertion {
    fn drop(&mut self) {
        self.events
            .lock()
            .unwrap()
            .push((false, thread::current().id()));
    }
}

#[test]
fn processing_leases_share_an_assertion_on_its_owner_thread() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&events);
    let service = power::ProcessingPower::with_factory(move || {
        observed
            .lock()
            .unwrap()
            .push((true, thread::current().id()));
        Some(Assertion {
            events: Arc::clone(&observed),
            _not_send: PhantomData,
        })
    })
    .unwrap();
    let first = service.acquire();
    let second = service.acquire();
    assert_eq!(events.lock().unwrap().len(), 1);
    thread::spawn(move || drop(first)).join().unwrap();
    assert_eq!(events.lock().unwrap().len(), 1);
    drop(second);
    let recorded = events.lock().unwrap().clone();
    assert_eq!(recorded.len(), 2);
    assert!(recorded[0].0);
    assert!(!recorded[1].0);
    assert_eq!(recorded[0].1, recorded[1].1);
    assert_ne!(recorded[0].1, thread::current().id());
    let lease = service.acquire();
    assert_eq!(events.lock().unwrap().len(), 3);
    assert!(
        thread::spawn(move || {
            let _lease = lease;
            panic!("job failed");
        })
        .join()
        .is_err()
    );
    assert_eq!(events.lock().unwrap().len(), 4);
}

#[test]
fn refusal_does_not_discard_work_or_stop_later_leases() {
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = Arc::clone(&attempts);
    let service = power::ProcessingPower::with_factory(move || {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        None::<()>
    })
    .unwrap();
    drop(service.acquire());
    drop(service.acquire());
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
}
