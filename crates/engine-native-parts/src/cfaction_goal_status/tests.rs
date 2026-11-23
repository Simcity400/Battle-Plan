use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::Cell;

const FACTION: usize = 0x1000;
const DEFINITIONS: usize = 0x3000;
const GOAL_DATA: usize = 0x4000;
const HISTORY: usize = 0x6000;
const SLOTS: usize = 0x6100;
const DATES: usize = 0x6200;

fn put(bytes: &mut [u8], at: usize, value: impl AsRef<[u8]>) {
    let value = value.as_ref();
    bytes[at..at + value.len()].copy_from_slice(value);
}
fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put(bytes, at, (data as u64).to_le_bytes());
    put(bytes, at + 8, count.to_le_bytes());
    put(bytes, at + 12, count.to_le_bytes());
}
fn date(bytes: &mut [u8], at: usize, hour: i32) {
    put(bytes, at, (cgame_date::VTABLE_RVA.0 as u64).to_le_bytes());
    put(bytes, at + cgame_date::DATE_BASE, hour.to_le_bytes());
    put(
        bytes,
        at + cgame_date::PERSISTENT_BASE,
        (cgame_date::PERSISTENT_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
}
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x7000];
    let status = FACTION + STATUS;
    put(&mut bytes, status, (VTABLE_RVA.0 as u64).to_le_bytes());
    put(&mut bytes, status + OWNER, (FACTION as u64).to_le_bytes());
    put(
        &mut bytes,
        status + MANIFEST,
        (DEFINITIONS as u64).to_le_bytes(),
    );
    for (offset, token) in [(0, 991_u32), (16, 1234)] {
        put(
            &mut bytes,
            DEFINITIONS + offset,
            (DEFINITION_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DEFINITIONS + offset + DEFINITION_TOKEN,
            token.to_le_bytes(),
        );
    }
    header(&mut bytes, status + GOALS, GOAL_DATA, 2);
    for (ordinal, state) in [(0, 3_i32), (1, 4)] {
        let goal = GOAL_DATA + ordinal * GOAL_STRIDE;
        put(
            &mut bytes,
            goal,
            ((DEFINITIONS + ordinal * 16) as u64).to_le_bytes(),
        );
        put(&mut bytes, goal + GOAL_STATUS, state.to_le_bytes());
        put(
            &mut bytes,
            goal + GOAL_PROGRESS,
            (12345_i64 + ordinal as i64).to_le_bytes(),
        );
        date(&mut bytes, goal + GOAL_DATE, -123 + ordinal as i32);
    }
    header(&mut bytes, status + COMPLETED, HISTORY, 3);
    put(&mut bytes, HISTORY, (DEFINITIONS as u64).to_le_bytes());
    put(&mut bytes, HISTORY + 16, (DEFINITIONS as u64).to_le_bytes());
    header(&mut bytes, status + CANCELLED, HISTORY + 8, 1);
    header(&mut bytes, status + EXTRA_SLOTS, SLOTS, 3);
    for (ordinal, value) in [0_i32, -2, 17].into_iter().enumerate() {
        put(&mut bytes, SLOTS + ordinal * 4, value.to_le_bytes());
    }
    header(&mut bytes, status + TERM_DATES, DATES, 3);
    for (ordinal, value) in [cgame_date::NULL_RAW_HOUR, 8, -1].into_iter().enumerate() {
        date(&mut bytes, DATES + ordinal * DATE_STRIDE, value);
    }
    bytes
}

#[test]
fn faction_goal_state_preserves_pending_status_signed_slots_dates_and_history_order() {
    let bytes = fixture();
    let mut output = Snapshot::default();
    read_into(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        FACTION,
        &mut output,
    )
    .unwrap();
    assert_eq!(output.manifest, Some(991));
    assert_eq!(
        output.goals,
        [
            Goal {
                token: 991,
                status: 3,
                progress_raw: 12345,
                date: -123
            },
            Goal {
                token: 1234,
                status: 4,
                progress_raw: 12346,
                date: -122
            }
        ]
    );
    assert_eq!(output.completed, [Some(991), None, Some(991)]);
    assert_eq!(output.cancelled, [None]);
    assert_eq!(output.extra_slots, [0, -2, 17]);
    assert_eq!(output.term_dates, [cgame_date::NULL_RAW_HOUR, 8, -1]);
    let addresses = (
        output.goals.as_ptr(),
        output.completed.as_ptr(),
        output.extra_slots.as_ptr(),
        output.term_dates.as_ptr(),
    );
    read_into(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        FACTION,
        &mut output,
    )
    .unwrap();
    assert_eq!(
        addresses,
        (
            output.goals.as_ptr(),
            output.completed.as_ptr(),
            output.extra_slots.as_ptr(),
            output.term_dates.as_ptr()
        )
    );
}

struct Fault<'a> {
    bytes: &'a [u8],
    address: usize,
    hits: Cell<u32>,
}
impl Memory for Fault<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address <= self.address && self.address < address + output.len() {
            self.hits.set(self.hits.get() + 1);
            false
        } else {
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }
}

