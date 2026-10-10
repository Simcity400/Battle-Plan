use super::*;
use crate::sampling::divisions_test_kit as kit;
use std::cell::RefCell;

struct Observed<'a> {
    bytes: &'a [u8],
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
    fault_at: Option<usize>,
}
impl Memory for Observed<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ordinal = self.reads.borrow().len();
        self.reads.borrow_mut().push((address, output.len()));
        if self.fault_at == Some(ordinal)
            || self.fault.is_some_and(|fault| {
                address <= fault && address.saturating_add(output.len()) > fault
            })
        {
            return false;
        }
        kit::memory(self.bytes).copy(address, output)
    }
}

/// Synthetic world of `countries` canonical countries, each with `per`
/// division slots pointing at one shared army object. Slot 0 is the
/// reserved sentinel; links are identity.
mod many {
    use super::*;
    pub(super) const GAME_STATE: usize = 0x1000;
    pub(super) const CTAB: usize = 0x10000;
    pub(super) const LTAB: usize = 0x30000;
    pub(super) const COUNTRY0: usize = 0x40000;
    pub(super) const COUNTRY_STRIDE: usize = 0x300;
    pub(super) const ROSTERS: usize = 0x400000;
    pub(super) const ARMY: usize = 0x430000;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }
    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        put(bytes, at, &(data as u64).to_le_bytes());
        put(bytes, at + 8, &count.to_le_bytes());
        put(bytes, at + 12, &count.to_le_bytes());
    }
    pub(super) fn country(slot: u32) -> usize {
        COUNTRY0 + (slot as usize - 1) * COUNTRY_STRIDE
    }
    pub(super) fn roster(slot: u32, per: u32) -> usize {
        ROSTERS + (slot as usize - 1) * per as usize * 8
    }
    pub(super) fn world(countries: u32, per: u32) -> Vec<u8> {
        assert!(roster(countries + 1, per) <= ARMY);
        let mut bytes = vec![0; 0x480000];
        header(
            &mut bytes,
            GAME_STATE + cgame_state::COUNTRIES,
            CTAB,
            countries + 1,
        );
        header(
            &mut bytes,
            GAME_STATE + cgame_state::COUNTRY_LINKS,
            LTAB,
            countries + 1,
        );
        for slot in 1..=countries {
            let object = country(slot);
            put(
                &mut bytes,
                CTAB + slot as usize * 8,
                &(object as u64).to_le_bytes(),
            );
            put(&mut bytes, LTAB + slot as usize * 4, &slot.to_le_bytes());
            put(
                &mut bytes,
                object,
                &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
            );
            put(&mut bytes, object + ccountry::ID, &slot.to_le_bytes());
            header(
                &mut bytes,
                object + ccountry::DIVISIONS,
                roster(slot, per),
                per,
            );
            for index in 0..per as usize {
                put(
                    &mut bytes,
                    roster(slot, per) + index * 8,
                    &(ARMY as u64).to_le_bytes(),
                );
            }
        }
        put(&mut bytes, ARMY, &(carmy::VTABLE_RVA as u64).to_le_bytes());
        put(
            &mut bytes,
            ARMY + carmy::SECONDARY_BASE,
            &(carmy::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
        );
        bytes
    }
}

