use super::*;
use collector_observers::wing_changes::{Counts, Watches};
use engine_mutations::{
    air_wing_equipment_mutations::MUTATIONS, air_wing_equipment_mutations::Receiver,
    air_wing_equipment_mutations::pools,
};
use engine_native::cequipment_variant::{self, pool as cequipment_variant_pool};
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

const REGION: usize = 0x800;
const WINGS: usize = 3;

fn wing(index: usize) -> usize {
    index * REGION
}

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

fn set_amount(bytes: &mut [u8], index: usize, ordinal: usize, amount: i64) {
    put(
        bytes,
        wing(index) + 0x400 + ordinal * equipment_entry::STRIDE + equipment_entry::AMOUNT,
        &amount.to_le_bytes(),
    );
}

fn world() -> Vec<u8> {
    let mut bytes = vec![0; WINGS * REGION];
    for index in 0..WINGS {
        let pool = wing(index) + cair_wing::EQUIPMENT_POOL;
        let (entries, variant) = (wing(index) + 0x400, wing(index) + 0x480);
        put(
            &mut bytes,
            pool,
            &(cequipment_variant_pool::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pool + cequipment_variant_pool::ITEMS,
            &(entries as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pool + cequipment_variant_pool::CAPACITY,
            &2_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            pool + cequipment_variant_pool::COUNT,
            &2_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            variant,
            &(engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            variant + cequipment_variant::TYPE,
            &((index as u64 + 1) << 32 | u64::from(cequipment_variant::REFERENCE_TYPE))
                .to_le_bytes(),
        );
        for ordinal in 0..2 {
            put(
                &mut bytes,
                entries + ordinal * equipment_entry::STRIDE,
                &(variant as u64).to_le_bytes(),
            );
            set_amount(&mut bytes, index, ordinal, 100_000 * (ordinal as i64 + 1));
        }
    }
    bytes
}

struct Calls<'a> {
    bytes: &'a [u8],
    calls: RefCell<Vec<usize>>,
}
impl Memory for Calls<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.borrow_mut().push(address);
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, output)
    }
}

struct Hour {
    rows: Vec<rows::WingEquipmentRow>,
    counts: Counts,
    reads: usize,
}

fn walk(bytes: &[u8], store: &mut WingStore, watches: &Watches, ids: [u64; WINGS]) -> Hour {
    let (rows, complete, reads) = settle(bytes, store, watches, ids);
    store.conclude(complete);
    collector_observers::wing_changes::record(&store.counts(), complete.then_some(WINGS));
    Hour {
        rows,
        counts: store.counts(),
        reads,
    }
}

fn settle(
    bytes: &[u8],
    store: &mut WingStore,
    watches: &Watches,
    ids: [u64; WINGS],
) -> (Vec<rows::WingEquipmentRow>, bool, usize) {
    settle_selected(
        bytes,
        store,
        watches,
        &ids.into_iter().enumerate().collect::<Vec<_>>(),
    )
}

fn settle_selected(
    bytes: &[u8],
    store: &mut WingStore,
    watches: &Watches,
    selected: &[(usize, u64)],
) -> (Vec<rows::WingEquipmentRow>, bool, usize) {
    let memory = Calls {
        bytes,
        calls: RefCell::default(),
    };
    let mut reuse = WingReuse::begin(store, watches);
    let mut rows = Vec::new();
    let mut complete = true;
    for &(index, id) in selected {
        let row = cair_wing::Row {
            id,
            ..Default::default()
        };
        let source = WingSource {
            pointer: wing(index),
            row: &row,
        };
        let start = rows.len();
        let (outcome, _) = read_equipment_carried(
            &memory,
            0,
            &source,
            engine_native_memory::walk::Buffer::Growing(&mut rows),
            start,
            Some(&mut reuse),
        );
        complete &= matches!(outcome, Outcome::Rows { .. });
        rows.truncate(start + outcome.len());
    }
    reuse.settle();
    (rows, complete, memory.calls.into_inner().len())
}

const IDS: [u64; WINGS] = [7, 8, 9];

fn first_hour(bytes: &[u8], store: &mut WingStore, watches: &Watches) -> Hour {
    let first = walk(bytes, store, watches, IDS);
    assert_eq!(first.rows.len(), 6);
    assert_eq!(
        (
            first.counts.read,
            first.counts.reused,
            first.counts.read_rows
        ),
        (3, 0, 6)
    );
    assert_ne!(first.reads, 0);
    watches.boundary(false);
    first
}

#[test]
fn a_settled_store_counts_its_held_rows_before_the_census_concludes() {
    let bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    assert_eq!(store.allocated_bytes(), 0);
    settle(&bytes, &mut store, &watches, IDS);
    let settled = store.allocated_bytes();
    assert_ne!(settled, 0);
    store.forget();
    assert_eq!(store.allocated_bytes(), settled);
}

