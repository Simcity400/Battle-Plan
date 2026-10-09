use super::*;
use engine::{
    cequipment_variant as variant, cequipment_variant_pool as pool, cgame_state,
    cland_combatant as side, cpdx_array::Array, cstats_observer as stats,
};

// Independent synthetic native objects. LiveMemory exercises the same SEH
// batches as capture; no game process or installed content is accessed.
pub(crate) struct World {
    pub(crate) combat: Vec<u8>,
    sides: [Vec<u8>; 2],
    entries: [Vec<stats::LossEntry>; 2],
    variants: Vec<[u64; 2]>,
}
impl World {
    pub(crate) fn new(count: usize) -> Self {
        let variants = (0..count)
            .map(|n| [variant::VTABLE_RVA as u64, ((n as u64 + 1) << 32) | 70])
            .collect::<Vec<_>>();
        let mut out = Self {
            combat: vec![0; cland_combat::DURATION + 4],
            sides: std::array::from_fn(|_| {
                vec![0; side::STATS_OBSERVER + stats::EQUIPMENT_LOSSES + pool::ALLOW_ZERO + 1]
            }),
            entries: std::array::from_fn(|_| {
                variants
                    .iter()
                    .enumerate()
                    .map(|(n, v)| stats::LossEntry {
                        variant: v.as_ptr() as u64,
                        amount_raw: n as i64 * 100_000 + 1,
                    })
                    .collect()
            }),
            variants,
        };
        let combat = out.combat.as_ptr() as u64;
        Self::q(&mut out.combat, 0, cland_combat::VTABLE_RVA as u64);
        Self::q(
            &mut out.combat,
            cland_combat::BODY,
            cland_combat::BODY_VTABLE_RVA as u64,
        );
        Self::q(&mut out.combat, cland_combat::HANDLE, 7 << 32 | 62);
        for n in 0..2 {
            Self::q(
                &mut out.combat,
                [cland_combat::ATTACKER, cland_combat::DEFENDER][n],
                out.sides[n].as_ptr() as u64,
            );
            Self::q(&mut out.sides[n], 0, side::VTABLE_RVA as u64);
            Self::q(&mut out.sides[n], side::COMBAT, combat);
            out.sides[n][side::IS_ATTACKER] = u8::from(n == 0);
            Self::q(
                &mut out.sides[n],
                side::STATS_OBSERVER,
                stats::VTABLE_RVA as u64,
            );
            Self::q(
                &mut out.sides[n],
                side::STATS_OBSERVER + stats::EQUIPMENT_LOSSES,
                pool::VTABLE_RVA as u64,
            );
            out.set(n, n as i32 + 11, count);
        }
        out
    }
    fn q(bytes: &mut [u8], at: usize, word: u64) {
        bytes[at..at + 8].copy_from_slice(&word.to_le_bytes());
    }
    pub(crate) fn set(&mut self, n: usize, manpower: i32, count: usize) {
        let observer = side::STATS_OBSERVER;
        self.sides[n][observer + stats::MANPOWER_LOSS..observer + stats::MANPOWER_LOSS + 4]
            .copy_from_slice(&manpower.to_le_bytes());
        let array = observer + stats::EQUIPMENT_LOSSES + pool::ITEMS;
        Self::q(&mut self.sides[n], array, self.entries[n].as_ptr() as u64);
        self.sides[n][array + 8..array + 12]
            .copy_from_slice(&(self.entries[n].len() as u32).to_le_bytes());
        self.sides[n][array + 12..array + 16].copy_from_slice(&(count as u32).to_le_bytes());
    }
    fn source(&self, id: u64) -> cland_combat::Source {
        cland_combat::Source {
            pointer: self.combat.as_ptr() as usize,
            id,
            sides: self.sides.each_ref().map(|v| v.as_ptr() as usize),
            attackers: Array::default(),
            defenders: Array::default(),
            provinces: Array::default(),
        }
    }
}
struct Registry {
    state: Vec<u8>,
    _pointers: Vec<usize>,
}
impl Registry {
    fn new(worlds: &[World], naval: &[u64; 1]) -> Self {
        let mut pointers = vec![naval.as_ptr() as usize];
        pointers.extend(worlds.iter().map(|world| world.combat.as_ptr() as usize));
        let mut state = vec![0; cgame_state::COMBATS + 24];
        let at = cgame_state::COMBATS + 8;
        World::q(&mut state, at, pointers.as_ptr() as u64);
        let count = pointers.len() as u32;
        state[at + 8..at + 12].copy_from_slice(&count.to_le_bytes());
        state[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
        Self {
            state,
            _pointers: pointers,
        }
    }
}
impl Memory for Registry {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == cgame_state::SINGLETON_RVA && output.len() == 8 {
            output.copy_from_slice(&(self.state.as_ptr() as u64).to_le_bytes());
            true
        } else {
            unsafe { engine::memory::LiveMemory::new() }.copy(address, output)
        }
    }
}