#[test]
fn roster_overflow_counts_every_country_and_grows_in_one_step() {
    const COUNTRIES: u32 = 5000;
    const PER: u32 = 4;
    let total = (COUNTRIES * PER) as usize;
    let bytes = many::world(COUNTRIES, PER);
    let memory = Observed {
        bytes: &bytes,
        reads: RefCell::new(Vec::new()),
        fault: None,
        fault_at: None,
    };
    let mut census = Census::new();
    let roster_capacity = census.unit_inputs().unwrap().roster_slots.capacity();
    assert!(total > roster_capacity && total > census.entries.capacity());
    census.capture_inputs(&memory, 0, many::GAME_STATE);
    assert!(census.overflowed);
    assert_eq!(
        census.unit_inputs().unwrap().intel_sources().outcome,
        Outcome::Unavailable {
            reason: "division owned roster capacity exhausted"
        }
    );
    // Every roster count was read, including those after the miss...
    assert_eq!(census.required_slots, total);
    assert_eq!(census.required, total);
    // ...but no pointer page after the one that exhausted the storage.
    let last = many::roster(COUNTRIES, PER);
    assert!(
        memory
            .reads
            .borrow()
            .iter()
            .any(|&(address, _)| { address == many::country(COUNTRIES) + ccountry::DIVISIONS })
    );
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(address, _)| (last..last + PER as usize * 8).contains(&address))
    );

    census.grow();
    census.clear();
    census.capture_inputs(&memory, 0, many::GAME_STATE);
    assert!(!census.overflowed);
    let inputs = census.unit_inputs().unwrap();
    assert_eq!(inputs.outcome(), Outcome::Rows { len: total });
    assert_eq!(inputs.seeds().len(), total);
    for (index, seed) in inputs.seeds().iter().enumerate() {
        assert_eq!(seed.country_ordinal, Some(index as u32 % PER));
    }
    let intel = inputs.intel_sources();
    assert_eq!(intel.outcome, Outcome::Rows { len: total });
    for (index, entry) in intel.entries.iter().enumerate() {
        assert_eq!(entry.pointer, many::ARMY);
        assert_eq!(entry.canonical, index as u32 / PER + 1);
    }
}

#[test]
fn entry_overflow_alone_also_grows_to_the_exact_count_in_one_step() {
    // 9,000 non-null entries exceed the 8,192 entry storage while the
    // 16,384 roster slots still fit.
    const COUNTRIES: u32 = 4500;
    const PER: u32 = 2;
    let total = (COUNTRIES * PER) as usize;
    let bytes = many::world(COUNTRIES, PER);
    let memory = kit::memory(&bytes);
    let mut census = Census::new();
    census.capture_inputs(&memory, 0, many::GAME_STATE);
    assert!(census.overflowed);
    assert_eq!((census.required, census.required_slots), (total, total));
    census.grow();
    census.clear();
    census.capture_inputs(&memory, 0, many::GAME_STATE);
    assert_eq!(
        census.unit_inputs().unwrap().outcome(),
        Outcome::Rows { len: total }
    );
}

#[test]
fn census_reuses_the_callback_country_roster_and_matches_the_native_checks() {
    use engine::country_memory::{Cache, SeedStatus};
    const COUNTRIES: u32 = 5000;
    let bytes = many::world(COUNTRIES, 1);
    let plain = kit::memory(&bytes);
    let mut direct = Census::new();
    direct.capture_inputs(&plain, 0, many::GAME_STATE);
    let direct_entries = direct
        .unit_inputs()
        .unwrap()
        .intel_sources()
        .entries
        .to_vec();
    assert_eq!(direct_entries.len(), COUNTRIES as usize);

    let mut owner = Arc::new(Cache::try_with_capacity(1 << 17).unwrap());
    let snapshot = Cache::prepare_shared(&mut owner, &plain, 0, many::GAME_STATE).unwrap();
    assert_eq!(snapshot.view(&plain).seed_status(), SeedStatus::Ready);
    let observed = Observed {
        bytes: &bytes,
        reads: RefCell::new(Vec::new()),
        fault: None,
        fault_at: None,
    };
    let view = snapshot.roster_view(&observed);
    let mut reused = Census::new();
    reused.capture_inputs(&view, 0, many::GAME_STATE);
    assert_eq!(
        reused.unit_inputs().unwrap().intel_sources().entries,
        direct_entries
    );
    assert_eq!(
        reused.unit_inputs().unwrap().outcome(),
        direct.unit_inputs().unwrap().outcome()
    );
    // No country-table slot, link or country identity is read again:
    // only each roster header, its pointer page and the division seeds.
    let reads = observed.reads.borrow();
    assert!(!reads.iter().any(|&(address, _)| {
        (many::CTAB..many::LTAB + (COUNTRIES as usize + 1) * 4).contains(&address)
            || (address >= many::COUNTRY0
                && address < many::country(COUNTRIES + 1)
                && (address - many::COUNTRY0) % many::COUNTRY_STRIDE != ccountry::DIVISIONS)
    }));
    assert_eq!(
        reads
            .iter()
            .filter(|&&(address, _)| address >= many::COUNTRY0
                && address < many::country(COUNTRIES + 1))
            .count(),
        COUNTRIES as usize
    );
}

