use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

const BASE: usize = 0x1_4000_0000;
const STATE: usize = 0x1000;
const LIST: usize = 0x2000;
const RECORD: usize = 0x3000;
const MEMBER_LIST: usize = 0x4000;
const MEMBER_A: usize = 0x5000;
const MEMBER_B: usize = 0x5200;
const RESULT_ID: u64 = 0x7654_3210_0000_0000;
const MEMBER_ID: u64 = 0xffff_ffff_1234_5678;

struct Heap(Vec<u8>);

struct Traced<'a> {
    memory: SliceMemory<'a>,
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
}

impl Memory for Traced<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ordinal = self.reads.borrow().len();
        self.reads.borrow_mut().push((address, output.len()));
        if self.fault == Some(ordinal) {
            output.fill(0xed);
            return false;
        }
        self.memory.copy(address, output)
    }
}

impl Heap {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0[at - STATE..at - STATE + bytes.len()].copy_from_slice(bytes);
    }

    fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }

    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }

    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.u64(at, data as u64);
        self.u32(at + 8, count);
        self.u32(at + 12, count);
    }

    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: STATE,
            bytes: &self.0,
        }
    }

    fn populated() -> Self {
        let mut heap = Self(vec![0; 0x7000]);
        heap.array(STATE + cgame_state::NAVAL_COMBAT_RESULTS, LIST, 1);
        heap.u64(LIST, RECORD as u64);
        heap.u64(RECORD, (BASE + VTABLE_RVA.0) as u64);
        heap.u64(RECORD + HANDLE, RESULT_ID);
        for offset in [FIRST_SIDE, SECOND_SIDE] {
            heap.u64(RECORD + offset, (BASE + side::VTABLE_RVA.0) as u64);
        }
        heap.u64(RECORD + WINNING_SIDE, (RECORD + SECOND_SIDE) as u64);
        heap.u64(RECORD + LOSING_SIDE, (RECORD + FIRST_SIDE) as u64);
        heap.u32(RECORD + PROVINCE, 12_345);
        heap.u32(RECORD + RESULT_HOUR, i32::MIN as u32);
        heap.u32(RECORD + RETIRE_HOUR, i32::MAX as u32);
        heap.u32(RECORD + CATEGORY, u32::MAX);
        heap.put(RECORD + FLAG_A, &[128, 3, 255, 2]);
        heap.array(RECORD + FIRST_SIDE + side::MEMBERS, MEMBER_LIST, 2);
        heap.u64(MEMBER_LIST, MEMBER_A as u64);
        heap.u64(MEMBER_LIST + 8, MEMBER_B as u64);
        for (pointer, id, raw) in [
            (MEMBER_A, MEMBER_ID, 0x1_0000_0001),
            (MEMBER_B, 0x2345_6789_0000_0000, 0),
        ] {
            heap.u64(pointer, (BASE + member::VTABLE_RVA.0) as u64);
            heap.u64(pointer + member::HANDLE, id);
            heap.u64(pointer + member::PRESENCE, raw);
        }
        heap.array(RECORD + FIRST_SIDE + side::COUNTRIES, 0x6000, 2);
        heap.u32(0x6000, 10);
        heap.u32(0x6004, 20);
        heap.array(RECORD + SECOND_SIDE + side::COUNTRIES, 0x6020, 1);
        heap.u32(0x6020, 30);
        heap.array(RECORD + ADMITTED, 0x6040, 2);
        heap.u32(0x6040, 10);
        heap.u32(0x6044, 30);
        heap.u64(STATE + cgame_state::NAVAL_RESULTS_MAP_LIST_HEAD, 0x6100);
        heap.u64(STATE + cgame_state::NAVAL_RESULTS_MAP_LIST_SIZE, 1);
        heap.u64(0x6100, 0x6300);
        heap.u64(0x6300, 0x6100);
        heap.u64(STATE + cgame_state::NAVAL_RESULTS_MAP_BUCKETS, 0x6200);
        heap.u64(STATE + cgame_state::NAVAL_RESULTS_MAP_MASK, 0);
        heap.u64(0x6200, 0x6300);
        heap.u64(0x6208, 0x6300);
        heap.u32(0x6310, 12_345);
        heap.array(0x6318, 0x6400, 2);
        heap.u64(0x6400, 0x6f00);
        heap.u64(0x6408, RECORD as u64);
        heap
    }
}

