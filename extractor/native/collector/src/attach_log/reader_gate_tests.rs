//! `reader_gate_tests`, extracted verbatim from `attach_log.rs`.

use super::*;

#[test]
fn reader_gate_zero_is_unbounded() {
    // 0 permits configured -> no gate installed -> spawn_gated is a plain spawn.
    assert!(resolve_reader_gate(0).is_none());
    assert!(resolve_reader_gate(1).is_some());
    assert!(resolve_reader_gate(8).is_some());
}

#[test]
fn reader_gate_never_exceeds_permit_count() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // With N permits and many contending workers, the number of workers
    // holding a permit simultaneously must never exceed N.
    for permits in [1usize, 2, 3, 6] {
        let gate = ReaderGate::new(permits);
        let live = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        thread::scope(|scope| {
            for _ in 0..48 {
                scope.spawn(|| {
                    let _p = gate.acquire();
                    let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    // brief contended window
                    for _ in 0..2000 {
                        std::hint::spin_loop();
                    }
                    live.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        assert_eq!(live.load(Ordering::SeqCst), 0, "all permits released");
        assert!(
            peak.load(Ordering::SeqCst) <= permits,
            "peak {} exceeded permits {}",
            peak.load(Ordering::SeqCst),
            permits
        );
        assert_eq!(gate.available(), permits, "gate restored to full");
    }
}

#[test]
fn reader_gate_permit_released_on_panic() {
    // A panicking worker must not leak its permit (RAII Drop fires while
    // unwinding), else a later frame would deadlock at reduced capacity.
    let gate = ReaderGate::new(1);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _p = gate.acquire();
        assert_eq!(gate.available(), 0);
        panic!("boom");
    }));
    assert!(panicked.is_err());
    assert_eq!(gate.available(), 1, "permit returned after panic unwind");
    // The gate is still usable.
    {
        let _p = gate.acquire();
        assert_eq!(gate.available(), 0);
    }
    assert_eq!(gate.available(), 1);
}

/// Tags reserved for the cost-table tests. `LANE_COST_US` is process-global
/// and the test binary runs tests in parallel, so a test that asserts on a
/// tag's learned value must own that tag outright.
const COST_TABLE_TEST_TAG: u16 = (LANE_COST_TABLE_LEN - 1) as u16;
const COST_LEARNED_TEST_TAG: u16 = (LANE_COST_TABLE_LEN - 2) as u16;

// THE POINT OF THE RANKED GATE. A freed lane goes to the most expensive
// BLOCKED waiter, because on a 12-lane fan-out carrying one 30 ms job the
// makespan is decided by how late that job is granted.
#[test]
fn reader_gate_grants_a_freed_lane_to_the_costliest_waiter() {
    let gate = ReaderGate::new(1);
    let served: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    // Deliberately neither ascending nor descending in spawn order, so an
    // arrival-ordered gate, a LIFO gate and a cheapest-first gate each produce
    // a sequence different from the expected one.
    const COSTS: [u64; 4] = [1_000, 9_000, 3_000, 7_000];
    thread::scope(|scope| {
        // The one permit is held here until every waiter is provably queued,
        // so the assertion is about the gate's CHOICE and never about which
        // thread the OS happened to start first.
        let held = gate.acquire_ranked(u64::MAX, None);
        for cost in COSTS {
            let gate = &gate;
            let served = &served;
            scope.spawn(move || {
                let _permit = gate.acquire_ranked(cost, None);
                served
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(cost);
            });
        }
        let deadline = Instant::now() + GATE_TEST_DEADLINE;
        while gate.waiting() < COSTS.len() && Instant::now() < deadline {
            std::hint::spin_loop();
        }
        assert_eq!(gate.waiting(), COSTS.len(), "every waiter is queued");
        drop(held);
    });
    assert_eq!(
        *served.lock().unwrap_or_else(|e| e.into_inner()),
        vec![9_000, 7_000, 3_000, 1_000],
        "a freed lane must go to the costliest waiter, not to an arbitrary one"
    );
}

// Ties are the common case once most domains are cheap, and an unstable tie
// break would make grant order depend on heap internals rather than arrival.
#[test]
fn reader_gate_serves_equal_cost_waiters_in_arrival_order() {
    let gate = ReaderGate::new(1);
    let served: Mutex<Vec<u16>> = Mutex::new(Vec::new());
    thread::scope(|scope| {
        let held = gate.acquire_ranked(u64::MAX, None);
        for id in 0u16..4 {
            let queued = gate.waiting();
            let gate = &gate;
            let served = &served;
            scope.spawn(move || {
                let _permit = gate.acquire_ranked(4_000, None);
                served.lock().unwrap_or_else(|e| e.into_inner()).push(id);
            });
            // Enqueue strictly one at a time so "arrival order" is a fact of
            // the test rather than a hope about thread startup.
            let deadline = Instant::now() + GATE_TEST_DEADLINE;
            while gate.waiting() == queued && Instant::now() < deadline {
                std::hint::spin_loop();
            }
            assert_eq!(gate.waiting(), queued + 1, "waiter {id} queued");
        }
        drop(held);
    });
    assert_eq!(
        *served.lock().unwrap_or_else(|e| e.into_inner()),
        vec![0, 1, 2, 3],
        "equal-cost waiters must be served first-come-first-served"
    );
}

// The direct handoff happens in `ReaderPermit::drop`, which for a panicking
// domain runs while unwinding. If the grant were skipped or the unpark lost
// there, a parked waiter would never wake and the whole fan-out would hang --
// a hung sweep is worse than a lost domain.
#[test]
fn reader_gate_hands_its_lane_to_a_parked_waiter_when_the_holder_panics() {
    let gate = ReaderGate::new(1);
    let served = AtomicBool::new(false);
    thread::scope(|scope| {
        // The holder is THIS thread and it takes the only permit before the
        // waiter exists. Racing two spawned threads for the permit was the
        // first shape of this test and it deadlocked: whichever thread lost
        // the race became the holder, and the holder waits for a waiter that
        // had already finished.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _permit = gate.acquire_ranked(0, None);
            scope.spawn(|| {
                let _permit = gate.acquire_ranked(0, None);
                served.store(true, Ordering::SeqCst);
            });
            let deadline = Instant::now() + GATE_TEST_DEADLINE;
            while gate.waiting() == 0 && Instant::now() < deadline {
                std::hint::spin_loop();
            }
            assert_eq!(gate.waiting(), 1, "the waiter is parked behind the permit");
            panic!("reader worker failed under its permit");
        }));
        assert!(outcome.is_err(), "the holder panicked while holding its lane");
    });
    assert!(
        served.load(Ordering::SeqCst),
        "a waiter parked behind a panicking holder must still be granted its lane"
    );
    assert_eq!(gate.available(), 1, "the lane came back after both finished");
}

