use super::*;
use engine_land_inputs::cstats_observer as stats;
use engine_native::{
    cequipment_variant::{self as variant, pool as pool},
    cgame_state,
    cland_combatant as side,
};
use engine_native_memory::cpdx_array::Array;

pub struct World {
    pub combat: Vec<u8>,
    sides: [Vec<u8>; 2],
    entries: [Vec<stats::LossEntry>; 2],
    variants: Vec<[u64; 2]>,
}
impl World {
    pub fn new(count: usize) -> Self {
        let variants = (0..count)
            .map(|n| [engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64, ((n as u64 + 1) << 32) | 70])
            .collect::<Vec<_>>();
        let mut out = Self {
            combat: vec![0; cland_combat::DURATION + 4],
            sides: std::array::from_fn(|_| {
                vec![0; side::STATS_OBSERVER + engine_land_inputs::combat_log_events::OBSERVER_SIZE]
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
        Self::q(&mut out.combat, 0, cland_combat::VTABLE_RVA.0 as u64);
        Self::q(
            &mut out.combat,
            cland_combat::BODY,
            cland_combat::BODY_VTABLE_RVA.0 as u64,
        );
        Self::q(&mut out.combat, cland_combat::HANDLE, 7 << 32 | 62);
        for n in 0..2 {
            Self::q(
                &mut out.combat,
                [cland_combat::ATTACKER, cland_combat::DEFENDER][n],
                out.sides[n].as_ptr() as u64,
            );
            Self::q(&mut out.sides[n], 0, side::VTABLE_RVA.0 as u64);
            Self::q(&mut out.sides[n], side::COMBAT, combat);
            out.sides[n][side::IS_ATTACKER] = u8::from(n == 0);
            Self::q(
                &mut out.sides[n],
                side::STATS_OBSERVER,
                stats::VTABLE_RVA.0 as u64,
            );
            Self::q(
                &mut out.sides[n],
                side::STATS_OBSERVER + stats::EQUIPMENT_LOSSES,
                pool::VTABLE_RVA.0 as u64,
            );
            Self::q(
                &mut out.sides[n],
                side::STATS_OBSERVER + engine_land_inputs::combat_log_events::OBSERVER_SIDE,
                engine_land_inputs::combat_log_history::SIDE_VTABLE_RVA.0 as u64,
            );
            for offset in [
                engine_land_inputs::combat_log_history::SIDE_CAPTURED_BY_ENEMY,
                engine_land_inputs::combat_log_history::SIDE_RECOVERED,
            ] {
                Self::q(
                    &mut out.sides[n],
                    side::STATS_OBSERVER
                        + engine_land_inputs::combat_log_events::OBSERVER_SIDE
                        + offset,
                    pool::VTABLE_RVA.0 as u64,
                );
            }
            out.set(n, n as i32 + 11, count);
        }
        out
    }
    fn q(bytes: &mut [u8], at: usize, word: u64) {
        bytes[at..at + 8].copy_from_slice(&word.to_le_bytes());
    }
    pub fn set(&mut self, n: usize, manpower: i32, count: usize) {
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
        if address == cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&(self.state.as_ptr() as u64).to_le_bytes());
            true
        } else {
            unsafe { engine_native_memory::memory::LiveMemory::new() }.copy(address, output)
        }
    }
}

fn read(c: &mut Captured, worlds: &[World]) {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    // SAFETY: tests serialize combat-progress globals with `test_globals()`.
    unsafe { c.begin() };
    for (n, world) in worlds.iter().enumerate() {
        c.visit(&memory, 0, world.source((n as u64 + 1) << 32 | 62));
    }
    // SAFETY: paired with `c.begin()` under `test_globals()`.
    unsafe { c.finish(Outcome::Rows { len: worlds.len() }) };
}
fn rows(c: &Captured) -> Vec<LandCombatProgressRow> {
    let [
        TableView::Complete(replay_data::rows::TableRows::LandCombatProgress(rows)),
        _,
    ] = c.tables()
    else {
        panic!("expected complete losses")
    };
    rows
}
#[test]
fn hourly_losses_changed_new_variant_empty_pool_and_capacity_growth() {
    let _globals = super::test_globals();
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
        c.losses_complete(),
        "first loss demand grows before copying the original pools"
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
    worlds[0].entries[0][0].amount_raw += 1;
    worlds[0].set(0, 15, 2);
    read(&mut c, &worlds);
    assert!(
        c.losses_complete(),
        "new variants grow storage during the original worker acquisition"
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
    // SAFETY: test holds `test_globals()` as the sole combat-progress caller.
    unsafe {
        c.finish(Outcome::Unavailable {
            reason: "census gap",
        });
    }
    assert!(!c.losses_complete());
    // SAFETY: test holds `test_globals()` as the sole combat-progress caller.
    unsafe {
        c.begin();
        c.finish(Outcome::Rows { len: 0 });
    }
    assert!(rows(&c).is_empty());
    assert!(!worlds[0].variants.is_empty());
}
#[test]
fn hourly_losses_parent_capacity_preserves_full_growth_demand_and_progress() {
    let _globals = super::test_globals();
    let worlds = [World::new(1), World::new(1)];
    let mut capacity = Capacities {
        rows: 1,
        variants: 4,
    };
    let mut c = Captured::with_capacities(&capacity);
    read(&mut c, &worlds);
    // SAFETY: test holds `test_globals()` as the sole combat-progress caller.
    unsafe {
        c.finish(Outcome::Truncated {
            len: 2,
            unread_rows: 3,
        });
    }
    assert_eq!(
        c.outcomes()[0],
        Outcome::Truncated {
            len: 2,
            unread_rows: 3
        }
    );
    assert_eq!(c.rows.rows[0], 1 << 32 | 62);
    c.grow(&mut capacity);
    assert!(capacity.rows >= 5);
    read(&mut c, &worlds);
    assert!(c.losses_complete());
    c.reset();
    // SAFETY: test holds `test_globals()` as the sole combat-progress caller.
    unsafe { c.finish(Outcome::Rows { len: 1 }) };
    assert!(matches!(c.outcomes()[0], Outcome::Unavailable { .. }));
}
#[test]
fn hourly_losses_worker_growth_captures_every_combat_without_coverage_loss() {
    let _globals = super::test_globals();
    use crate::land_events::test_support;
    let fixture = test_support::Fixture::new(1024);
    let mut events = crate::extended::land_events::Captured::with_capacities(
        &crate::extended::land_events::Capacities { words: 0 },
    );
    events.drain_from(&fixture.observer, 10, true);
    events.drain_from(&fixture.observer, 11, false);
    let mut captured = Captured::with_capacities(&Capacities {
        rows: 1,
        variants: 1,
    });
    let worlds = (0..20).map(|_| World::new(3)).collect::<Vec<_>>();
    read(&mut captured, &worlds);
    assert_eq!(
        captured.outcomes(),
        [Outcome::Rows { len: 20 }, Outcome::Rows { len: 440 }]
    );
    assert_eq!(captured.equipment.required, 120);
    assert_eq!(captured.equipment.len, 120);
    assert_eq!(captured.lost_reads(), 0);
    let actual = rows(&captured);
    assert_eq!(actual.len(), 20);
    assert!(actual.iter().all(|row| {
        row.attacker_equipment_losses
            .require("fixture")
            .unwrap()
            .len()
            == 3
            && row
                .defender_equipment_losses
                .require("fixture")
                .unwrap()
                .len()
                == 3
    }));
    let allocated = captured.allocated_bytes();
    read(&mut captured, &worlds);
    assert_eq!(captured.allocated_bytes(), allocated);
    events.lost_casualty_reads_from(&fixture.observer, captured.lost_reads());
    let [
        _,
        _,
        TableView::Complete(replay_data::rows::TableRows::LandEventCoverage(coverage)),
        _,
    ] = events.tables()
    else {
        panic!("coverage missing")
    };
    assert_eq!(coverage[0].lost_events, 0);
    assert!(coverage[0].complete);
    fixture.stop();
}
#[test]
fn hourly_losses_off_thread_census_presizes_all_sides() {
    let _globals = super::test_globals();
    let mut worlds = [World::new(3), World::new(9)];
    worlds[1].set(1, 12, 0);
    let naval = [engine_native::cnaval_combat::VTABLE_RVA.0 as u64];
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
        captured.losses_complete(),
        "native pool growth is captured without waiting for another census"
    );
    assert!(captured.equipment.first.len() > first_capacity);
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
    let _globals = super::test_globals();
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
    // SAFETY: test holds `test_globals()` as the sole combat-progress caller.
    unsafe { c.begin() };
    c.visit(&Unreadable, 0, world.source(62));
    // SAFETY: paired with `c.begin()` under `test_globals()`.
    unsafe { c.finish(Outcome::Rows { len: 1 }) };
    assert!(matches!(c.outcomes()[0], Outcome::Unavailable { .. }));
}

#[test]
fn combat_log_complete_observer_uses_the_loss_census_and_preserves_signed_counters() {
    let _globals = super::test_globals();
    let mut world = World::new(3);
    World::q(
        &mut world.sides[0],
        side::STATS_OBSERVER + engine_land_inputs::combat_log_events::OBSERVER_PROGRESS,
        987_654,
    );
    let counter = side::STATS_OBSERVER + engine_land_inputs::combat_log_events::OBSERVER_COUNTERS;
    world.sides[0][counter..counter + 4].copy_from_slice(&i32::MIN.to_le_bytes());
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let mut captured = Captured::with_capacities(&Capacities::default());
    // SAFETY: test holds `test_globals()` as the sole combat-progress caller.
    unsafe { captured.begin() };
    captured.visit(&memory, 0, world.source(7 << 32 | 62));
    // SAFETY: paired with `captured.begin()` under `test_globals()`.
    unsafe { captured.finish(Outcome::Rows { len: 1 }) };
    assert_eq!(captured.outcomes()[1], Outcome::Rows { len: 22 });
    let rows = &captured.observers;
    assert!(rows.iter().any(|r| r.attacker
        && r.kind == engine_land_inputs::combat_log_events::OBSERVER_STATE as u8
        && r.b == 987_654));
    assert!(rows.iter().any(|r| r.attacker
        && r.kind == engine_land_inputs::combat_log_events::OBSERVER_MODIFIERS as u8
        && r.a as i32 == i32::MIN));
    assert!(
        rows.iter()
            .filter(|r| r.kind == engine_land_inputs::combat_log_events::SIDE as u8)
            .all(|r| r.fact().2[0] == 0)
    );
    assert!(
        !rows.iter().any(
            |r| r.kind == engine_land_inputs::combat_log_events::EQUIPMENT as u8 && r.item == 0
        )
    );
}

#[test]
fn combat_log_projection_uses_committed_labels_across_capture_buffers_and_keyframes() {
    let _globals = super::test_globals();
    use collector_projection::projection::{Projection, ProjectionScratch};
    use replay_data::rows::{CombatLogObserverKind as K, CombatLogObserverRow, TableRows};
    use replay_frame::frame::{FrameEncoder, FrameInput, Metrics, TableInput, TableUpdate};
    let scratch = ProjectionScratch::default();
    let mut encoder = FrameEncoder::new();
    let mut previous: Vec<CombatLogObserverRow> = Vec::new();
    for (branch, hour) in [(1, 1), (1, 2), (1, 3), (1, 4), (1, 6), (2, 7)] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.rows.outcome = Outcome::Rows { len: 0 };
        for (kind, item, values) in [
            (K::State, 0, [hour as u64, 0, 0, 0, 0]),
            (K::Activity, 0, [71, 0, 0, 0, 0]),
            (K::ActivityDamage, 0, [0, 72, 10, 0, 0]),
        ] {
            captured.observers.push(
                CombatLogObserverRow::from_fact(u64::MAX, true, kind as u8, item, values).unwrap(),
            );
        }
        if hour > 1 {
            captured.observers.insert(
                1,
                CombatLogObserverRow::from_fact(
                    u64::MAX,
                    true,
                    K::Leader as u8,
                    0,
                    [73, 1, 0, 0, 0],
                )
                .unwrap(),
            );
        }
        let force = hour == 2 || hour == 6;
        let mut projection = Projection::new(&encoder, branch, hour, force, &scratch);
        let [_, observers] = captured.table_inputs(Some(&mut projection));
        let TableInput::Sampled(TableView::Complete(TableRows::CombatLogObservers(rows))) =
            &observers
        else {
            panic!("complete observer census missing")
        };
        if hour == 2 {
            for old in &previous {
                let new = rows
                    .iter()
                    .find(|row| row.semantic_key() == old.semantic_key())
                    .unwrap();
                assert_eq!(new.key(), old.key());
            }
        }
        if hour == 4 || hour == 6 || branch == 2 {
            assert_eq!(
                rows,
                &collector_combat_log::observer_projection::project(&captured.observers, &[])
                    .unwrap(),
                "a discontinuity cannot reuse another observation interval's labels"
            );
        }
        previous = rows.clone();
        let mut input = FrameInput {
            branch,
            hour,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        };
        input.tables[TableId::CombatLogObservers.index()] = if hour == 3 {
            TableView::Unavailable {
                reason: "synthetic observer refusal".into(),
            }
            .into()
        } else {
            observers
        };
        let encoded = encoder.encode_input(input, force).unwrap();
        if hour == 4 || hour == 6 || branch == 2 {
            assert!(
                matches!(
                    encoded.tables[TableId::CombatLogObservers.index()],
                    TableUpdate::Snapshot(_)
                ),
                "refusals, gaps and branches require a fresh snapshot"
            );
        }
    }
}

