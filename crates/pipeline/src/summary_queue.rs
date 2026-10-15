//! The summary queue (KTD1, KTD2): one job at a time does the work that
//! sends requests to the summary endpoint and writes a note folder. A job
//! is the Summarize and Save stages of a new note, one Regenerate Summary,
//! or one title job. The audio stages use the local permit, not this queue.
//!
//! The queue has three classes. A job of a new note goes before a
//! regeneration, and a regeneration goes before a title job (R3). In each
//! class, jobs get their turn in the order that they arrived. A tokio
//! `Semaphore` is first come, first served, so it cannot give this order.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::Notify;

/// The class of a job in the summary queue, in the order of priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// The Summarize and Save stages of a new note, also of a Transcribe
    /// Again run.
    NewNote,
    /// A Regenerate Summary of a saved note.
    Regeneration,
    /// The title job of a saved note.
    Title,
}

impl Class {
    const ALL: [Class; 3] = [Class::NewNote, Class::Regeneration, Class::Title];

    fn index(self) -> usize {
        match self {
            Class::NewNote => 0,
            Class::Regeneration => 1,
            Class::Title => 2,
        }
    }
}

#[derive(Default)]
struct State {
    /// True while a job has its turn.
    busy: bool,
    /// The tickets of the jobs that wait, one list for each class.
    waiting: [VecDeque<u64>; 3],
    next_ticket: u64,
}

impl State {
    /// The ticket that gets the next turn: the first job of the first
    /// class that has a job.
    fn head(&self) -> Option<u64> {
        Class::ALL
            .iter()
            .find_map(|class| self.waiting[class.index()].front().copied())
    }

    fn remove(&mut self, class: Class, ticket: u64) {
        self.waiting[class.index()].retain(|t| *t != ticket);
    }
}

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    changed: Notify,
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The summary queue. A clone uses the same queue.
#[derive(Clone, Default)]
pub struct SummaryQueue {
    inner: Arc<Inner>,
}

/// The turn of one job. Dropping it gives the turn to the next job.
pub struct Turn {
    inner: Arc<Inner>,
}

impl Drop for Turn {
    fn drop(&mut self) {
        self.inner.state().busy = false;
        self.inner.changed.notify_waiters();
    }
}

/// A job that waits for its turn. Dropping it before the turn removes the
/// job from the queue, so a cancelled wait does not keep a place.
struct Waiter<'a> {
    inner: &'a Arc<Inner>,
    class: Class,
    ticket: u64,
    done: bool,
}

impl Drop for Waiter<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        self.inner.state().remove(self.class, self.ticket);
        // The job after this one can now be the first.
        self.inner.changed.notify_waiters();
    }
}

impl SummaryQueue {
    /// Wait for a turn of `class`. The future is cancel-safe: when it is
    /// dropped before the turn, the job leaves the queue.
    pub async fn acquire(&self, class: Class) -> Turn {
        let ticket = {
            let mut state = self.inner.state();
            let ticket = state.next_ticket;
            state.next_ticket += 1;
            state.waiting[class.index()].push_back(ticket);
            ticket
        };
        let mut waiter = Waiter {
            inner: &self.inner,
            class,
            ticket,
            done: false,
        };
        loop {
            // Register for the next change before the check, so that a
            // change between the check and the wait is not lost.
            let changed = self.inner.changed.notified();
            let mut changed = std::pin::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = self.inner.state();
                if !state.busy && state.head() == Some(ticket) {
                    state.remove(class, ticket);
                    state.busy = true;
                    waiter.done = true;
                    return Turn {
                        inner: Arc::clone(&self.inner),
                    };
                }
            }
            changed.await;
        }
    }

    /// True while a job has its turn or waits for one. This takes only the
    /// lock of the queue, so a caller can hold any other lock.
    pub fn is_active(&self) -> bool {
        let state = self.inner.state();
        state.busy || state.waiting.iter().any(|w| !w.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    fn poll<F: Future>(future: std::pin::Pin<&mut F>) -> Poll<F::Output> {
        future.poll(&mut Context::from_waker(Waker::noop()))
    }

    #[tokio::test]
    async fn a_new_note_goes_before_a_regeneration_and_a_regeneration_before_a_title() {
        let queue = SummaryQueue::default();
        let order = Arc::new(Mutex::new(Vec::new()));
        let running = queue.acquire(Class::Title).await;
        let mut jobs = Vec::new();
        for (name, class) in [
            ("title 1", Class::Title),
            ("regeneration 1", Class::Regeneration),
            ("new note 1", Class::NewNote),
            ("regeneration 2", Class::Regeneration),
            ("title 2", Class::Title),
            ("new note 2", Class::NewNote),
        ] {
            let job_queue = queue.clone();
            let order = Arc::clone(&order);
            jobs.push(tokio::spawn(async move {
                let _turn = job_queue.acquire(class).await;
                order.lock().unwrap().push(name);
                tokio::task::yield_now().await;
            }));
            // Each job is in the queue before the next one arrives.
            while queue
                .inner
                .state()
                .waiting
                .iter()
                .map(VecDeque::len)
                .sum::<usize>()
                < jobs.len()
            {
                tokio::task::yield_now().await;
            }
        }
        drop(running);
        for job in jobs {
            job.await.unwrap();
        }
        assert_eq!(
            *order.lock().unwrap(),
            vec![
                "new note 1",
                "new note 2",
                "regeneration 1",
                "regeneration 2",
                "title 1",
                "title 2",
            ]
        );
        assert!(!queue.is_active());
    }

    #[tokio::test]
    async fn a_job_waits_while_another_job_has_the_turn() {
        let queue = SummaryQueue::default();
        assert!(!queue.is_active());
        let first = queue.acquire(Class::Title).await;
        assert!(queue.is_active());
        let second = queue.acquire(Class::NewNote);
        let mut second = std::pin::pin!(second);
        assert!(poll(second.as_mut()).is_pending(), "one job at a time");
        drop(first);
        let Poll::Ready(turn) = poll(second.as_mut()) else {
            panic!("the next job gets the turn");
        };
        assert!(queue.is_active());
        drop(turn);
        assert!(!queue.is_active());
    }

    #[tokio::test]
    async fn a_cancelled_wait_leaves_the_queue() {
        let queue = SummaryQueue::default();
        let first = queue.acquire(Class::NewNote).await;
        {
            let cancelled = queue.acquire(Class::NewNote);
            let mut cancelled = std::pin::pin!(cancelled);
            assert!(poll(cancelled.as_mut()).is_pending());
        }
        let later = queue.acquire(Class::Title);
        let mut later = std::pin::pin!(later);
        assert!(poll(later.as_mut()).is_pending());
        drop(first);
        // The cancelled job does not keep the turn from the job after it.
        assert!(poll(later.as_mut()).is_ready());
    }
}