/// How long a test may spin for a state that should arrive in microseconds.
/// Bounded so a regression FAILS with a readable assertion instead of hanging
/// the suite.
const GATE_TEST_DEADLINE: Duration = Duration::from_secs(5);

// A held gate must grant NOTHING, even with every lane free. Without this the
// hold cannot fill the queue it exists to fill: the first worker to arrive
// would take a lane and start reading before any ranking is possible.
#[test]
fn reader_gate_hold_queues_an_acquirer_even_with_every_lane_free() {
    let gate = ReaderGate::new(4);
    let running = AtomicBool::new(false);
    thread::scope(|scope| {
        let hold = gate.hold_admission();
        scope.spawn(|| {
            let _permit = gate.acquire_ranked(0, None);
            running.store(true, Ordering::SeqCst);
        });
        let deadline = Instant::now() + GATE_TEST_DEADLINE;
        while gate.waiting() == 0 && Instant::now() < deadline {
            std::hint::spin_loop();
        }
        assert_eq!(
            gate.waiting(),
            1,
            "the acquirer must be QUEUED under the hold, not running"
        );
        assert!(
            !running.load(Ordering::SeqCst),
            "a held gate granted a lane it should have withheld"
        );
        assert_eq!(gate.available(), 4, "no lane left the free list under a hold");
        drop(hold);
    });
    assert!(
        running.load(Ordering::SeqCst),
        "opening admission must release the queued acquirer"
    );
}

