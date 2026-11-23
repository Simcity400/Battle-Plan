use super::*;
use engine_native_parts::{creference_registry as registry, reference_bucket};
use crate::cunit;
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

const STATE: usize = 0x1000;
const ROOT: usize = STATE + cgame_state::PROVINCES;
const PROVINCE: usize = 0x2000;
const LANES: [usize; 2] = [0x3000, 0x3400];
const TABLE: usize = 0x4000;
const BUCKETS: usize = 0x4100;
const UNITS: [usize; 3] = [0x6000, 0x7000, 0x8000];
const HANDLES: [u64; 3] = [(1 << 32) | 4713, (2 << 32) | 4713, (3 << 32) | 4713];

fn bucket(handle: u64) -> usize {
    let hash = handle
        .rotate_left(32)
        .wrapping_mul(registry::HASH_MULTIPLIER);
    BUCKETS + (((hash >> 32) as u32 ^ hash as u32) & 7) as usize * reference_bucket::STRIDE
}

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

fn fixture(counts: [usize; 2]) -> Vec<u8> {
    fn array(bytes: &mut [u8], at: usize, data: usize, count: usize) {
        put(bytes, at, &(data as u64).to_le_bytes());
        put(bytes, at + 8, &(count as u32).to_le_bytes());
        put(bytes, at + 12, &(count as u32).to_le_bytes());
    }
    let mut bytes = vec![0; 0x9000];
    array(&mut bytes, ROOT, 0x1800, 2);
    put(&mut bytes, 0x1808, &(PROVINCE as u64).to_le_bytes());
    put(&mut bytes, PROVINCE, &(VTABLE_RVA.0 as u64).to_le_bytes());
    put(&mut bytes, PROVINCE + ID, &1u32.to_le_bytes());
    for (lane, offset) in [OCCUPANTS, NAVAL_OCCUPANTS].into_iter().enumerate() {
        array(&mut bytes, PROVINCE + offset, LANES[lane], counts[lane]);
        for index in 0..counts[lane] {
            let unit = UNITS[if lane == 0 { index % 3 } else { 1 }];
            put(
                &mut bytes,
                LANES[lane] + index * 8,
                &(unit as u64).to_le_bytes(),
            );
        }
    }
    put(
        &mut bytes,
        TABLE + registry::ENTRIES,
        &(BUCKETS as u64).to_le_bytes(),
    );
    put(&mut bytes, TABLE + registry::COUNT, &3u32.to_le_bytes());
    put(&mut bytes, TABLE + registry::MASK, &7u32.to_le_bytes());
    bytes[TABLE + registry::OVERFLOW_DISTANCE] = 5;
    bytes[BUCKETS + 13 * reference_bucket::STRIDE + reference_bucket::DISTANCE] = 255;
    let mut occupied = [false; 8];
    for (index, (class, vtable)) in [
        (0u32, crate::carmy::VTABLE_RVA),
        (1, crate::ctask_force::VTABLE_RVA),
        (13, engine_native_parts::crailway_gun::VTABLE_RVA),
    ]
    .into_iter()
    .enumerate()
    {
        let unit = UNITS[index];
        put(&mut bytes, unit, &(vtable.0 as u64).to_le_bytes());
        put(&mut bytes, unit + cunit::CLASS, &class.to_le_bytes());
        put(
            &mut bytes,
            unit + cunit::HANDLE,
            &HANDLES[index].to_le_bytes(),
        );
        put(
            &mut bytes,
            unit + cunit::LOCATION,
            &(PROVINCE as u64).to_le_bytes(),
        );
        let cell = bucket(HANDLES[index]);
        let slot = (cell - BUCKETS) / reference_bucket::STRIDE;
        assert!(!occupied[slot], "fixture keys occupy distinct buckets");
        occupied[slot] = true;
        bytes[cell + reference_bucket::DISTANCE] = 1;
        put(
            &mut bytes,
            cell + reference_bucket::TYPE,
            &HANDLES[index].to_le_bytes(),
        );
        put(
            &mut bytes,
            cell + reference_bucket::OBJECT,
            &((unit + 0x10) as u64).to_le_bytes(),
        );
    }
    bytes
}

