use super::*;

/// Long enough that only the tick's end releases a waiter.
const LONG: Duration = Duration::from_secs(600);

/// Ends the gate's tick however the test leaves.
struct Leave<'a>(&'a Gate);

impl Drop for Leave<'_> {
    fn drop(&mut self) {
        self.0.leave();
    }
}

/// A pipeline thread waits until the tick returns; the ticking thread and a quiet gate never wait.
#[test]
fn pipeline_threads_wait_out_the_tick_and_the_ticking_thread_never_waits() {
    let gate = Gate::new();
    gate.wait(LONG, false);
    assert_eq!(gate.entered.load(Ordering::SeqCst), 0, "no tick: no wait");

    gate.enter(false);
    let leave = Leave(&gate);
    collector_extended_support::hourly::in_tick(|| gate.wait(LONG, false));
    assert_eq!(gate.entered.load(Ordering::SeqCst), 0, "the ticking thread never waits");
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| gate.wait(LONG, false));
        while gate.entered.load(Ordering::SeqCst) == 0 {
            std::thread::yield_now();
        }
        assert!(!waiter.is_finished(), "a pipeline thread waits while the game ticks");
        drop(leave);
        waiter.join().unwrap();
    });
    assert_eq!(gate.waits.load(Ordering::SeqCst), 1);
    assert_eq!(gate.capped.load(Ordering::SeqCst), 0, "the tick's end released it");
}

/// A tick that began with no free frame is worked through, except by a live game-memory read.
#[test]
fn a_tick_without_a_free_frame_is_worked_through_but_not_by_live_reads() {
    let gate = Gate::new();
    gate.enter(true);
    let leave = Leave(&gate);
    std::thread::scope(|scope| {
        scope.spawn(|| gate.wait(LONG, false)).join().unwrap();
        assert_eq!(gate.entered.load(Ordering::SeqCst), 0, "pipeline work runs on");
        let reader = scope.spawn(|| gate.wait(LONG, true));
        while gate.entered.load(Ordering::SeqCst) == 0 {
            std::thread::yield_now();
        }
        assert!(!reader.is_finished(), "a live read waits out the open tick");
        drop(leave);
        reader.join().unwrap();
    });
    assert_eq!(gate.open_ticks.load(Ordering::SeqCst), 1);
    assert_eq!(gate.capped.load(Ordering::SeqCst), 0);
}