// THE WHOLE MECHANISM, end to end: hold, let the workers queue, open, and the
// costliest runs first however the OS ordered their arrival.
//
// The hold is released on the QUEUE, not on a clock. An earlier form of this
// test relied on `open_admission`'s own arrival wait to fill the queue, and it
// failed in the full suite -- 1195 other tests running beside it pushed some of
// these eight threads past the 2 ms bound, admission opened on a partial queue
// and the order came out arbitrary. That degradation is the design working as
// intended (a bounded wait must be allowed to expire), but it is not what THIS
// test is about, so the ordering claim is separated from the timing one.
#[test]
fn reader_gate_open_grants_a_held_queue_costliest_first() {
    let gate = ReaderGate::new(1);
    let served: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    // Neither ascending nor descending in spawn order.
    const COSTS: [u64; 8] = [2_000, 8_000, 4_000, 15_000, 6_000, 1_000, 11_000, 3_000];
    thread::scope(|scope| {
        let hold = gate.hold_admission();
        for cost in COSTS {
            // What `spawn_gated` does on the spawning thread.
            let arrival = gate.note_spawn_under_hold();
            let gate = &gate;
            let served = &served;
            scope.spawn(move || {
                let _permit = gate.acquire_ranked(cost, arrival);
                served.lock().unwrap_or_else(|e| e.into_inner()).push(cost);
            });
        }
        let deadline = Instant::now() + GATE_TEST_DEADLINE;
        while gate.waiting() < COSTS.len() && Instant::now() < deadline {
            std::hint::spin_loop();
        }
        assert_eq!(
            gate.waiting(),
            COSTS.len(),
            "every worker is queued before the hold is released"
        );
        drop(hold);
    });
    let mut expected = COSTS.to_vec();
    expected.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(
        *served.lock().unwrap_or_else(|e| e.into_inner()),
        expected,
        "a held queue must run strictly costliest-first once it opens"
    );
}

// The arrival accounting itself, without a clock anywhere in the assertions:
// promises are made on the spawning thread and redeemed by the worker they
// were made for.
#[test]
fn reader_gate_counts_promised_arrivals_and_clears_them_as_workers_queue() {
    let gate = ReaderGate::new(2);
    let hold = gate.hold_admission();
    let tickets: Vec<Option<AdmissionTicket>> =
        (0..3).map(|_| gate.note_spawn_under_hold()).collect();
    assert_eq!(gate.pending_arrivals(), 3, "three workers were promised");
    assert!(
        tickets.iter().all(|ticket| ticket.is_some()),
        "a hold issues a ticket for every promise"
    );
    thread::scope(|scope| {
        for ticket in &tickets {
            let gate = &gate;
            let ticket = *ticket;
            scope.spawn(move || {
                let _permit = gate.acquire_ranked(0, ticket);
            });
        }
        let deadline = Instant::now() + GATE_TEST_DEADLINE;
        while gate.pending_arrivals() > 0 && Instant::now() < deadline {
            std::hint::spin_loop();
        }
        assert_eq!(
            gate.pending_arrivals(),
            0,
            "each worker redeems its own promise on arrival"
        );
        drop(hold);
    });
}

// THE TICKET IS THE POINT. An anonymous count lets any arriving worker redeem
// any promise, so a worker spawned before the hold and slow to start would
// clear a promise made for a different worker and admission would open on a
// queue it was told was complete.
#[test]
fn reader_gate_arrival_without_a_matching_ticket_redeems_nothing() {
    let gate = ReaderGate::new(4);
    // A worker that was spawned while the gate was open gets no ticket at all.
    assert!(
        gate.note_spawn_under_hold().is_none(),
        "an open gate promises nothing"
    );
    // A ticket from a hold that has since been superseded.
    let stale = {
        let hold = gate.hold_admission();
        let ticket = gate.note_spawn_under_hold();
        drop(hold);
        ticket.expect("the hold issued a ticket")
    };
    let hold = gate.hold_admission();
    gate.note_spawn_under_hold();
    assert_eq!(gate.pending_arrivals(), 1, "one worker is promised");
    thread::scope(|scope| {
        // Neither of these is the promised worker.
        scope.spawn(|| drop(gate.acquire_ranked(0, None)));
        scope.spawn(|| drop(gate.acquire_ranked(0, Some(stale))));
        let deadline = Instant::now() + GATE_TEST_DEADLINE;
        while gate.waiting() < 2 && Instant::now() < deadline {
            std::hint::spin_loop();
        }
        assert_eq!(
            gate.pending_arrivals(),
            1,
            "an untitled or stale arrival must not redeem the outstanding promise"
        );
        drop(hold);
    });
}

// The promise is a promise, not a guarantee: a worker whose spawn failed, or
// whose spawner is unwinding, never arrives. Opening must still happen.
#[test]
fn reader_gate_open_is_bounded_when_a_promised_arrival_never_comes() {
    let gate = ReaderGate::new(2);
    let hold = gate.hold_admission();
    gate.note_spawn_under_hold();
    let started = Instant::now();
    drop(hold);
    let waited = started.elapsed();
    assert!(
        waited < Duration::from_millis(500),
        "open_admission must be bounded; it waited {waited:?} for a worker that never came"
    );
    assert!(gate.is_admitting(), "the gate opened anyway");
    // And it is a working gate again.
    let permit = gate.acquire_ranked(0, None);
    assert_eq!(gate.available(), 1);
    drop(permit);
    assert_eq!(gate.available(), 2);
}