#[test]
fn null_slots_are_skipped_but_a_foreign_object_fails_the_census() {
    let mut bytes = many::world(4, 1);
    // A null slot has no native precedent, but skipping it is harmless.
    bytes[many::CTAB + 16..many::CTAB + 24].fill(0);
    let mut census = Census::new();
    census.capture_inputs(&kit::memory(&bytes), 0, many::GAME_STATE);
    assert_eq!(
        census.unit_inputs().unwrap().outcome(),
        Outcome::Rows { len: 3 }
    );
    // Every native country loop dereferences each slot as a CCountry.
    let foreign = many::country(3);
    bytes[foreign..foreign + 8].copy_from_slice(&1_u64.to_le_bytes());
    census.capture_inputs(&kit::memory(&bytes), 0, many::GAME_STATE);
    assert_eq!(
        census.unit_inputs().unwrap().outcome(),
        Outcome::Unavailable {
            reason: "country vtable mismatch"
        }
    );
}

#[test]
fn input_stage_can_be_leased_before_core_without_repeating_the_census() {
    let bytes = kit::fixture();
    let memory = Observed {
        bytes: &bytes,
        reads: RefCell::new(Vec::new()),
        fault: None,
        fault_at: None,
    };
    let mut staged = Census::new();
    let allocated = staged.allocated_bytes();
    staged.capture_inputs(&memory, kit::BASE, kit::GAME_STATE);
    assert_eq!(
        staged.outcome(0),
        Outcome::Unavailable {
            reason: "division core not captured"
        }
    );
    assert_eq!(
        staged.outcome(64),
        Outcome::Unavailable {
            reason: "division core not captured"
        }
    );
    let lease = Arc::clone(staged.unit_inputs().unwrap());
    assert_eq!(lease.outcome(), Outcome::Rows { len: 3 });
    let boundary = memory.reads.borrow().len();
    assert!(!memory.reads.borrow().iter().any(|&(address, _)| {
        address == kit::GAME_STATE + cgame_state::PROVINCES
            || address == 0x40000
            || address == 0x41000
    }));
    let input_bytes = staged.payload_bytes();
    assert_eq!(
        input_bytes,
        memory
            .reads
            .borrow()
            .iter()
            .map(|&(_, len)| len as u64)
            .sum::<u64>()
    );

    staged.capture_core(&memory, 64);
    assert_eq!(staged.outcome(64), Outcome::Rows { len: 3 });
    assert!(Arc::ptr_eq(&lease, staged.unit_inputs().unwrap()));
    assert_eq!(staged.allocated_bytes(), allocated);
    assert!(staged.payload_bytes() > input_bytes);
    assert!(
        !memory.reads.borrow()[boundary..]
            .iter()
            .any(|&(address, _)| {
                address == kit::GAME_STATE + cgame_state::COUNTRIES
                    || address == kit::GAME_STATE + cgame_state::COUNTRY_LINKS
                    || (0x23000..0x23010).contains(&address)
                    || address == 0x23100
            })
    );
    assert_eq!(
        staged.payload_bytes(),
        memory
            .reads
            .borrow()
            .iter()
            .map(|&(_, len)| len as u64)
            .sum::<u64>()
    );

    let mut combined = Census::new();
    combined.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 64);
    assert_eq!(staged.rows(64), combined.rows(64));
    assert_eq!(staged.sources(64), combined.sources(64));
    assert_eq!(staged.payload_bytes(), combined.payload_bytes());
    let completed_reads = memory.reads.borrow().len();
    staged.capture_core(&memory, 64);
    assert_eq!(memory.reads.borrow().len(), completed_reads);
    drop(lease);
    staged.clear();
}

