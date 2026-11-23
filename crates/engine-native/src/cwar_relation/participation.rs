
use engine_native_memory::memory::{RecordField, RecordLayout, RecordSources};
use engine_native_memory::{Memory, read};
use engine_allocation as allocation;
use engine_sites::Rva;

pub const DIRECTIONAL_LEDGERS: [usize; 2] = [0x70, 0xe0];
pub const LEDGER_VTABLE_RVA: Rva = Rva(0x029d_caa8);
pub const FIRST: usize = 8;
pub const SECOND: usize = 12;
pub const EQUIPMENT_DAMAGE: usize = 0x10;
pub const PROVINCE_CAPTURE: usize = 0x18;
pub const AIR_DAMAGE: usize = 0x20;
pub const STRATEGIC_BOMBING: usize = 0x28;
pub const SUNK_SHIPS: usize = 0x30;
pub const SUNK_CONVOYS: usize = 0x38;
pub const CASUALTIES: usize = 0x40;
pub const UNKNOWN_CASUALTIES: usize = 0x48;
pub const LEND_LEASE_SENT: usize = 0x50;
pub const LEND_LEASE_RECEIVED: usize = 0x58;
pub const CAPTURED_PROVINCES: usize = 0x60;
const SCORE_BYTES: usize = 0x60;
const NODE_BYTES: usize = 0x20;
const NODE_LEFT: usize = 0;
const NODE_PARENT: usize = 8;
const NODE_RIGHT: usize = 0x10;
const NODE_NIL: usize = 0x19;
const NODE_KEY: usize = 0x1c;
pub const SCORE_FIELDS: [usize; 10] = [
    EQUIPMENT_DAMAGE,
    PROVINCE_CAPTURE,
    AIR_DAMAGE,
    STRATEGIC_BOMBING,
    SUNK_SHIPS,
    SUNK_CONVOYS,
    CASUALTIES,
    UNKNOWN_CASUALTIES,
    LEND_LEASE_SENT,
    LEND_LEASE_RECEIVED,
];
pub const ALL_SCORE_FIELDS: u16 = (1 << SCORE_FIELDS.len()) - 1;

pub use game_model::war_participation::Scores;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ledger {
    pub scores: Scores,
    pub captured_provinces: Vec<i32>,
}

pub fn read_into(
    memory: &impl Memory,
    base: usize,
    ledger: usize,
    output: &mut Ledger,
) -> Result<(), &'static str> {
    output.scores = read_scores(memory, base, ledger)?;
    read_captured_provinces(memory, ledger, &mut output.captured_provinces)
}

pub fn read_scores(
    memory: &impl Memory,
    base: usize,
    ledger: usize,
) -> Result<Scores, &'static str> {
    let bytes =
        read::<[u8; SCORE_BYTES]>(memory, ledger).ok_or("war participation ledger unreadable")?;
    let word = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    if word(0) as usize
        != engine_sites::at(base, LEDGER_VTABLE_RVA).ok_or("war participation vtable withheld")?
    {
        return Err("war participation ledger vtable mismatch");
    }
    let fixed = |at| word(at) as i64;
    Ok(Scores {
        first: u32::from_le_bytes(bytes[FIRST..FIRST + 4].try_into().unwrap()),
        second: u32::from_le_bytes(bytes[SECOND..SECOND + 4].try_into().unwrap()),
        equipment_damage: fixed(EQUIPMENT_DAMAGE),
        province_capture: fixed(PROVINCE_CAPTURE),
        air_damage: fixed(AIR_DAMAGE),
        strategic_bombing: fixed(STRATEGIC_BOMBING),
        sunk_ships: fixed(SUNK_SHIPS),
        sunk_convoys: fixed(SUNK_CONVOYS),
        casualties: fixed(CASUALTIES),
        unknown_casualties: fixed(UNKNOWN_CASUALTIES),
        lend_lease_sent: fixed(LEND_LEASE_SENT),
        lend_lease_received: fixed(LEND_LEASE_RECEIVED),
    })
}

