use super::*;
#[cfg(test)]
use crate::map::test_support::TEST_HANG_GUARD;
use std::{
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{Receiver, Sender},
    },
    time::Instant,
};

#[test]
fn retirements_are_bounded_even_when_the_destructor_stalls() {
    /// Counts its drop; the first one stalls the destructor until released.
    struct Counted(
        Arc<std::sync::atomic::AtomicUsize>,
        Option<(Sender<()>, Receiver<()>)>,
    );
    impl Drop for Counted {
        fn drop(&mut self) {
            if let Some((started, release)) = self.1.take() {
                started.send(()).unwrap();
                release.recv().unwrap();
            }
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    for capacity in [1, builder_count()] {
        let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let frame = |pieces| {
            let mut retired = Retired::default();
            for _ in 0..pieces {
                retired.push(Counted(Arc::clone(&dropped), None));
            }
            retired
        };
        let (started, stalled) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let dropper = Dropper::with_capacity(capacity).unwrap();
        dropper.drop_later(Counted(Arc::clone(&dropped), Some((started, wait))));
        stalled.recv().unwrap();
        // A frame's pieces take one place in the queue however many.
        for _ in 0..capacity {
            dropper.drop_later(frame(3));
        }
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        // Full: this frame's retirement is freed here, without waiting on
        // the stalled destructor; every queued one is still held.
        dropper.drop_later(frame(2));
        assert_eq!(dropped.load(Ordering::SeqCst), 2);
        release.send(()).unwrap();
        // The destructor frees the queue in order, then the thread ends.
        drop(dropper);
        let deadline = Instant::now() + TEST_HANG_GUARD;
        while dropped.load(Ordering::SeqCst) < 3 + 3 * capacity {
            assert!(
                Instant::now() < deadline,
                "the queued retirements were not freed"
            );
            thread::yield_now();
        }
    }
}
