use super::*;
use crate::army_stat_changes::Watches;
use engine_native::cunit_supply_inputs as supply;

struct NoFooter<'a, M> {
    memory: &'a M,
    armies: &'a [usize],
}
impl<M: Memory> Memory for NoFooter<'_, M> {
    fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
        assert!(
            !self
                .armies
                .iter()
                .any(|army| address < army + supply::DISRUPTION + 8
                    && address + bytes.len() > army + supply::ASKED),
            "carried native footer read"
        );
        self.memory.copy(address, bytes)
    }
}

fn acquire(
    memory: &impl Memory,
    capture: &mut Captured,
    store: &mut held_store::Loan,
    context: CaptureContext,
    rows: &[DivisionSeed],
    outcome: Outcome,
    changes: &Watches,
) {
    let sources = || Sources { rows, outcome };
    capture.prepare_status_observed(memory, context, sources(), Some(store), changes);
    for stage in 0..2 {
        for index in 0..capture.status_part_count() {
            let mut part = capture.take_status_part(index);
            part.capture(memory, context, rows, changes);
            capture.restore_status_part(index, part, 0);
        }
        if stage == 0 {
            capture.prepare_status_observed(memory, context, sources(), Some(store), changes);
        }
    }
    capture.capture_status_observed(memory, context, sources(), Some(store), changes);
    capture.capture_attack_observed(memory, context, sources(), Some(store), changes);
}

#[test]
fn daily_status_owned_ranges_carry_footer_and_xp_but_keep_ratio_hourly() {
    let mut populated = Populated::new();
    let original = populated.census.unit_inputs().unwrap();
    let mut rows = original.seeds().to_vec();
    let changes = Watches::enabled(2 * rows.len());
    let slot = held_store::Slot::new(Box::new(
        super::super::super::attack_stats::ArmyStatStore::new(),
    ));
    let mut capture = populated.capture();
    capture.status_packet = carmy::StatusPacket::vacant();
    capture.enable_owned_settlement();
    capture.status_workers = 2;
    for hour in [1, 2, 12, 14] {
        if hour == 14 {
            // A forced-refresh request that never acquires the loan must still
            // revoke carry on the next off-slot request.
            slot.prepare_hour(13, true);
        }
        slot.prepare_hour(hour, false);
        let mut store = slot.take().unwrap();
        crate::sampled_verify::begin_hour(hour);
        for (&army, row) in populated.world.armies.iter().zip(&mut rows) {
            let at = army - populated.world.base + supply::ASKED;
            populated.world.bytes[at..at + 8]
                .copy_from_slice(&(i64::from(hour) * 100_000).to_le_bytes());
            let at = army - populated.world.base + carmy::EXPERIENCE;
            populated.world.bytes[at..at + 8].copy_from_slice(&i64::from(hour).to_le_bytes());
            drop(changes.notify(
                army,
                engine_mutations::army_stat_mutations::EXPERIENCE_FIRST,
            ));
            row.counter_supply_status = Some(i64::from(hour) * 10_000);
        }
        let context = CaptureContext {
            hour,
            ..populated.context()
        };
        // The last division first appears off-slot, while packet growth must
        // preserve the known divisions' skip selection.
        let count = rows.len() - usize::from(hour == 1);
        let acquired_rows = &rows[..count];
        let outcome = Outcome::Rows { len: count };
        let live = populated.world.live();
        let guarded = NoFooter {
            memory: &live,
            armies: &populated.world.armies[..rows.len() - 1],
        };
        // Off-slot, every known footer must skip the entire native read. The
        // initial/daily/full passes use the identical unfiltered layout.
        if hour == 2 {
            acquire(
                &guarded,
                &mut capture,
                &mut store,
                context,
                acquired_rows,
                outcome,
                &changes,
            );
        } else {
            acquire(
                &live,
                &mut capture,
                &mut store,
                context,
                acquired_rows,
                outcome,
                &changes,
            );
        }
        assert_eq!(
            capture.attack.exercise_experience[0],
            (hour != 2).then_some(i64::from(hour))
        );
        let owned = Arc::new(crate::division_census::UnitInputs::fixture(
            acquired_rows.to_vec(), outcome,
        ));
        assert!(capture.retain_owned_settlement(store, Some(owned), context).is_none());
        // Decode has no memory capability. Changing the live footer now cannot
        // change either the carried sample or the owned fresh observations.
        drop(guarded);
        drop(live);
        for &army in &populated.world.armies {
            let at = army - populated.world.base + supply::ASKED;
            populated.world.bytes[at..at + 8].copy_from_slice(&i64::MIN.to_le_bytes());
        }
        capture.decode_inputs();
        let TableView::Complete(TableRows::DivisionStatus(published)) = capture.tables()[1].clone()
        else {
            panic!("complete owned daily status");
        };
        for row in published {
            let sample = if hour == 2 && row.division != rows.last().unwrap().division {
                1
            } else {
                hour
            };
            assert_eq!(row.experience, Recorded::Value(Some(i64::from(sample))));
            let Recorded::Value(Some(footer)) = row.supply else {
                panic!("owned footer");
            };
            assert_eq!(footer.asked, i64::from(sample) * 100_000);
            assert_eq!(
                row.supply_ratio,
                DivisionStatusRow::SUPPLY_RATIO_STEP.round(i64::from(hour) * 10_000)
            );
        }
        drop(capture.take_settled_store().unwrap());
    }
}

#[test]
fn footer_daily_cache_requires_the_complete_occurrence_and_preserves_ordered_origins() {
    use collector_division_parts::{attack_stats::ArmyStatStore, supply_inputs::Sample};
    let seed = DivisionSeed {
        pointer: 0x1000,
        division: 7,
        handle: Some(7),
        class: Some(0),
        primary_vtable: Some(carmy::VTABLE_RVA as u64),
        secondary_vtable: Some(carmy::SECONDARY_VTABLE_RVA as u64),
        ..Default::default()
    };
    let origins = [supply::Origin {
        province: 3,
        requested: -5,
        received: 9,
    }; 2];
    let sample = Sample::owned(
        Some(supply::Inputs {
            origins_len: Ok(2),
            ..Default::default()
        }),
        None,
        Some(&origins),
    )
    .unwrap();
    let mut store = ArmyStatStore::new();
    store.daily_progress(true);
    store.remember_supply(0, &[seed], &[Some(Arc::clone(&sample))], true);
    let mut order = Default::default();
    assert_eq!(
        store
            .daily_supply(0, 0, &seed, &mut order)
            .unwrap()
            .origins(),
        Some(origins.as_slice())
    );
    for other in [
        DivisionSeed {
            pointer: 0x2000,
            ..seed
        },
        DivisionSeed {
            division: 8,
            handle: Some(8),
            ..seed
        },
    ] {
        assert!(store.daily_supply(0, 0, &other, &mut order).is_none());
    }
    store.remember_supply(0, &[seed], &[Some(Arc::clone(&sample))], false);
    assert!(store.daily_supply(0, 0, &seed, &mut order).is_none());
    // The same range scratch must also survive a replacement baseline in a different order.
    let other = DivisionSeed {
        division: 8,
        handle: Some(8),
        ..seed
    };
    let empty = Sample::owned(None, None, None).unwrap();
    store.remember_supply(0, &[other, seed], &[Some(empty), Some(sample)], true);
    assert_eq!(
        store.daily_supply(0, 0, &seed, &mut order).unwrap().origins(),
        Some(origins.as_slice())
    );
}
