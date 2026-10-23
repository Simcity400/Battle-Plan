//! Undecided atlas keys wait for the preceding branch to preserve serial UV assignments.

use std::{
    cell::RefCell,
    sync::{Arc, Condvar, Mutex},
};

/// The preceding branch's completion, which a following branch's undecided requests wait for.
#[derive(Default)]
pub struct Preceding {
    done: Mutex<bool>,
    changed: Condvar,
}

/// Unwinding must also release the following branch.
pub struct Finishing<'a>(&'a Preceding);

impl Drop for Finishing<'_> {
    fn drop(&mut self) {
        *self.0.done.lock().unwrap_or_else(|p| p.into_inner()) = true;
        self.0.changed.notify_all();
    }
}

impl Preceding {
    /// Held by the preceding branch for its whole draw.
    pub fn finishing(&self) -> Finishing<'_> {
        Finishing(self)
    }

    fn wait(&self) {
        let mut done = self.done.lock().unwrap_or_else(|p| p.into_inner());
        while !*done {
            done = self.changed.wait(done).unwrap_or_else(|p| p.into_inner());
        }
    }
}

thread_local! {
    static FOLLOWING: RefCell<Option<Arc<Preceding>>> = const { RefCell::new(None) };
}

/// Restores the thread's previous branch on exit, including on unwind.
struct Restore(Option<Arc<Preceding>>);

impl Drop for Restore {
    fn drop(&mut self) {
        let previous = self.0.take();
        FOLLOWING.with(|following| *following.borrow_mut() = previous);
    }
}

/// The caller must hold no lock a preceding atlas request needs.
pub fn following<R>(preceding: &Arc<Preceding>, work: impl FnOnce() -> R) -> R {
    let previous =
        FOLLOWING.with(|following| following.borrow_mut().replace(Arc::clone(preceding)));
    let _restore = Restore(previous);
    work()
}

/// Whether this thread is drawing a following branch.
pub(crate) fn is_following() -> bool {
    FOLLOWING.with(|following| following.borrow().is_some())
}

/// Wait only before an undecided request, never while holding the atlas lock.
pub(crate) fn before_deciding() {
    let preceding = FOLLOWING.with(|following| following.borrow().clone());
    if let Some(preceding) = preceding {
        preceding.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// An unwinding preceding branch still releases undecided requests.
    #[test]
    fn following_waits_only_before_deciding_and_unwind_releases_it() {
        let preceding = Arc::new(Preceding::default());
        let (reached, reach) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        std::thread::scope(|scope| {
            let first = Arc::clone(&preceding);
            let leader = scope.spawn(move || {
                let _finish = first.finishing();
                reached.send(()).unwrap();
                released.recv().unwrap();
                panic!("preceding branch unwinds");
            });
            reach.recv().unwrap();
            let waited = following(&preceding, || {
                assert!(is_following());
                // A decided read does not wait: the preceding branch is still blocked.
                release.send(()).unwrap();
                before_deciding();
                true
            });
            assert!(waited);
            assert!(!is_following(), "the thread's previous branch is restored");
            assert!(leader.join().is_err());
        });
        // Off a following branch nothing waits.
        before_deciding();
    }
}
