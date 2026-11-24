use super::*;
use crate::census_test_fixture as kit;
use std::cell::{Cell, RefCell};
use std::sync::atomic::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoreEvent {
    Copy(usize, usize),
    Hint(usize, usize),
}

struct CoreHintMemory<'a> {
    bytes: &'a [u8],
    events: RefCell<Vec<CoreEvent>>,
    fault: Cell<Option<usize>>,
    once: bool,
    hints: bool,
}

#[test]
fn core_hints_cover_template_skip_and_preserve_ordered_partial_faults() {
    let mut bytes = kit::fixture();
    let roster = kit::C0 + ccountry::DIVISIONS;
    for offset in [8, 12] {
        bytes[roster + offset..roster + offset + 4].copy_from_slice(&9_u32.to_le_bytes());
    }
    for index in 0..9 {
        let pointer = [kit::A1, kit::A2][index % 2];
        bytes[0x23000 + index * 8..0x23008 + index * 8]
            .copy_from_slice(&(pointer as u64).to_le_bytes());
    }
    for once in [false, true] {
        for fault in [
            None,
            Some(kit::GAME_STATE + cgame_state::PROVINCES + 7),
            Some(kit::LTAB + 4),
            Some(kit::P1 + 7),
            Some(kit::P1 + cprovince::ID + 2),
            Some(kit::PTAB + 8 + 7),
            Some(0x40000 + 7),
            Some(0x40000 + creferenced_division_template::TYPE + 7),
            Some(0x40000 + creferenced_division_template::REVEAL_SKIP),
            Some(0x41000 + 7),
        ] {
            let make_census = || {
                let mut census = Census::new();
                census.capture_inputs(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
                census
            };
            let mut actual = make_census();
            let mut direct = make_census();
            let make_memory = |hints| CoreHintMemory {
                bytes: &bytes,
                events: RefCell::new(Vec::new()),
                fault: Cell::new(fault),
                once,
                hints,
            };
            let hinted = make_memory(true);
            let unhinted = make_memory(false);
            let allocated = actual.allocated_bytes();
            actual.capture_core(&hinted);
            direct.capture_core(&unhinted);
            assert_eq!(actual.outcome(), direct.outcome());
            assert_eq!(actual.rows, direct.rows);
            assert_eq!(actual.sources, direct.sources);
            assert_eq!(actual.allocated_bytes(), allocated);
            assert_eq!(hinted.copies(), unhinted.copies());
            assert_eq!(hinted.fault.get(), unhinted.fault.get());
            if fault.is_none() {
                let events = hinted.events.borrow();
                let skip =
                    CoreEvent::Hint(0x40000 + creferenced_division_template::REVEAL_SKIP, 1);
                assert_eq!(
                    events.iter().filter(|&&event| event == skip).count(),
                    9,
                    "only nonnull templates are hinted"
                );
                let hint = events.iter().position(|&event| event == skip).unwrap();
                let owner = events
                    .iter()
                    .position(|&event| event == CoreEvent::Copy(kit::LTAB + 4, 4))
                    .unwrap();
                assert!(hint < owner, "child hints precede serial projection");
            }
        }
    }
}

impl CoreHintMemory<'_> {
    fn copies(&self) -> Vec<(usize, usize)> {
        self.events
            .borrow()
            .iter()
            .filter_map(|event| match *event {
                CoreEvent::Copy(at, len) => Some((at, len)),
                CoreEvent::Hint(..) => None,
            })
            .collect()
    }
}