#[test]
fn quiet_hours_read_no_pool_and_publish_identical_rows() {
    let bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    let first = first_hour(&bytes, &mut store, &watches);
    for _ in 0..3 {
        let quiet = walk(&bytes, &mut store, &watches, IDS);
        assert_eq!(quiet.rows, first.rows);
        assert_eq!(
            (
                quiet.counts.read,
                quiet.counts.reused,
                quiet.counts.reused_rows
            ),
            (0, 3, 6)
        );
        assert_eq!(quiet.reads, 0);
        watches.boundary(false);
    }
}

#[test]
fn a_hooked_write_rereads_only_its_wing() {
    let mut bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    let first = first_hour(&bytes, &mut store, &watches);
    let notification = watches.kill(wing(1));
    set_amount(&mut bytes, 1, 0, 5);
    drop(notification);
    watches.boundary(false);
    let hour = walk(&bytes, &mut store, &watches, IDS);
    assert_eq!((hour.counts.read, hour.counts.reused), (1, 2));
    let mut expected = first.rows.clone();
    expected[2].amount_raw = 5;
    assert_eq!(hour.rows, expected);
    watches.boundary(false);
    let quiet = walk(&bytes, &mut store, &watches, IDS);
    assert_eq!((quiet.counts.read, quiet.counts.reused), (0, 3));
    assert_eq!(quiet.rows, expected);
}

#[test]
fn a_split_notifies_both_wings() {
    let mut bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    first_hour(&bytes, &mut store, &watches);
    let site = MUTATIONS
        .iter()
        .position(|(site, _)| site.name == "move_airplanes")
        .unwrap();
    let [source, destination] = pools(Receiver::Wings, wing(0), wing(2));
    let notifications = (
        watches.notify(source, site),
        watches.notify(destination, site),
    );
    set_amount(&mut bytes, 0, 1, 1);
    set_amount(&mut bytes, 2, 1, 3);
    drop(notifications);
    watches.boundary(false);
    let hour = walk(&bytes, &mut store, &watches, IDS);
    assert_eq!((hour.counts.read, hour.counts.reused), (2, 1));
    assert_eq!((hour.rows[1].amount_raw, hour.rows[5].amount_raw), (1, 3));
}

#[test]
fn an_invalidating_hour_or_a_new_wing_id_rereads() {
    let bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    let first = first_hour(&bytes, &mut store, &watches);
    watches.boundary(true);
    let hour = walk(&bytes, &mut store, &watches, IDS);
    assert_eq!((hour.counts.read, hour.counts.reused), (3, 0));
    assert_eq!(hour.rows, first.rows);
    watches.boundary(false);
    let renamed = walk(&bytes, &mut store, &watches, [7, 18, 9]);
    assert_eq!((renamed.counts.read, renamed.counts.reused), (1, 2));
    assert!(renamed.rows[2..4].iter().all(|row| row.wing == 18));
    watches.boundary(false);
    store.forget();
    let forgotten = walk(&bytes, &mut store, &watches, IDS);
    assert_eq!((forgotten.counts.read, forgotten.counts.reused), (3, 0));
}

#[test]
fn an_unobserved_write_is_carried_stale() {
    let mut bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    let first = first_hour(&bytes, &mut store, &watches);
    set_amount(&mut bytes, 0, 0, 9);
    let stale = walk(&bytes, &mut store, &watches, IDS);
    assert_eq!(stale.counts.reused, 3);
    assert_eq!(stale.rows, first.rows);
}

#[test]
fn a_failed_pass_holds_nothing() {
    let mut bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    first_hour(&bytes, &mut store, &watches);
    let [pool, _] = pools(Receiver::Wing, wing(1), 0);
    drop(watches.notify(pool, 0));
    let mut broken = bytes.clone();
    put(&mut broken, pool, &0_u64.to_le_bytes());
    watches.boundary(false);
    let failed = walk(&broken, &mut store, &watches, IDS);
    assert_eq!(failed.counts.reused, 2);
    set_amount(&mut bytes, 0, 0, 5);
    watches.boundary(false);
    let hour = walk(&bytes, &mut store, &watches, IDS);
    assert_eq!((hour.counts.read, hour.counts.reused), (3, 0));
    assert_eq!(hour.rows[0].amount_raw, 5);
}

#[test]
fn the_registered_pool_is_the_wings_equipment_pool() {
    assert_eq!(
        engine_mutations::air_wing_equipment_mutations::POOL,
        cair_wing::EQUIPMENT_POOL
    );
}

#[test]
fn every_wing_writer_but_kill_airplanes_is_a_wing_site() {
    assert!(
        MUTATIONS
            .iter()
            .all(|(site, _)| site.rva != engine_events::equipment_destruction::AIR.rva)
    );
    let [pool, _] = pools(Receiver::Wing, wing(1), 0);
    let watches = Watches::enabled(16);
    let certified = collector_observers::wing_changes::Changes::sequence(&watches, pool).unwrap();
    drop(watches.kill(wing(1)));
    assert_ne!(
        collector_observers::wing_changes::Changes::sequence(&watches, pool),
        Some(certified)
    );
}