pub fn read_changed_scores(
    memory: &impl Memory,
    ledger: usize,
    changed: u16,
    output: &mut Scores,
) -> Result<(), &'static str> {
    if changed & !ALL_SCORE_FIELDS != 0 {
        return Err("war participation change mask has unknown columns");
    }
    if changed == 0 {
        return Ok(());
    }
    let mut fields = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    }; 10];
    let mut count = 0;
    for (index, &source_offset) in SCORE_FIELDS.iter().enumerate() {
        if changed & (1 << index) != 0 {
            fields[count] = RecordField {
                source_offset,
                destination_offset: index * 8,
                len: 8,
            };
            count += 1;
        }
    }
    let layout = RecordLayout::new(&fields[..count], 80).expect("valid participation field layout");
    let mut bytes = [0_u8; 80];
    if memory.copy_records(RecordSources::Pointers(&[ledger]), &layout, &mut bytes) != count {
        return Err("changed war participation score unreadable");
    }
    let destinations = [
        &mut output.equipment_damage,
        &mut output.province_capture,
        &mut output.air_damage,
        &mut output.strategic_bombing,
        &mut output.sunk_ships,
        &mut output.sunk_convoys,
        &mut output.casualties,
        &mut output.unknown_casualties,
        &mut output.lend_lease_sent,
        &mut output.lend_lease_received,
    ];
    for (index, destination) in destinations.into_iter().enumerate() {
        if changed & (1 << index) != 0 {
            *destination = i64::from_le_bytes(bytes[index * 8..index * 8 + 8].try_into().unwrap());
        }
    }
    Ok(())
}

pub fn read_captured_provinces(
    memory: &impl Memory,
    ledger: usize,
    output: &mut Vec<i32>,
) -> Result<(), &'static str> {
    let address = ledger
        .checked_add(CAPTURED_PROVINCES)
        .ok_or("war captured-province header address overflow")?;
    let bytes =
        read::<[u8; 16]>(memory, address).ok_or("war captured-province header unreadable")?;
    read_captures(
        memory,
        usize::from_le_bytes(bytes[..8].try_into().unwrap()),
        u64::from_le_bytes(bytes[8..].try_into().unwrap()),
        output,
    )
}

fn read_captures(
    memory: &impl Memory,
    head: usize,
    size: u64,
    output: &mut Vec<i32>,
) -> Result<(), &'static str> {
    let node = |pointer| {
        if pointer == 0 {
            return Err("war captured-province tree has a null link");
        }
        read::<[u8; NODE_BYTES]>(memory, pointer)
            .ok_or("war captured-province tree node unreadable")
    };
    let link =
        |bytes: &[u8; NODE_BYTES], at| usize::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let sentinel = node(head)?;
    if sentinel[NODE_NIL] == 0 {
        return Err("war captured-province tree sentinel invalid");
    }
    output.clear();
    let mut remaining = size;
    let mut budget = size
        .checked_mul(8)
        .and_then(|n| n.checked_add(16))
        .ok_or("war captured-province tree size overflow")?;
    let mut step = |pointer| {
        budget = budget
            .checked_sub(1)
            .ok_or("war captured-province tree cycle")?;
        node(pointer)
    };
    let mut current = link(&sentinel, NODE_LEFT);
    while current != head {
        remaining = remaining
            .checked_sub(1)
            .ok_or("war captured-province tree has excess nodes")?;
        let bytes = step(current)?;
        if bytes[NODE_NIL] != 0 {
            return Err("war captured-province tree foreign sentinel");
        }
        let key = i32::from_le_bytes(bytes[NODE_KEY..NODE_KEY + 4].try_into().unwrap());
        if output.last().is_some_and(|&previous| previous >= key) {
            return Err("war captured-province tree key order invalid");
        }
        allocation::reserve(output, 1)
            .map_err(|_| "war captured-province storage allocation failed")?;
        output.push(key);
        let right = link(&bytes, NODE_RIGHT);
        let mut next = step(right)?;
        if next[NODE_NIL] == 0 {
            current = right;
            loop {
                let left = link(&next, NODE_LEFT);
                let candidate = step(left)?;
                if candidate[NODE_NIL] != 0 {
                    break;
                }
                current = left;
                next = candidate;
            }
        } else {
            let mut child = current;
            let mut parent = link(&bytes, NODE_PARENT);
            loop {
                let above = step(parent)?;
                if above[NODE_NIL] != 0 || link(&above, NODE_RIGHT) != child {
                    break;
                }
                child = parent;
                parent = link(&above, NODE_PARENT);
            }
            current = parent;
        }
    }
    if remaining != 0 {
        return Err("war captured-province tree has missing nodes");
    }
    Ok(())
}

