use super::*;
use collector_extended_support::bench_world::Heap;
use engine_land_inputs::{
    ncombat_log_cequipment_loss as loss, ncombat_log_corders_group_logs as logs,
};
use engine_native::cequipment_variant::{self, pool};
use std::cell::Cell;

struct World {
    heap: Heap,
    group: usize,
    months: Vec<usize>,
    pools: Vec<usize>,
    variant: usize,
    rows_read: Cell<usize>,
}

impl Memory for World {
    fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
        if self.pools.iter().any(|at| self.heap.origin + at == address) {
            self.rows_read.set(self.rows_read.get() + bytes.len() / 16);
        }
        engine_native_memory::memory::SliceMemory {
            base: self.heap.origin,
            bytes: &self.heap.bytes,
        }
        .copy(address, bytes)
    }
}

impl World {
    fn new() -> Self {
        let mut heap = Heap {
            bytes: game_state(4),
            origin: 0x1000,
        };
        let manager = heap.alloc(native::SIZE);
        heap.vtable(manager, native::VTABLE_RVA);
        heap.u32(manager + native::COUNTRY, 1);
        heap.pointers(engine_native::cgame_state::COMBAT_LOGS, &[0, manager]);
        let group = heap.alloc(logs::SIZE);
        heap.vtable(group, logs::VTABLE_RVA);
        heap.u64(
            group + logs::GROUP,
            (3 << 32) | u64::from(engine_native::corders_group::REFERENCE_TYPE),
        );
        heap.pointers(manager + native::GROUPS, &[group]);
        let variant = heap.alloc(16);
        heap.vtable(
            variant,
            engine_native_parts::equipment_variant_identity::VTABLE_RVA,
        );
        heap.u64(
            variant + 8,
            (5 << 32) | u64::from(cequipment_variant::REFERENCE_TYPE),
        );
        let mut months = Vec::new();
        let mut pools = Vec::new();
        for (i, day_of_year) in [2, 34, 65, 96].into_iter().enumerate() {
            let entry = heap.alloc(loss::SIZE);
            heap.vtable(entry, loss::VTABLE_RVA);
            heap.u32(entry + loss::DATE, day(11, day_of_year) as u32);
            heap.vtable(entry + loss::POOL, pool::VTABLE_RVA);
            let item = heap.alloc(32);
            heap.u64(item, heap.ptr(variant));
            heap.u64(item + 8, 20 + i as u64);
            heap.array(entry + loss::POOL + pool::ITEMS, item, 1);
            months.push(entry);
            pools.push(item);
        }
        heap.pointers(group + logs::REASONS, &months);
        Self {
            heap,
            group,
            months,
            pools,
            variant,
            rows_read: Cell::new(0),
        }
    }

    fn sample(&self, captured: &mut Captured, hour: i32, full: bool) -> TableRows {
        self.rows_read.set(0);
        captured.prepare_hour(hour, full);
        captured.selection.select(1);
        captured.rows.outcome = with_store(|store| {
            captured.read_log(store, self, self.heap.origin, self.heap.origin, hour, full)
        });
        let [TableView::Complete(rows), ..] = captured.ordinary_tables() else {
            panic!("incomplete equipment read: {:?}", captured.rows.outcome);
        };
        rows
    }

    fn set_count(&mut self, index: usize, count: u32) {
        self.heap.array(
            self.months[index] + loss::POOL + pool::ITEMS,
            self.pools[index],
            count,
        );
    }

    fn clock_sample(
        &self,
        captured: &mut Captured,
        signal: i64,
        hour: i32,
        full: bool,
    ) -> Vec<i64> {
        captured.set_signal(signal);
        self.rows_read.set(0);
        // SAFETY: This test owns the fixture and its thread-local capture store.
        unsafe { captured.capture(self, self.heap.origin, self.heap.origin, hour, full) };
        let [TableView::Complete(rows), ..] = captured.ordinary_tables() else {
            panic!("incomplete read at {hour}: {:?}", captured.rows.outcome);
        };
        values(&rows)
    }
}

fn values(rows: &TableRows) -> Vec<i64> {
    let TableRows::CombatLogEquipment(rows) = rows else {
        panic!("wrong table")
    };
    rows.iter().map(|row| row.amount_raw).collect()
}