struct Observed<'a> {
    bytes: &'a [u8],
    reads: RefCell<Vec<(usize, usize)>>,
    fault_at: Option<usize>,
    change_at: Option<(usize, usize)>,
}
impl Memory for Observed<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ordinal = self.reads.borrow().len();
        self.reads.borrow_mut().push((address, output.len()));
        if self.fault_at == Some(ordinal) {
            output.fill(0xee);
            return false;
        }
        if address == registry::HIGH_ROOT_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&(TABLE as u64).to_le_bytes());
        } else if !(SliceMemory {
            base: 0,
            bytes: self.bytes,
        })
        .copy(address, output)
        {
            return false;
        }
        if let Some((at, byte)) = self.change_at
            && at == ordinal
        {
            output[byte] ^= 1;
        }
        true
    }
}

fn trace(counts: [usize; 2]) -> Vec<(usize, usize, &'static str)> {
    let mut reads = vec![
        (ROOT, 16, "province order root unreadable"),
        (0x1808, 8, "province order slot unreadable"),
        (PROVINCE, 8, "province order identity mismatch"),
        (PROVINCE + ID, 4, "province order identity mismatch"),
        (PROVINCE + OCCUPANTS, 16, "province order header unreadable"),
        (
            PROVINCE + NAVAL_OCCUPANTS,
            16,
            "province order header unreadable",
        ),
    ];
    let mut registry_seen = false;
    for lane in 0..2 {
        for first in (0..counts[lane]).step_by(64) {
            let n = (counts[lane] - first).min(64);
            reads.push((
                LANES[lane] + first * 8,
                n * 8,
                "province order pointers unreadable",
            ));
            for index in first..first + n {
                let index = if lane == 0 { index % 3 } else { 1 };
                reads.push((UNITS[index], 32, "counter unit header unreadable"));
                if !registry_seen {
                    reads.extend([
                        (registry::HIGH_ROOT_RVA.0, 8, "reference root unreadable"),
                        (TABLE + registry::ENTRIES, 17, "reference header unreadable"),
                        (
                            BUCKETS + 13 * reference_bucket::STRIDE + reference_bucket::DISTANCE,
                            1,
                            "reference sentinel or count invalid",
                        ),
                    ]);
                    registry_seen = true;
                }
                reads.extend([
                    (bucket(HANDLES[index]), 24, "reference bucket unreadable"),
                    (
                        UNITS[index] + cunit::LOCATION,
                        8,
                        "province unit roster backlink mismatch",
                    ),
                ]);
            }
        }
    }
    reads
}

fn expected_handles(counts: [usize; 2]) -> Vec<u64> {
    (0..counts[0])
        .map(|index| HANDLES[index % 3])
        .chain(std::iter::repeat_n(HANDLES[1], counts[1]))
        .collect()
}