#[test]
fn core_failure_is_not_retried_and_does_not_change_leased_intel_inputs() {
    let mut bytes = kit::fixture();
    // The full native roster retains counted nulls while typed unit seeds
    // omit them. UnitIntel must still observe the original null ordinal.
    bytes[0x23008..0x23010].fill(0);
    let mut census = Census::new();
    census.capture_inputs(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
    let lease = Arc::clone(census.unit_inputs().unwrap());
    assert_eq!(lease.outcome(), Outcome::Rows { len: 2 });
    assert_eq!(lease.intel_sources().outcome, Outcome::Rows { len: 3 });
    assert_eq!(lease.intel_sources().entries[1].pointer, 0);
    let memory = Observed {
        bytes: &bytes,
        reads: RefCell::new(Vec::new()),
        fault: Some(kit::GAME_STATE + cgame_state::PROVINCES),
        fault_at: None,
    };
    census.capture_core(&memory, 0);
    assert_eq!(
        census.outcome(0),
        Outcome::Unavailable {
            reason: "province array unreadable or invalid"
        }
    );
    assert_eq!(memory.reads.borrow().len(), 1);
    census.capture_core(&memory, 64);
    assert_eq!(memory.reads.borrow().len(), 1);
    assert_eq!(lease.intel_sources().outcome, Outcome::Rows { len: 3 });
    assert_eq!(
        census.intel_sources().entries,
        lease.intel_sources().entries
    );
    drop(lease);
    census.clear();
}

#[test]
fn intel_pointer_outcome_survives_private_seed_capacity_failure() {
    let bytes = kit::fixture();
    let mut census = Census::new();
    Arc::get_mut(census.unit_inputs.as_mut().unwrap())
        .unwrap()
        .seeds = Vec::new();
    census.capture_inputs(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
    let lease = Arc::clone(census.unit_inputs().unwrap());
    assert_eq!(
        lease.outcome(),
        Outcome::Unavailable {
            reason: "division owned scalar capacity exhausted"
        }
    );
    assert_eq!(lease.intel_sources().outcome, Outcome::Rows { len: 3 });
    assert_eq!(lease.intel_sources().entries.len(), 3);
    let bytes_before = census.payload_bytes();
    census.capture_core(&kit::memory(&bytes), 64);
    assert_eq!(census.payload_bytes(), bytes_before);
}

#[test]
fn shared_seeds_keep_complete_census_beyond_core_capacity_and_private_faults() {
    let bytes = kit::fixture();
    let mut census = Census::new();
    census.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 1);
    assert_eq!(
        census.rows(1).1,
        Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }
    );
    let inputs = Arc::clone(census.unit_inputs().unwrap());
    assert_eq!(inputs.outcome(), Outcome::Rows { len: 3 });
    assert_eq!(inputs.seeds().len(), 3);
    drop(inputs);

    struct Fault<'a>(&'a [u8]);
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let fault = kit::A1 + carmy::STRENGTH;
            if address <= fault && address.saturating_add(output.len()) > fault {
                return false;
            }
            kit::memory(self.0).copy(address, output)
        }
    }
    census.capture(&Fault(&bytes), kit::BASE, kit::GAME_STATE, 64);
    assert_eq!(
        census.rows(64).1,
        Outcome::Unavailable {
            reason: "division hit points unreadable"
        }
    );
    let inputs = census.unit_inputs().unwrap();
    assert_eq!(inputs.outcome(), Outcome::Rows { len: 3 });
    assert_eq!(inputs.seeds()[0].strength, None);
    assert!(inputs.seeds()[0].organisation.is_some());
    assert!(inputs.seeds()[2].handle.is_some());
}

