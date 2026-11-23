use super::*;
use crate::tests::LOCK;

fn event(family: u8, hour: i32, payload: &[u8]) -> Result<u64, Refusal> {
    publish(family, hour, 1, payload.len(), |writer| {
        writer.put(payload);
        Ok(())
    })
}

fn drained(seal: Seal) -> Vec<(u64, i32, u8, Vec<u8>)> {
    let mut assembled = Assembled::default();
    // SAFETY: tests serialize lane access with `LOCK` and are the sole assembler.
    unsafe { assemble(seal, &mut assembled) }.unwrap();
    let rows = assembled
        .frames
        .iter()
        .map(|frame| {
            (
                frame.sequence,
                frame.hour,
                frame.family,
                assembled.bytes[frame.start..frame.start + frame.len].to_vec(),
            )
        })
        .collect();
    // SAFETY: `assembled` was produced by `assemble` under `LOCK` in this test.
    unsafe { assembled.commit() };
    rows
}

#[test]
fn an_interval_is_every_event_numbered_up_to_the_seal_in_sequence_order() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset_for_tests();
    assert_eq!(event(1, 5, b"x"), Err(Refusal::Staging));
    assert!(grow(2, MINIMUM_LANE_BYTES).unwrap());
    assert_eq!(event(1, 5, b"first"), Ok(1));
    std::thread::scope(|scope| {
        scope.spawn(|| assert_eq!(event(2, 5, b"second"), Ok(2)));
    });
    assert_eq!(event(3, 6, b""), Ok(3));
    let seal = seal();
    assert_eq!(
        seal,
        Seal {
            last: 3,
            complete: true
        }
    );
    assert_eq!(event(4, 6, b"later"), Ok(4));
    assert_eq!(
        drained(seal),
        vec![
            (1, 5, 1, b"first".to_vec()),
            (2, 5, 2, b"second".to_vec()),
            (3, 6, 3, Vec::new()),
        ]
    );
    let second = super::seal();
    assert_eq!(drained(second), vec![(4, 6, 4, b"later".to_vec())]);
    assert!(drained(super::seal()).is_empty());
}

#[test]
fn an_uncommitted_interval_is_copied_again_and_a_failed_writer_takes_no_number() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset_for_tests();
    grow(1, MINIMUM_LANE_BYTES).unwrap();
    assert_eq!(
        publish(0, 1, 1, 4, |writer| {
            writer.put(b"ab");
            Err("operand unreadable")
        }),
        Err(Refusal::Read("operand unreadable"))
    );
    assert_eq!(event(0, 1, b"kept"), Ok(1));
    let seal = seal();
    let mut assembled = Assembled::default();
    // SAFETY: test holds `LOCK` as the sole assembler.
    unsafe { assemble(seal, &mut assembled) }.unwrap();
    assert_eq!(assembled.frames.len(), 1);
    assert_eq!(event(0, 2, b"next"), Ok(2));
    let rows = drained(super::seal());
    assert_eq!(rows.iter().map(|row| row.0).collect::<Vec<_>>(), vec![1, 2]);
}

#[test]
fn a_full_lane_spills_without_overwriting_and_its_demand_grows_the_next_generation() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset_for_tests();
    grow(1, MINIMUM_LANE_BYTES).unwrap();
    let payload = vec![7_u8; MINIMUM_LANE_BYTES / 4];
    for sequence in 1..=3 {
        assert_eq!(event(6, 9, &payload), Ok(sequence));
    }
    assert_eq!(event(6, 9, &payload), Ok(4));
    let before = capacity();
    assert_eq!(
        (before.sequence, before.spilled, before.spill_pending),
        (4, 1, 1)
    );
    assert!(grow(1, MINIMUM_LANE_BYTES).unwrap());
    let after = capacity();
    assert_eq!(after.generations, 2);
    assert!(after.lane_bytes > before.lane_bytes);
    assert_eq!(event(6, 10, &payload), Ok(5));
    let rows = drained(seal());
    assert_eq!(
        rows.iter()
            .map(|row| (row.0, row.3.clone()))
            .collect::<Vec<_>>(),
        (1..=5)
            .map(|sequence| (sequence, payload.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(capacity().generations, 1);
    assert_eq!(capacity().spill_pending, 0);
    assert!(!grow(1, MINIMUM_LANE_BYTES).unwrap());
}

#[test]
fn a_spill_never_waits_for_the_spool_and_a_burst_keeps_every_event_in_order() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset_for_tests();
    grow(1, MINIMUM_LANE_BYTES).unwrap();
    let log = unsafe { &*CURRENT.load(Ordering::SeqCst) };
    let payload = |n: u64| vec![n as u8; MINIMUM_LANE_BYTES / 4];
    let consumer = log.spill.lock().unwrap();
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            for n in 1..=20 {
                assert_eq!(event(6, 9, &payload(n)), Ok(n));
            }
            done.send(()).unwrap();
        });
        finished
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("a map event spill waited for the spool's lock");
        drop(consumer);
    });
    let rows = drained(seal());
    assert_eq!(
        rows.iter()
            .map(|row| (row.0, row.3.clone()))
            .collect::<Vec<_>>(),
        (1..=20).map(|n| (n, payload(n))).collect::<Vec<_>>()
    );
    assert_eq!(capacity().spill_pending, 0);
}