#[test]
fn without_the_kill_airplanes_detour_nothing_is_carried() {
    let mut bytes = world();
    let watches = Watches::with_kills(16, false);
    let mut store = WingStore::new();
    for _ in 0..3 {
        let hour = walk(&bytes, &mut store, &watches, IDS);
        assert_eq!((hour.counts.read, hour.counts.reused), (3, 0));
        watches.boundary(false);
    }
    set_amount(&mut bytes, 0, 0, 9);
    let hour = walk(&bytes, &mut store, &watches, IDS);
    assert_eq!((hour.counts.read, hour.counts.reused), (3, 0));
    assert_eq!(hour.rows[0].amount_raw, 9);
    watches.boundary(false);
    watches.observe_kills(true);
    let first = walk(&bytes, &mut store, &watches, IDS);
    assert_eq!((first.counts.read, first.counts.reused), (3, 0));
    watches.boundary(false);
    let quiet = walk(&bytes, &mut store, &watches, IDS);
    assert_eq!((quiet.counts.read, quiet.counts.reused), (0, 3));
    watches.observe_kills(false);
    watches.boundary(false);
    let after = walk(&bytes, &mut store, &watches, IDS);
    assert_eq!((after.counts.read, after.counts.reused), (3, 0));
}

#[test]
fn quiet_wings_keep_the_whole_owned_row_buffer() {
    let bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    let first = first_hour(&bytes, &mut store, &watches);
    let storage = store.held_rows().as_ptr();
    for _ in 0..3 {
        let hour = walk(&bytes, &mut store, &watches, IDS);
        assert_eq!(hour.rows, first.rows);
        assert_eq!(store.held_rows().as_ptr(), storage);
        assert_eq!(store.held_rows().len(), first.rows.len());
    }
}

#[test]
fn changed_new_removed_reordered_and_refused_wings_match_fresh_rows() {
    let mut bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    let mut capture = |bytes: &[u8], selected: &[(usize, u64)], expected_reads| {
        let (actual, complete, _) = settle_selected(bytes, &mut store, &watches, selected);
        assert_eq!(store.counts().read, expected_reads);
        let (fresh, fresh_complete, _) =
            settle_selected(bytes, &mut WingStore::new(), &watches, selected);
        assert_eq!((actual, complete), (fresh, fresh_complete));
        store.conclude(complete);
    };
    capture(&bytes, &[(0, 7), (1, 8)], 2);
    capture(&bytes, &[(0, 7), (1, 8)], 0);
    capture(&bytes, &[(2, 9), (0, 7)], 1);
    drop(watches.notify(wing(0) + cair_wing::EQUIPMENT_POOL, 0));
    set_amount(&mut bytes, 0, 1, -123);
    capture(&bytes, &[(0, 7), (2, 9)], 1);
    put(
        &mut bytes,
        wing(0) + cair_wing::EQUIPMENT_POOL + cequipment_variant_pool::COUNT,
        &1_i32.to_le_bytes(),
    );
    drop(watches.notify(wing(0) + cair_wing::EQUIPMENT_POOL, 0));
    capture(&bytes, &[(2, 9), (0, 7)], 1);
    put(
        &mut bytes,
        wing(2) + cair_wing::EQUIPMENT_POOL,
        &0_u64.to_le_bytes(),
    );
    drop(watches.notify(wing(2) + cair_wing::EQUIPMENT_POOL, 0));
    capture(&bytes, &[(2, 9), (0, 7)], 1);
    capture(&bytes, &[(0, 7)], 1);
    capture(&bytes, &[], 0);
    capture(&bytes, &[(0, 7)], 1);
}

#[test]
fn quiet_wing_cache_needs_no_allocation_and_duplicate_wings_never_certify_a_read() {
    let bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    first_hour(&bytes, &mut store, &watches);
    walk(&bytes, &mut store, &watches, IDS);
    let storage = store.held_rows().as_ptr();
    collector_test_support::with_allocations_denied(|| {
        let mut reuse = WingReuse::begin(&mut store, &watches);
        for (index, id) in IDS.into_iter().enumerate() {
            let plan = reuse.plan(wing(index), id);
            assert_eq!(plan.carry_rows(), Some(2));
            assert_eq!(reuse.carry(&plan).len(), 2);
        }
        reuse.settle();
    });
    store.conclude(true);
    assert_eq!(store.held_rows().as_ptr(), storage);
    let (_, complete, _) =
        settle_selected(&bytes, &mut store, &watches, &[(0, 7), (0, 7), (1, 8)]);
    store.conclude(complete);
    let (_, complete, _) = settle_selected(&bytes, &mut store, &watches, &[(0, 7), (1, 8)]);
    assert!(complete);
    assert_eq!((store.counts().read, store.counts().reused), (1, 1));
}
