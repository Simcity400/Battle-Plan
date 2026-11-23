use super::*;
use engine_native::{cgame_state, cprovince};
use engine_native_naval::{cnaval_base, cstrategic_navy, cstrategic_navy_manager};
use std::cell::RefCell;

mod launch;

const STATE: usize = 0x1000;
const OWNER: usize = 0x8000;
const ZERO: usize = 0x4000;
const MANAGER: usize = 0xa000;
const NAVY: usize = 0xb000;
const FOREIGN_NAVY: usize = 0xb800;
const NEAREST_BASES: usize = 0x30;
const FOREIGN_NAVY_SLOTS: usize = 0x48;
const SUPPLY_HUBS: usize = engine_native_naval::naval_mission_inputs::HUBS;
const PROVINCES: [usize; 3] = [0xe000, 0xe200, 0xe400];

struct TestMemory {
    bytes: Vec<u8>,
    fault: Option<usize>,
    calls: RefCell<Vec<(usize, usize)>>,
}
impl Memory for TestMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls.borrow_mut().push((address, destination.len()));
        if self.fault.is_some_and(|fault| {
            address <= fault && fault < address.saturating_add(destination.len())
        }) {
            return false;
        }
        let Some(source) = address
            .checked_add(destination.len())
            .and_then(|end| self.bytes.get(address..end))
        else {
            return false;
        };
        destination.copy_from_slice(source);
        true
    }
}
impl TestMemory {
    fn count(&self, address: usize) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == address)
            .count()
    }
    fn touched(&self, start: usize, end: usize) -> bool {
        self.calls
            .borrow()
            .iter()
            .any(|&(at, len)| at < end && start < at.saturating_add(len))
    }
}
fn word(bytes: &mut [u8], address: usize, value: u32) {
    bytes[address..address + 4].copy_from_slice(&value.to_le_bytes());
}
fn pointer(bytes: &mut [u8], address: usize, value: usize) {
    bytes[address..address + 8].copy_from_slice(&(value as u64).to_le_bytes());
}
fn header(bytes: &mut [u8], address: usize, data: usize, count: u32) {
    pointer(bytes, address, data);
    word(bytes, address + 8, count);
    word(bytes, address + 12, count);
}
fn world() -> TestMemory {
    let mut bytes = vec![0; 0x20000];
    header(&mut bytes, STATE + cgame_state::COUNTRIES, 0x3000, 4);
    header(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, 0x3100, 4);
    for (slot, country) in [(0, ZERO), (1, OWNER), (3, OWNER)] {
        pointer(&mut bytes, 0x3000 + slot * 8, country);
    }
    for (slot, canonical) in [(0, 0), (1, 1), (2, 2), (3, 1)] {
        word(&mut bytes, 0x3100 + slot * 4, canonical);
    }
    for (country, id) in [(ZERO, 0), (OWNER, 1)] {
        pointer(&mut bytes, country, ccountry::VTABLE_RVA.0);
        word(&mut bytes, country + ccountry::ID, id);
    }
    pointer(
        &mut bytes,
        STATE + cgame_state::STRATEGIC_NAVY_MANAGER,
        MANAGER,
    );
    pointer(&mut bytes, MANAGER, cstrategic_navy_manager::VTABLE_RVA.0);
    header(
        &mut bytes,
        MANAGER + cstrategic_navy_manager::NAVIES,
        0xa100,
        4,
    );
    pointer(&mut bytes, 0xa108, NAVY);
    pointer(&mut bytes, 0xa118, FOREIGN_NAVY);
    for navy in [NAVY, FOREIGN_NAVY] {
        pointer(&mut bytes, navy, cstrategic_navy::VTABLE_RVA.0);
        pointer(
            &mut bytes,
            navy + cstrategic_navy::SECONDARY,
            cstrategic_navy::SECONDARY_VTABLE_RVA.0,
        );
        pointer(&mut bytes, navy + native::MANAGER, MANAGER);
    }
    header(&mut bytes, NAVY + FOREIGN_NAVY_SLOTS, 0xc000, 3);
    for (ordinal, slot) in [3, 0, 3].into_iter().enumerate() {
        word(&mut bytes, 0xc000 + ordinal * 4, slot);
    }
    header(&mut bytes, NAVY + SUPPLY_HUBS, 0xc100, 3);
    for (ordinal, id) in [3, 0, 3].into_iter().enumerate() {
        word(&mut bytes, 0xc100 + ordinal * 4, id);
    }
    header(&mut bytes, NAVY + NEAREST_BASES, 0xc200, 4);
    header(&mut bytes, FOREIGN_NAVY + NEAREST_BASES, 0xc300, 4);
    pointer(&mut bytes, 0xc208, 0xd000);
    pointer(&mut bytes, 0xc218, 0xd100);
    pointer(&mut bytes, 0xc300, 0xd100);
    for (base, id) in [(0xd000, 0), (0xd100, 3)] {
        pointer(&mut bytes, base, cnaval_base::VTABLE_RVA.0);
        word(&mut bytes, base + cnaval_base::PROVINCE_ID, id);
    }
    header(&mut bytes, STATE + cgame_state::PROVINCES, 0x3200, 4);
    for (id, province, area) in [
        (1, PROVINCES[0], 0x19000),
        (2, PROVINCES[1], 0),
        (3, PROVINCES[2], 0x19100),
    ] {
        pointer(&mut bytes, 0x3200 + id * 8, province);
        pointer(&mut bytes, province, cprovince::VTABLE_RVA.0);
        word(&mut bytes, province + cprovince::ID, id as u32);
        pointer(&mut bytes, province + cprovince::CONTROLLER_AREA, area);
        word(&mut bytes, province + cprovince::CONTROLLER, 1);
    }
    TestMemory {
        bytes,
        fault: None,
        calls: RefCell::new(Vec::new()),
    }
}
fn capacity() -> (Capacities, LaunchCapacities) {
    (
        Capacities { navies: 4 },
        LaunchCapacities { launch_bases: 0 },
    )
}
fn pass<M: Memory>(captured: &mut Captured, launch: &mut Launch, memory: &M, hour: Option<i32>) {
    let mut reader = captured.begin(memory, 0, STATE, hour, launch);
    captured.finish();
    reader.clear();
}

