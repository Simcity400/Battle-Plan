use super::*;

const LONG: Duration = Duration::from_secs(600);

struct Leave<'a>(&'a Gate);

impl Drop for Leave<'_> {
    fn drop(&mut self) {
        self.0.leave();
    }
}

#[test]
fn pipeline_threads_wait_out_the_tick_and_the_ticking_thread_never_waits() {
    let gate = Gate::new();
    gate.wait(LONG);
    assert_eq!(gate.entered.load(Ordering::SeqCst), 0, "no tick: no wait");

    gate.enter();
    let leave = Leave(&gate);
    collector_extended_support::hourly::in_tick(|| gate.wait(LONG));
    assert_eq!(
        gate.entered.load(Ordering::SeqCst),
        0,
        "the ticking thread never waits"
    );
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| gate.wait(LONG));
        while gate.entered.load(Ordering::SeqCst) == 0 {
            std::thread::yield_now();
        }
        assert!(
            !waiter.is_finished(),
            "a pipeline thread waits while the game ticks"
        );
        drop(leave);
        waiter.join().unwrap();
    });
    assert_eq!(gate.waits.load(Ordering::SeqCst), 1);
    assert_eq!(
        gate.capped.load(Ordering::SeqCst),
        0,
        "the tick's end released it"
    );
}