fn read(c: &mut Captured, worlds: &[World]) {
    let memory = unsafe { engine::memory::LiveMemory::new() };
    c.begin();
    for (n, world) in worlds.iter().enumerate() {
        c.visit(&memory, 0, world.source((n as u64 + 1) << 32 | 62));
    }
    c.finish(Outcome::Rows { len: worlds.len() });
}
fn rows(c: &Captured) -> Vec<LandCombatProgressRow> {
    let [TableView::Complete(replay::rows::TableRows::LandCombatProgress(rows))] = c.tables()
    else {
        panic!("expected complete losses")
    };
    rows
}
#[test]
fn hourly_losses_changed_new_variant_empty_pool_and_capacity_growth() {
    let mut capacity = Capacities {
        rows: 1,
        variants: 0,
    };
    let mut c = Captured::with_capacities(&capacity);
    let mut worlds = vec![World::new(3)];
    worlds[0].set(0, -7, 1);
    worlds[0].set(1, 13, 1);
    read(&mut c, &worlds);
    assert!(
        !c.losses_complete(),
        "first loss demand waits for off-thread growth"
    );
    assert_eq!(c.equipment.required, 2);
    c.grow(&mut capacity);
    read(&mut c, &worlds);
    let first = rows(&c);
    let allocated = c.allocated_bytes();
    assert_eq!(first[0].attacker_manpower_loss, (-7).into());
    assert_eq!(
        first[0]
            .attacker_equipment_losses
            .require("fixture")
            .unwrap()[0]
            .variant,
        1 << 32 | 70
    );
    read(&mut c, &worlds);
    assert_eq!(rows(&c), first);
    assert_eq!(
        c.allocated_bytes(),
        allocated,
        "stable census does not grow"
    );
    // Equal size and sum do not certify unchanged per-variant amounts.
    worlds[0].entries[0][0].amount_raw += 1;
    worlds[0].set(0, 15, 2); // New variant joins mid-battle.
    read(&mut c, &worlds);
    assert!(
        !c.losses_complete(),
        "a new variant can exceed the last off-thread census"
    );
    c.grow(&mut capacity);
    read(&mut c, &worlds);
    let changed = rows(&c);
    assert_eq!(
        changed[0]
            .attacker_equipment_losses
            .require("fixture")
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        changed[0]
            .attacker_equipment_losses
            .require("fixture")
            .unwrap()[0]
            .amount_raw,
        2
    );
    let sum = changed[0]
        .attacker_equipment_losses
        .require("fixture")
        .unwrap()
        .iter()
        .map(|e| e.amount_raw)
        .sum::<i64>();
    worlds[0].entries[0][0].amount_raw += 3;
    worlds[0].entries[0][1].amount_raw -= 3;
    read(&mut c, &worlds);
    let swapped = rows(&c);
    assert_eq!(
        swapped[0]
            .attacker_equipment_losses
            .require("fixture")
            .unwrap()
            .iter()
            .map(|e| e.amount_raw)
            .sum::<i64>(),
        sum
    );
    assert_ne!(
        swapped, changed,
        "equal count and sum must not reuse an old pool"
    );
    worlds[0].set(0, 16, 0);
    read(&mut c, &worlds);
    assert!(
        rows(&c)[0]
            .attacker_equipment_losses
            .require("fixture")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        rows(&c)[0].defender_equipment_losses,
        first[0].defender_equipment_losses
    );
    c.finish(Outcome::Unavailable {
        reason: "census gap",
    });
    assert!(!c.losses_complete());
    c.begin();
    c.finish(Outcome::Rows { len: 0 }); // Ended between hourly reads.
    assert!(rows(&c).is_empty());
    assert!(!worlds[0].variants.is_empty()); // Keep identity storage alive.
}
#[test]
fn hourly_losses_parent_capacity_preserves_full_growth_demand_and_progress() {
    let worlds = [World::new(1), World::new(1)];
    let mut capacity = Capacities {
        rows: 1,
        variants: 4,
    };
    let mut c = Captured::with_capacities(&capacity);
    read(&mut c, &worlds);
    c.finish(Outcome::Truncated {
        len: 2,
        unread_rows: 3,
    });
    assert_eq!(
        c.outcomes()[0],
        Outcome::Truncated {
            len: 1,
            unread_rows: 4
        }
    );
    assert_eq!(c.rows.rows[0].attacker_progress, 50_000);
    assert_eq!(c.rows.rows[0].defender_progress, 50_000);
    c.grow(&mut capacity);
    assert!(capacity.rows >= 5);
    read(&mut c, &worlds);
    assert!(c.losses_complete());
    c.reset();
    c.finish(Outcome::Rows { len: 1 });
    assert!(matches!(c.outcomes()[0], Outcome::Unavailable { .. }));
}
#[test]
fn hourly_losses_visit_never_grows_and_counts_shortfall_in_coverage() {
    use crate::land_events::test_support;
    let _guard = test_support::EXCLUSIVE.lock().unwrap();
    test_support::reset();
    let mut events = crate::extended::land_events::Captured::with_capacities(
        &crate::extended::land_events::Capacities { words: 0 },
    );
    events.drain(10, true);
    events.drain(11, false);
    let mut capacity = Capacities {
        rows: 1,
        variants: 1,
    };
    let mut captured = Captured::with_capacities(&capacity);
    let worlds = [World::new(3), World::new(5)];
    let allocated = captured.allocated_bytes();
    let capacities = (
        captured.rows.rows.capacity(),
        captured.losses.capacity(),
        captured.equipment.first.capacity(),
        captured.pointers.capacity(),
        captured.identities.capacity(),
    );
    let lengths = (
        captured.rows.rows.len(),
        captured.losses.len(),
        captured.equipment.first.len(),
        captured.pointers.len(),
        captured.identities.len(),
    );
    read(&mut captured, &worlds);
    assert_eq!(captured.allocated_bytes(), allocated);
    assert_eq!(
        (
            captured.rows.rows.capacity(),
            captured.losses.capacity(),
            captured.equipment.first.capacity(),
            captured.pointers.capacity(),
            captured.identities.capacity()
        ),
        capacities
    );
    assert_eq!(
        (
            captured.rows.rows.len(),
            captured.losses.len(),
            captured.equipment.first.len(),
            captured.pointers.len(),
            captured.identities.len()
        ),
        lengths
    );
    assert_eq!(
        captured.equipment.required, 16,
        "count every side even after shortage"
    );
    assert_eq!(captured.lost_reads(), 2);
    events.lost_casualty_reads(captured.lost_reads());
    let [
        _,
        _,
        TableView::Complete(replay::rows::TableRows::LandEventCoverage(coverage)),
    ] = events.tables()
    else {
        panic!("coverage missing")
    };
    assert_eq!(coverage[0].lost_events, 2);
    assert!(
        coverage[0].complete,
        "independent event interval remains certified"
    );
    let epoch = coverage[0].epoch;
    captured.grow(&mut capacity);
    read(&mut captured, &worlds);
    assert!(captured.losses_complete());
    events.drain(12, false);
    let [
        _,
        _,
        TableView::Complete(replay::rows::TableRows::LandEventCoverage(coverage)),
    ] = events.tables()
    else {
        panic!("coverage missing")
    };
    assert!(coverage[0].complete);
    assert_eq!(coverage[0].epoch, epoch);
    test_support::disable();
}
#[test]
fn hourly_losses_off_thread_census_presizes_all_sides() {
    let mut worlds = [World::new(3), World::new(9)];
    worlds[1].set(1, 12, 0);
    let naval = [engine::cnaval_combat::VTABLE_RVA as u64];
    let registry = Registry::new(&worlds, &naval);
    let mut capacity = Capacities::default();
    capacity.presize(&registry, 0).unwrap();
    assert!(capacity.rows >= 2);
    assert!(capacity.variants >= 15);
    let mut captured = Captured::with_capacities(&capacity);
    read(&mut captured, &worlds);
    assert!(
        captured.losses_complete(),
        "attach census includes first-hour losses"
    );
    let first_capacity = captured.equipment.first.len();
    worlds[1].set(1, 12, 9);
    read(&mut captured, &worlds);
    assert!(
        !captured.losses_complete(),
        "native pool grew past the earlier census"
    );
    assert_eq!(captured.equipment.first.len(), first_capacity);
    capacity.presize(&registry, 0).unwrap();
    captured.grow(&mut capacity);
    assert!(captured.equipment.first.len() > first_capacity);
    read(&mut captured, &worlds);
    assert!(
        captured.losses_complete(),
        "recycle census includes later pool growth"
    );
}
#[test]
fn hourly_losses_read_failure_never_becomes_zero() {
    struct Unreadable;
    impl Memory for Unreadable {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let world = World::new(0);
    let mut c = Captured::with_capacities(&Capacities {
        rows: 1,
        variants: 0,
    });
    c.begin();
    c.visit(&Unreadable, 0, world.source(62));
    c.finish(Outcome::Rows { len: 1 });
    assert!(matches!(c.outcomes()[0], Outcome::Unavailable { .. }));
}

#[test]
#[ignore = "print-only hourly casualty bench after live-count presize; lead runs through build slot"]
fn land_casualty_hourly_timing() {
    // Fixture population, never a production admission limit. Both native sides
    // have complete per-variant pools; no installed content/process is read.
    let provinces: usize = std::env::var("BATTLE_PLAN_LAND_PROVINCES").or_else(|_| std::env::var("HOI4_LAND_PROVINCES"))
        .map(|value| value.parse().unwrap())
        .unwrap_or(20_000);
    let fraction: usize = std::env::var("BATTLE_PLAN_LAND_BATTLE_PERCENT").or_else(|_| std::env::var("HOI4_LAND_BATTLE_PERCENT"))
        .map(|value| value.parse().unwrap())
        .unwrap_or(1);
    let variants: usize = std::env::var("BATTLE_PLAN_LAND_LOSS_VARIANTS").or_else(|_| std::env::var("HOI4_LAND_LOSS_VARIANTS"))
        .map(|value| value.parse().unwrap())
        .unwrap_or(12);
    let count = provinces.checked_mul(fraction).unwrap() / 100;
    let worlds = (0..count).map(|_| World::new(variants)).collect::<Vec<_>>();
    let naval = [engine::cnaval_combat::VTABLE_RVA as u64];
    let registry = Registry::new(&worlds, &naval);
    let mut capacity = Capacities::default();
    capacity.presize(&registry, 0).unwrap();
    let mut captured = Captured::with_capacities(&capacity);
    read(&mut captured, &worlds);
    assert!(captured.losses_complete());
    let bytes = captured.allocated_bytes();
    const SAMPLES: usize = 201;
    let mut times = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = std::time::Instant::now();
        read(&mut captured, std::hint::black_box(&worlds));
        let elapsed = start.elapsed().as_nanos();
        // Correctness checks and sample storage are outside the measured hour.
        assert!(captured.losses_complete());
        assert_eq!(captured.allocated_bytes(), bytes);
        times.push(elapsed);
    }
    times.sort_unstable();
    println!(
        "LAND_CASUALTY_HOURLY provinces={provinces} combats={count} sides_per_combat=2 variants_per_side={variants} samples={SAMPLES} p50_ns_per_game_hour={} p95_ns_per_game_hour={} max_ns_per_game_hour={} budget_ns_per_game_hour=100000 budget_ms_per_game_hour=0.1 arena_bytes={bytes} profile={} scope=synthetic_steady_presized_loss_and_progress_read_excludes_parent_census_and_spool",
        times[SAMPLES / 2],
        times[(SAMPLES - 1) * 95 / 100],
        times[SAMPLES - 1],
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "optimized"
        }
    );
}