#[test]
fn threads_beyond_the_rings_publish_through_the_shared_lane_in_sequence() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset_for_tests();
    grow(1, MINIMUM_LANE_BYTES).unwrap();
    assert_eq!(event(0, 1, b"ring"), Ok(1));
    std::thread::scope(|scope| {
        for thread in 0..4_u8 {
            scope.spawn(move || event(1, 1, &[thread; 3]).unwrap());
        }
    });
    assert_eq!(event(0, 1, b"ring"), Ok(6));
    let rows = drained(seal());
    assert_eq!(
        rows.iter().map(|row| row.0).collect::<Vec<_>>(),
        (1..=6).collect::<Vec<_>>()
    );
    assert_eq!(capacity().required_lanes, 5);
    assert!(grow(1, MINIMUM_LANE_BYTES).unwrap());
    assert_eq!(capacity().lanes, 5);
}

#[test]
fn a_number_taken_but_not_published_at_the_cut_marks_the_seal_incomplete() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset_for_tests();
    grow(1, MINIMUM_LANE_BYTES).unwrap();
    event(0, 1, b"a").unwrap();
    SEQUENCE.fetch_add(1, Ordering::SeqCst);
    let seal = seal();
    assert_eq!(
        seal,
        Seal {
            last: 2,
            complete: false
        }
    );
    PUBLISHED.fetch_add(1, Ordering::SeqCst);
    assert!(super::seal().complete);
}

fn current() -> &'static Log {
    unsafe { &*CURRENT.load(Ordering::SeqCst) }
}

fn sequences(assembled: &Assembled) -> Vec<u64> {
    assembled
        .frames
        .iter()
        .map(|frame| frame.sequence)
        .collect()
}

fn shared_event(family: u8, hour: i32, payload: &[u8]) -> u64 {
    std::thread::scope(|scope| {
        scope
            .spawn(|| event(family, hour, payload).unwrap())
            .join()
            .unwrap()
    })
}

#[test]
fn queued_intervals_copy_disjointly_and_hold_their_space_until_written() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset_for_tests();
    grow(1, MINIMUM_LANE_BYTES).unwrap();
    let quarter = vec![7_u8; MINIMUM_LANE_BYTES / 4];
    assert_eq!(event(0, 1, &quarter), Ok(1));
    assert_eq!(shared_event(1, 1, b"shared"), 2);
    let mut first = Assembled::default();
    // SAFETY: test holds `LOCK` as the sole assembler.
    unsafe { assemble(seal(), &mut first) }.unwrap();
    assert_eq!(sequences(&first), [1, 2]);
    // SAFETY: test holds `LOCK` and admits intervals in assembly order.
    unsafe { first.admitted() };
    assert_eq!(event(0, 2, &quarter), Ok(3));
    assert_eq!(shared_event(1, 2, b"later"), 4);
    let mut second = Assembled::default();
    // SAFETY: test holds `LOCK` as the sole assembler.
    unsafe { assemble(seal(), &mut second) }.unwrap();
    assert_eq!(sequences(&second), [3, 4]);
    assert_eq!(&second.bytes[second.frames[1].start..], b"later");
    // SAFETY: test holds `LOCK` and admits intervals in assembly order.
    unsafe { second.admitted() };
    let mut quiet = Assembled::default();
    // SAFETY: test holds `LOCK` as the sole assembler.
    unsafe { assemble(seal(), &mut quiet) }.unwrap();
    assert!(quiet.frames.is_empty());
    // SAFETY: test holds `LOCK` and admits intervals in assembly order.
    unsafe { quiet.admitted() };
    let log = current();
    assert_eq!(log.lanes[0].consumed.load(Ordering::Acquire), 0);
    assert_eq!(
        log.shared
            .published(|record| record.sequence)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(event(0, 3, &quarter), Ok(5));
    assert_eq!(event(0, 3, &quarter), Ok(6));
    // SAFETY: test holds `LOCK` and commits intervals in assembly order.
    unsafe { first.commit() };
    assert_eq!(
        log.lanes[0].consumed.load(Ordering::Acquire),
        (HEADER + quarter.len()) as u64
    );
    assert_eq!(
        log.shared
            .published(|record| record.sequence)
            .unwrap()
            .iter()
            .map(|entry| entry.0)
            .collect::<Vec<_>>(),
        [4]
    );
    assert_eq!(event(0, 3, &quarter), Ok(7));
    assert_eq!(event(0, 3, &quarter), Ok(8));
    assert_eq!(capacity().spill_pending, 2);
    // SAFETY: test holds `LOCK` and commits intervals in assembly order.
    unsafe {
        second.commit();
        quiet.commit();
    }
    assert_eq!(ADMITTED.load(Ordering::Acquire), 0);
    let rows = drained(seal());
    assert_eq!(
        rows.iter().map(|row| row.0).collect::<Vec<_>>(),
        [5, 6, 7, 8]
    );
    assert!(log.spill().is_empty());
    assert!(
        log.shared
            .published(|record| record.sequence)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        log.lanes[0].consumed.load(Ordering::Acquire),
        log.lanes[0].published.load(Ordering::Acquire)
    );
}