#[test]
fn physical_controls_borrow_without_reading_access_mode_headers() {
    let mut memory = world();
    header(
        &mut memory.bytes,
        NAVY + cstrategic_navy::ACCESS_MODES,
        0x18000,
        5000,
    );
    header(
        &mut memory.bytes,
        FOREIGN_NAVY + cstrategic_navy::ACCESS_MODES,
        0x14000,
        7,
    );
    let (mut capacity, launch_capacity) = capacity();
    capacity.navies = 1;
    let mut captured = Captured::new(capacity);
    let mut launch = Launch::new(launch_capacity);
    let mut reader = captured.begin(&memory, 0, STATE, Some(7), &mut launch);
    assert_eq!(captured.physical_count(), Ok(4));
    assert!(captured.borrow_navy(1).is_ok());
    for navy in [NAVY, FOREIGN_NAVY] {
        let access = navy + cstrategic_navy::ACCESS_MODES;
        assert!(!memory.touched(access, access + 16), "{navy:#x}");
    }
    assert_eq!(memory.count(0x18000), 0);
    assert_eq!(memory.count(0x14000), 0);
    captured.finish();
    reader.clear();
    captured.reset();
}

#[test]
fn physical_pass_reads_no_retired_navy_headers_controllers_or_reserved_root() {
    let memory = world();
    let (capacity, launch_capacity) = capacity();
    let mut captured = Captured::new(capacity);
    let mut launch = Launch::new(launch_capacity);
    pass(&mut captured, &mut launch, &memory, Some(71));
    assert_eq!(captured.physical_count(), Ok(4));
    assert_eq!(memory.count(NAVY), 1);
    assert_eq!(memory.count(FOREIGN_NAVY), 1);
    let provinces = STATE + cgame_state::PROVINCES;
    assert!(!memory.touched(provinces, provinces + 16));
    assert!(!memory.touched(0x3200, 0x3200 + 4 * 8));
    for province in PROVINCES {
        assert!(!memory.touched(province, province + 0x200), "{province:#x}");
    }
    for navy in [NAVY, FOREIGN_NAVY] {
        for offset in [NEAREST_BASES, FOREIGN_NAVY_SLOTS, SUPPLY_HUBS] {
            assert!(
                !memory.touched(navy + offset, navy + offset + 16),
                "navy {navy:#x} header {offset:#x} read"
            );
        }
    }
    assert!(!memory.touched(0xc000, 0xc400));
    assert!(!memory.touched(0xd000, 0xd200));
    let countries = STATE + cgame_state::COUNTRIES;
    assert!(!memory.touched(countries, countries + 16));
    assert!(!memory.touched(0x3000, 0x3000 + 4 * 8));
    assert!(!memory.touched(ZERO, ZERO + 0x1000));
    assert!(!memory.touched(OWNER, OWNER + 0x1000));
}