// The hold lives inside `thread::scope`, which will not return until every
// worker finishes, and a worker under a held gate never starts. So a spawning
// thread that unwinds part-way through its block MUST still open the gate, or
// the panic becomes a hang.
#[test]
fn reader_gate_admission_opens_when_the_spawning_thread_unwinds() {
    let gate = ReaderGate::new(2);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _hold = gate.hold_admission();
        assert!(!gate.is_admitting(), "the hold is up");
        panic!("the spawn block failed part-way through");
    }));
    assert!(outcome.is_err(), "the spawn block unwound");
    assert!(
        gate.is_admitting(),
        "the hold must open the gate while unwinding, or the scope hangs"
    );
}

#[test]
fn lane_cost_estimate_seeds_on_the_first_observation_then_smooths() {
    let tag = COST_TABLE_TEST_TAG;
    assert_eq!(lane_cost_hint(tag), 0, "an unseen tag is unranked");
    // Seeding directly matters: an EWMA started from zero would rank a heavy
    // domain at a quarter of its true cost for several frames, which is
    // exactly the window the ranking exists to fix.
    lane_cost_observe(tag, 20_000);
    assert_eq!(lane_cost_hint(tag), 20_000, "first sample seeds the estimate");
    lane_cost_observe(tag, 4_000);
    assert_eq!(lane_cost_hint(tag), 16_000, "then 3/4 old + 1/4 new");
    for _ in 0..64 {
        lane_cost_observe(tag, 4_000);
    }
    assert!(
        (4_000..=4_010).contains(&lane_cost_hint(tag)),
        "the estimate converges on a stable cost, got {}",
        lane_cost_hint(tag)
    );
}

#[test]
fn lane_cost_estimate_ignores_a_tag_past_the_table() {
    // `LW_MAIN_LANE`-shaped and other out-of-range tags must fall out of the
    // table silently rather than index out of bounds.
    let tag = u16::MAX;
    lane_cost_observe(tag, 50_000);
    assert_eq!(lane_cost_hint(tag), 0);
}

// Without this the ranking would never learn anything: the table would stay
// all-zero and every frame would grant lanes in arrival order forever.
#[test]
fn spawn_gated_learns_the_hold_time_of_the_tag_it_ran() {
    let tag = COST_LEARNED_TEST_TAG;
    assert_eq!(lane_cost_hint(tag), 0, "the reserved tag starts unranked");
    let gate = ReaderGate::new(2);
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    thread::scope(|scope| {
        let handle = spawn_gated(scope, Some(&gate), &lanes, tag, || {
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(8) {
                std::hint::spin_loop();
            }
        });
        handle.join().unwrap();
    });
    let learned = lane_cost_hint(tag);
    assert!(
        learned >= 6_000,
        "the tag's ~8 ms hold must reach the cost table, got {learned} us"
    );
}

#[test]
fn spawn_gated_without_gate_runs_all_workers() {
    // None gate: every worker runs and returns its value; each ungated
    // worker gets its own lane in the span report.
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let out: Vec<usize> = thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|i| spawn_gated(scope, None, &lanes, 300 + i as u16, move || i * 2))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(out, vec![0, 2, 4, 6, 8, 10, 12, 14]);
    let spans = lanes.take_spans();
    assert_eq!(spans.len(), 8);
    let mut seen_lanes: Vec<u16> = spans.iter().map(|s| s.lane).collect();
    seen_lanes.sort_unstable();
    seen_lanes.dedup();
    assert_eq!(seen_lanes.len(), 8, "unbounded workers get distinct lanes");
}

#[test]
fn spawn_gated_with_gate_returns_same_results() {
    // Gated: same results as ungated; the gate only affects scheduling.
    let gate = ReaderGate::new(2);
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let out: Vec<usize> = thread::scope(|scope| {
        let gref = Some(&gate);
        let handles: Vec<_> = (0..8)
            .map(|i| spawn_gated(scope, gref, &lanes, 300 + i as u16, move || i * i))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(out, vec![0, 1, 4, 9, 16, 25, 36, 49]);
    assert_eq!(gate.available(), 2);
    let spans = lanes.take_spans();
    assert_eq!(spans.len(), 8);
    assert!(
        spans.iter().all(|s| s.lane < 2),
        "lane ids are permit slots"
    );
}
