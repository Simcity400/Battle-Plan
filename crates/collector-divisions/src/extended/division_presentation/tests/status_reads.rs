use super::*;
use collector_division_parts::{army_stat_changes::{Changes, Watches}, attack_stats::ArmyStatStore};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

struct ObservedChanges {
    watches: Watches,
    experience: AtomicUsize,
    refuse_experience: AtomicBool,
}
impl Changes for ObservedChanges {
    fn experience_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.experience.fetch_add(1, Relaxed);
        (!self.refuse_experience.load(Relaxed))
            .then(|| self.watches.experience_sequence(receiver))
            .flatten()
    }
    fn progression_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.watches.progression_sequence(receiver)
    }
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.watches.take(visit)
    }
    fn register(&self, receiver: usize) -> bool {
        self.watches.register(receiver)
    }
}

#[test]
fn division_presentation_rows_share_one_experience_certificate() {
    const DIVISIONS: usize = 48;
    let shape = scale_fixture::Shape {
        countries: 16,
        divisions: DIVISIONS,
        wings: 1,
        planes: 1,
        ships: 0,
    };
    let mut world = scale_fixture::build(shape);
    let _registry = template_registry(&mut world);
    let _members = division_name_members(&mut world);
    let changes = ObservedChanges {
        watches: Watches::enabled(2 * DIVISIONS),
        experience: AtomicUsize::new(0),
        refuse_experience: AtomicBool::new(false),
    };
    let mut census = crate::division_census::Census::new();
    let capacities = Capacities {
        status: DIVISIONS,
        name_groups: shape.countries,
        ..Default::default()
    };
    let mut held = Captured::with_capacities(&capacities);
    let mut fresh = Captured::with_capacities(&capacities);
    held.reserve_sources(DIVISIONS);
    fresh.reserve_sources(DIVISIONS);
    let mut store = ArmyStatStore::new();
    for hour in 1..=4 {
        changes.refuse_experience.store(hour == 4, Relaxed);
        if hour == 3 {
            for (index, &army) in world.armies.iter().enumerate().step_by(7) {
                let at = army - world.base + carmy::EXPERIENCE;
                let value = if index % 2 == 0 { i64::MIN } else { i64::MAX };
                world.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
                drop(changes.watches.notify(
                    army,
                    engine_mutations::army_stat_mutations::EXPERIENCE_FIRST,
                ));
            }
        }
        loop {
            census.clear();
            census.capture(&world.live(), world.base, world.state);
            if !census.overflowed {
                break;
            }
            census.grow();
        }
        let inputs = census.unit_inputs().unwrap();
        let context = CaptureContext {
            base: world.base,
            game_state: world.state,
            hour,
        };
        let sources = || Sources {
            rows: inputs.seeds(),
            outcome: inputs.outcome(),
        };
        changes.experience.store(0, Relaxed);
        held.capture_owned_observed(
            &world.live(),
            context,
            sources(),
            Some(&mut store),
            &changes,
        );
        assert_eq!(changes.experience.swap(0, Relaxed), DIVISIONS);
        held.capture_attack(&world.live(), context, sources(), Some(&mut store));
        held.decode_inputs();
        let mut baseline = ArmyStatStore::new();
        fresh.capture_owned_observed(
            &world.live(),
            context,
            sources(),
            Some(&mut baseline),
            &changes,
        );
        assert_eq!(changes.experience.swap(0, Relaxed), DIVISIONS);
        fresh.capture_attack(&world.live(), context, sources(), Some(&mut baseline));
        fresh.decode_inputs();
        assert_eq!(held.outcomes(), fresh.outcomes());
        assert_eq!(held.tables(), fresh.tables());
        let TableView::Complete(TableRows::DivisionStatus(rows)) = &held.tables()[1] else {
            panic!("complete status rows");
        };
        assert_eq!(rows.len(), DIVISIONS);
        if hour == 4 {
            assert!(
                rows.iter()
                    .all(|row| row.experience == Recorded::Value(None))
            );
        }
        assert!(!changes.watches.poisoned());
    }
}

#[test]
#[ignore = "synthetic watch timing; run explicitly with --nocapture"]
fn bench_division_presentation_experience_certificate() {
    use std::{hint::black_box, time::Instant};
    const DIVISIONS: usize = 5_000;
    let changes = Watches::enabled(2 * DIVISIONS);
    let receivers: Vec<_> = (1..=DIVISIONS).map(|index| index * 0x2000).collect();
    for &receiver in &receivers {
        assert!(changes.experience_sequence(receiver).is_some());
    }
    let mut samples = Vec::new();
    for run in 0..28 {
        let begin = Instant::now();
        for &receiver in &receivers {
            let _ = black_box(changes.experience_sequence(black_box(receiver)));
        }
        if run >= 3 {
            samples.push(begin.elapsed().as_nanos() as f64 / DIVISIONS as f64);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "DIVISION_STATUS_EXPERIENCE divisions={DIVISIONS} p50_ns_per_certificate={:.1}",
        samples[samples.len() / 2]
    );
}
