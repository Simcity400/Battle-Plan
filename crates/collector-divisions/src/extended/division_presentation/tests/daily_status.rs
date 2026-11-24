use super::*;
use collector_division_parts::army_stat_changes::Watches;
use engine_native_parts::cunit_supply_inputs as supply;

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

pub(super) fn acquire(
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
            assert!(
                capture.preparation_parts[..capture.status_part_count()]
                    .iter()
                    .all(|part| part
                        .range
                        .clone()
                        .all(|index| part.daily_supply(index).is_some()))
            );
            capture.prepare_status_observed(memory, context, sources(), Some(store), changes);
        }
    }
    capture.capture_status_observed(memory, context, sources(), Some(store), changes);
    capture.capture_attack(memory, context, sources(), Some(store));
}

#[test]
fn daily_status_owned_ranges_carry_footer_and_xp_but_keep_ratio_hourly() {
    let mut populated = Populated::new();
    let original = populated.census.unit_inputs().unwrap();
    let mut rows = original.seeds().to_vec();
    let changes = Watches::enabled(2 * rows.len());
    let slot = held_store::Slot::new(Box::new(
        collector_division_parts::attack_stats::ArmyStatStore::new(),
    )).unwrap();
    let mut capture = populated.capture();
    capture.status_packet = carmy::StatusPacket::vacant();
    capture.enable_owned_settlement();
    capture.status_workers = 2;
    let daily = replay_data::cadence::DIVISION_STATUS_DAILY_SLOT as i32;
    for hour in [1, 2, daily, daily + 2] {
        if hour == daily + 2 {
            slot.prepare_hour(daily + 1, true);
        }
        slot.prepare_hour(hour, false);
        let mut store = slot.take().unwrap();
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
        let count = rows.len() - usize::from(hour == 1);
        let acquired_rows = &rows[..count];
        let outcome = Outcome::Rows { len: count };
        let live = populated.world.live();
        let guarded = NoFooter {
            memory: &live,
            armies: &populated.world.armies[..rows.len() - 1],
        };
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
        let owned = engine_allocation::shared(crate::division_census::UnitInputs::fixture(
            acquired_rows.to_vec(), outcome,
        )).unwrap();
        assert!(capture.retain_owned_settlement(store, Some(owned), context).is_none());
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
            let Recorded::Value(Some(ref footer)) = row.supply else {
                panic!("owned footer");
            };
            assert_eq!(footer.asked, i64::from(sample) * 100_000);
            assert_eq!(row.supply_ratio, None);
            assert_eq!(
                row.supply_ratio_exact,
                Recorded::Value(Some(i64::from(hour) * 10_000))
            );
            assert_eq!(
                row.displayed_supply_ratio(),
                Some(DivisionStatusRow::SUPPLY_RATIO_STEP.round(i64::from(hour) * 10_000))
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
        primary_vtable: Some(carmy::VTABLE_RVA.0 as u64),
        secondary_vtable: Some(carmy::SECONDARY_VTABLE_RVA.0 as u64),
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

#[test]
fn staged_supply_matches_ordered_selection_across_aliases_misses_and_allocation_failures() {
    use super::super::daily_status::DailyStatus;
    use collector_division_parts::{
        attack_stats::{ArmyStatStore, PreparationPart},
        supply_inputs::{self, Sample},
    };

    struct Trace<'a, M> {
        memory: &'a M,
        reads: std::cell::RefCell<Vec<(usize, usize)>>,
    }
    impl<M: Memory> Memory for Trace<'_, M> {
        fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, bytes.len()));
            self.memory.copy(address, bytes)
        }
    }

    let populated = Populated::with_shape(scale_fixture::Shape {
        countries: 4,
        divisions: 6,
        wings: 1,
        planes: 1,
        ships: 0,
    });
    let original = populated.census.unit_inputs().unwrap().seeds();
    let baseline = [
        original[3],
        original[1],
        original[0],
        original[5],
        original[3],
        original[0],
    ];
    let rows = [
        original[0],
        original[1],
        original[2],
        original[3],
        original[3],
        original[5],
    ];
    let origins = [supply::Origin {
        province: 3,
        requested: -5,
        received: 9,
    }; 2];
    let samples: Vec<_> = (0..baseline.len())
        .map(|index| {
            Some(
                Sample::owned(
                    (index != 1).then_some(supply::Inputs {
                        asked: (index as i64 + 1) * 100_000,
                        origins_len: Ok(origins.len()),
                        ..Default::default()
                    }),
                    (index == 1).then_some(supply::Failure::Unreadable),
                    (index != 1).then_some(origins.as_slice()),
                )
                .unwrap(),
            )
        })
        .collect();
    let changes = Watches::enabled(2 * rows.len());
    let unreadable = engine_native_memory::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    let mut store = ArmyStatStore::new();
    let mut parts: [PreparationPart; 3] = std::array::from_fn(|index| {
        let mut part = PreparationPart::default();
        part.range = index * 2..(index + 1) * 2;
        part
    });
    for part in &mut parts {
        part.capture(
            &unreadable,
            populated.world.base,
            populated.world.state,
            &rows,
            Some(&store),
            &changes,
        );
    }
    let mut refused_stage = PreparationPart::default();
    refused_stage.range = 4..6;
    refused_stage.capture(
        &unreadable,
        populated.world.base,
        populated.world.state,
        &rows,
        Some(&store),
        &changes,
    );
    store.daily_progress(true);
    store.remember_supply(populated.world.base, &baseline, &samples, true);
    for part in &mut parts[1..] {
        part.capture(
            &unreadable,
            populated.world.base,
            populated.world.state,
            &rows,
            Some(&store),
            &changes,
        );
    }
    assert!(matches!(parts[1].daily_supply(2), Some(None)));
    assert!(parts[0].daily_supply(0).is_none());
    assert_eq!(
        parts[2]
            .daily_supply(4)
            .unwrap()
            .as_ref()
            .unwrap()
            .inputs
            .unwrap()
            .asked,
        500_000
    );
    collector_test_support::with_allocations_denied(|| {
        refused_stage.capture(
            &unreadable,
            populated.world.base,
            populated.world.state,
            &rows,
            Some(&store),
            &changes,
        );
    });
    assert!(collector_test_support::DENIALS.load(std::sync::atomic::Ordering::Relaxed) > 0);
    assert!(refused_stage.daily_supply(4).is_none());
    assert!(
        refused_stage.get(4).is_some(),
        "only optional supply staging failed"
    );

    let stage = |index| {
        parts
            .iter()
            .find(|part| part.range.contains(&index))
            .and_then(|part| part.daily_supply(index))
            .cloned()
    };
    let observe = |daily: &DailyStatus, packet: &mut carmy::StatusPacket| {
        let fresh: Vec<_> = (0..rows.len())
            .map(|index| !daily.pending() || daily.sample(index).is_none())
            .collect();
        let live = populated.world.live();
        let trace = Trace {
            memory: &live,
            reads: Default::default(),
        };
        assert_eq!(
            packet.capture(
                &trace,
                populated.world.base,
                populated.world.state,
                &rows,
                Outcome::Rows { len: rows.len() },
            ),
            Outcome::Rows { len: rows.len() }
        );
        let footers: Vec<_> = (0..rows.len())
            .map(|index| {
                let (inputs, failure, origins) = packet.supply_at(index);
                assert_eq!(
                    inputs.is_some(),
                    fresh[index],
                    "native footer selection {index}"
                );
                let held = daily.pending().then(|| daily.sample(index)).flatten();
                let (inputs, failure, origins) = held
                    .map_or((inputs, failure, origins), |sample| {
                        (sample.inputs, sample.failure, sample.origins())
                    });
                (supply_inputs::project_division(inputs, origins), failure)
            })
            .collect();
        (fresh, footers, trace.reads.into_inner())
    };
    let select = |staged: &dyn Fn(usize) -> Option<Option<Arc<Sample>>>,
                  store: &mut ArmyStatStore| {
        let mut daily = DailyStatus::default();
        let mut packet = carmy::StatusPacket::new(rows.len()).unwrap();
        daily.prepare(
            &mut packet,
            Some(store),
            populated.world.base,
            &rows,
            staged,
        );
        observe(&daily, &mut packet)
    };
    let ordered = select(&|_| None, &mut store);
    assert_eq!(ordered.0, [false, false, true, false, false, false]);
    assert_eq!(select(&stage, &mut store), ordered);
    assert_eq!(
        select(
            &|index| {
                refused_stage.daily_supply(index).cloned().or_else(|| {
                    (!refused_stage.range.contains(&index))
                        .then(|| stage(index))
                        .flatten()
                })
            },
            &mut store
        ),
        ordered
    );

    let missed = select(
        &|index| if index == 0 { Some(None) } else { stage(index) },
        &mut store,
    );
    assert!(missed.0[0]);
    assert_ne!(missed.1[0], ordered.1[0]);

    let mut daily = DailyStatus::default();
    let mut warm = carmy::StatusPacket::new(rows.len()).unwrap();
    daily.prepare(
        &mut warm,
        Some(&mut store),
        populated.world.base,
        &rows,
        stage,
    );
    let mut packet = carmy::StatusPacket::vacant();
    collector_test_support::with_allocations_denied(|| {
        daily.prepare(
            &mut packet,
            Some(&mut store),
            populated.world.base,
            &rows,
            stage,
        );
    });
    assert!(collector_test_support::DENIALS.load(std::sync::atomic::Ordering::Relaxed) > 0);
    assert!(!daily.pending());
    assert!(
        store
            .daily_supply(populated.world.base, 0, &rows[0], &mut Default::default())
            .is_none()
    );
    let fresh = observe(&daily, &mut packet);
    assert!(fresh.0.iter().all(|&requested| requested));
    assert_eq!(fresh, select(&|_| None, &mut store));
}