fn daily_capture(memory: &impl Memory, world: &World, hour: i32, full: bool) -> Captured {
    let mut captured = Captured::with_capacities(&Capacities::default());
    // SAFETY: tests serialize combat-progress globals with `test_globals()`.
    unsafe {
        captured.prepare_hour(hour, full);
        captured.begin();
    }
    captured.visit(memory, 0, world.source(7 << 32 | 62));
    // SAFETY: paired with `captured.begin()` under `test_globals()`.
    unsafe { captured.finish(Outcome::Rows { len: 1 }) };
    assert!(
        matches!(captured.outcomes()[1], Outcome::Rows { .. }),
        "{:?}",
        captured.outcomes()
    );
    captured
}
fn observer_row(
    captured: &Captured,
    kind: u64,
    item: u64,
) -> &replay_data::rows::CombatLogObserverRow {
    captured
        .observers
        .iter()
        .find(|row| row.attacker && row.kind as u64 == kind && row.item == item)
        .unwrap()
}
fn native_array(bytes: &mut [u8], offset: usize, pointer: usize, count: u32) {
    World::q(bytes, offset, pointer as u64);
    bytes[offset + 8..offset + 12].copy_from_slice(&count.to_le_bytes());
    bytes[offset + 12..offset + 16].copy_from_slice(&count.to_le_bytes());
}

