use super::*;
use std::sync::{
    Arc,
    atomic::Ordering,
    mpsc::{Receiver, Sender},
};

#[test]
fn a_stalled_destructor_never_makes_the_caller_free_a_retirement() {
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
    let dropper = Dropper::new().unwrap();
    dropper.drop_later(Counted(Arc::clone(&dropped), Some((started, wait))));
    stalled.recv().unwrap();
    let frames = 4 * builder_count();
    for _ in 0..frames {
        dropper.drop_later(frame(3));
    }
    assert_eq!(dropped.load(Ordering::SeqCst), 0, "the caller freed nothing");
    release.send(()).unwrap();
    struct Last(Sender<()>);
    impl Drop for Last {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let (freed, reached) = mpsc::channel();
    dropper.drop_later(Last(freed));
    drop(dropper);
    reached.recv().expect("the queue frees in order, so its last retirement is freed last");
    assert_eq!(dropped.load(Ordering::SeqCst), 1 + 3 * frames, "every queued retirement was freed");
}