#[test]
fn unit_order_lookahead_keeps_chunk_boundaries_duplicates_capacity_and_primary_reads() {
    for counts in [
        [0, 0],
        [1, 0],
        [3, 3],
        [4, 4],
        [5, 5],
        [63, 5],
        [64, 5],
        [65, 5],
        [67, 5],
    ] {
        let bytes = fixture(counts);
        let expected = trace(counts);
        let total = counts.iter().sum::<usize>();
        assert_eq!(
            expected.len(),
            6 + counts[0].div_ceil(64)
                + counts[1].div_ceil(64)
                + 3 * total
                + 3 * usize::from(total != 0)
        );
        assert_eq!(
            expected.iter().map(|&(_, len, _)| len).sum::<usize>(),
            68 + 72 * total + 26 * usize::from(total != 0)
        );
        for (rows, capacity) in [(1, total)] {
            let memory = Observed {
                bytes: &bytes,
                reads: RefCell::new(Vec::new()),
                fault_at: None,
                change_at: None,
            };
            let mut output = vec![UnitOrderRow::default(); rows];
            let mut handles = vec![u64::MAX; capacity];
            let result = walk_unit_order(&memory, 0, STATE, &mut output, &mut handles);
            assert_eq!(result.required, total);
            let admitted = rows != 0 && capacity >= total;
            let observed: Vec<_> = if admitted {
                assert_eq!(result.outcome, Outcome::Rows { len: 1 });
                assert_eq!(result.len, total);
                assert_eq!(handles, expected_handles(counts));
                assert_eq!(
                    output[0],
                    UnitOrderRow {
                        province: 1,
                        mixed: [0, counts[0]],
                        naval: [counts[0], counts[1]]
                    }
                );
                expected
                    .iter()
                    .map(|&(address, len, _)| (address, len))
                    .collect()
            } else {
                assert_eq!(
                    result.outcome,
                    Outcome::Truncated {
                        len: 0,
                        unread_rows: 1
                    }
                );
                assert_eq!(result.len, 0);
                assert!(handles.iter().all(|&handle| handle == u64::MAX));
                assert_eq!(output[0], UnitOrderRow::default());
                expected[..6]
                    .iter()
                    .map(|&(address, len, _)| (address, len))
                    .collect()
            };
            assert_eq!(*memory.reads.borrow(), observed);
        }
    }
}

#[test]
fn unit_order_empty_headers_keep_capacity_pointer_and_identity_validation() {
    let expected: Vec<_> = trace([0, 0])
        .into_iter()
        .map(|(address, len, _)| (address, len))
        .collect();
    for data in [0u64, 1, usize::MAX as u64] {
        for capacities in [
            [0, 0],
            [1, i32::MAX as u32],
            [i32::MAX as u32, 0],
            [i32::MAX as u32 + 1, 0],
            [0, u32::MAX],
        ] {
            for rows in [1] {
                let mut bytes = fixture([0, 0]);
                for (lane, offset) in [OCCUPANTS, NAVAL_OCCUPANTS].into_iter().enumerate() {
                    put(&mut bytes, PROVINCE + offset, &data.to_le_bytes());
                    put(
                        &mut bytes,
                        PROVINCE + offset + 8,
                        &capacities[lane].to_le_bytes(),
                    );
                }
                let memory = Observed {
                    bytes: &bytes,
                    reads: RefCell::new(Vec::new()),
                    fault_at: None,
                    change_at: None,
                };
                let mut output = vec![UnitOrderRow::default(); rows];
                let result = walk_unit_order(&memory, 0, STATE, &mut output, &mut Vec::new());
                assert_eq!(*memory.reads.borrow(), expected);
                assert_eq!((result.len, result.required), (0, 0));
                let valid = capacities.into_iter().all(|n| n <= i32::MAX as u32);
                assert_eq!(
                    result.outcome,
                    if !valid {
                        Outcome::Unavailable {
                            reason: "province order array invalid or bound exceeded",
                        }
                    } else if rows == 0 {
                        Outcome::Truncated {
                            len: 0,
                            unread_rows: 1,
                        }
                    } else {
                        Outcome::Rows { len: 1 }
                    }
                );
                assert_eq!(output[0].province, u32::from(valid && rows != 0));
            }
        }
    }
    let bytes = fixture([0, 0]);
    for fault_at in 0..expected.len() {
        let memory = Observed {
            bytes: &bytes,
            reads: RefCell::new(Vec::new()),
            fault_at: Some(fault_at),
            change_at: None,
        };
        let result = walk_unit_order(&memory, 0, STATE, &mut Vec::new(), &mut Vec::new());
        assert_eq!(*memory.reads.borrow(), expected[..=fault_at]);
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: trace([0, 0])[fault_at].2,
            }
        );
    }
    for identity_read in [2, 3] {
        let memory = Observed {
            bytes: &bytes,
            reads: RefCell::new(Vec::new()),
            fault_at: None,
            change_at: Some((identity_read, 0)),
        };
        let result = walk_unit_order(&memory, 0, STATE, &mut Vec::new(), &mut Vec::new());
        assert_eq!(*memory.reads.borrow(), expected);
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "province order identity mismatch",
            }
        );
    }
}