#[test]
fn a_lost_queue_copies_its_intervals_again_and_reports_the_lost_output() {
    use replay_data::rows::MapEventFault as Fault;
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset_for_tests();
    crate::interval::reset_for_tests();
    grow(1, MINIMUM_LANE_BYTES).unwrap();
    assert_eq!(event(0, 1, b"ring"), Ok(1));
    assert_eq!(shared_event(1, 1, b"shared"), 2);
    let mut first = Assembled::default();
    // SAFETY: test holds `LOCK` as the sole assembler.
    unsafe {
        assemble(seal(), &mut first).unwrap();
        first.admitted();
        first.admitted();
    }
    assert_eq!(event(0, 2, b"next"), Ok(3));
    let mut second = Assembled::default();
    // SAFETY: test holds `LOCK` as the sole assembler.
    unsafe { assemble(seal(), &mut second) }.unwrap();
    assert_eq!(sequences(&second), [3]);
    // SAFETY: test holds `LOCK` and admits intervals in assembly order.
    unsafe { second.admitted() };
    assert_eq!(ADMITTED.load(Ordering::Acquire), 2);
    drop((first, second));
    // SAFETY: test holds `LOCK` and dropped all uncommitted intervals above.
    unsafe { reset_admitted() };
    assert_eq!(ADMITTED.load(Ordering::Acquire), 0);
    // SAFETY: Unit test exclusively owns map-event observation and sealing.
    let lost = unsafe { crate::interval::seal(3, Some(1)) };
    assert!(
        lost.health
            .iter()
            .all(|health| health.faults.contains(Fault::OutputLost))
    );
    assert_eq!(
        drained(seal()),
        vec![
            (1, 1, 0, b"ring".to_vec()),
            (2, 1, 1, b"shared".to_vec()),
            (3, 2, 0, b"next".to_vec()),
        ]
    );
    // SAFETY: test holds `LOCK` with no in-flight intervals remaining.
    unsafe { reset_admitted() };
    assert!(
        // SAFETY: Unit test exclusively owns map-event observation and sealing.
        unsafe { crate::interval::seal(4, Some(1)) }
            .health
            .iter()
            .all(|health| !health.faults.contains(Fault::OutputLost))
    );
    assert!(drained(seal()).is_empty());
}

#[test]
fn capacity_and_allocated_bytes_observe_spilled_records_without_popping_or_losing_them() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset_for_tests();
    grow(1, MINIMUM_LANE_BYTES).unwrap();
    let bytes_before = allocated_bytes();
    let seq = spill(current(), 1, 1, 1, b"spilled".len(), |writer| {
        writer.put(b"spilled");
        Ok(())
    })
    .unwrap();
    assert_eq!(seq, 1);
    let (cap, bytes_after) =
        collector_test_support::with_allocations_denied(|| (capacity(), allocated_bytes()));
    assert_eq!(cap.spill_pending, 1);
    let payload_capacity = current().pushed_bytes.load(Ordering::Acquire);
    assert!(payload_capacity >= b"spilled".len());
    assert_eq!(
        bytes_after - bytes_before,
        SpillStack::<Spilled>::node_size() + payload_capacity
    );
    assert_eq!(current().pushed.len(), 1);
    assert_eq!(drained(seal()), vec![(1, 1, 1, b"spilled".to_vec())]);
    assert_eq!(capacity().spill_pending, 0);
}
