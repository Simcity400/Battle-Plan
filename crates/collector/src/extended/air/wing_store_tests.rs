//! WingEquipment rows carried between hours per air wing
//! ([`super::WingStore`], [`crate::wing_changes`]): quiet hours read no pool,
//! entry or variant and publish the same rows; a hooked write re-reads only
//! its wing; an invalidation re-reads every wing; verify catches a write no
//! hook saw and poisons reuse.
use super::*;
use crate::{
    sampled_verify::{Check, ROTATION},
    wing_changes::{Counts, Watches},
};
use engine::{
    air_wing_equipment_mutations::{MUTATIONS, Receiver, pools},
    cequipment_variant, cequipment_variant_pool,
    memory::SliceMemory,
};
use std::cell::RefCell;

/// Each wing's objects lie in its own 0x800-byte region: the wing at +0,
/// its entries at +0x400 and its variant at +0x480.
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

/// Each wing: two entries of its own variant (id index + 1).
fn world() -> Vec<u8> {
    let mut bytes = vec![0; WINGS * REGION];
    for index in 0..WINGS {
        let pool = wing(index) + cair_wing::EQUIPMENT_POOL;
        let (entries, variant) = (wing(index) + 0x400, wing(index) + 0x480);
        put(
            &mut bytes,
            pool,
            &(cequipment_variant_pool::VTABLE_RVA as u64).to_le_bytes(),
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
            &(cequipment_variant::VTABLE_RVA as u64).to_le_bytes(),
        );
        // The reference type word, then the variant id.
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

/// Every guarded call's first address.
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
    /// Guarded reads of any pool, entry or variant.
    reads: usize,
}

/// One pass over every wing, as the air pass makes it; `ids` are the
/// wings' AirWings row ids.
fn walk(
    bytes: &[u8],
    store: &mut WingStore,
    watches: &Watches,
    check: Check,
    ids: [u64; WINGS],
) -> Hour {
    let (rows, complete, reads) = settle(bytes, store, watches, check, ids);
    store.conclude(complete);
    crate::wing_changes::record(&store.counts(), complete.then_some(WINGS));
    Hour {
        rows,
        counts: store.counts(),
        reads,
    }
}

/// [`walk`]'s pass up to its settlement, before the census concludes:
/// the rows, whether every wing was complete and the guarded reads.
fn settle(
    bytes: &[u8],
    store: &mut WingStore,
    watches: &Watches,
    check: Check,
    ids: [u64; WINGS],
) -> (Vec<rows::WingEquipmentRow>, bool, usize) {
    let memory = Calls {
        bytes,
        calls: RefCell::default(),
    };
    let mut reuse = WingReuse::begin(store, watches, check);
    let mut rows = Vec::new();
    let mut complete = true;
    for (index, id) in ids.into_iter().enumerate() {
        let row = cair_wing::Row {
            id,
            ..Default::default()
        };
        let source = WingSource {
            pointer: wing(index),
            pool: 0,
            row: &row,
        };
        let start = rows.len();
        let (outcome, _) = read_equipment_carried(
            &memory,
            0,
            &source,
            engine::walk::Buffer::Growing(&mut rows),
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

/// The first hour reads and holds every wing; this is what the old
/// behaviour does every hour.
fn first_hour(bytes: &[u8], store: &mut WingStore, watches: &Watches) -> Hour {
    let first = walk(bytes, store, watches, Check::Nothing, IDS);
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

/// A part's store counts the rows its pass holds once it settles, so a
/// census that fails before concluding still reports them.
#[test]
fn a_settled_store_counts_its_held_rows_before_the_census_concludes() {
    let bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    assert_eq!(store.allocated_bytes(), 0);
    settle(&bytes, &mut store, &watches, Check::Nothing, IDS);
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
        let quiet = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
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
    // Aircraft loss writer on the second wing: the equipment-destruction family's
    // detour notifies its pool.
    let notification = watches.kill(wing(1));
    set_amount(&mut bytes, 1, 0, 5);
    drop(notification);
    watches.boundary(false);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
    assert_eq!((hour.counts.read, hour.counts.reused), (1, 2));
    let mut expected = first.rows.clone();
    expected[2].amount_raw = 5;
    assert_eq!(hour.rows, expected);
    // The re-read wing is carried again from the next quiet hour.
    watches.boundary(false);
    let quiet = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
    assert_eq!((quiet.counts.read, quiet.counts.reused), (0, 3));
    assert_eq!(quiet.rows, expected);
}

#[test]
fn a_split_notifies_both_wings() {
    let mut bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    first_hour(&bytes, &mut store, &watches);
    // A plane transfer moves aircraft from the first wing to the third.
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
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
    assert_eq!((hour.counts.read, hour.counts.reused), (2, 1));
    assert_eq!((hour.rows[1].amount_raw, hour.rows[5].amount_raw), (1, 3));
}

#[test]
fn an_invalidating_hour_or_a_new_wing_id_rereads() {
    let bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    let first = first_hour(&bytes, &mut store, &watches);
    // Load, gap or recovery.
    watches.boundary(true);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
    assert_eq!((hour.counts.read, hour.counts.reused), (3, 0));
    assert_eq!(hour.rows, first.rows);
    // A wing with another id at the same address is read afresh, and its
    // rows carry the current id.
    watches.boundary(false);
    let renamed = walk(&bytes, &mut store, &watches, Check::Nothing, [7, 18, 9]);
    assert_eq!((renamed.counts.read, renamed.counts.reused), (1, 2));
    assert!(renamed.rows[2..4].iter().all(|row| row.wing == 18));
    // A forgotten store (a failed job) reads every wing.
    watches.boundary(false);
    store.forget();
    let forgotten = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
    assert_eq!((forgotten.counts.read, forgotten.counts.reused), (3, 0));
}

#[test]
fn verify_catches_an_unobserved_write_and_poisons_reuse() {
    let mut bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    first_hour(&bytes, &mut store, &watches);
    let agreed = walk(&bytes, &mut store, &watches, Check::Every, IDS);
    assert_eq!(
        (
            agreed.counts.read,
            agreed.counts.verified,
            agreed.counts.mismatches
        ),
        (3, 3, 0)
    );
    assert!(!watches.poisoned());
    watches.boundary(false);
    // A write no hook observed.
    set_amount(&mut bytes, 0, 0, 9);
    let caught = walk(&bytes, &mut store, &watches, Check::Every, IDS);
    // The first wing's mismatch poisons at once: the wings after it are no
    // longer carryable, so they are read but not verified.
    assert_eq!(
        (
            caught.counts.read,
            caught.counts.verified,
            caught.counts.mismatches
        ),
        (3, 1, 1)
    );
    assert!(watches.poisoned());
    assert_eq!(caught.rows[0].amount_raw, 9);
    watches.boundary(false);
    let after = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
    assert_eq!((after.counts.read, after.counts.reused), (3, 0));
}

#[test]
fn an_unobserved_write_without_verify_is_carried_stale() {
    // Guards the premise of the family: only hooked writers move a
    // certificate, so a missed writer would be carried until verify or an
    // invalidation, which is why every writer site is hooked.
    let mut bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    let first = first_hour(&bytes, &mut store, &watches);
    set_amount(&mut bytes, 0, 0, 9);
    let stale = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
    assert_eq!(stale.counts.reused, 3);
    assert_eq!(stale.rows, first.rows);
}

#[test]
fn a_failed_pass_holds_nothing() {
    let mut bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    first_hour(&bytes, &mut store, &watches);
    // The second wing is notified and its pool no longer authenticates.
    let [pool, _] = pools(Receiver::Wing, wing(1), 0);
    drop(watches.notify(pool, 0));
    let mut broken = bytes.clone();
    put(&mut broken, pool, &0_u64.to_le_bytes());
    watches.boundary(false);
    let failed = walk(&broken, &mut store, &watches, Check::Nothing, IDS);
    assert_eq!(failed.counts.reused, 2);
    // The incomplete pass certifies nothing afterwards.
    set_amount(&mut bytes, 0, 0, 5);
    watches.boundary(false);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
    assert_eq!((hour.counts.read, hour.counts.reused), (3, 0));
    assert_eq!(hour.rows[0].amount_raw, 5);
}

#[test]
fn an_hourly_slice_verifies_only_its_wings_and_carries_the_rest() {
    let bytes = world();
    let watches = Watches::enabled(16);
    let mut store = WingStore::new();
    first_hour(&bytes, &mut store, &watches);
    let mut verified = 0;
    for hour in 0..ROTATION as i32 {
        let check = Check::Slice { hour };
        let sampled = walk(&bytes, &mut store, &watches, check, IDS);
        let due = IDS.iter().filter(|&&id| check.covers(id)).count() as u64;
        assert_eq!(
            (sampled.counts.verified, sampled.counts.reused),
            (due, 3 - due)
        );
        assert_eq!(sampled.counts.mismatches, 0);
        verified += due;
        watches.boundary(false);
    }
    assert_eq!(verified, 3);
}

#[test]
fn the_registered_pool_is_the_wings_equipment_pool() {
    assert_eq!(
        engine::air_wing_equipment_mutations::POOL,
        cair_wing::EQUIPMENT_POOL
    );
}

#[test]
fn every_wing_writer_but_kill_airplanes_is_a_wing_site() {
    // Aircraft loss writer has one patch, the equipment-destruction
    // family's, which notifies this family instead.
    assert!(
        MUTATIONS
            .iter()
            .all(|(site, _)| site.rva != engine::equipment_destruction::AIR.rva)
    );
    let [pool, _] = pools(Receiver::Wing, wing(1), 0);
    let watches = Watches::enabled(16);
    let certified = crate::wing_changes::Changes::sequence(&watches, pool).unwrap();
    drop(watches.kill(wing(1)));
    assert_ne!(
        crate::wing_changes::Changes::sequence(&watches, pool),
        Some(certified)
    );
}

#[test]
fn without_the_kill_airplanes_detour_nothing_is_carried() {
    let mut bytes = world();
    // The equipment-destruction family is not installed or not enabled:
    // aircraft loss writer writes go unobserved.
    let watches = Watches::with_kills(16, false);
    let mut store = WingStore::new();
    for _ in 0..3 {
        let hour = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
        assert_eq!((hour.counts.read, hour.counts.reused), (3, 0));
        watches.boundary(false);
    }
    // An unobserved loss is read, never carried stale.
    set_amount(&mut bytes, 0, 0, 9);
    let hour = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
    assert_eq!((hour.counts.read, hour.counts.reused), (3, 0));
    assert_eq!(hour.rows[0].amount_raw, 9);
    // Once the detour is live the domain starts a new epoch: the first hour
    // reads afresh, the next carries.
    watches.boundary(false);
    watches.observe_kills(true);
    let first = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
    assert_eq!((first.counts.read, first.counts.reused), (3, 0));
    watches.boundary(false);
    let quiet = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
    assert_eq!((quiet.counts.read, quiet.counts.reused), (0, 3));
    // Disabling the detour stops certification at once.
    watches.observe_kills(false);
    watches.boundary(false);
    let after = walk(&bytes, &mut store, &watches, Check::Nothing, IDS);
    assert_eq!((after.counts.read, after.counts.reused), (3, 0));
}