#[test]
fn unit_order_strided_census_keeps_batch_order_short_tails_and_failure_prefixes() {
    for count in [1usize, 63, 64, 65] {
        let mut bytes = fixture([0, 0]);
        bytes.resize(0xa000 + count * 0x200, 0);
        for offset in [8, 12] {
            put(&mut bytes, ROOT + offset, &(count as u32 + 1).to_le_bytes());
        }
        let pointers: Vec<_> = (0..count).map(|index| 0xa000 + index * 0x200).collect();
        for (index, &pointer) in pointers.iter().enumerate() {
            put(
                &mut bytes,
                0x1808 + index * 8,
                &(pointer as u64).to_le_bytes(),
            );
            put(&mut bytes, pointer, &(VTABLE_RVA.0 as u64).to_le_bytes());
            put(&mut bytes, pointer + ID, &(index as u32 + 1).to_le_bytes());
        }
        let mut expected = vec![(ROOT, 16, "province order root unreadable")];
        for (batch, pointers) in pointers.chunks(64).enumerate() {
            for slot in 0..pointers.len() {
                expected.push((
                    0x1808 + (batch * 64 + slot) * 8,
                    8,
                    "province order slot unreadable",
                ));
            }
            for &pointer in pointers {
                expected.extend([
                    (pointer, 8, "province order identity mismatch"),
                    (pointer + ID, 4, "province order identity mismatch"),
                    (pointer + OCCUPANTS, 16, "province order header unreadable"),
                    (
                        pointer + NAVAL_OCCUPANTS,
                        16,
                        "province order header unreadable",
                    ),
                ]);
            }
        }
        let addresses: Vec<_> = expected.iter().map(|&(at, len, _)| (at, len)).collect();
        for fault_at in std::iter::once(None).chain((0..expected.len()).map(Some)) {
            let memory = Observed {
                bytes: &bytes,
                reads: RefCell::new(Vec::new()),
                fault_at,
                change_at: None,
            };
            let mut output = vec![UnitOrderRow::default(); count];
            let result = walk_unit_order(&memory, 0, STATE, &mut output, &mut Vec::new());
            assert_eq!((result.len, result.required), (0, 0));
            if let Some(at) = fault_at {
                assert_eq!(*memory.reads.borrow(), addresses[..=at]);
                assert_eq!(
                    result.outcome,
                    Outcome::Unavailable {
                        reason: expected[at].2
                    }
                );
            } else {
                assert_eq!(*memory.reads.borrow(), addresses);
                assert_eq!(result.outcome, Outcome::Rows { len: count });
                for (index, row) in output.iter().enumerate() {
                    assert_eq!(
                        *row,
                        UnitOrderRow {
                            province: index as u32 + 1,
                            mixed: [0, 0],
                            naval: [0, 0],
                        }
                    );
                }
            }
        }
    }
}