#[test]
fn core_identity_reuse_keeps_serial_rows_and_first_source_failures() {
    let mut bytes = kit::fixture();
    // Ten entries cross two four-seed boundaries, retaining repeated child
    // pointers and the fixture's independent null location/template/group.
    let roster = kit::C0 + ccountry::DIVISIONS;
    for offset in [8, 12] {
        bytes[roster + offset..roster + offset + 4].copy_from_slice(&9u32.to_le_bytes());
    }
    for index in 0..9 {
        let pointer = [kit::A1, kit::A2][index % 2];
        bytes[0x23000 + index * 8..0x23008 + index * 8]
            .copy_from_slice(&(pointer as u64).to_le_bytes());
    }
    for capacity in [0, 1, 3, 4, 5, 8, 10] {
        // A failure at any original source must remain the first failure.
        // Successful repeated child reads are deliberately absent, so an
        // injected fault on only a removed occurrence is not a source fault.
        for fault in std::iter::once(None).chain(
            [
                kit::GAME_STATE + cgame_state::PROVINCES,
                kit::LTAB + 4,
                kit::LTAB + 8,
                kit::P1,
                kit::P1 + cprovince::ID,
                kit::PTAB + 8,
                0x40000,
                0x40000 + creferenced_division_template::TYPE,
                0x40000 + creferenced_division_template::REVEAL_SKIP,
                0x41000,
            ]
            .map(Some),
        ) {
            let mut census = Census::new();
            census.capture_inputs(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
            let allocated = census.allocated_bytes();
            let input_bytes = census.payload_bytes();
            let seeds = census.unit_inputs().unwrap().seeds().to_vec();
            assert_eq!(seeds.len(), 10);
            let roots = census.core_roots.unwrap();
            let make_memory = || Observed {
                bytes: &bytes,
                reads: RefCell::new(Vec::new()),
                fault,
                fault_at: None,
            };
            let serial = make_memory();
            let mut rows = Vec::new();
            // A fresh identity set per row forces the original scalar
            // child reads, including repeated addresses and their gates.
            let expected = (|| {
                let provinces = Array::read(&serial, kit::GAME_STATE + cgame_state::PROVINCES, 8)
                    .ok_or("province array unreadable or invalid")?;
                for (index, seed) in seeds.iter().take(capacity).enumerate() {
                    let row = project(
                        &serial,
                        &roots.countries,
                        census.entries[index],
                        seed,
                        &mut CoreIdentities::new(roots.base, provinces),
                    )?;
                    rows.push(row);
                }
                Ok::<_, &'static str>(())
            })();

            let observed = make_memory();
            census.capture_core(&observed, capacity);
            let actual_reads = observed.reads.borrow();
            let serial_reads = serial.reads.borrow();
            let mut remaining = serial_reads.iter();
            for read in actual_reads.iter() {
                assert!(remaining.any(|original| original == read));
            }
            if expected.is_err() {
                assert_eq!(actual_reads.last(), serial_reads.last());
            } else if capacity > 0 {
                // One root, one owner link per row, and exactly one complete
                // authentication of each shared child (3 + 3 + 1 calls).
                assert_eq!(actual_reads.len(), 1 + capacity + 7);
            }
            assert_eq!(census.rows, rows);
            assert_eq!(census.sources.len(), rows.len());
            assert_eq!(census.unit_inputs().unwrap().seeds(), seeds);
            assert_eq!(census.allocated_bytes(), allocated);
            assert_eq!(
                census.payload_bytes() - input_bytes,
                observed
                    .reads
                    .borrow()
                    .iter()
                    .filter(|&&(address, len)| !fault.is_some_and(|fault| {
                        address <= fault && address.saturating_add(len) > fault
                    }))
                    .map(|&(_, len)| len as u64)
                    .sum::<u64>()
            );
            assert_eq!(
                census.outcome(capacity),
                match expected {
                    Ok(()) => complete(capacity, seeds.len() - capacity),
                    Err(reason) => Outcome::Unavailable { reason },
                }
            );
        }
    }
}

#[test]
fn core_identity_collisions_failures_and_next_walk_use_original_sources() {
    fn read_children(
        memory: &impl Memory,
        identities: &mut CoreIdentities,
        pointers: [usize; 3],
    ) -> Result<[u64; 3], &'static str> {
        Ok([
            identities.province(memory, pointers[0])? as u64,
            identities.template(memory, pointers[1])?.0,
            identities.group(memory, pointers[2])?,
        ])
    }
    let mut bytes = kit::fixture();
    let original = [kit::P1, 0x40000, 0x41000];
    let collision = |pointer, range: std::ops::Range<usize>| {
        range
            .step_by(16)
            .find(|&other| {
                other != pointer && CoreIdentities::slot(other) == CoreIdentities::slot(pointer)
            })
            .unwrap()
    };
    let colliding = [
        collision(original[0], 0x60000..0x67000),
        collision(original[1], 0x56000..0x59000),
        collision(original[2], 0x5c000..0x5f000),
    ];
    let group_handle = (u32::MAX as u64) << 32 | engine::corders_group::REFERENCE_TYPE as u64;
    for (address, value) in [
        (colliding[0], cprovince::VTABLE_RVA as u64),
        (kit::PTAB + 16, colliding[0] as u64),
        (
            colliding[1],
            creferenced_division_template::VTABLE_RVA as u64,
        ),
        // The original core accepts any nonzero template handle. Do not
        // strengthen that contract by borrowing a stricter engine helper.
        (colliding[1] + creferenced_division_template::TYPE, u64::MAX),
        (colliding[2], engine::carmy_group::VTABLE_RVA as u64),
        (colliding[2] + 8, group_handle),
    ] {
        bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }
    let province_id = colliding[0] + cprovince::ID;
    bytes[province_id..province_id + 4].copy_from_slice(&2_u32.to_le_bytes());
    for offset in [8, 12] {
        let address = kit::GAME_STATE + cgame_state::PROVINCES + offset;
        bytes[address..address + 4].copy_from_slice(&3_u32.to_le_bytes());
    }
    let provinces = Array {
        data: kit::PTAB,
        count: 3,
    };
    let memory = Observed {
        bytes: &bytes,
        reads: RefCell::new(Vec::new()),
        fault: None,
        fault_at: None,
    };
    let mut identities = CoreIdentities::new(kit::BASE, provinces);
    assert_eq!(read_children(&memory, &mut identities, [0; 3]), Ok([0; 3]));
    assert!(memory.reads.borrow().is_empty());
    let expected_original = [
        1,
        0x3455_0000_0034,
        (57_u64 << 32) | engine::corders_group::REFERENCE_TYPE as u64,
    ];
    let expected_collision = [2, u64::MAX, group_handle];
    for (pointers, expected) in [
        (original, expected_original),
        (colliding, expected_collision),
        (original, expected_original),
    ] {
        assert_eq!(
            read_children(&memory, &mut identities, pointers),
            Ok(expected)
        );
    }
    assert_eq!(memory.reads.borrow().len(), 21);
    assert_eq!(
        read_children(&memory, &mut identities, original),
        Ok(expected_original)
    );
    assert_eq!(memory.reads.borrow().len(), 21);

    for fault in [
        colliding[0],
        province_id,
        kit::PTAB + 16,
        colliding[1],
        colliding[1] + 8,
        colliding[2],
    ] {
        let mut identities = CoreIdentities::new(kit::BASE, provinces);
        assert_eq!(
            read_children(&kit::memory(&bytes), &mut identities, original),
            Ok(expected_original)
        );
        let failing = Observed {
            bytes: &bytes,
            reads: RefCell::new(Vec::new()),
            fault: Some(fault),
            fault_at: None,
        };
        let expected = read_children(
            &failing,
            &mut CoreIdentities::new(kit::BASE, provinces),
            colliding,
        )
        .unwrap_err();
        let expected_reads = failing.reads.take();
        assert_eq!(
            read_children(&failing, &mut identities, colliding),
            Err(expected)
        );
        assert_eq!(*failing.reads.borrow(), expected_reads);
        assert_eq!(
            read_children(&kit::memory(&bytes), &mut identities, colliding),
            Ok(expected_collision)
        );
    }

    // Reusing Census storage begins a new core walk, even when all child
    // addresses survive. It must observe the new identities and backlink.
    let mut census = Census::new();
    census.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 3);
    assert_eq!(census.rows[0].province, 1);
    assert_eq!(census.rows[0].template, expected_original[1]);
    assert_eq!(census.rows[0].orders_group, expected_original[2]);
    for (address, value) in [
        (kit::PTAB + 16, kit::P1 as u64),
        (original[1] + 8, u64::MAX),
        (original[2] + 8, group_handle),
    ] {
        bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[kit::P1 + cprovince::ID..kit::P1 + cprovince::ID + 4]
        .copy_from_slice(&2_u32.to_le_bytes());
    census.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 3);
    assert_eq!(census.outcome(3), Outcome::Rows { len: 3 });
    assert_eq!(census.rows[0].province, expected_collision[0] as u32);
    assert_eq!(census.rows[0].template, expected_collision[1]);
    assert_eq!(census.rows[0].orders_group, expected_collision[2]);
}