#[test]
fn faction_goal_state_faults_are_not_retried_or_published_as_empty() {
    let bytes = fixture();
    for address in [
        FACTION + STATUS,
        FACTION + STATUS + OWNER,
        DEFINITIONS,
        GOAL_DATA + GOAL_STATUS,
        GOAL_DATA + GOAL_DATE,
        HISTORY,
        SLOTS,
        DATES + cgame_date::PERSISTENT_BASE,
    ] {
        let memory = Fault {
            bytes: &bytes,
            address,
            hits: Cell::new(0),
        };
        assert!(read_into(&memory, 0, FACTION, &mut Snapshot::default()).is_err());
        assert_eq!(memory.hits.get(), 1);
    }
}

#[test]
fn faction_goal_state_checks_owner_and_both_date_bases_and_allows_empty_lists() {
    for address in [
        FACTION + STATUS + OWNER,
        GOAL_DATA + GOAL_DATE,
        GOAL_DATA + GOAL_DATE + cgame_date::PERSISTENT_BASE,
    ] {
        let mut bytes = fixture();
        put(&mut bytes, address, 7_u64.to_le_bytes());
        assert!(
            read_into(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                FACTION,
                &mut Snapshot::default()
            )
            .is_err()
        );
    }
    let mut bytes = fixture();
    put(&mut bytes, FACTION + STATUS + MANIFEST, 0_u64.to_le_bytes());
    for offset in [GOALS, COMPLETED, CANCELLED, EXTRA_SLOTS, TERM_DATES] {
        header(&mut bytes, FACTION + STATUS + offset, usize::MAX, 0);
    }
    let mut output = Snapshot::default();
    read_into(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        FACTION,
        &mut output,
    )
    .unwrap();
    assert_eq!(output, Snapshot::default());
}

#[test]
fn faction_goal_state_sizes_history_from_live_demand() {
    let mut bytes = fixture();
    let data = bytes.len();
    bytes.resize(data + 100 * 8, 0);
    header(&mut bytes, FACTION + STATUS + COMPLETED, data, 100);
    for ordinal in 0..100 {
        put(
            &mut bytes,
            data + ordinal * 8,
            (DEFINITIONS as u64).to_le_bytes(),
        );
    }
    let mut output = Snapshot::default();
    read_into(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        FACTION,
        &mut output,
    )
    .unwrap();
    assert_eq!(output.completed, vec![Some(991); 100]);
}

#[test]
fn faction_goal_held_sections_preserve_order_and_signed_invalid_indices() {
    let mut bytes = fixture();
    header(&mut bytes, GOAL_DATA + GOAL_PROGRESS_STATUS, 0x6600, 3);
    for (index, value) in [u32::MAX, 7, 3].into_iter().enumerate() {
        put(&mut bytes, 0x6600 + index * 4, value.to_le_bytes());
    }
    let mut output = Snapshot::default();
    read_into(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        FACTION,
        &mut output,
    )
    .unwrap();
    assert_eq!(output.active_sections[0], [u32::MAX, 7, 3]);
    assert!(output.active_sections[1].is_empty());
}

#[test]
fn faction_goal_structure_and_selected_cache_equal_the_full_reader_without_cross_reads() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut full = Snapshot::default();
    read_into(&memory, 0, FACTION, &mut full).unwrap();
    let mut split = Snapshot::default();
    let structure = Fault {
        bytes: &bytes,
        address: GOAL_DATA + GOAL_PROGRESS,
        hits: Cell::new(0),
    };
    read_structure_into(&structure, 0, FACTION, &mut split).unwrap();
    assert_eq!(structure.hits.get(), 0);
    for address in [
        FACTION + STATUS,
        DEFINITIONS,
        GOAL_DATA + GOAL_STATUS,
        HISTORY,
        SLOTS,
        DATES,
    ] {
        let memory = Fault {
            bytes: &bytes,
            address,
            hits: Cell::new(0),
        };
        for ordinal in 0..split.goals.len() {
            read_progress_into(
                &memory,
                split.goal_addresses[ordinal],
                &mut split.goals[ordinal],
                &mut split.active_sections[ordinal],
            )
            .unwrap();
        }
        assert_eq!(memory.hits.get(), 0);
    }
    assert_eq!(split, full);
}