#[test]
fn retained_census_and_side_entries_exceed_16384() {
    const COUNT: usize = 16_385;
    const DATA: usize = 0x8000;
    let mut heap = Heap::populated();
    heap.0.resize(DATA + COUNT * 8 - STATE, 0);
    heap.array(
        STATE + cgame_state::NAVAL_COMBAT_RESULTS,
        DATA,
        COUNT as u32,
    );
    for index in 0..COUNT {
        heap.u64(DATA + index * 8, RECORD as u64);
    }
    let mut visited = 0;
    assert_eq!(
        visit_sources(&heap.memory(), BASE, STATE, |_| {
            visited += 1;
            Ok(())
        }),
        Ok(COUNT)
    );
    assert_eq!(visited, COUNT);
    for air in [false, true] {
        heap.array(
            RECORD + FIRST_SIDE + if air { 8 } else { side::MEMBERS },
            DATA,
            COUNT as u32,
        );
        let mut visited = 0;
        assert_eq!(
            side::visit_entries(&heap.memory(), BASE, RECORD + FIRST_SIDE, air, |_, _| {
                visited += 1;
                Ok(())
            }),
            Ok(COUNT)
        );
        assert_eq!(visited, COUNT);
    }
}

#[test]
fn absent_retained_hq_ignores_a_large_borrowed_range() {
    const COUNT: usize = 257;
    const DATA: usize = 0x8000;
    let mut heap = Heap::populated();
    heap.0.resize(DATA + COUNT * 8 - STATE, 0);
    let owner = RECORD + FIRST_SIDE;
    heap.u64(owner + 0x60, DATA as u64);
    heap.u64(owner + 0x68, (DATA + COUNT * 8) as u64);
    for index in 0..COUNT {
        heap.u64(DATA + index * 8, MEMBER_A as u64);
    }
    let row = side::capture_details(&heap.memory(), BASE, owner).unwrap();
    assert!(row.hq_influences.is_empty());
}

#[test]
fn source_visitor_reads_initial_census_once_and_keeps_fault_prefixes() {
    let heap = Heap::populated();
    let expected = [
        (STATE + cgame_state::NAVAL_COMBAT_RESULTS, 16),
        (LIST, 8),
        (RECORD, 16),
    ];
    for fault in [None, Some(0), Some(1), Some(2)] {
        let memory = Traced {
            memory: heap.memory(),
            reads: RefCell::default(),
            fault,
        };
        let mut sources = Vec::new();
        let result = visit_sources(&memory, BASE, STATE, |source| {
            sources.push(source);
            Ok(())
        });
        if let Some(ordinal) = fault {
            assert!(result.is_err());
            assert!(sources.is_empty());
            assert_eq!(*memory.reads.borrow(), expected[..=ordinal]);
        } else {
            assert_eq!(result, Ok(1));
            assert_eq!(
                sources,
                [Source {
                    pointer: RECORD,
                    id: RESULT_ID
                }]
            );
            assert_eq!(*memory.reads.borrow(), expected);
        }
    }
    let memory = Traced {
        memory: heap.memory(),
        reads: RefCell::default(),
        fault: None,
    };
    assert_eq!(
        visit_sources(&memory, BASE, STATE, |_| Err("visitor failed")),
        Err("visitor failed")
    );
    assert_eq!(*memory.reads.borrow(), expected);
}