#[test]
fn observer_daily_numeric_carry_keeps_hourly_columns_and_reads_new_rows_fresh() {
    let _globals = super::test_globals();
    use engine_land_inputs::combat_log_events as event;
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let mut world = World::new(0);
    let origin = side::STATS_OBSERVER;
    World::q(
        &mut world.sides[0],
        origin + event::OBSERVER_TOTAL_DAMAGE,
        11,
    );
    World::q(&mut world.sides[0], origin + event::OBSERVER_PROGRESS, 12);
    let before = daily_capture(&memory, &world, 12, true);
    let old = observer_row(&before, event::OBSERVER_STATE, 0).clone();
    World::q(
        &mut world.sides[0],
        origin + event::OBSERVER_TOTAL_DAMAGE,
        99,
    );
    World::q(&mut world.sides[0], origin + event::OBSERVER_PROGRESS, 98);
    world.sides[0][origin + event::OBSERVER_FLAGS] = 7;
    let counters = origin + event::OBSERVER_COUNTERS;
    for (lane, value) in [41_u32, 42, 43, 44, 45].into_iter().enumerate() {
        world.sides[0][counters + lane * 4..counters + lane * 4 + 4]
            .copy_from_slice(&value.to_le_bytes());
    }
    let mut leader = [0; 12];
    World::q(&mut leader, 0, 51);
    leader[8..].copy_from_slice(&(-17_i32).to_le_bytes());
    native_array(
        &mut world.sides[0],
        origin + stats::LEADERS,
        leader.as_ptr() as usize,
        1,
    );
    let after = daily_capture(&memory, &world, 13, false);
    let state = observer_row(&after, event::OBSERVER_STATE, 0);
    assert_eq!((state.a, state.b, state.c), (old.a, old.b, 7));
    let counters = observer_row(&after, event::OBSERVER_MODIFIERS, 0);
    assert_eq!(
        (counters.a, counters.b, counters.c, counters.d, counters.e),
        (0, 0, 43, 44, 45)
    );
    assert_eq!(
        observer_row(&after, event::OBSERVER_LEADER, 0).a as i32,
        -17
    );
    leader[8..].copy_from_slice(&23_i32.to_le_bytes());
    let carried = daily_capture(&memory, &world, 14, false);
    assert_eq!(
        observer_row(&carried, event::OBSERVER_LEADER, 0).a as i32,
        -17
    );
    World::q(&mut leader, 0, 52);
    let changed = daily_capture(&memory, &world, 15, false);
    assert_eq!(observer_row(&changed, event::OBSERVER_LEADER, 0).a, 23);
    let refreshed = daily_capture(&memory, &world, 16, true);
    assert_eq!(observer_row(&refreshed, event::OBSERVER_STATE, 0).a, 99);
    let mut fresh = Vec::new();
    event::observer(&memory, 0, world.sides[0].as_ptr() as usize, &mut |fact| {
        fresh.push(fact)
    })
    .unwrap();
    let actual: Vec<_> = refreshed
        .observers
        .iter()
        .filter(|row| row.attacker)
        .map(|row| row.fact())
        .collect();
    assert_eq!(
        actual,
        fresh
            .iter()
            .map(|fact| (fact.kind as u8, fact.item, fact.values))
            .collect::<Vec<_>>(),
        "forced refresh and the unchanged full terminal reader agree"
    );
    native_array(&mut world.sides[0], origin + stats::LEADERS, 0, 0);
    daily_capture(&memory, &world, 17, false);
    leader[8..].copy_from_slice(&31_i32.to_le_bytes());
    native_array(
        &mut world.sides[0],
        origin + stats::LEADERS,
        leader.as_ptr() as usize,
        1,
    );
    let reappeared = daily_capture(&memory, &world, 18, false);
    assert_eq!(observer_row(&reappeared, event::OBSERVER_LEADER, 0).a, 31);
    leader[8..].copy_from_slice(&32_i32.to_le_bytes());
    let slot = 24 + replay_data::cadence::COMBAT_LOG_NUMERIC_SLOT as i32;
    let daily = daily_capture(&memory, &world, slot, false);
    assert_eq!(observer_row(&daily, event::OBSERVER_LEADER, 0).a, 32);
}