impl Memory for CoreHintMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.events
            .borrow_mut()
            .push(CoreEvent::Copy(address, output.len()));
        if self
            .fault
            .get()
            .is_some_and(|at| address <= at && at - address < output.len())
        {
            if self.once {
                self.fault.set(None);
            }
            output.fill(0xff);
            return false;
        }
        kit::memory(self.bytes).copy(address, output)
    }

    fn prefetch(&self, address: usize, len: usize) {
        if self.hints {
            self.events.borrow_mut().push(CoreEvent::Hint(address, len));
        }
    }
}

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
                &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
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
        put(
            &mut bytes,
            ARMY,
            &(carmy::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            ARMY + carmy::SECONDARY_BASE,
            &(carmy::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        bytes
    }
}

#[test]
fn a_roster_past_the_initial_storage_grows_in_one_capture() {
    const COUNTRIES: u32 = 5000;
    const PER: u32 = 4;
    let total = (COUNTRIES * PER) as usize;
    let bytes = many::world(COUNTRIES, PER);
    let memory = kit::memory(&bytes);
    let mut census = Census::new();
    let roster_capacity = census.unit_inputs().unwrap().roster_slots.capacity();
    assert!(total > roster_capacity && total > census.entries.capacity());
    census.capture_inputs(&memory, 0, many::GAME_STATE);
    assert!(!census.overflowed);
    assert_eq!((census.required, census.required_slots), (total, total));
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
fn a_one_shot_fault_on_a_batch_first_roster_pointer_is_read_once_more() {
    let bytes = many::world(2, 2);
    let first = many::roster(1, 2);
    let memory = CoreHintMemory {
        bytes: &bytes,
        events: RefCell::new(Vec::new()),
        fault: Cell::new(Some(first)),
        once: true,
        hints: false,
    };
    let mut census = Census::new();
    census.capture_inputs(&memory, 0, many::GAME_STATE);
    assert_eq!(
        census.unit_inputs().unwrap().outcome(),
        Outcome::Rows { len: 4 }
    );
    let attempts = memory
        .copies()
        .iter()
        .filter(|&&(at, len)| at <= first && first < at + len)
        .count();
    assert_eq!(attempts, 2);
}

#[test]
fn entries_past_the_initial_storage_grow_to_the_exact_count_in_one_capture() {
    const COUNTRIES: u32 = 4500;
    const PER: u32 = 2;
    let total = (COUNTRIES * PER) as usize;
    let bytes = many::world(COUNTRIES, PER);
    let memory = kit::memory(&bytes);
    let mut census = Census::new();
    census.capture_inputs(&memory, 0, many::GAME_STATE);
    assert!(!census.overflowed);
    assert_eq!((census.required, census.required_slots), (total, total));
    assert_eq!(
        census.unit_inputs().unwrap().outcome(),
        Outcome::Rows { len: total }
    );
}

#[test]
fn census_reuses_the_callback_country_roster_and_matches_the_native_checks() {
    use engine_native_intelligence::country_memory::{Cache, SeedStatus};
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

    let mut owner = engine_allocation::shared(Cache::try_with_capacity(1 << 17).unwrap()).unwrap();
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
    bytes[many::CTAB + 16..many::CTAB + 24].fill(0);
    let mut census = Census::new();
    census.capture_inputs(&kit::memory(&bytes), 0, many::GAME_STATE);
    assert_eq!(
        census.unit_inputs().unwrap().outcome(),
        Outcome::Rows { len: 3 }
    );
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
    staged.capture_inputs(&memory, kit::BASE, kit::GAME_STATE);
    let allocated = staged.allocated_bytes();
    assert_eq!(
        staged.outcome(),
        Outcome::Unavailable {
            reason: "division core not captured"
        }
    );
    let lease = engine_allocation::SharedBox::clone(staged.unit_inputs().unwrap());
    assert_eq!(lease.outcome(), Outcome::Rows { len: 3 });
    let boundary = memory.reads.borrow().len();
    assert!(!memory.reads.borrow().iter().any(|&(address, _)| {
        address == kit::GAME_STATE + cgame_state::PROVINCES || address == 0x40000
    }));

    staged.capture_core(&memory);
    assert_eq!(staged.outcome(), Outcome::Rows { len: 3 });
    assert!(engine_allocation::SharedBox::ptr_eq(&lease, staged.unit_inputs().unwrap()));
    assert_eq!(staged.allocated_bytes(), allocated);
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

    let mut combined = Census::new();
    combined.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
    assert_eq!(staged.rows(), combined.rows());
    assert_eq!(staged.sources(), combined.sources());
    let completed_reads = memory.reads.borrow().len();
    staged.capture_core(&memory);
    assert_eq!(memory.reads.borrow().len(), completed_reads);
    drop(lease);
    staged.clear();
}

#[test]
fn core_failure_is_not_retried_and_does_not_change_leased_intel_inputs() {
    let mut bytes = kit::fixture();
    bytes[0x23008..0x23010].fill(0);
    let mut census = Census::new();
    census.capture_inputs(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
    let lease = engine_allocation::SharedBox::clone(census.unit_inputs().unwrap());
    assert_eq!(lease.outcome(), Outcome::Rows { len: 2 });
    assert_eq!(lease.intel_sources().outcome, Outcome::Rows { len: 3 });
    assert_eq!(lease.intel_sources().entries[1].pointer, 0);
    let memory = Observed {
        bytes: &bytes,
        reads: RefCell::new(Vec::new()),
        fault: Some(kit::GAME_STATE + cgame_state::PROVINCES),
        fault_at: None,
    };
    census.capture_core(&memory);
    assert_eq!(
        census.outcome(),
        Outcome::Unavailable {
            reason: "province array unreadable or invalid"
        }
    );
    assert_eq!(memory.reads.borrow().len(), 1);
    census.capture_core(&memory);
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
fn intel_pointer_outcome_survives_empty_private_seed_storage() {
    let bytes = kit::fixture();
    let mut census = Census::new();
    engine_allocation::SharedBox::get_mut(census.unit_inputs.as_mut().unwrap())
        .unwrap()
        .seeds = Vec::new();
    census.capture_inputs(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
    let lease = engine_allocation::SharedBox::clone(census.unit_inputs().unwrap());
    assert_eq!(lease.outcome(), Outcome::Rows { len: 3 });
    assert_eq!(lease.intel_sources().outcome, Outcome::Rows { len: 3 });
    assert_eq!(lease.intel_sources().entries.len(), 3);
    census.capture_core(&kit::memory(&bytes));
}

#[test]
fn shared_seeds_keep_the_complete_census_and_private_faults() {
    let bytes = kit::fixture();
    let mut census = Census::new();
    census.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
    assert_eq!(census.rows().1, Outcome::Rows { len: 3 });
    let inputs = engine_allocation::SharedBox::clone(census.unit_inputs().unwrap());
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
    census.capture(&Fault(&bytes), kit::BASE, kit::GAME_STATE);
    assert_eq!(
        census.rows().1,
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
    let roster = kit::C0 + ccountry::DIVISIONS;
    for offset in [8, 12] {
        bytes[roster + offset..roster + offset + 4].copy_from_slice(&9u32.to_le_bytes());
    }
    for index in 0..9 {
        let pointer = [kit::A1, kit::A2][index % 2];
        bytes[0x23000 + index * 8..0x23008 + index * 8]
            .copy_from_slice(&(pointer as u64).to_le_bytes());
    }
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
        let expected = (|| {
            let provinces = Array::read(&serial, kit::GAME_STATE + cgame_state::PROVINCES, 8)
                .ok_or("province array unreadable or invalid")?;
            for (index, seed) in seeds.iter().enumerate() {
                let mut theatres = ctheatre::UnitTheatres::default();
                let row = project(
                    &serial,
                    &roots.countries,
                    census.entries[index],
                    seed,
                    &mut CoreIdentities::new(roots.base, provinces),
                    &mut TheatreLookup::new(&mut theatres),
                )?;
                rows.push(row);
            }
            Ok::<_, &'static str>(())
        })();

        let observed = make_memory();
        census.capture_core(&observed);
        let actual_reads = observed.reads.borrow();
        if expected.is_ok() {
            assert_eq!(actual_reads.len(), 1 + seeds.len() + 7);
        }
        assert_eq!(census.rows, rows);
        assert_eq!(census.sources.len(), rows.len());
        assert_eq!(census.unit_inputs().unwrap().seeds(), seeds);
        assert_eq!(census.allocated_bytes(), allocated);
        let mut installed = Vec::<carmy::Row>::new();
        assert_eq!(
            census.install_rows(&mut installed),
            match expected {
                Ok(()) => Outcome::Rows { len: seeds.len() },
                Err(reason) => Outcome::Unavailable { reason },
            }
        );
        if expected.is_ok() {
            assert_eq!(installed[..seeds.len()], rows[..]);
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
    let group_handle =
        (u32::MAX as u64) << 32 | engine_native::corders_group::REFERENCE_TYPE as u64;
    for (address, value) in [
        (colliding[0], cprovince::VTABLE_RVA.0 as u64),
        (kit::PTAB + 16, colliding[0] as u64),
        (
            colliding[1],
            creferenced_division_template::VTABLE_RVA.0 as u64,
        ),
        (colliding[1] + creferenced_division_template::TYPE, u64::MAX),
        (
            colliding[2],
            engine_native_parts::carmy_group::VTABLE_RVA.0 as u64,
        ),
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
        (57_u64 << 32) | engine_native::corders_group::REFERENCE_TYPE as u64,
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

    let mut census = Census::new();
    census.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
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
    census.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
    assert_eq!(census.outcome(), Outcome::Rows { len: 3 });
    assert_eq!(census.rows[0].province, expected_collision[0] as u32);
    assert_eq!(census.rows[0].template, expected_collision[1]);
    assert_eq!(census.rows[0].orders_group, expected_collision[2]);
}

#[test]
fn parts_joined_in_slot_order_equal_one_walk() {
    use many::*;
    const COUNTRIES: u32 = 6;
    const PER: u32 = 2;
    let army = |slot: u32, index: u32| ARMY + ((slot - 1) * PER + index) as usize * 0x800;
    let mut bytes = world(COUNTRIES, PER);
    let base = &bytes[ARMY..ARMY + 0x100].to_vec();
    for slot in 1..=COUNTRIES {
        for index in 0..PER {
            let at = army(slot, index);
            bytes[at..at + 0x100].copy_from_slice(base);
            let handle = ((slot * PER + index) as u64) << 32 | 1;
            bytes[at + engine_native::cunit::HANDLE..][..8].copy_from_slice(&handle.to_le_bytes());
            let entry = roster(slot, PER) + index as usize * 8;
            bytes[entry..entry + 8].copy_from_slice(&(at as u64).to_le_bytes());
        }
    }
    let faults = [army(1, 1), army(4, 0), army(6, 1)].map(|at| at + carmy::STRENGTH);
    struct Faults<'a>(&'a [u8], Vec<usize>);
    impl Memory for Faults<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self
                .1
                .iter()
                .any(|&fault| address <= fault && address.saturating_add(output.len()) > fault)
            {
                return false;
            }
            kit::memory(self.0).copy(address, output)
        }
    }
    let walk = |memory: &Faults| {
        let mut serial = Census::new_worker();
        serial.capture_inputs(memory, 0, GAME_STATE);
        let mut parted = Census::new_worker();
        parted.capture_inputs_part(memory, 0, GAME_STATE, (1, 3));
        parted.reserve_join(serial.join_capacity());
        for slots in [(3, 5), (5, u32::MAX)] {
            let mut part = Census::new_worker();
            part.capture_inputs_part(memory, 0, GAME_STATE, slots);
            if parted.joinable() {
                parted.absorb(&mut part).unwrap();
            }
        }
        for census in [&mut serial, &mut parted] {
            census.capture_core(memory);
        }
        (serial, parted)
    };
    let observe = |census: &Census| {
        let inputs = census.unit_inputs().unwrap();
        let mut counts = vec![replay_data::rows::DivisionCensusRow::default(); 16];
        let mut rows = vec![carmy::Row::default(); 16];
        let rows_outcome = census.install_rows(&mut rows);
        (
            (
                inputs.seeds().to_vec(),
                inputs.outcome(),
                inputs.intel_sources().entries.to_vec(),
                inputs.intel_sources().outcome,
            ),
            (census.census_result, census.row_error, census.overflowed),
            (census.install_counts(&mut counts), counts),
            (rows_outcome, rows, census.sources.clone()),
            (
                census.required,
                census.required_slots,
                census.core_roots.map(|roots| roots.found),
            ),
        )
    };
    let (serial, parted) = walk(&Faults(&bytes, faults.to_vec()));
    assert_eq!(serial.census_result, Outcome::Rows { len: 12 });
    assert_eq!(
        serial
            .unit_inputs()
            .unwrap()
            .seeds()
            .iter()
            .filter(|seed| seed.strength.is_none())
            .count(),
        3
    );
    assert_eq!(observe(&parted), observe(&serial));
    let capacity = parted.entries.capacity();
    assert!(capacity >= 12);
    let mut starts = [0; MAX_PARTS - 1];
    assert_eq!(parts_for(12), 1);
    assert_eq!(serial.part_starts(&mut starts), 0);

    let mut failing = faults.to_vec();
    failing.push(country(4) + ccountry::DIVISIONS);
    let (serial, parted) = walk(&Faults(&bytes, failing));
    assert!(matches!(serial.census_result, Outcome::Unavailable { .. }));
    let refusal = |census: &Census| {
        let inputs = census.unit_inputs().unwrap();
        let mut rows = vec![carmy::Row::default(); 16];
        (
            census.census_result,
            census.row_error,
            inputs.outcome(),
            inputs.intel_sources().outcome,
            census.install_rows(&mut rows),
            census.core_roots.is_some(),
            census.joinable(),
        )
    };
    assert_eq!(refusal(&parted), refusal(&serial));
    assert!(!parted.joinable());
}

#[test]
fn part_starts_split_entries_evenly_at_country_boundaries() {
    let mut census = Census::new_worker();
    let entries = 6_000;
    census.entries = vec![Entry::default(); entries];
    census.slot_ends = (1..=60).map(|slot| (slot, slot * 100)).collect();
    let mut starts = [0; MAX_PARTS - 1];
    assert_eq!(parts_for(entries), 6);
    assert_eq!(census.part_starts(&mut starts), 5);
    assert_eq!(starts, [11, 21, 31, 41, 51]);
}

#[test]
fn input_stage_gathers_the_mover_packet_and_each_group_once() {
    use engine_native::{corders_group, cunit};
    let mut bytes = kit::fixture();
    let route = kit::A1 + cunit::ORDINARY_ROUTE_DATA;
    bytes[route..route + 8].copy_from_slice(&0x43000_u64.to_le_bytes());
    for offset in [8, 12] {
        bytes[route + offset..route + offset + 4].copy_from_slice(&2_u32.to_le_bytes());
    }
    bytes[0x43000..0x43008].copy_from_slice(&(1_u64 | (1_u64 << 32)).to_le_bytes());
    bytes[0x41000 + corders_group::TRAINING] = 1;
    let memory = Observed {
        bytes: &bytes,
        reads: RefCell::new(Vec::new()),
        fault: None,
        fault_at: None,
    };
    let mut census = Census::new();
    census.capture_inputs(&memory, kit::BASE, kit::GAME_STATE);
    let seeds = census.unit_inputs().unwrap().seeds();
    assert_eq!(seeds.len(), 3);
    assert_eq!(seeds[0].route_head, Some(Some(1)));
    assert_eq!(seeds[0].location_id, Some(Some(1)));
    assert_eq!((seeds[1].route_head, seeds[2].location_id), (None, None));
    for index in [0, 2] {
        let selected = seeds[index].selected.unwrap();
        assert_eq!(selected.leader, Some(Ok(0)));
        assert_eq!(selected.training, Some(Ok(1)));
        assert_eq!(selected.block, Ok(None));
    }
    assert_eq!(seeds[1].selected.unwrap().leader, None);
    assert!(seeds[0].selected.unwrap().group_identity);
    let reads = memory.reads.borrow();
    for address in [
        0x41000,
        0x41000 + corders_group::LEADER,
        0x41000 + corders_group::TRAINING,
        0x43000,
        kit::P1 + cprovince::ID,
    ] {
        assert_eq!(
            reads.iter().filter(|&&(at, _)| at == address).count(),
            1,
            "{address:#x}"
        );
    }
}

#[test]
fn refused_selection_storage_never_ends_the_recording() {
    use engine_native::{
        corders_group as group,
        corders_group_member as member,
        cunit,
        cunit_leader,
    };
    use engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA;
    const CHILD: &str = "BATTLEPLAN_DIVISION_SELECTION_ALLOCATION_TEST";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "division_census::tests::refused_selection_storage_never_ends_the_recording",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    const LEADER: usize = 0x1000;
    const GROUPS: [(usize, usize, usize); 2] = [(0x2000, 0x3000, 0x4000), (0x5000, 0x6000, 0x7000)];
    const ARMIES: [(usize, usize); 4] = [
        (0x8000, 0x2000),
        (0x9000, 0x5000),
        (0xa000, 0x2000),
        (0xb000, 0),
    ];
    struct Commanded(Vec<u8>);
    impl Memory for Commanded {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == TRADE_FEATURE_FLAGS_RVA.0 && output.len() == 4 {
                output.copy_from_slice(&(1_u32 << 17).to_le_bytes());
                return true;
            }
            kit::memory(&self.0).copy(address, output)
        }
    }
    struct Certified;
    impl collector_observers::unit_leader_changes::Changes for Certified {
        fn quiet(&self) -> Option<(u64, u64)> {
            Some((1, 1))
        }
        fn sequence(&self, _: usize) -> Option<(u64, u64)> {
            Some((1, 1))
        }
    }
    impl collector_observers::orders_group_changes::Changes for Certified {
        fn sequence(&self, _: usize) -> Option<(u64, u64)> {
            Some((1, 1))
        }
    }
    let mut bytes = vec![0_u8; 0x10000];
    let mut put = |at: usize, value: u64| bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    let handle = (7 << 32) | u64::from(cunit_leader::REFERENCE_TYPE);
    put(LEADER, carmy_leader::VTABLE_RVA.0 as u64);
    put(LEADER + cunit_leader::TYPE, handle);
    for (own, parent, host) in GROUPS {
        for at in [own, parent] {
            put(at, group::VTABLE_RVA.0 as u64);
            put(at + group::HANDLE, u64::from(group::REFERENCE_TYPE));
        }
        put(own + group::LEADER, LEADER as u64);
        put(own + group::PARENT, parent as u64);
        put(
            own + group::HQ_PROVIDER,
            (host + member::HOST_OFFSET) as u64,
        );
        put(host, carmy::VTABLE_RVA.0 as u64);
        put(
            host + carmy::SECONDARY_BASE,
            carmy::SECONDARY_VTABLE_RVA.0 as u64,
        );
        put(host + cunit::HANDLE, 1);
        put(host + member::HOST_OFFSET + member::GROUP, own as u64);
    }
    for (army, own) in ARMIES {
        put(army + member::HOST_OFFSET + member::GROUP, own as u64);
    }
    let memory = Commanded(bytes);
    let template: Vec<_> = ARMIES
        .iter()
        .map(|&(pointer, own)| carmy::DivisionSeed {
            pointer,
            orders_group: Some(own),
            class: Some(0),
            ..Default::default()
        })
        .collect();
    let hour = |(leaders, groups): &mut (HeldLeaders, HeldGroups), ordinal: usize| {
        let mut seeds = template.clone();
        let denied = collector_test_support::NullGuard::new(ordinal);
        select(
            &memory,
            kit::BASE,
            &mut seeds,
            leaders,
            groups,
            &Certified,
            &Certified,
        );
        drop(denied);
        let attempts = collector_test_support::ATTEMPTS.load(Ordering::Relaxed);
        let denials = collector_test_support::DENIALS.load(Ordering::Relaxed);
        (seeds, attempts, denials)
    };
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    let mut warm = <(HeldLeaders, HeldGroups)>::default();
    let (first, cold_attempts, _) = hour(&mut warm, usize::MAX);
    let (carried, warm_attempts, _) = hour(&mut warm, usize::MAX);
    assert_eq!(carried, first);
    assert_eq!((warm.0.held.len(), warm.1.held.len()), (1, 2));
    assert_eq!(first[0].selected.unwrap().handle, Some(handle));
    assert_eq!(
        first[0].selected.unwrap().block,
        Ok(Some((
            LEADER + carmy_leader::MOVEMENT_MODIFIER + carmy_leader::MODIFIER_ACCUMULATOR,
            false
        )))
    );
    for (seed, selected) in template.iter().zip(&first) {
        assert_eq!(
            selected.selected.map(|selected| selected.block),
            Some(carmy_leader::movement_block_memoized(
                &memory,
                kit::BASE,
                seed,
                &mut carmy_leader::LeaderMemo::vacant()
            ))
        );
    }
    for (cold, attempts) in [(true, cold_attempts), (false, warm_attempts)] {
        assert!(attempts > 0);
        for ordinal in 1..=attempts {
            let mut held = <(HeldLeaders, HeldGroups)>::default();
            if !cold {
                hour(&mut held, usize::MAX);
            }
            let (seeds, _, denials) = hour(&mut held, ordinal);
            assert!(denials > 0, "cold {cold} ordinal {ordinal}");
            assert!(
                !collector_memory::terminal::failed(),
                "cold {cold} ordinal {ordinal}"
            );
            assert!(
                seeds == first || seeds == template,
                "cold {cold} ordinal {ordinal}"
            );
            for (leader, carry) in &held.0.held {
                assert_eq!(warm.0.held.get(leader), Some(carry));
            }
            for (group, carry) in &held.1.held {
                assert_eq!(warm.1.held.get(group), Some(carry));
            }
            let (next, _, _) = hour(&mut held, usize::MAX);
            assert_eq!(next, first, "cold {cold} ordinal {ordinal}");
        }
    }
    assert!(!engine_allocation::capture_stopped());
    engine_allocation::set_hooks(None, None);
    engine_allocation::set_stop_hook(None);
}

#[test]
fn held_groups_are_read_again_only_after_a_writer_names_them() {
    use engine_mutations::orders_group_mutations::{DAILY_UPDATE, MUTATIONS};
    use engine_native::{corders_group as group, cunit_leader};
    const LEADER: usize = 0x1000;
    const GROUPS: [usize; 3] = [0x2000, 0x3000, 0x4000];
    const NEW: usize = 0x5000;
    struct Live {
        bytes: RefCell<Vec<u8>>,
        reads: RefCell<Vec<usize>>,
    }
    impl Memory for Live {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(address);
            kit::memory(&self.bytes.borrow()).copy(address, output)
        }
    }
    struct Unwatched;
    impl collector_observers::unit_leader_changes::Changes for Unwatched {
        fn quiet(&self) -> Option<(u64, u64)> {
            None
        }
        fn sequence(&self, _: usize) -> Option<(u64, u64)> {
            None
        }
    }
    let mut bytes = vec![0_u8; 0x10000];
    let mut put = |at: usize, value: u64| bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    put(LEADER, carmy_leader::VTABLE_RVA.0 as u64);
    put(
        LEADER + cunit_leader::TYPE,
        (7 << 32) | u64::from(cunit_leader::REFERENCE_TYPE),
    );
    for at in GROUPS.into_iter().chain([NEW]) {
        put(at, group::VTABLE_RVA.0 as u64);
        put(at + group::HANDLE, u64::from(group::REFERENCE_TYPE));
        put(at + group::LEADER, LEADER as u64);
    }
    let memory = Live {
        bytes: RefCell::new(bytes),
        reads: RefCell::new(Vec::new()),
    };
    let watches = collector_observers::orders_group_changes::Watches::new();
    watches.enable(16);
    let mut leaders = HeldLeaders::default();
    let mut held = HeldGroups::default();
    let mut hour = |groups: &[usize]| {
        memory.reads.borrow_mut().clear();
        let mut seeds: Vec<_> = groups
            .iter()
            .enumerate()
            .map(|(index, &group)| carmy::DivisionSeed {
                pointer: 0x8000 + index * 0x100,
                orders_group: Some(group),
                class: Some(1),
                ..Default::default()
            })
            .collect();
        select(
            &memory,
            kit::BASE,
            &mut seeds,
            &mut leaders,
            &mut held,
            &Unwatched,
            &watches,
        );
        let reads = memory.reads.borrow();
        let read: Vec<_> = GROUPS
            .into_iter()
            .chain([NEW])
            .filter(|&at| {
                reads
                    .iter()
                    .any(|&read| (at..at + group::SIZE).contains(&read))
            })
            .collect();
        let training: Vec<_> = seeds
            .iter()
            .map(|seed| seed.selected.unwrap().training)
            .collect();
        for seed in &seeds {
            let selected = seed.selected.unwrap();
            assert_eq!(selected.leader, Some(Ok(LEADER)));
            assert!(selected.group_identity);
        }
        (read, training)
    };
    assert_eq!(hour(&GROUPS).0, GROUPS);
    memory.bytes.borrow_mut()[GROUPS[0] + group::TRAINING] = 1;
    assert_eq!(hour(&GROUPS), (vec![], vec![Some(Ok(0)); 3]));
    for site in (0..MUTATIONS.len()).filter(|&site| site != DAILY_UPDATE) {
        drop(watches.notify(GROUPS[0], site));
        let (read, training) = hour(&GROUPS);
        assert_eq!(read, [GROUPS[0]], "{}", MUTATIONS[site].0.name);
        assert_eq!(training[0], Some(Ok(1)));
        assert!(hour(&GROUPS).0.is_empty(), "{}", MUTATIONS[site].0.name);
    }
    drop(watches.daily(GROUPS[1], false));
    assert!(hour(&GROUPS).0.is_empty());
    drop(watches.daily(GROUPS[1], true));
    assert_eq!(hour(&GROUPS).0, [GROUPS[1]]);
    drop(watches.destroy(GROUPS[2]));
    assert_eq!(hour(&GROUPS).0, [GROUPS[2]]);
    assert_eq!(hour(&[GROUPS[0], NEW]).0, [NEW]);
    assert_eq!(hour(&GROUPS).0, [GROUPS[1], GROUPS[2]]);
    watches.boundary(true);
    assert_eq!(hour(&GROUPS).0, GROUPS);
    watches.disable();
    assert_eq!(hour(&GROUPS).0, GROUPS);
    assert_eq!(hour(&GROUPS).0, GROUPS);
}