pub use game_model::war_participation::DeliveryHistory;

pub const DELIVERY_FIELDS: [usize; 4] = [0x240, 0x248, 0x250, 0x258];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeliverySeedWork {
    pub matrix_entries: usize,
    pub status_entries: usize,
    pub nonzero_histories: usize,
}

pub fn walk_delivery_histories(
    memory: &impl Memory,
    base: usize,
    diplomacy: usize,
    mut target_matches: impl FnMut(u32, u32) -> bool,
    mut visit: impl FnMut(usize, u32, DeliveryHistory) -> Result<(), &'static str>,
) -> Result<DeliverySeedWork, &'static str> {
    let matrix = diplomacy
        .checked_add(engine_native_parts::cdiplomacy_status_layout::RELATIONS)
        .and_then(|p| engine_native_memory::cpdx_array::Array::read(memory, p, 8))
        .ok_or("war lend-lease relation matrix unreadable")?;
    let expected_vtable = engine_sites::at(base, crate::crelation::status::VTABLE_RVA)
        .ok_or("war lend-lease vtable withheld")? as u64;
    let fields = [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        },
        RecordField {
            source_offset: crate::crelation::status::TARGET_COUNTRY,
            destination_offset: 8,
            len: 4,
        },
        RecordField {
            source_offset: crate::crelation::status::DIPLOMACY_STATUS,
            destination_offset: 12,
            len: 8,
        },
        RecordField {
            source_offset: DELIVERY_FIELDS[0],
            destination_offset: 20,
            len: 32,
        },
    ];
    let layout = RecordLayout::new(&fields, 52).unwrap();
    let mut work = DeliverySeedWork::default();
    for start in (0..matrix.count).step_by(64) {
        let count = ((matrix.count - start) as usize).min(64);
        let address = matrix
            .element(start, 8)
            .ok_or("war delivery matrix address overflow")?;
        let mut pointers = [0u8; 64 * 8];
        if !memory.copy(address, &mut pointers[..count * 8]) {
            return Err("war delivery matrix entries unreadable");
        }
        work.matrix_entries += count;
        let mut addresses = [0usize; 64];
        let mut slots = [0u32; 64];
        let mut used = 0;
        for index in 0..count {
            let pointer =
                u64::from_le_bytes(pointers[index * 8..index * 8 + 8].try_into().unwrap()) as usize;
            if pointer != 0 {
                addresses[used] = pointer;
                slots[used] = start + index as u32;
                used += 1;
            }
        }
        if used == 0 {
            continue;
        }
        let mut bytes = [0u8; 64 * 52];
        if memory.copy_records(
            RecordSources::Pointers(&addresses[..used]),
            &layout,
            &mut bytes,
        ) != used * fields.len()
        {
            return Err("war delivery status batch unreadable");
        }
        work.status_entries += used;
        for index in 0..used {
            let record = &bytes[index * 52..index * 52 + 52];
            let word = |at| u64::from_le_bytes(record[at..at + 8].try_into().unwrap());
            if word(0) != expected_vtable || word(12) as usize != diplomacy {
                return Err("war delivery status authentication mismatch");
            }
            let value = DeliveryHistory {
                industry_sent: word(20) as i64,
                fuel_sent: word(28) as i64,
                industry_received: word(36) as i64,
                fuel_received: word(44) as i64,
            };
            if value == DeliveryHistory::default() {
                continue;
            }
            let target = u32::from_le_bytes(record[8..12].try_into().unwrap());
            if !target_matches(target, slots[index]) {
                return Err("war delivery target disagrees with matrix slot");
            }
            visit(addresses[index], target, value)?;
            work.nonzero_histories += 1;
        }
    }
    Ok(work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn participation_delivery_seed_walks_native_entries_once_and_zero_histories_allocate_nothing() {
        use std::cell::RefCell;
        const DIPLOMACY: usize = 0x100;
        const MATRIX: usize = 0x200;
        let mut bytes = vec![0u8; 0x4000];
        let mut word =
            |at: usize, value: u64| bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        word(
            DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::RELATIONS,
            MATRIX as u64,
        );
        word(
            DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::RELATIONS + 8,
            7 | (7u64 << 32),
        );
        for (slot, status, target) in [(2, 0x1000, 22), (3, 0x2000, 33), (5, 0x3000, 55)] {
            word(MATRIX + slot * 8, status as u64);
            word(status, crate::crelation::status::VTABLE_RVA.0 as u64);
            word(status + crate::crelation::status::TARGET_COUNTRY, target);
            word(
                status + crate::crelation::status::DIPLOMACY_STATUS,
                DIPLOMACY as u64,
            );
            if slot != 3 {
                word(status + DELIVERY_FIELDS[0], 3);
                word(status + DELIVERY_FIELDS[2], i64::MIN as u64);
            }
        }
        struct Traced {
            bytes: Vec<u8>,
            reads: RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Traced {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((at, out.len()));
                SliceMemory {
                    base: 0,
                    bytes: &self.bytes,
                }
                .copy(at, out)
            }
        }
        let memory = Traced {
            bytes,
            reads: RefCell::default(),
        };
        let mut output = Vec::new();
        let work = walk_delivery_histories(
            &memory,
            0,
            DIPLOMACY,
            |raw, slot| {
                assert_ne!(slot, 3, "zero histories need no target resolution");
                raw == slot * 11
            },
            |status, target, value| {
                output.push((status, target, value));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            work,
            DeliverySeedWork {
                matrix_entries: 7,
                status_entries: 3,
                nonzero_histories: 2
            }
        );
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].2.industry_sent, 3);
        assert_eq!(output[1].2.industry_received, i64::MIN);
        let reads = memory.reads.borrow();
        assert_eq!(
            reads
                .iter()
                .filter(|(at, _)| (MATRIX..MATRIX + 7 * 8).contains(at))
                .copied()
                .collect::<Vec<_>>(),
            [(MATRIX, 7 * 8)]
        );
        for status in [0x1000, 0x2000, 0x3000] {
            assert_eq!(
                reads
                    .iter()
                    .filter(|&&(at, size)| at == status + DELIVERY_FIELDS[0] && size == 32)
                    .count(),
                1
            );
        }
        drop(reads);
        let mut bytes = memory.bytes;
        for status in [0x1000, 0x3000] {
            bytes[status + DELIVERY_FIELDS[0]..status + DELIVERY_FIELDS[0] + 32].fill(0);
        }
        let zero = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let growths = allocation::growths();
        let work = walk_delivery_histories(
            &zero,
            0,
            DIPLOMACY,
            |_, _| panic!("zero histories need no alias lookup"),
            |_, _, _| panic!("zero histories must not materialize rows"),
        )
        .unwrap();
        assert_eq!(allocation::growths(), growths);
        assert_eq!(work.nonzero_histories, 0);
        bytes[0x1000..0x1008].fill(0);
        assert!(
            walk_delivery_histories(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                DIPLOMACY,
                |_, _| true,
                |_, _, _| Ok(())
            )
            .is_err()
        );
    }

    const LEDGER: usize = 0x100;
    const HEAD: usize = 0x200;
    const NODES: usize = 0x240;

    fn fixture(count: usize) -> Vec<u8> {
        let mut bytes = vec![0; NODES + count * NODE_BYTES];
        let mut word =
            |at: usize, value: u64| bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        word(LEDGER, LEDGER_VTABLE_RVA.0 as u64);
        word(LEDGER + FIRST, 5000 | (1500_u64 << 32));
        for (index, value) in [i64::MAX, -1, 4, -5, 6, 7, 8, 9, 10, i64::MIN]
            .into_iter()
            .enumerate()
        {
            word(LEDGER + EQUIPMENT_DAMAGE + index * 8, value as u64);
        }
        word(LEDGER + CAPTURED_PROVINCES, HEAD as u64);
        word(LEDGER + CAPTURED_PROVINCES + 8, count as u64);
        word(
            HEAD + NODE_LEFT,
            if count == 0 { HEAD } else { NODES } as u64,
        );
        word(
            HEAD + NODE_PARENT,
            if count == 0 { HEAD } else { NODES } as u64,
        );
        word(
            HEAD + NODE_RIGHT,
            if count == 0 {
                HEAD
            } else {
                NODES + (count - 1) * NODE_BYTES
            } as u64,
        );
        for index in 0..count {
            let at = NODES + index * NODE_BYTES;
            word(at + NODE_LEFT, HEAD as u64);
            word(
                at + NODE_PARENT,
                if index == 0 { HEAD } else { at - NODE_BYTES } as u64,
            );
            word(
                at + NODE_RIGHT,
                if index + 1 == count {
                    HEAD
                } else {
                    at + NODE_BYTES
                } as u64,
            );
        }
        bytes[HEAD + NODE_NIL] = 1;
        for index in 0..count {
            bytes[NODES + index * NODE_BYTES + NODE_KEY..NODES + (index + 1) * NODE_BYTES]
                .copy_from_slice(&(index as i32).to_le_bytes());
        }
        bytes
    }

    #[test]
    fn participation_reader_retains_every_component_and_reuses_capture_storage() {
        let bytes = fixture(1500);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut ledger = Ledger::default();
        read_into(&memory, 0, LEDGER, &mut ledger).unwrap();
        assert_eq!((ledger.scores.first, ledger.scores.second), (5000, 1500));
        assert_eq!(ledger.scores.equipment_damage, i64::MAX);
        assert_eq!(ledger.scores.lend_lease_received, i64::MIN);
        assert_eq!(ledger.scores.unknown_casualties, 9);
        assert_eq!(ledger.captured_provinces, (0..1500).collect::<Vec<_>>());
        let allocation = ledger.captured_provinces.as_ptr();
        read_into(&memory, 0, LEDGER, &mut ledger).unwrap();
        assert_eq!(ledger.captured_provinces.as_ptr(), allocation);
        let empty = fixture(0);
        read_into(
            &SliceMemory {
                base: 0,
                bytes: &empty,
            },
            0,
            LEDGER,
            &mut ledger,
        )
        .unwrap();
        assert!(ledger.captured_provinces.is_empty());
        assert_eq!(ledger.captured_provinces.as_ptr(), allocation);
    }

    #[test]
    fn participation_reader_refuses_unreadable_ledgers_and_incomplete_capture_sets() {
        let mut bytes = fixture(2);
        let mut ledger = Ledger::default();
        assert_eq!(
            read_into(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes[..LEDGER + 10]
                },
                0,
                LEDGER,
                &mut ledger
            ),
            Err("war participation ledger unreadable")
        );
        bytes[LEDGER] ^= 1;
        assert_eq!(
            read_into(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                LEDGER,
                &mut ledger
            ),
            Err("war participation ledger vtable mismatch")
        );
        bytes[LEDGER] ^= 1;
        bytes[LEDGER + CAPTURED_PROVINCES + 8..LEDGER + CAPTURED_PROVINCES + 16]
            .copy_from_slice(&3_u64.to_le_bytes());
        assert_eq!(
            read_into(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                LEDGER,
                &mut ledger
            ),
            Err("war captured-province tree has missing nodes")
        );
        bytes[LEDGER + CAPTURED_PROVINCES + 8..LEDGER + CAPTURED_PROVINCES + 16]
            .copy_from_slice(&1_u64.to_le_bytes());
        assert_eq!(
            read_into(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                LEDGER,
                &mut ledger
            ),
            Err("war captured-province tree has excess nodes")
        );
    }

    #[test]
    fn participation_scalar_refresh_never_reads_captured_membership() {
        let bytes = fixture(2);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes[..LEDGER + SCORE_BYTES],
        };
        let scores = read_scores(&memory, 0, LEDGER).unwrap();
        assert_eq!(scores.equipment_damage, i64::MAX);
        assert_eq!(scores.unknown_casualties, 9);
        assert_eq!(
            read_captured_provinces(&memory, LEDGER, &mut Vec::new()),
            Err("war captured-province header unreadable")
        );
    }

    #[test]
    fn participation_changed_columns_read_only_the_demanded_words_and_carry_everything_else() {
        use std::cell::RefCell;
        struct Selected<'a> {
            bytes: &'a [u8],
            reads: RefCell<Vec<usize>>,
            refuse_received: bool,
        }
        impl Memory for Selected<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.reads.borrow_mut().push(address);
                if destination.len() != 8
                    || ![LEDGER + EQUIPMENT_DAMAGE, LEDGER + LEND_LEASE_RECEIVED].contains(&address)
                    || (self.refuse_received && address == LEDGER + LEND_LEASE_RECEIVED)
                {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, destination)
            }
        }
        let bytes = fixture(2);
        let mut memory = Selected {
            bytes: &bytes,
            reads: RefCell::default(),
            refuse_received: false,
        };
        let mut scores = Scores {
            first: 5000,
            second: 1500,
            province_capture: 55,
            unknown_casualties: 66,
            ..Default::default()
        };
        let changed = 1 | (1 << 9);
        read_changed_scores(&memory, LEDGER, changed, &mut scores).unwrap();
        assert_eq!(
            *memory.reads.borrow(),
            [LEDGER + EQUIPMENT_DAMAGE, LEDGER + LEND_LEASE_RECEIVED]
        );
        assert_eq!(
            (scores.equipment_damage, scores.lend_lease_received),
            (i64::MAX, i64::MIN)
        );
        assert_eq!(
            (
                scores.first,
                scores.second,
                scores.province_capture,
                scores.unknown_casualties
            ),
            (5000, 1500, 55, 66)
        );
        memory.reads.borrow_mut().clear();
        read_changed_scores(&memory, LEDGER, 0, &mut scores).unwrap();
        assert!(memory.reads.borrow().is_empty());
        memory.refuse_received = true;
        scores.equipment_damage = 123;
        let before = scores;
        assert_eq!(
            read_changed_scores(&memory, LEDGER, changed, &mut scores),
            Err("changed war participation score unreadable")
        );
        assert_eq!(scores, before);
        assert_eq!(
            read_changed_scores(&memory, LEDGER, ALL_SCORE_FIELDS + 1, &mut scores),
            Err("war participation change mask has unknown columns")
        );
    }

    #[test]
    fn participation_reader_has_no_chosen_capture_or_country_limit() {
        let bytes = fixture(5000);
        let mut ledger = Ledger::default();
        read_into(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            LEDGER,
            &mut ledger,
        )
        .unwrap();
        assert_eq!(ledger.captured_provinces.len(), 5000);
        assert_eq!(ledger.scores.first, 5000);
    }

    #[test]
    fn participation_total_clamps_after_wrapping_and_can_exclude_casualties() {
        let mut score = Scores {
            equipment_damage: 9,
            lend_lease_received: -12,
            casualties: 4,
            unknown_casualties: 2,
            ..Default::default()
        };
        assert_eq!(score.total(false), 3);
        assert_eq!(score.total(true), 0);
        score.equipment_damage = i64::MAX;
        score.lend_lease_received = 0;
        assert_eq!(score.total(false), 0);
    }

    #[test]
    fn participation_aggregation_keeps_the_greatest_unknown_score() {
        let mut score = Scores {
            equipment_damage: i64::MAX,
            casualties: 3,
            unknown_casualties: 7,
            ..Default::default()
        };
        score.add(Scores {
            equipment_damage: 1,
            casualties: 4,
            unknown_casualties: 5,
            lend_lease_received: -2,
            ..Default::default()
        });
        assert_eq!(score.equipment_damage, i64::MIN);
        assert_eq!(score.casualties, 7);
        assert_eq!(score.unknown_casualties, 7);
        assert_eq!(score.lend_lease_received, -2);
    }
}