#[test]
fn unit_order_lookahead_keeps_every_failed_copy_prefix_and_partial_handle_count() {
    let counts = [67, 5];
    let bytes = fixture(counts);
    let expected = trace(counts);
    let addresses: Vec<_> = expected
        .iter()
        .map(|&(address, len, _)| (address, len))
        .collect();
    let all_handles = expected_handles(counts);
    for fault_at in 0..expected.len() {
        let memory = Observed {
            bytes: &bytes,
            reads: RefCell::new(Vec::new()),
            fault_at: Some(fault_at),
            change_at: None,
        };
        let mut output = vec![UnitOrderRow::default(); 1];
        let mut handles = vec![u64::MAX; all_handles.len()];
        let result = walk_unit_order(&memory, 0, STATE, &mut output, &mut handles);
        assert_eq!(*memory.reads.borrow(), addresses[..=fault_at]);
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: expected[fault_at].2
            }
        );
        let completed = addresses[..fault_at]
            .iter()
            .filter(|&&(address, len)| {
                len == 8 && UNITS.iter().any(|unit| address == unit + cunit::LOCATION)
            })
            .count();
        assert_eq!(result.len, completed);
        assert_eq!(
            result.required,
            if fault_at < 6 { 0 } else { all_handles.len() }
        );
        assert_eq!(handles[..completed], all_handles[..completed]);
        assert!(
            handles[completed..]
                .iter()
                .all(|&handle| handle == u64::MAX)
        );
        assert_eq!(output[0], UnitOrderRow::default());
    }
}

