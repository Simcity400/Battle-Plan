use super::*;
use crate::sampling::Shared;

#[test]
fn free_frames_hand_out_the_newest_and_idle_leases_keep_their_age() {
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let free = &shared.frames;
    let all: Vec<_> = std::iter::from_fn(||
    // SAFETY: This consumer owns the queue removal for the named pool.
    unsafe { free.pop() })
    .collect();
    assert_eq!(all.len(), STEADY);
    for index in 0..STEADY {
        free.push(index).unwrap();
    }
    assert_eq!(free.restore(0), Err(0));
    assert_eq!(free.len(), STEADY);
    assert!(free.take(1, 1));
    assert!(!free.take(1, 0));
    assert!(!free.take(0, STEADY - 1));
    assert_eq!(free.len(), STEADY - 1);
    let newest =
    // SAFETY: This consumer owns the queue removal for the named pool.
    unsafe { free.pop() }.unwrap();
    assert_eq!(newest, 0);
    free.push(newest).unwrap();
    free.restore(1).unwrap();
    assert_eq!(free.restore(1), Err(1));
    assert!(free.take(STEADY - 1, 0));
    free.restore(STEADY - 1).unwrap();
    let newest_first: Vec<_> = std::iter::from_fn(||
    // SAFETY: This consumer owns the queue removal for the named pool.
    unsafe { free.pop() })
    .collect();
    let expected: Vec<_> = std::iter::once(newest)
        .chain((0..STEADY).rev().filter(|&index| index != newest))
        .collect();
    assert_eq!(newest_first, expected);
    assert_eq!(free.len(), 0);
    assert!(!free.take(0, 0));
    free.push(1).unwrap();
    free.push(0).unwrap();
    assert_eq!(
        // SAFETY: This consumer owns the queue removal for the named pool.
        unsafe { free.pop() },
        Some(0)
    );
    free.push(0).unwrap();
    assert_eq!(
        // SAFETY: This consumer owns the queue removal for the named pool.
        unsafe { free.pop() },
        Some(0)
    );
    assert!(!free.take(1, 1));
    assert!(free.take(1, 0));
    assert_eq!(free.len(), 0);
    for index in 0..STEADY {
        free.push(index).unwrap();
    }
}

#[test]
fn a_stalled_spool_adds_no_frame_and_its_free_frames_are_still_taken() {
    assert_eq!(hour_frame(Some(3), false, false), HourFrame::Free(3));
    assert_eq!(hour_frame(Some(3), true, false), HourFrame::Free(3));
    assert_eq!(hour_frame(Some(3), false, true), HourFrame::Free(3));
    assert_eq!(hour_frame(None, false, false), HourFrame::Grow);
    assert_eq!(hour_frame(None, true, false), HourFrame::DropStalled);
    assert_eq!(hour_frame(None, false, true), HourFrame::DropLimit);
    assert_eq!(hour_frame(None, true, true), HourFrame::DropStalled);
    assert!(builds_ahead(AHEAD - 1, false));
    assert!(!builds_ahead(AHEAD, false));
    assert!(!builds_ahead(AHEAD - 1, true));
    assert!(!builds_ahead(0, true));
}

#[test]
fn slots_keep_their_place_as_segments_are_added() {
    for index in 0..10_000 {
        let (segment, offset) = locate(index);
        assert!(offset < STEADY << segment, "slot {index}");
        assert_eq!(STEADY * ((1 << segment) - 1) + offset, index);
    }
    let pool = FramePool::new();
    let first = pool.vacant().unwrap();
    let address = std::ptr::from_ref::<Slot>(pool.slot(first));
    let made: Vec<_> = (0..200).map(|_| pool.vacant().unwrap()).collect();
    assert_eq!(made, (1..=200).collect::<Vec<_>>());
    assert_eq!(std::ptr::from_ref::<Slot>(pool.slot(first)), address);
    assert_eq!(pool.made(), 201);
    assert_eq!(pool.live(), 0);
    pool.abandon(57);
    assert_eq!(pool.vacant().unwrap(), 57);
    assert!(pool.slot_bytes() >= 201 * size_of::<Slot>() as u64);
}

