use super::*;
use engine_native_parts::creference_registry as registry;
use std::{cell::RefCell, collections::BTreeMap};

const POOL: usize = 0x10000;
const DEFINITION: usize = 0x20000;
const GLOBAL: usize = 0x21000;
const DATABASE: usize = 0x30000;
const ENTRIES: usize = 0x31000;
const WING: usize = 0x40000;
const OTHER_WING: usize = 0x42000;
const RAID: usize = 0x50000;
const HANDLE: u64 = (7 << 32) | crate::craid_instance::REFERENCE_TYPE_VALUE as u64;

struct Fixture {
    bytes: BTreeMap<usize, u8>,
    registry: registry::test_support::Fixture,
    fault: Option<usize>,
    reads: RefCell<Vec<(usize, usize)>>,
}

impl Fixture {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.bytes
            .extend(bytes.iter().enumerate().map(|(i, &value)| (at + i, value)));
    }
    fn qword(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn dword(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn header(&mut self, pointer: usize, null: bool) {
        self.put(pointer, &[0; 17]);
        self.qword(
            pointer,
            if null {
                definition::NULL_VTABLE_RVA.0
            } else {
                definition::VTABLE_RVA.0
            } as u64,
        );
        self.dword(
            pointer + definition::TOKEN,
            if null { definition::NULL_TOKEN } else { 900 },
        );
        self.put(pointer + definition::IS_REAL, &[u8::from(!null)]);
    }
    fn new() -> Self {
        let mut memory = Self {
            bytes: BTreeMap::new(),
            registry: registry::test_support::Fixture::new(84),
            fault: None,
            reads: RefCell::default(),
        };
        memory.qword(POOL + cair_wing_pool::DEFINITION, DEFINITION as u64);
        memory.header(DEFINITION, false);
        memory.header(GLOBAL, false);
        memory.dword(DEFINITION + definition::DATABASE_INDEX, 1);
        memory.dword(DEFINITION + definition::AIR_FLAGS, TRANSPORT);
        memory.qword(definition::DATABASE_RVA.0, DATABASE as u64);
        memory.qword(DATABASE + definition::DATABASE_VECTOR, ENTRIES as u64);
        memory.dword(DATABASE + definition::DATABASE_VECTOR + 8, 2);
        memory.dword(DATABASE + definition::DATABASE_VECTOR + 12, 2);
        memory.qword(ENTRIES + 8, GLOBAL as u64);
        memory.qword(WING + RAID_REFERENCE, HANDLE);
        memory.qword(OTHER_WING + RAID_REFERENCE, HANDLE);
        memory.qword(RAID, crate::craid_instance::VTABLE_RVA.0 as u64);
        memory.qword(RAID + 8, HANDLE);
        memory.registry.entry(
            registry::test_support::Fixture::index(HANDLE),
            1,
            HANDLE,
            RAID as u64,
        );
        memory
            .registry
            .put(0x1000 + registry::COUNT, &1_u32.to_le_bytes());
        memory
    }
}

impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        if self.fault.is_some_and(|fault| {
            address <= fault && fault < address.saturating_add(destination.len())
        }) {
            return false;
        }
        if (0..destination.len()).all(|i| self.bytes.contains_key(&(address + i))) {
            for (i, byte) in destination.iter_mut().enumerate() {
                *byte = self.bytes[&(address + i)];
            }
            true
        } else {
            self.registry.copy(address, destination)
        }
    }
}

#[test]
fn null_definition_uses_native_none_token_without_database_or_raid_reads() {
    let mut memory = Fixture::new();
    memory.header(DEFINITION, true);
    let mut scratch = Scratch::with_capacity(1).unwrap();
    let mut row = super::super::Row::default();
    Pass::new(0, 1, &mut scratch).capture(&memory, WING, POOL, &mut row);
    assert_eq!(row.pool_subunit, Some(0));
    assert_eq!(row.raid_reference, None);
    assert_eq!(
        *memory.reads.borrow(),
        [(POOL + cair_wing_pool::DEFINITION, 8), (DEFINITION, 17)]
    );
    memory.dword(DEFINITION + definition::TOKEN, 0);
    row = Default::default();
    Pass::new(0, 1, &mut scratch).capture(&memory, WING, POOL, &mut row);
    assert_eq!(
        row.pool_subunit, None,
        "zero is not the null subtype's token"
    );
}

#[test]
fn internal_mask_fault_preserves_index_and_failed_raw_reads_do_not_retry() {
    let mut memory = Fixture::new();
    let mut scratch = Scratch::with_capacity(2).unwrap();
    memory.fault = Some(DEFINITION + definition::AIR_FLAGS);
    let mut row = super::super::Row::default();
    Pass::new(0, 2, &mut scratch).capture(&memory, WING, POOL, &mut row);
    assert_eq!(row.pool_subunit, Some(1));
    assert_eq!(row.raid_reference, None);
    memory.fault = Some(WING + RAID_REFERENCE);
    memory.reads.borrow_mut().clear();
    let mut pass = Pass::new(0, 2, &mut scratch);
    pass.capture(&memory, WING, POOL, &mut row);
    memory.reads.borrow_mut().clear();
    pass.capture(&memory, WING, POOL, &mut row);
    assert!(memory.reads.borrow().is_empty());
}

#[test]
fn live_identity_mismatch_is_a_cached_fault_and_null_raw_needs_no_registry() {
    let mut memory = Fixture::new();
    memory.qword(RAID + 8, HANDLE ^ (1 << 32));
    let mut scratch = Scratch::with_capacity(2).unwrap();
    let mut row = super::super::Row::default();
    {
        let mut pass = Pass::new(0, 2, &mut scratch);
        pass.capture(&memory, WING, POOL, &mut row);
        assert_eq!(row.raid_reference, None);
        memory.reads.borrow_mut().clear();
        pass.capture(&memory, OTHER_WING, POOL, &mut row);
        assert_eq!(*memory.reads.borrow(), [(OTHER_WING + RAID_REFERENCE, 8)]);
    }
    memory.qword(WING + RAID_REFERENCE, 0);
    memory.reads.borrow_mut().clear();
    Pass::new(0, 2, &mut scratch).capture(&memory, WING, POOL, &mut row);
    assert_eq!(
        row.raid_reference,
        Some(RaidReference {
            raw: 0,
            resolves: false
        })
    );
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|(address, _)| *address == registry::LOW_ROOTS_RVA.0 + 84 * 8)
    );
}

#[test]
fn generation_wrap_invalidates_prior_slots_and_matching_global_skips_duplicate_header_read() {
    let mut memory = Fixture::new();
    memory.qword(ENTRIES + 8, DEFINITION as u64);
    let mut scratch = Scratch::with_capacity(2).unwrap();
    let mut row = super::super::Row::default();
    Pass::new(0, 2, &mut scratch).capture(&memory, WING, POOL, &mut row);
    assert_eq!(row.pool_subunit, Some(1));
    assert_eq!(
        row.raid_reference,
        Some(RaidReference {
            raw: HANDLE,
            resolves: true,
        })
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(address, len)| address == DEFINITION && len == 17)
            .count(),
        1
    );
    scratch.generation = u32::MAX;
    memory.dword(DEFINITION + definition::AIR_FLAGS, 0);
    row = Default::default();
    Pass::new(0, 2, &mut scratch).capture(&memory, WING, POOL, &mut row);
    assert_eq!(row.pool_subunit, Some(1));
    assert_eq!(row.raid_reference, None);
    assert_eq!(scratch.generation, 1);
}