#[test]
fn a_window_keeps_older_equipment_until_the_game_removes_it() {
    let mut world = World::new();
    let [mut first, mut second, ..] = buffers();
    let hour = day(11, 100);
    let whole = world.sample(&mut first, hour, true);
    assert_eq!(world.rows_read.get(), 4);
    assert_eq!(values(&whole), [20, 21, 22, 23]);
    assert_eq!(world.sample(&mut second, hour + 1, false), whole);
    assert_eq!(world.rows_read.get(), 2);
    world.set_count(2, 0);
    world.heap.u64(world.pools[3] + 8, 91);
    let ordinary = world.sample(&mut second, hour + 2, false);
    assert_eq!(world.rows_read.get(), 1);
    let merge = |old: &TableRows, fresh: TableRows| {
        replay_data::sliced::merge_slice(old, fresh, |owner| owner == 1, |_| true).unwrap()
    };
    let merged = merge(&whole, ordinary);
    assert_eq!(values(&merged), [20, 21, 91]);
    world
        .heap
        .pointers(world.group + logs::REASONS, &world.months[1..]);
    assert_eq!(
        values(&merge(&merged, world.sample(&mut first, hour + 3, true))),
        [21, 91]
    );
    world
        .heap
        .pointers(world.group + logs::REASONS, &world.months[2..]);
    assert_eq!(values(&world.sample(&mut second, hour + 4, false)), [91]);
}

#[test]
fn sealed_month_identity_changes_read_the_pool_and_a_whole_audit_reports_changed_values() {
    let mut world = World::new();
    let mut buffer = Captured::vacant();
    let april = day(11, 100);
    world.sample(&mut buffer, april, true);
    let other = world.heap.alloc(16);
    world.heap.vtable(
        other,
        engine_native_parts::equipment_variant_identity::VTABLE_RVA,
    );
    world.heap.u64(
        other + 8,
        (6 << 32) | u64::from(cequipment_variant::REFERENCE_TYPE),
    );
    world.heap.u64(world.pools[1] + 16, world.heap.ptr(other));
    world.heap.u64(world.pools[1] + 24, 17);
    world.set_count(1, 2);
    assert_eq!(
        values(&world.sample(&mut buffer, april + 1, false)),
        [20, 21, 17, 22, 23]
    );
    assert_eq!(world.rows_read.get(), 4);
    world
        .heap
        .u32(world.months[0] + loss::DATE, day(11, 3) as u32);
    world.heap.u64(world.pools[0] + 8, 18);
    assert_eq!(
        values(&world.sample(&mut buffer, april + 2, false)),
        [18, 21, 17, 22, 23]
    );
    assert_eq!(world.rows_read.get(), 3);
    let relocated = world.heap.alloc(16);
    world.heap.u64(relocated, world.heap.ptr(world.variant));
    world.heap.u64(relocated + 8, 14);
    world.pools[0] = relocated;
    world.set_count(0, 1);
    assert_eq!(values(&world.sample(&mut buffer, april + 3, false))[0], 14);
    assert_eq!(world.rows_read.get(), 3);
    world.set_count(0, 0);
    assert_eq!(
        values(&world.sample(&mut buffer, day(11, 130), false)),
        [21, 17, 22, 23]
    );
    world.heap.u64(world.pools[1] + 8, 9);
    let june = day(11, 160);
    assert_eq!(
        values(&world.sample(&mut buffer, june, false)),
        [9, 17, 22, 23]
    );
    assert_eq!(world.rows_read.get(), 4);
    let diff = last_sealed_month_difference().unwrap();
    assert_eq!(
        (diff.hour, diff.country, diff.group, diff.kind, diff.date),
        (june, 1, 3, 0, day(11, 34))
    );
}

#[test]
fn a_clock_discontinuity_reads_every_pool_on_the_first_selected_hour() {
    let mut world = World::new();
    let [mut first, mut second, ..] = buffers();
    let hour = day(11, 100);
    assert_eq!(
        world.clock_sample(&mut first, SIGNAL, hour, true),
        [20, 21, 22, 23]
    );
    assert_eq!(
        world.clock_sample(&mut second, SIGNAL, hour + 1, false),
        [20, 21, 22, 23]
    );
    assert_eq!(world.rows_read.get(), 2);
    for (i, (signal, at)) in [
        (SIGNAL, hour + 25),
        (SIGNAL, hour + 1),
        (SIGNAL + 1, hour + 25),
    ]
    .into_iter()
    .enumerate()
    {
        world.heap.u64(world.pools[0] + 8, 40 + i as u64);
        assert_eq!(
            world.clock_sample(&mut first, signal, at, false)[0],
            40 + i as i64
        );
        assert_eq!(world.rows_read.get(), 4);
        assert!(last_sealed_month_difference().is_none());
    }
}
