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
    assemble(seal, &mut assembled).unwrap();
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
    assembled.commit();
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
    // Another thread takes the second lane; the order is the sequence's.
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
    // Published after the cut: the next interval's.
    assert_eq!(event(4, 6, b"later"), Ok(4));
    assert_eq!(
        drained(seal),
        vec![
            (1, 5, 1, b"first".to_vec()),
            (2, 5, 2, b"second".to_vec()),
            (3, 6, 3, Vec::new()),
        ]
    );
    // A quiet interval after a commit copies nothing twice.
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
    assemble(seal, &mut assembled).unwrap();
    assert_eq!(assembled.frames.len(), 1);
    // The frame was never written: nothing is freed, and the next interval
    // copies the event again.
    assert_eq!(event(0, 2, b"next"), Ok(2));
    let rows = drained(super::seal());
    assert_eq!(rows.iter().map(|row| row.0).collect::<Vec<_>>(), vec![1, 2]);
}

#[test]
fn a_full_lane_refuses_without_overwriting_and_its_demand_grows_the_next_generation() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset_for_tests();
    grow(1, MINIMUM_LANE_BYTES).unwrap();
    let payload = vec![7_u8; MINIMUM_LANE_BYTES / 4];
    let mut published = 0;
    while event(6, 9, &payload).is_ok() {
        published += 1;
    }
    assert_eq!(published, 3);
    assert_eq!(event(6, 9, &payload), Err(Refusal::Capacity));
    // Nothing the full lane refused was taken or overwritten.
    let before = capacity();
    assert_eq!(before.sequence, 3);
    assert!(grow(1, MINIMUM_LANE_BYTES).unwrap());
    let after = capacity();
    assert_eq!(after.generations, 2);
    assert!(after.lane_bytes > before.lane_bytes);
    // Callbacks move to the new generation; the old one drains, then unlinks.
    assert_eq!(event(6, 10, &payload), Ok(4));
    let rows = drained(seal());
    assert_eq!(
        rows.iter()
            .map(|row| (row.0, row.3.len()))
            .collect::<Vec<_>>(),
        (1..=4)
            .map(|sequence| (sequence, payload.len()))
            .collect::<Vec<_>>()
    );
    assert_eq!(capacity().generations, 1);
    // An unchanged demand installs nothing.
    assert!(!grow(1, MINIMUM_LANE_BYTES).unwrap());
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
    // The registered thread count sizes the next generation's rings.
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
    // A producer between taking its number and publishing.
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