#[test]
fn unit_order_lookahead_retains_class_registry_and_backlink_short_circuits() {
    let counts = [67, 5];
    let bytes = fixture(counts);
    let expected = trace(counts);
    let addresses: Vec<_> = expected
        .iter()
        .map(|&(address, len, _)| (address, len))
        .collect();
    let identities: Vec<_> = addresses
        .iter()
        .enumerate()
        .filter_map(|(index, &(address, len))| {
            (len == 32 && UNITS.contains(&address)).then_some(index)
        })
        .collect();
    let mut changes: Vec<_> = [0, 3, 4, 63, 64]
        .map(|unit| {
            let at = identities[unit];
            (at, 0, at, "counter unit identity invalid")
        })
        .into_iter()
        .collect();
    let first = identities[0];
    changes.extend([
        (
            first + 4,
            reference_bucket::OBJECT,
            first + 4,
            "counter unit registry identity mismatch",
        ),
        (
            first + 5,
            0,
            first + 5,
            "province unit roster backlink mismatch",
        ),
    ]);
    for (at, byte, last, reason) in changes {
        let memory = Observed {
            bytes: &bytes,
            reads: RefCell::new(Vec::new()),
            fault_at: None,
            change_at: Some((at, byte)),
        };
        let result = walk_unit_order(
            &memory,
            0,
            STATE,
            &mut vec![UnitOrderRow::default(); 1],
            &mut vec![0; 72],
        );
        assert_eq!(result.outcome, Outcome::Unavailable { reason });
        assert_eq!(*memory.reads.borrow(), addresses[..=last]);
    }
    let mut wrong_class = bytes.clone();
    put(
        &mut wrong_class,
        UNITS[1],
        &(crate::carmy::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut wrong_class,
        UNITS[1] + cunit::CLASS,
        &0u32.to_le_bytes(),
    );
    let memory = Observed {
        bytes: &wrong_class,
        reads: RefCell::new(Vec::new()),
        fault_at: None,
        change_at: None,
    };
    let result = walk_unit_order(
        &memory,
        0,
        STATE,
        &mut vec![UnitOrderRow::default(); 1],
        &mut vec![0; 72],
    );
    assert_eq!(
        result.outcome,
        Outcome::Unavailable {
            reason: "province unit roster backlink mismatch"
        }
    );
    assert_eq!(*memory.reads.borrow(), addresses[..=identities[67] + 1]);
    assert_eq!(result.len, 67);

    let mut invalid_pending = bytes;
    put(&mut invalid_pending, LANES[0] + 8, &u64::MAX.to_le_bytes());
    put(&mut invalid_pending, LANES[0] + 16, &0u64.to_le_bytes());
    let memory = Observed {
        bytes: &invalid_pending,
        reads: RefCell::new(Vec::new()),
        fault_at: None,
        change_at: Some((first, 0)),
    };
    let result = walk_unit_order(
        &memory,
        0,
        STATE,
        &mut vec![UnitOrderRow::default(); 1],
        &mut vec![0; 72],
    );
    assert_eq!(
        result.outcome,
        Outcome::Unavailable {
            reason: "counter unit identity invalid"
        }
    );
    assert_eq!(*memory.reads.borrow(), addresses[..=first]);
}

#[test]
fn unit_order_reacquires_changed_registry_header_on_each_walk() {
    let mut bytes = fixture([1, 0]);
    let walk = |bytes: &[u8]| {
        walk_unit_order(
            &Observed {
                bytes,
                reads: RefCell::new(Vec::new()),
                fault_at: None,
                change_at: None,
            },
            0,
            STATE,
            &mut vec![UnitOrderRow::default(); 1],
            &mut vec![0; 1],
        )
        .outcome
    };
    assert_eq!(walk(&bytes), Outcome::Rows { len: 1 });
    put(&mut bytes, TABLE + registry::MASK, &6_u32.to_le_bytes());
    assert_eq!(
        walk(&bytes),
        Outcome::Unavailable {
            reason: "reference header invalid"
        }
    );
    put(&mut bytes, TABLE + registry::MASK, &7_u32.to_le_bytes());
    assert_eq!(walk(&bytes), Outcome::Rows { len: 1 });
}

#[test]
fn worker_province_order_population_jump_grows_from_original_headers() {
    let bytes = fixture([2, 1]);
    let fixed = Observed {
        bytes: &bytes,
        reads: Default::default(),
        fault_at: None,
        change_at: None,
    };
    let grown = Observed {
        bytes: &bytes,
        reads: Default::default(),
        fault_at: None,
        change_at: None,
    };
    let mut expected_rows = vec![UnitOrderRow::default(); 1];
    let mut expected_handles = vec![0; 3];
    let expected = walk_unit_order_with_land(
        &fixed,
        0,
        STATE,
        &mut expected_rows,
        &mut expected_handles,
        |_, _| Ok(()),
    );
    let (mut rows, mut handles) = (Vec::new(), Vec::new());
    let actual =
        walk_unit_order_with_land(&grown, 0, STATE, &mut rows, &mut handles, |_, _| Ok(()));
    assert_eq!(actual, expected);
    assert_eq!(actual.outcome, Outcome::Rows { len: 1 });
    assert_eq!(rows[..1], expected_rows);
    assert_eq!(handles[..3], expected_handles);
    assert_eq!(*grown.reads.borrow(), *fixed.reads.borrow());
}

#[test]
fn census_divisions_take_identity_and_backlink_from_the_census_without_reads() {
    fn observed(bytes: &[u8]) -> Observed<'_> {
        Observed {
            bytes,
            reads: RefCell::new(Vec::new()),
            fault_at: None,
            change_at: None,
        }
    }
    let counts = [5, 2];
    let bytes = fixture(counts);
    let plain = observed(&bytes);
    let (mut rows, mut handles) = (Vec::new(), Vec::new());
    let mut plain_land = Vec::new();
    let expected = walk_unit_order_with_land(
        &plain,
        0,
        STATE,
        &mut rows,
        &mut handles,
        |province, handle| {
            plain_land.push((province, handle));
            Ok(())
        },
    );
    assert_eq!(expected.outcome, Outcome::Rows { len: 1 });
    let division = KnownUnit {
        handle: HANDLES[0],
        location: PROVINCE,
    };
    let census = observed(&bytes);
    let (mut known_rows, mut known_handles) = (Vec::new(), Vec::new());
    let mut land = Vec::new();
    let actual = walk_unit_order_with_known(
        &census,
        0,
        STATE,
        &mut known_rows,
        &mut known_handles,
        |unit| (unit == UNITS[0]).then_some(division),
        |province, handle| {
            land.push((province, handle));
            Ok(())
        },
    );
    assert_eq!(actual, expected);
    assert_eq!((known_rows, known_handles), (rows, handles));
    assert_eq!(land, plain_land);
    assert_eq!(land, [(1, HANDLES[0]); 2]);
    let skipped = [
        (UNITS[0], 32),
        (bucket(HANDLES[0]), 24),
        (UNITS[0] + cunit::LOCATION, 8),
    ];
    let mut remaining: Vec<_> = plain
        .reads
        .borrow()
        .iter()
        .copied()
        .filter(|read| !skipped.contains(read))
        .collect();
    let mut reads = census.reads.borrow().clone();
    assert!(
        skipped
            .iter()
            .all(|read| plain.reads.borrow().contains(read))
    );
    assert!(skipped.iter().all(|read| !reads.contains(read)));
    remaining.sort_unstable();
    reads.sort_unstable();
    assert_eq!(reads, remaining);

    for (unit, location) in [(UNITS[0], PROVINCE + 8), (UNITS[1], PROVINCE)] {
        let census = observed(&bytes);
        let refused = walk_unit_order_with_known(
            &census,
            0,
            STATE,
            &mut Vec::new(),
            &mut Vec::new(),
            |at| {
                (at == unit).then_some(KnownUnit {
                    handle: HANDLES[if unit == UNITS[0] { 0 } else { 1 }],
                    location,
                })
            },
            |_, _| Ok(()),
        );
        assert_eq!(
            refused.outcome,
            Outcome::Unavailable {
                reason: "province unit roster backlink mismatch"
            }
        );
        assert!(
            census
                .reads
                .borrow()
                .iter()
                .all(|&(address, _)| address != unit && address != unit + cunit::LOCATION)
        );
    }
}