#[test]
fn two_builders_making_slots_at_once_never_share_one() {
    let pool = std::sync::Arc::new(FramePool::new());
    let per_builder = 40 * STEADY;
    let start = std::sync::Arc::new(std::sync::Barrier::new(2));
    let builders: Vec<_> = (0..2)
        .map(|_| {
            let pool = std::sync::Arc::clone(&pool);
            let start = std::sync::Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                (0..per_builder)
                    .map(|_| pool.vacant().unwrap())
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut made: Vec<_> = builders
        .into_iter()
        .flat_map(|builder| builder.join().unwrap())
        .collect();
    made.sort_unstable();
    assert_eq!(made, (0..2 * per_builder).collect::<Vec<_>>());
    assert_eq!(pool.made(), 2 * per_builder);
    let segments = pool
        .segments
        .iter()
        .take_while(|segment| !segment.load(Ordering::Acquire).is_null())
        .count();
    assert_eq!(locate(2 * per_builder - 1).0 + 1, segments);
    pool.abandon(5);
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| pool.vacant().unwrap());
        let second = scope.spawn(|| pool.vacant().unwrap());
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_ne!(first, second);
    assert!(first == 5 || second == 5);
}

#[test]
fn fifos_keep_order_while_a_producer_pushes_and_never_fill() {
    let pool = std::sync::Arc::new(FramePool::new());
    let count = 50 * STEADY;
    for _ in 0..count {
        pool.vacant().unwrap();
    }
    assert_eq!(
        // SAFETY: The sole decode consumer owns this queue and takes responsibility for the slot.
        unsafe { pool.pop_decode() },
        None
    );
    let producer = {
        let pool = std::sync::Arc::clone(&pool);
        std::thread::spawn(move || {
            for index in 0..count {
                // SAFETY: The producer owns this slot and its initialized decode job.
                unsafe { pool.push_decode(index) };
            }
        })
    };
    let deadline = std::time::Instant::now() + collector_test_support::TEST_HANG_GUARD;
    let mut popped = Vec::new();
    while popped.len() < count {
        assert!(
            std::time::Instant::now() < deadline,
            "the FIFO lost an index"
        );
        match
        // SAFETY: The sole decode consumer owns this queue and takes responsibility for the slot.
        unsafe { pool.pop_decode() } {
            Some(index) => {
                popped.push(index);
                pool.push_ready(index);
            }
            None => std::thread::yield_now(),
        }
    }
    producer.join().unwrap();
    assert_eq!(popped, (0..count).collect::<Vec<_>>());
    assert_eq!(pool.decode_len(), 0);
    assert_eq!(pool.ready_len(), count);
    let ready: Vec<_> = std::iter::from_fn(|| {
        // SAFETY: The test is the only consumer of this fixture's ready queue.
        unsafe { pool.pop_ready() }
    })
    .collect();
    assert_eq!(ready, (0..count).collect::<Vec<_>>());
    assert!(pool.ready_is_empty());
    // SAFETY: The producer owns this slot and its initialized decode job.
    unsafe { pool.push_decode(3) };
    // SAFETY: The producer owns this slot and its initialized decode job.
    unsafe { pool.push_decode(1) };
    assert_eq!(
        (
            // SAFETY: The sole decode consumer owns this queue and takes responsibility for the slot.
            unsafe { pool.pop_decode() },
            // SAFETY: The sole decode consumer owns this queue and takes responsibility for the slot.
            unsafe { pool.pop_decode() },
            // SAFETY: The sole decode consumer owns this queue and takes responsibility for the slot.
            unsafe { pool.pop_decode() }
        ),
        (Some(3), Some(1), None)
    );
}

#[test]
fn surplus_waits_for_a_sustained_idle_streak_and_resets_when_popped() {
    // SAFETY: Isolated unit test owns this `Shared` fixture and its frame slots exclusively.
    let shared = unsafe { Shared::new(0, 0, 1) };
    let pool = &shared.frames;
    let mut taken: Vec<_> = (0..=STEADY)
        .map(|_| unsafe { shared.take_frame(&mut 0) }.unwrap())
        .collect();
    for _ in 0..AHEAD {
        pool.push(taken.pop().unwrap()).unwrap();
    }
    let in_flight = taken.pop().unwrap();
    let extra = taken.pop().unwrap();
    assert_eq!(pool.live(), STEADY + 1);
    assert_eq!(pool.len(), AHEAD);
    assert!(!pool.surplus_after(4));
    assert!(!pool.surplus_after(4));
    let popped = unsafe { pool.pop() }.unwrap();
    assert!(!pool.surplus_after(4));
    pool.push(popped).unwrap();
    for _ in 0..3 {
        assert!(!pool.surplus_after(4));
    }
    assert!(pool.surplus_after(4));
    assert!(!pool.surplus_after(4));
    pool.push(in_flight).unwrap();
    assert_eq!(pool.len(), STEADY);
    assert!(pool.surplus_after(4));
    let mut all: Vec<_> = (0..STEADY).map(|_| unsafe { pool.pop() }.unwrap()).collect();
    all.push(extra);
    for _ in 0..(SURPLUS_CAP - STEADY) {
        all.push(unsafe { shared.take_frame(&mut 0) }.unwrap());
    }
    for _ in 0..AHEAD {
        pool.push(all.pop().unwrap()).unwrap();
    }
    assert_eq!(pool.live(), SURPLUS_CAP + 1);
    assert!(pool.surplus_after(4));
    for slot in all {
        drop(unsafe { pool.vacate(slot) });
    }
}

#[test]
fn byte_and_frame_limits_stop_pool_growth_and_resume_when_frames_vacate() {
    // SAFETY: Isolated unit test owns this `Shared` fixture and its frame slots exclusively.
    let shared = unsafe { Shared::new(0, 0, 1) };
    let pool = &shared.frames;
    let initial = pool.live_bytes();
    assert!(initial > 0);
    assert!(!pool.at_limit_with(STEADY + 1, initial + 1));
    assert!(pool.at_limit_with(STEADY, u64::MAX));
    assert!(pool.at_limit_with(usize::MAX, initial));
    assert_eq!(
        hour_frame(None, false, pool.at_limit_with(usize::MAX, initial)),
        HourFrame::DropLimit
    );
    assert!(!builds_ahead(0, pool.at_limit_with(usize::MAX, initial)));

    let mut held: Vec<_> = (0..STEADY)
        .map(|_| unsafe { shared.take_frame(&mut 0) }.unwrap())
        .collect();
    pool.set_slot_bytes(0, MAX_LIVE_BYTES);
    assert!(pool.at_limit());
    assert_eq!(unsafe { shared.take_frame(&mut 0) }, None);
    assert_eq!(pool.limit_drops(), 1);
    assert_eq!(pool.stalled_drops(), 0);
    assert_eq!(pool.live(), STEADY);

    pool.set_slot_bytes(0, 1);
    assert!(!pool.at_limit());
    let claim_a = pool
        .try_claim_growth(STEADY + 1, MAX_LIVE_BYTES)
        .expect("one growth claim up to the cap");
    assert!(
        pool.try_claim_growth(STEADY + 1, MAX_LIVE_BYTES).is_none(),
        "second concurrent claim at the cap is refused"
    );
    drop(claim_a);
    let extra = unsafe { shared.take_frame(&mut 0) }.unwrap();
    let second_extra = unsafe { shared.take_frame(&mut 0) }.unwrap();
    assert_eq!(pool.live(), STEADY + 2);
    held.push(extra);
    held.push(second_extra);
    for slot in held {
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.recycle_frame_prepared(slot) };
    }
    assert_eq!(pool.live(), STEADY);
    assert_eq!(
        pool.live_bytes(),
        pool.gauge().live_bytes + shared.parked_bytes()
    );
    assert!(shared.has_retired_parked());
    let before_drain = pool.live_bytes();
    assert!(shared.drain_retired_parked());
    assert!(pool.live_bytes() < before_drain);
    assert_eq!(
        pool.live_bytes(),
        pool.gauge().live_bytes + shared.parked_bytes()
    );
    assert!(!shared.has_retired_parked());
    assert!(!shared.drain_retired_parked());

    let box_bytes = size_of::<crate::extended::ParkedPacks>() as u64;
    let parked = shared.parked_bytes();
    assert!(parked > box_bytes);
    assert!(shared.drain_surplus_parked());
    assert_eq!(shared.parked_bytes(), 0);
    assert_eq!(pool.live_bytes(), pool.gauge().live_bytes);
    let reused = unsafe { shared.take_frame(&mut 0) }.unwrap();
    let second = unsafe { shared.take_frame(&mut 0) }.unwrap();
    let grown = unsafe { shared.take_frame(&mut 0) }.unwrap();
    assert_eq!(pool.live(), STEADY + 1);
    let transfer_total = pool.live_bytes();
    let owned_total = shared.pool_owned();
    assert!(!pool.at_limit_with(usize::MAX, transfer_total + box_bytes + 1));
    let done = std::sync::atomic::AtomicBool::new(false);
    let spiked = std::sync::atomic::AtomicBool::new(false);
    let dipped = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Acquire) {
                let bytes = pool.live_bytes();
                let owned = shared.pool_owned();
                if bytes > transfer_total + box_bytes || owned > owned_total + box_bytes {
                    spiked.store(true, Ordering::Relaxed);
                }
                if bytes < transfer_total || owned < owned_total {
                    dipped.store(true, Ordering::Relaxed);
                }
            }
        });
        struct Finish<'a>(&'a std::sync::atomic::AtomicBool);
        impl Drop for Finish<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let _finish = Finish(&done);
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.prepare_frame_for_reuse(reused) };
        assert_eq!(pool.live_bytes(), transfer_total + box_bytes);
        assert_eq!(shared.pool_owned(), owned_total + box_bytes);
        assert!(shared.parked_bytes() > box_bytes);
        let restored = shared.frames.push(reused);
        assert!(restored.is_ok());
        let again = unsafe { shared.take_frame(&mut 0) }.unwrap();
        assert_eq!(again, reused);
        assert_eq!(pool.live_bytes(), transfer_total);
        assert_eq!(shared.pool_owned(), owned_total);
    });
    assert!(!spiked.load(Ordering::Relaxed));
    assert!(!dipped.load(Ordering::Relaxed));
    assert_eq!(
        pool.live_bytes(),
        pool.gauge().live_bytes + shared.parked_bytes()
    );
    for slot in [reused, second, grown] {
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.recycle_frame_prepared(slot) };
    }
    let _ = shared.drain_retired_parked();
    assert_eq!(
        pool.live_bytes(),
        pool.gauge().live_bytes + shared.parked_bytes()
    );
}
