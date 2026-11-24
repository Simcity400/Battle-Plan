use super::*;
use engine_native::cprovince;
use std::cell::{Cell, RefCell};

const STATE: usize = 0x1000;
const COUNTRY_ZERO: usize = 0x4000;
const COUNTRY_ONE: usize = 0x4500;
const MANAGER_ROOT: usize = 0x6000;
const NAVY_ONE: usize = 0x7000;
const NAVY_ALIAS_SLOT: usize = 0x7400;
const ACCESS: usize = 0x8000;

struct TestMemory {
    bytes: Vec<u8>,
    fail_once: Option<usize>,
    failed: Cell<bool>,
    calls: RefCell<Vec<(usize, usize)>>,
}

impl Memory for TestMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls.borrow_mut().push((address, destination.len()));
        if self.fail_once.is_some_and(|fault| {
            address <= fault && fault < address.saturating_add(destination.len())
        }) && !self.failed.replace(true)
        {
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

fn u32_at(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn u64_at(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    u64_at(bytes, at, data as u64);
    u32_at(bytes, at + 8, count);
    u32_at(bytes, at + 12, count);
}

fn list(bytes: &mut [u8], at: usize, values: &[u32]) {
    for (index, &value) in values.iter().enumerate() {
        u32_at(bytes, at + index * 4, value);
    }
}

fn pointers(bytes: &mut [u8], at: usize, values: &[usize]) {
    for (index, &value) in values.iter().enumerate() {
        u64_at(bytes, at + index * 8, value as u64);
    }
}

fn world() -> TestMemory {
    let mut bytes = vec![0; 0x10000];
    header(&mut bytes, STATE + cgame_state::COUNTRIES, 0x3000, 4);
    header(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, 0x3100, 4);
    pointers(
        &mut bytes,
        0x3000,
        &[COUNTRY_ZERO, COUNTRY_ONE, 0x4800, COUNTRY_ONE],
    );
    list(&mut bytes, 0x3100, &[0, 1, 2, 1]);
    for (country, id) in [(COUNTRY_ZERO, 0), (COUNTRY_ONE, 1), (0x4800, 2)] {
        u64_at(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
        u32_at(&mut bytes, country + ccountry::ID, id);
    }
    u64_at(
        &mut bytes,
        STATE + cgame_state::STRATEGIC_NAVY_MANAGER,
        MANAGER_ROOT as u64,
    );
    u64_at(
        &mut bytes,
        MANAGER_ROOT,
        cstrategic_navy_manager::VTABLE_RVA.0 as u64,
    );
    header(
        &mut bytes,
        MANAGER_ROOT + cstrategic_navy_manager::NAVIES,
        0x6100,
        4,
    );
    pointers(&mut bytes, 0x6100, &[0, NAVY_ONE, 0x7200, NAVY_ALIAS_SLOT]);
    for navy in [NAVY_ONE, 0x7200, NAVY_ALIAS_SLOT] {
        u64_at(&mut bytes, navy, cstrategic_navy::VTABLE_RVA.0 as u64);
        u64_at(
            &mut bytes,
            navy + cstrategic_navy::SECONDARY,
            cstrategic_navy::SECONDARY_VTABLE_RVA.0 as u64,
        );
        u64_at(&mut bytes, navy + MANAGER, MANAGER_ROOT as u64);
        u32_at(&mut bytes, navy + cstrategic_navy::OWNER, u32::MAX);
    }
    header(
        &mut bytes,
        NAVY_ONE + cstrategic_navy::ACCESS_MODES,
        ACCESS,
        2,
    );
    header(&mut bytes, STATE + cgame_state::PROVINCES, 0x3200, 5);
    pointers(&mut bytes, 0x3200, &[0, 0xa000, 0xa200, 0xa400, 0xa600]);
    for (id, province, area) in [
        (1, 0xa000, 0xd100),
        (2, 0xa200, 0xd200),
        (3, 0xa400, 0),
        (4, 0xa600, 0xd300),
    ] {
        u64_at(&mut bytes, province, cprovince::VTABLE_RVA.0 as u64);
        u32_at(&mut bytes, province + cprovince::ID, id);
        u64_at(&mut bytes, province + cprovince::CONTROLLER_AREA, area);
    }
    TestMemory {
        bytes,
        fail_once: None,
        failed: Cell::new(false),
        calls: RefCell::new(Vec::new()),
    }
}

fn countries(memory: &TestMemory) -> ccountry::Reader<'_, TestMemory> {
    ccountry::Reader::new(memory, 0, STATE).unwrap()
}

#[test]
fn reserved_launch_null_control_is_observed_once_and_cannot_be_reused() {
    use ccountry::launch_sources::NavalProvider;

    let memory = world();
    let reader = Reader::new(&memory, 0, STATE);
    let mut inputs = reader.observe_navy(0).unwrap();
    memory.calls.borrow_mut().clear();
    let captured = inputs.capture_launch(&memory, 0, &mut Vec::new(), &mut Vec::new());
    assert_eq!(captured.provider, Ok(NavalProvider::NativeNull));
    assert_eq!(captured.required, 0);
    assert_eq!(
        inputs.capture_launch(&memory, 0, &mut Vec::new(), &mut Vec::new()).provider,
        Err("reserved launch navy control consumed")
    );
    assert!(memory.calls.borrow().is_empty());
}

#[test]
fn physical_alias_slot_and_null_slot_zero_are_exact() {
    let memory = world();
    let countries = countries(&memory);
    assert!(countries.at(3).unwrap().is_none());
    let reader = Reader::new(&memory, 0, STATE);
    assert_eq!(reader.bounds(), Bounds { navy_slots: Ok(4) });
    let alias = reader.observe_navy(3).unwrap();
    assert!(alias.present);
    assert_eq!(alias.ledger_navy(), Ok(NAVY_ALIAS_SLOT));
    let own = reader.observe_navy(1).unwrap();
    assert!(own.present);
    assert_eq!(own.ledger_navy(), Ok(NAVY_ONE));
    let zero = reader.observe_navy(0).unwrap();
    assert!(!zero.present);
    assert_eq!(zero.ledger_navy(), Ok(0));
    assert_eq!(
        reader.observe_navy(4).err(),
        Some("naval movement physical navy slot outside manager")
    );
    assert!(!memory.calls.borrow().iter().any(|&(at, len)| {
        [
            NAVY_ONE + cstrategic_navy::OWNER,
            NAVY_ALIAS_SLOT + cstrategic_navy::OWNER,
        ]
        .iter()
        .any(|&field| at <= field && field < at + len)
    }));
}

#[test]
fn failed_header_is_not_retried_and_other_headers_remain() {
    let mut memory = world();
    memory.fail_once = Some(NAVY_ONE + cstrategic_navy::REGION_MISSIONS);
    let reader = Reader::new(&memory, 0, STATE);
    let mut inputs = reader.observe_navy(1).unwrap();
    let before = memory.calls.borrow().len();
    assert!(inputs.take_ledger_regions().is_err());
    assert_eq!(inputs.take_ledger_regions(), Err(CONSUMED));
    assert_eq!(memory.calls.borrow().len(), before);

    for (count, fault_slot) in [(2, 1_u32), (4, 1_u32), (4, 3_u32)] {
        let mut memory = world();
        memory.fail_once = Some(0x6100 + fault_slot as usize * 8);
        let reader = Reader::new(&memory, 0, STATE);
        let mut results = Vec::new();
        reader.observe_navies_range(count, |slot, res| {
            results.push((slot, res.map(|n| n.ledger_navy())));
        });
        assert_eq!(results.len(), count);
        for (slot, res) in results {
            if slot == fault_slot {
                assert_eq!(res, Err("naval movement pointer unreadable"));
            } else {
                assert!(res.is_ok(), "slot {slot} unexpectedly failed: {res:?}");
            }
        }
    }
}

#[test]
fn navy_pass_reads_no_province_roster_or_access_modes() {
    let memory = world();
    let mut reader = Reader::new(&memory, 0, STATE);
    for slot in 0..4 {
        let _ = reader.observe_navy(slot).unwrap();
    }
    reader.clear();
    let touched = |from: usize, to: usize| {
        memory
            .calls
            .borrow()
            .iter()
            .any(|&(at, len)| at < to && from < at + len)
    };
    let provinces = STATE + cgame_state::PROVINCES;
    assert!(!touched(provinces, provinces + 16));
    assert!(!touched(0x3200, 0x3200 + 5 * 8));
    assert!(!touched(0xa000, 0xa800));
    for navy in [NAVY_ONE, 0x7200, NAVY_ALIAS_SLOT] {
        let access = navy + cstrategic_navy::ACCESS_MODES;
        assert!(!touched(access, access + 16), "{navy:#x}");
    }
    assert!(!touched(ACCESS, ACCESS + 2));
}

#[test]
fn navy_manager_failure_and_cleared_pass_refuse_by_name() {
    let mut memory = world();
    u64_at(&mut memory.bytes, MANAGER_ROOT, 0);
    let reader = Reader::new(&memory, 0, STATE);
    assert!(reader.observe_navy(0).is_err());
    assert!(reader.bounds().navy_slots.is_err());
    drop(reader);

    let memory = world();
    let mut reader = Reader::new(&memory, 0, STATE);
    assert!(reader.observe_navy(1).unwrap().present);
    reader.clear();
    assert_eq!(
        reader.observe_navy(1).err(),
        Some("naval movement pass cleared")
    );
}