#[test]
fn native_handles_dates_flags_and_member_values_survive_the_walk() {
    let heap = Heap::populated();
    let mut records = vec![Row::default(); 1];
    let mut members = vec![MemberRow::default(); 2];
    let mut countries = vec![CountryRow::default(); 5];
    let result = walk(
        &heap.memory(),
        BASE,
        STATE,
        &mut records,
        &mut vec![ResultSlot::default(); 1],
        &mut members,
        &mut countries,
    );
    assert_eq!(
        result,
        Walk {
            records: Outcome::Rows { len: 1 },
            members: Outcome::Rows { len: 2 },
            countries: Outcome::Rows { len: 5 },
        }
    );
    assert_eq!(
        records[0],
        Row {
            id: RESULT_ID,
            province: 12_345,
            result_hour: i32::MIN,
            retire_hour: i32::MAX,
            category: u32::MAX,
            flag_a: 128,
            flag_b: 3,
            subtype: 255,
            readiness: 2,
            winning_side: 2,
            losing_side: 1,
            province_ordinal: 1,
        }
    );
    assert_eq!(members[0].member, MEMBER_ID);
    assert_eq!(members[0].presence_value, 0x1_0000_0001);
    assert_eq!(
        members[0].key(),
        (u128::from(RESULT_ID) << 64) | u128::from(MEMBER_ID)
    );
    assert_eq!(members[1].presence_value, 0);
    assert_eq!(members[1].ordinal, 1);
    assert_eq!(
        countries.iter().map(|row| (row.list, row.ordinal, row.country)).collect::<Vec<_>>(),
        [(0, 0, 10), (0, 1, 20), (1, 0, 30), (2, 0, 10), (2, 1, 30)]
    );
    assert_ne!(countries[0].key(), countries[3].key());
}

#[test]
fn broken_member_and_result_fields_only_invalidate_their_own_tables() {
    let mut heap = Heap::populated();
    heap.u64(MEMBER_A, 0);
    let result = walk(
        &heap.memory(),
        BASE,
        STATE,
        &mut vec![Row::default(); 1],
        &mut vec![ResultSlot::default(); 1],
        &mut vec![MemberRow::default(); 2],
        &mut vec![CountryRow::default(); 5],
    );
    assert!(matches!(result.members, Outcome::Unavailable { .. }));
    assert_eq!(result.records, Outcome::Rows { len: 1 });
    assert_eq!(result.countries, Outcome::Rows { len: 5 });
    let mut heap = Heap::populated();
    heap.u64(RECORD + WINNING_SIDE, 1);
    let result = walk(
        &heap.memory(),
        BASE,
        STATE,
        &mut vec![Row::default(); 1],
        &mut vec![ResultSlot::default(); 1],
        &mut vec![MemberRow::default(); 2],
        &mut vec![CountryRow::default(); 5],
    );
    assert!(matches!(result.records, Outcome::Unavailable { .. }));
    assert_eq!(result.members, Outcome::Rows { len: 2 });
    assert_eq!(result.countries, Outcome::Rows { len: 5 });
}

#[test]
fn empty_retention_is_valid_and_corrupt_root_is_never_a_successful_empty() {
    let mut heap = Heap(vec![0; 0x7000]);
    let empty = Outcome::Rows { len: 0 };
    assert_eq!(
        walk(
            &heap.memory(),
            BASE,
            STATE,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new()
        ),
        Walk {
            records: empty,
            members: empty,
            countries: empty
        }
    );
    heap.array(STATE + cgame_state::NAVAL_COMBAT_RESULTS, LIST, 1);
    let result = walk(
        &heap.memory(),
        BASE,
        STATE,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
    );
    assert!(matches!(result.records, Outcome::Unavailable { .. }));
    assert!(matches!(result.members, Outcome::Unavailable { .. }));
    assert!(matches!(result.countries, Outcome::Unavailable { .. }));
}

