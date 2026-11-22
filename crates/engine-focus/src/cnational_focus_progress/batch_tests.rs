use super::*;
use engine_governance_tests::{Trace, World, addresses, never_harsher};

fn capture(
    memory: &Trace<'_, World>,
    country: u32,
    serial: bool,
) -> (FocusWalk, Vec<Row>, Vec<CompletedRow>, Vec<u8>) {
    let mut rows = vec![Row::default()];
    let mut completed = vec![CompletedRow::default(); 3];
    let mut tokens = vec![0; 256];
    let result = if serial {
        super::serial::walk_buffer(
            memory,
            0,
            World::country(country),
            country,
            crate::walk::Buffer::Growing(&mut rows),
            crate::walk::Buffer::Growing(&mut completed),
            crate::walk::Buffer::Growing(&mut tokens),
        )
    } else {
        walk(
            memory,
            0,
            World::country(country),
            country,
            &mut rows,
            &mut completed,
            &mut tokens,
        )
    };
    (
        result,
        rows[..result.outcome.len()].to_vec(),
        completed[..result.completed_outcome.len()].to_vec(),
        tokens[..result.bytes_len].to_vec(),
    )
}
#[test]
fn governance_batch_focus_persistent_and_one_shot_every_serial_operand() {
    let world = World::new(1);
    let country = World::country(1);
    for (offset, reader) in [
        (engine_governance_tests::FOCUS + PROGRESS, "focus_progress"),
        (
            0x6100 + crate::cnational_focus_tree::TOKEN,
            "focus_definitions",
        ),
        (0x6400, "focus_pointers"),
        (0x6500 + crate::cnational_focus::TOKEN, "focus_definitions"),
    ] {
        for once in [false, true] {
            let before = Trace::fault(&world, "Focus serial", country + offset, once);
            let after = Trace::fault(&world, "Focus batch", country + offset, once);
            let old = capture(&before, 1, true);
            let new = capture(&after, 1, false);
            if once {
                never_harsher(old.0.outcome, new.0.outcome);
                never_harsher(old.0.completed_outcome, new.0.completed_outcome);
            } else {
                assert_eq!(new, old);
            }
            assert_eq!(
                &*after.fallbacks.witnesses(),
                &[reader],
                "{reader}, once={once}"
            );
        }
    }
    let census = Trace::new(&world, "Focus serial census");
    capture(&census, 1, true);
    for address in addresses(&census) {
        for once in [false, true] {
            let before = Trace::fault(&world, "Focus serial", address, once);
            let after = Trace::fault(&world, "Focus batch", address, once);
            let old = capture(&before, 1, true);
            let new = capture(&after, 1, false);
            if once {
                never_harsher(old.0.outcome, new.0.outcome);
                never_harsher(old.0.completed_outcome, new.0.completed_outcome);
            } else {
                assert_eq!(new, old, "persistent {address:#x}");
            }
            assert!(
                after
                    .fallbacks
                    .witnesses()
                    .iter()
                    .filter(|&&r| r == "focus_progress")
                    .count()
                    <= 1
            );
            assert!(
                after
                    .fallbacks
                    .witnesses()
                    .iter()
                    .filter(|&&r| r == "focus_pointers")
                    .count()
                    <= 1
            );
            assert!(
                after
                    .fallbacks
                    .witnesses()
                    .iter()
                    .filter(|&&r| r == "focus_definitions")
                    .count()
                    <= 2
            );
        }
    }
}
#[test]
fn governance_batch_focus_truncation_keeps_serial_arenas_and_demand() {
    let world = World::new(1);
    for row_capacity in 0..=1 {
        for completed_capacity in 0..=3 {
            for bytes in 0..=100 {
                let mut before_rows = vec![Row::default(); row_capacity];
                let mut after_rows = before_rows.clone();
                let mut before_completed = vec![CompletedRow::default(); completed_capacity];
                let mut after_completed = before_completed.clone();
                let mut before_tokens = vec![0; bytes];
                let mut after_tokens = before_tokens.clone();
                let old = super::serial::walk_buffer(
                    &world,
                    0,
                    World::country(1),
                    1,
                    crate::walk::Buffer::Growing(&mut before_rows),
                    crate::walk::Buffer::Growing(&mut before_completed),
                    crate::walk::Buffer::Growing(&mut before_tokens),
                );
                let new = walk(
                    &world,
                    0,
                    World::country(1),
                    1,
                    &mut after_rows,
                    &mut after_completed,
                    &mut after_tokens,
                );
                assert_eq!(new, old);
                assert_eq!(
                    after_tokens[..new.bytes_len],
                    before_tokens[..old.bytes_len]
                );
                assert_eq!(
                    after_rows[..new.outcome.len()],
                    before_rows[..old.outcome.len()]
                );
                assert_eq!(
                    after_completed[..new.completed_outcome.len()],
                    before_completed[..old.completed_outcome.len()]
                );
            }
        }
    }
}

#[test]
fn focus_root_completed_count_double_fault_refuses_both_summary_and_completed_without_third_read() {
    use std::cell::Cell;

    struct TwoShotFault<'a> {
        world: &'a World,
        fault: usize,
        attempts: Cell<usize>,
    }

    impl Memory for TwoShotFault<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address <= self.fault && self.fault < address.saturating_add(destination.len()) {
                let attempt = self.attempts.get() + 1;
                self.attempts.set(attempt);
                if attempt <= 2 {
                    destination.fill(0xff);
                    return false;
                }
            }
            self.world.copy(address, destination)
        }
    }

    let world = World::new(1);
    let country = World::country(1);
    let memory = TwoShotFault {
        world: &world,
        fault: country + engine_governance_tests::FOCUS + COMPLETED_COUNT,
        attempts: Cell::new(0),
    };
    let mut rows = vec![Row::default()];
    let mut completed = vec![CompletedRow::default(); 3];
    let mut tokens = vec![0; 256];
    let result = walk(&memory, 0, country, 1, &mut rows, &mut completed, &mut tokens);
    assert!(matches!(result.outcome, Outcome::Unavailable { .. }));
    assert!(matches!(
        result.completed_outcome,
        Outcome::Unavailable { .. }
    ));
    assert_eq!(memory.attempts.get(), 2);
}