fn settled_hour(
    populated: &Populated,
    capture: &mut Captured,
    mut store: held_store::Loan,
    hour: i32,
    rows: &[DivisionSeed],
    outcome: Outcome,
    changes: &Watches,
) -> [TableView; N] {
    let context = CaptureContext {
        hour,
        ..populated.context()
    };
    acquire(&populated.world.live(), capture, &mut store, context, rows, outcome, changes);
    let owned = engine_allocation::shared(
        crate::division_census::UnitInputs::fixture(rows.to_vec(), outcome),
    )
    .unwrap();
    assert!(capture.retain_owned_settlement(store, Some(owned), context).is_none());
    capture.decode_inputs();
    capture.tables()
}

#[test]
fn an_hour_never_waits_for_the_store_the_hour_before_still_settles() {
    let populated = Populated::new();
    let original = populated.census.unit_inputs().unwrap();
    let rows = original.seeds().to_vec();
    let outcome = original.outcome();
    let changes = Watches::enabled(2 * rows.len());
    let fresh_capture = || {
        let mut capture = populated.capture();
        capture.status_packet = carmy::StatusPacket::vacant();
        capture.enable_owned_settlement();
        capture.status_workers = 2;
        capture
    };
    let new_slot = || {
        held_store::Slot::new(Box::new(
            collector_division_parts::attack_stats::ArmyStatStore::new(),
        )).unwrap()
    };
    let reference_slot = new_slot();
    let mut reference = fresh_capture();
    let expected: Vec<_> = (1..=2)
        .map(|hour| {
            reference_slot.prepare_hour(hour, false);
            let store = reference_slot.take().unwrap();
            let tables =
                settled_hour(&populated, &mut reference, store, hour, &rows, outcome, &changes);
            drop(reference.take_settled_store().unwrap());
            tables
        })
        .collect();
    let slot = new_slot();
    let mut first = fresh_capture();
    slot.prepare_hour(1, false);
    let tables = settled_hour(&populated, &mut first, slot.take().unwrap(), 1, &rows, outcome, &changes);
    assert_eq!(tables, expected[0]);
    slot.prepare_hour(2, false);
    let taker = {
        let slot = slot.clone();
        std::thread::spawn(move || slot.take())
    };
    let deadline = std::time::Instant::now() + collector_test_support::TEST_HANG_GUARD;
    while !taker.is_finished() {
        if std::time::Instant::now() >= deadline {
            drop(first.take_settled_store());
            let _ = taker.join();
            panic!("the hour waited for the store the hour before still settles");
        }
        std::thread::yield_now();
    }
    let store = taker.join().unwrap().expect("the hour's own store");
    let mut second = fresh_capture();
    let tables = settled_hour(&populated, &mut second, store, 2, &rows, outcome, &changes);
    assert!(matches!(tables[5], TableView::Complete(_)), "panel sources withheld");
    assert_eq!(tables, expected[1]);
    drop(first.take_settled_store().unwrap());
    let newer = second.take_settled_store().unwrap();
    let newer_address = &raw const *newer;
    drop(newer);
    slot.prepare_hour(3, false);
    assert_eq!(&raw const *slot.take().unwrap(), newer_address);
}