fn ordinal_fixture(count: usize, provinces: usize) -> (Vec<u8>, Vec<Row>, Vec<ResultSlot>) {
    const GAME_STATE: usize = 0x1000;
    const SENTINEL: usize = 0x2000;
    const NODES: usize = 0x3000;
    let vectors = NODES + provinces * 0x40;
    let mut bytes = vec![0u8; vectors + count * 8];
    let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
    put(
        GAME_STATE + cgame_state::NAVAL_RESULTS_MAP_LIST_HEAD,
        &(SENTINEL as u64).to_le_bytes(),
    );
    put(
        GAME_STATE + cgame_state::NAVAL_RESULTS_MAP_LIST_SIZE,
        &(provinces as u64).to_le_bytes(),
    );
    put(SENTINEL, &(NODES as u64).to_le_bytes());
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); provinces];
    for result in 0..count {
        members[result % provinces].push(result);
    }
    let mut rows = Vec::new();
    let mut slots = Vec::new();
    let mut vector = vectors;
    for (province, results) in members.iter().enumerate() {
        let node = NODES + province * 0x40;
        let next = if province + 1 == provinces {
            SENTINEL
        } else {
            node + 0x40
        };
        put(node, &(next as u64).to_le_bytes());
        put(node + 0x10, &(province as u32 + 1).to_le_bytes());
        put(node + 0x18, &(vector as u64).to_le_bytes());
        put(node + 0x20, &(results.len() as u32).to_le_bytes());
        put(node + 0x24, &(results.len() as u32).to_le_bytes());
        for (ordinal, &result) in results.iter().rev().enumerate() {
            put(
                vector + ordinal * 8,
                &(0x100_0000 + result as u64 * 0x200).to_le_bytes(),
            );
        }
        vector += results.len() * 8;
    }
    for result in 0..count {
        rows.push(Row {
            id: result as u64 + 1,
            province: (result % provinces) as u32 + 1,
            ..Row::default()
        });
        slots.push(ResultSlot {
            pointer: 0x100_0000 + result * 0x200,
            row: result as u32,
        });
    }
    (bytes, rows, slots)
}

#[test]
fn province_ordinals_refuse_a_missing_misfiled_or_repeated_result() {
    let (bytes, mut rows, mut slots) = ordinal_fixture(4, 2);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut misfiled = rows.clone();
    misfiled[0].province = 2;
    assert_eq!(
        province_ordinals(&memory, 0x1000, &mut misfiled, &mut slots.clone()),
        Err("naval result filed under another province")
    );
    let mut extra = rows.clone();
    extra.push(Row {
        province: 1,
        ..Row::default()
    });
    let mut extra_slots = slots.clone();
    extra_slots.push(ResultSlot {
        pointer: 0x7fff_0000,
        row: 4,
    });
    assert_eq!(
        province_ordinals(&memory, 0x1000, &mut extra, &mut extra_slots),
        Err("naval result absent from its province bucket")
    );
    let mut short = bytes.clone();
    short[0x1000 + cgame_state::NAVAL_RESULTS_MAP_LIST_SIZE] = 3;
    let memory = SliceMemory {
        base: 0,
        bytes: &short,
    };
    assert_eq!(
        province_ordinals(&memory, 0x1000, &mut rows, &mut slots),
        Err("naval result province list shorter than its size")
    );
}

#[test]
fn growing_result_storage_keeps_the_original_read_trace_and_fault_boundaries() {
    let heap = Heap::populated();
    for fault in [None, Some(0), Some(1), Some(2), Some(3), Some(7), Some(19)] {
        let fixed = Traced {
            memory: heap.memory(),
            reads: RefCell::default(),
            fault,
        };
        let expected = walk(
            &fixed,
            BASE,
            STATE,
            &mut vec![Row::default(); 1],
            &mut vec![ResultSlot::default(); 1],
            &mut vec![MemberRow::default(); 2],
            &mut vec![CountryRow::default(); 5],
        );
        let growing = Traced {
            memory: heap.memory(),
            reads: RefCell::default(),
            fault,
        };
        let actual = walk(
            &growing,
            BASE,
            STATE,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
        );
        assert_eq!(actual, expected, "fault {fault:?}");
        assert_eq!(
            *growing.reads.borrow(),
            *fixed.reads.borrow(),
            "fault {fault:?}"
        );
    }
}