#[test]
fn observer_daily_children_follow_current_parent_and_refusal_forces_fresh_recovery() {
    let _globals = super::test_globals();
    use engine_land_inputs::combat_log_events as event;
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let mut world = World::new(0);
    let mut activity = [0; event::ACTIVITY_SIZE];
    World::q(&mut activity, 0, event::ACTIVITY_VTABLE_RVA.0 as u64);
    World::q(&mut activity, event::ACTIVITY_GROUP, 41);
    World::q(
        &mut activity,
        event::ACTIVITY_EQUIPMENT_POOL,
        pool::VTABLE_RVA.0 as u64,
    );
    let mut damage = [0; 16];
    World::q(&mut damage, 0, 61);
    World::q(&mut damage, 8, 101);
    native_array(
        &mut activity,
        event::ACTIVITY_DAMAGE_ENTRIES,
        damage.as_ptr() as usize,
        1,
    );
    let group = [activity.as_ptr() as u64];
    native_array(
        &mut world.sides[0],
        side::STATS_OBSERVER + event::OBSERVER_GROUPS,
        group.as_ptr() as usize,
        1,
    );
    daily_capture(&memory, &world, 12, true);
    World::q(&mut damage, 8, 202);
    let carried = daily_capture(&memory, &world, 13, false);
    assert_eq!(observer_row(&carried, event::ACTIVITY_DAMAGE, 0).a, 101);
    World::q(&mut activity, event::ACTIVITY_GROUP, 42);
    let changed = daily_capture(&memory, &world, 14, false);
    assert_eq!(observer_row(&changed, event::ACTIVITY_DAMAGE, 0).a, 202);
    let mut failed = Captured::with_capacities(&Capacities::default());
    // SAFETY: test holds `test_globals()` as the sole combat-progress caller.
    unsafe {
        failed.prepare_hour(15, false);
        failed.begin();
        failed.finish(Outcome::Unavailable {
            reason: "synthetic census refusal",
        });
    }
    World::q(&mut damage, 8, 303);
    let recovered = daily_capture(&memory, &world, 16, false);
    assert_eq!(observer_row(&recovered, event::ACTIVITY_DAMAGE, 0).a, 303);
    let mut rejected = Captured::with_capacities(&Capacities::default());
    let slot = 24 + replay_data::cadence::COMBAT_LOG_NUMERIC_SLOT as i32;
    for (hour, full, value) in [(slot, false, 404), (slot + 2, true, 505)] {
        World::q(&mut damage, 8, value);
        // SAFETY: test holds `test_globals()` as the sole combat-progress caller.
        unsafe { rejected.prepare_hour(hour, full) };
        let recovered = daily_capture(&memory, &world, hour + 1, false);
        assert_eq!(
            observer_row(&recovered, event::ACTIVITY_DAMAGE, 0).a,
            value as i64
        );
    }
}