#[test]
fn physical_identity_failure_refuses_only_that_slot_and_is_not_reacquired() {
    let mut memory = world();
    pointer(&mut memory.bytes, FOREIGN_NAVY, 0);
    let (capacity, launch_capacity) = capacity();
    let mut captured = Captured::new(capacity);
    let mut launch = Launch::new(launch_capacity);
    let mut reader = captured.begin(&memory, 0, STATE, Some(71), &mut launch);
    let reason = captured.borrow_navy(3).err().expect("slot 3 identity");
    assert_eq!(reason, "naval movement navy identity or manager mismatch");
    assert!(captured.borrow_navy(1).unwrap().present);
    assert!(captured.borrow_navy(3).is_err());
    captured.finish();
    reader.clear();
    assert_eq!(memory.count(FOREIGN_NAVY), 1);
}

#[test]
fn absent_hour_publishes_nothing_and_reset_retires_every_private_source() {
    let memory = world();
    let (capacity, launch_capacity) = capacity();
    let mut captured = Captured::new(capacity);
    let mut launch = Launch::new(launch_capacity);
    pass(&mut captured, &mut launch, &memory, Some(71));
    assert_eq!(captured.sample_hour, Some(71));
    let mut reader = captured.begin(&memory, 0, STATE, None, &mut launch);
    assert_eq!(captured.sample_hour, None);
    assert!(launch.projection_launch(None).unwrap().is_none());
    captured.reset();
    launch.reset();
    reader.clear();
    assert!(captured.navies.first.iter().all(|row| row.inputs.is_none()));
    assert!(captured.borrow_navy(1).is_err());
    assert!(launch.projection_launch(None).unwrap().is_none());
}

#[test]
fn original_full_physical_holder_is_borrowed_before_movement_and_retired_once() {
    let memory = world();
    let (capacity, launch_capacity) = capacity();
    let mut captured = Captured::new(capacity);
    let mut launch = Launch::new(launch_capacity);
    let mut reader = captured.begin(&memory, 0, STATE, Some(-71), &mut launch);
    assert_eq!(captured.physical_count(), Ok(4));
    let original = captured.borrow_navy(3).unwrap();
    assert!(original.present);
    assert_eq!(original.ledger_navy(), Ok(FOREIGN_NAVY));
    let outer = original.take_ledger_regions().unwrap();
    assert_eq!(outer.count, 0);
    assert!(original.take_ledger_regions().is_err());
    assert!(!captured.borrow_navy(2).unwrap().present);
    captured.finish();
    reader.clear();
    assert!(captured.borrow_navy(3).is_err());
    assert!(captured.navies.first.iter().all(|row| row.inputs.is_none()));
    assert_eq!(memory.count(FOREIGN_NAVY), 1);
}

#[test]
fn unreadable_provinces_never_reach_the_physical_navy_pass() {
    let mut memory = world();
    memory.fault = Some(PROVINCES[0]);
    let (capacity, launch_capacity) = capacity();
    let mut captured = Captured::new(capacity);
    let mut launch = Launch::new(launch_capacity);
    let mut reader = captured.begin(&memory, 0, STATE, Some(71), &mut launch);
    assert_eq!(captured.physical_count(), Ok(4));
    assert_eq!(
        captured.borrow_navy(3).unwrap().ledger_navy(),
        Ok(FOREIGN_NAVY)
    );
    captured.finish();
    reader.clear();
    assert_eq!(memory.count(PROVINCES[0]), 0);
    assert!(captured.navies.first.iter().all(|row| row.inputs.is_none()));
}