#[test]
fn worker_first_inputs_and_later_mod_rosters_fit_original_headers_for_5000_countries() {
    for countries in [1500, 5000] {
        let mut census = Census::new_worker();
        for per in [1, 4] {
            let bytes = many::world(countries, per);
            let memory = Observed {
                bytes: &bytes,
                reads: RefCell::new(Vec::new()),
                fault: None,
                fault_at: None,
            };
            census.capture_inputs_growing(&memory, 0, many::GAME_STATE);
            let required = (countries * per) as usize;
            assert!(!census.overflowed);
            let inputs = census.unit_inputs().unwrap();
            assert_eq!(inputs.outcome(), Outcome::Rows { len: required });
            assert_eq!(
                inputs.intel_sources().outcome,
                Outcome::Rows { len: required }
            );
            assert_eq!(inputs.seeds().len(), required);
            assert_eq!(census.required_counts, countries as usize + 1);
            assert!(census.counts.len() > countries as usize);
            for slot in 1..=countries {
                assert_eq!(
                    memory
                        .reads
                        .borrow()
                        .iter()
                        .filter(|&&(at, _)| at == many::country(slot) + ccountry::DIVISIONS)
                        .count(),
                    1
                );
            }
        }
    }
}

#[test]
fn growing_inputs_keep_original_fault_precedence_and_read_trace() {
    let bytes = many::world(4, 2);
    let direct = Observed {
        bytes: &bytes,
        reads: RefCell::new(Vec::new()),
        fault: None,
        fault_at: None,
    };
    let mut fixed = Census::new();
    fixed.capture_inputs(&direct, 0, many::GAME_STATE);
    let count = direct.reads.borrow().len();
    for ordinal in 0..count {
        let fixed_memory = Observed {
            bytes: &bytes,
            reads: RefCell::new(Vec::new()),
            fault: None,
            fault_at: Some(ordinal),
        };
        let growing_memory = Observed {
            bytes: &bytes,
            reads: RefCell::new(Vec::new()),
            fault: None,
            fault_at: Some(ordinal),
        };
        let mut fixed = Census::new();
        let mut growing = Census::new();
        growing.counts.clear();
        growing.entries = Vec::new();
        growing.unit_inputs = Some(Arc::new(UnitInputs::new(0)));
        fixed.capture_inputs(&fixed_memory, 0, many::GAME_STATE);
        growing.capture_inputs_growing(&growing_memory, 0, many::GAME_STATE);
        assert_eq!(
            growing.census_result, fixed.census_result,
            "fault {ordinal}"
        );
        assert_eq!(
            growing.unit_inputs().unwrap().outcome(),
            fixed.unit_inputs().unwrap().outcome(),
            "fault {ordinal}"
        );
        assert_eq!(
            *growing_memory.reads.borrow(),
            *fixed_memory.reads.borrow(),
            "fault {ordinal}"
        );
    }
}