#[test]
fn province_order_demand_has_no_chosen_occupant_or_aggregate_ceiling() {
    let mut bytes = fixture([0, 0]);
    let count = 1_100_000u32;
    put(&mut bytes, PROVINCE + OCCUPANTS + 8, &count.to_le_bytes());
    put(&mut bytes, PROVINCE + OCCUPANTS + 12, &count.to_le_bytes());
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let demand = count_unit_order_with_land(&memory, 0, STATE, |_, _| Ok(()));
    assert_eq!(demand.required, count as usize);
    assert_eq!(
        demand.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
}

#[test]
fn listed_provinces_read_only_their_slots() {
    fn observed(bytes: &[u8]) -> Observed<'_> {
        Observed {
            bytes,
            reads: RefCell::new(Vec::new()),
            fault_at: None,
            change_at: None,
        }
    }
    let bytes = fixture([5, 2]);
    let plain = observed(&bytes);
    let (mut rows, mut handles, mut land) = (Vec::new(), Vec::new(), Vec::new());
    let expected = walk_unit_order_with_land(
        &plain,
        0,
        STATE,
        &mut rows,
        &mut handles,
        |province, handle| {
            land.push((province, handle));
            Ok(())
        },
    );
    let listed = |list: &[u32]| {
        let memory = observed(&bytes);
        let (mut rows, mut handles, mut land) = (Vec::new(), Vec::new(), Vec::new());
        let walk = walk_unit_order_listed(
            &memory,
            0,
            STATE,
            list,
            &mut rows,
            &mut handles,
            |_| None,
            |province, handle| {
                land.push((province, handle));
                Ok(())
            },
        );
        handles.truncate(walk.len);
        let reads = memory.reads.borrow().clone();
        (walk, rows, handles, land, reads)
    };
    let (walk, listed_rows, listed_handles, listed_land, reads) = listed(&[1]);
    assert_eq!(walk, expected);
    assert_eq!(listed_rows[..1], rows[..1]);
    assert_eq!(listed_handles, handles[..expected.len]);
    assert_eq!(listed_land, land);
    assert_eq!(reads, *plain.reads.borrow());
    let (walk, _, _, land, reads) = listed(&[]);
    assert_eq!(walk.outcome, Outcome::Rows { len: 0 });
    assert!(land.is_empty());
    assert_eq!(reads, [(ROOT, 16)]);
    assert_eq!(
        listed(&[2]).0.outcome,
        Outcome::Unavailable {
            reason: "province order listed province outside census"
        }
    );
}
