use super::*;
use crate::memory::SliceMemory;

struct TracedMemory<'a> {
    bytes: &'a [u8],
    blocked: Option<usize>,
    guards: std::cell::Cell<usize>,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
}

impl Memory for TracedMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        if self
            .blocked
            .is_some_and(|blocked| (address..address + destination.len()).contains(&blocked))
        {
            destination.fill(0xff);
            return false;
        }
        SliceMemory {
            base: 0x1000,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.guards.set(self.guards.get() + 1);
        for (index, (address, destination)) in addresses.into_iter().zip(destinations).enumerate() {
            if destination.is_empty() {
                continue;
            }
            if address == 0
                || address.checked_add(destination.len()).is_none()
                || !self.copy(address, destination)
            {
                return index;
            }
        }
        N
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.guards.set(self.guards.get() + 1);
        struct Scalar<'a, 'b>(&'a TracedMemory<'b>);
        impl Memory for Scalar<'_, '_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.0.copy(address, destination)
            }
        }
        Scalar(self).copy_records(sources, layout, destination)
    }
}

fn put_movement(bytes: &mut [u8], province: usize, id: u32, definition: usize, terrain: usize) {
    use crate::{cterrain_type, cprovince::definition as definition_layout};
    put(
        bytes,
        province + DEFINITION,
        &(definition as u64 + 0x1000).to_le_bytes(),
    );
    put(bytes, definition + definition_layout::ID, &id.to_le_bytes());
    put(
        bytes,
        definition + definition_layout::TERRAIN,
        &(terrain as u64 + 0x1000).to_le_bytes(),
    );
    put(
        bytes,
        definition + definition_layout::TERRAIN_FLAGS,
        &[(id % 4) as u8],
    );
    put(
        bytes,
        terrain,
        &(cterrain_type::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        bytes,
        terrain + cterrain_type::DATABASE_INDEX,
        &7_i32.to_le_bytes(),
    );
}

fn many_provinces(count: usize) -> Vec<u8> {
    let mut bytes = vec![0; 0x14000];
    put(
        &mut bytes,
        cgame_state::PROVINCES,
        &0x1300_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        cgame_state::PROVINCES + 8,
        &(count as u32 + 1).to_le_bytes(),
    );
    put(
        &mut bytes,
        cgame_state::PROVINCES + 12,
        &(count as u32 + 1).to_le_bytes(),
    );
    for index in 1..=count {
        let offset = 0x1000 + (index - 1) * 0x200;
        put(
            &mut bytes,
            0x300 + index * 8,
            &(offset as u64 + 0x1000).to_le_bytes(),
        );
        put(&mut bytes, offset, &(VTABLE_RVA.0 as u64).to_le_bytes());
        put(&mut bytes, offset + ID, &(index as u32).to_le_bytes());
        put(&mut bytes, offset + STATE, &0x13000_u64.to_le_bytes());
        put(
            &mut bytes,
            offset + CONTROLLER,
            &(u32::MAX - index as u32).to_le_bytes(),
        );
        put_movement(
            &mut bytes,
            offset,
            index as u32,
            0xa000 + (index - 1) * 0x100,
            0xf000,
        );
    }
    put(
        &mut bytes,
        0x12000,
        &(cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, 0x12000 + cstate::ID, &89_u32.to_le_bytes());
    put(&mut bytes, 0x12000 + cstate::OWNER, &u32::MAX.to_le_bytes());
    bytes
}

#[test]
fn owned_border_conflict_rows_and_groups_grow_from_original_tree_header() {
    let memory = conflict_fixture();
    let mut capture = Capture::default();
    assert_eq!(conflict_group_demand(&memory, 0, 0x1000), Ok(2));
    assert_eq!(
        capture.walk(&memory, 0, 0x1000),
        Outcome::Rows { len: 3 }
    );
    assert_eq!(capture.conflict_walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(capture.conflict_walk.groups_len, 1);
    assert!(capture.groups.len() >= 2);
    assert_eq!(capture.conflicts[0].province, 2);
}

fn province_names_fixture() -> Vec<u8> {
    let mut bytes = many_provinces(3);
    for index in 0..3 {
        put(
            &mut bytes,
            0x1000 + index * 0x200 + NAME_OVERRIDE + 24,
            &15_u64.to_le_bytes(),
        );
    }
    let inline = 0x1200 + NAME_OVERRIDE;
    put(&mut bytes, inline, &[b'A', 0, 255, 0]);
    put(&mut bytes, inline + 16, &3_u64.to_le_bytes());
    let heap = 0x1400 + NAME_OVERRIDE;
    put(&mut bytes, heap, &0x9000_u64.to_le_bytes());
    put(&mut bytes, heap + 16, &6_u64.to_le_bytes());
    put(&mut bytes, heap + 24, &31_u64.to_le_bytes());
    put(&mut bytes, 0x8000, b"Sample\0");
    bytes
}

#[test]
fn name_census_is_sparse_lossless_and_grows_from_any_starting_storage() {
    let bytes = province_names_fixture();
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    for (row_capacity, byte_capacity) in [(2, 9), (1, 9), (2, 3), (0, 0)] {
        let mut rows = vec![NameRow::default(); row_capacity];
        let mut arena = vec![0; byte_capacity];
        let report = walk_names(&memory, 0, 0x1000, &mut rows, &mut arena);
        assert_eq!(report.required_bytes, 9);
        assert_eq!(report.outcome, Outcome::Rows { len: 2 });
        assert_eq!(
            rows[0],
            NameRow {
                province: 2,
                start: 0,
                len: 3
            }
        );
        assert_eq!(&arena[..3], &[b'A', 0, 255]);
        assert_eq!(&arena[3..9], b"Sample");
    }
    let counted = count_names(&memory, 0, 0x1000);
    assert_eq!(counted.required_bytes, 9);
    assert_eq!(
        counted.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
}

#[test]
fn names_require_a_real_census_even_when_no_overrides_exist() {
    let mut bytes = province_names_fixture();
    put(&mut bytes, 0x310, &0_u64.to_le_bytes());
    put(&mut bytes, 0x318, &0_u64.to_le_bytes());
    let outcome = |bytes: &[u8]| {
        walk_names(
            &SliceMemory {
                base: 0x1000,
                bytes,
            },
            0,
            0x1000,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .outcome
    };
    assert_eq!(outcome(&bytes), Outcome::Rows { len: 0 });
    put(&mut bytes, 0x308, &0_u64.to_le_bytes());
    assert_eq!(
        outcome(&bytes),
        Outcome::Unavailable {
            reason: "province names census contains no provinces",
        }
    );
}

#[test]
fn names_reject_bad_identity_header_and_terminator_including_after_capacity() {
    for (offset, value) in [
        (0x1400, 0_u64),
        (0x1400 + ID, 4),
        (0x1400 + NAME_OVERRIDE + 16, 32),
        (0x1400 + NAME_OVERRIDE, 0),
        (0x8006, 1),
    ] {
        let mut bytes = province_names_fixture();
        put(&mut bytes, offset, &value.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let report = walk_names(&memory, 0, 0x1000, &mut Vec::new(), &mut Vec::new());
        assert!(
            matches!(report.outcome, Outcome::Unavailable { .. }),
            "offset {offset:x}"
        );
    }
}

#[test]
fn exact_field_batches_cover_boundaries_and_shared_states_are_read_once_per_walk() {
    let bytes = many_provinces(67);
    let memory = TracedMemory {
        bytes: &bytes,
        blocked: None,
        guards: std::cell::Cell::new(0),
        reads: std::cell::RefCell::new(Vec::new()),
    };
    let mut rows = vec![Row::default(); 67];
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut rows),
        Outcome::Rows { len: 67 }
    );
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            *row,
            Row {
                id: index as u32 + 1,
                state: 89,
                owner: u32::MAX,
                controller: u32::MAX - index as u32 - 1,
                victory_points: 0,
                fort_positive: 0,
                move_gate: u8::from((index + 1) % 4 == 1) * Row::MOVE_NULL_OWNER_AREA,
                port_sea: 0,
                fort_bunker: None,
                controller_area: None,
                owner_area: None,
                area_node: None,
            }
        );
    }
    assert_eq!(memory.guards.get(), 9);
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(address, _)| address == 0x13000)
            .count(),
        1
    );
    memory.guards.set(0);
    memory.reads.borrow_mut().clear();
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut rows),
        Outcome::Rows { len: 67 }
    );
    assert_eq!(memory.guards.get(), 9);
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(address, _)| address == 0x13000)
            .count(),
        1
    );
}

#[test]
fn batching_preserves_holes_and_earlier_errors_before_later_prefetch_faults() {
    let mut bytes = many_provinces(67);
    put(&mut bytes, 0x300 + 2 * 8, &0_u64.to_le_bytes());
    put(&mut bytes, 0x12000 + cstate::ID, &0_u32.to_le_bytes());
    let memory = TracedMemory {
        bytes: &bytes,
        blocked: Some(0x2400),
        guards: std::cell::Cell::new(0),
        reads: std::cell::RefCell::new(Vec::new()),
    };
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut vec![Row::default(); 67]),
        Outcome::Unavailable {
            reason: "non-null state has zero id"
        }
    );

    put(&mut bytes, 0x1000 + ID, &99_u32.to_le_bytes());
    let memory = TracedMemory {
        bytes: &bytes,
        blocked: Some(0x1300 + 4 * 8),
        guards: std::cell::Cell::new(0),
        reads: std::cell::RefCell::new(Vec::new()),
    };
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut vec![Row::default(); 67]),
        Outcome::Unavailable {
            reason: "province id differs from slot"
        }
    );
}

#[test]
fn state_cache_full_pointer_keys_survive_collisions_and_failures_are_not_cached() {
    use std::cell::Cell;
    let first = 0x13000_usize;
    let second = (0x14000_usize..0x1000000)
        .step_by(16)
        .find(|&pointer| {
            StateCache::new(2048).unwrap().slot(pointer)
                == StateCache::new(2048).unwrap().slot(first)
        })
        .unwrap();
    struct States {
        pointers: [usize; 2],
        fail: Cell<bool>,
        owner_reads: Cell<usize>,
    }
    impl Memory for States {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (index, &pointer) in self.pointers.iter().enumerate() {
                if address == pointer {
                    destination.copy_from_slice(&(cstate::VTABLE_RVA.0 as u64).to_le_bytes());
                    return true;
                }
                if address == pointer + cstate::ID {
                    destination.copy_from_slice(&(index as u32 + 1).to_le_bytes());
                    return true;
                }
                if address == pointer + cstate::OWNER {
                    self.owner_reads.set(self.owner_reads.get() + 1);
                    if self.fail.get() {
                        return false;
                    }
                    destination.copy_from_slice(&(u32::MAX - index as u32).to_le_bytes());
                    return true;
                }
            }
            false
        }
    }
    let memory = States {
        pointers: [first, second],
        fail: Cell::new(false),
        owner_reads: Cell::new(0),
    };
    let mut cache = StateCache::new(2048).unwrap();
    assert_eq!(cache.resolve(&memory, 0, first), Ok((1, u32::MAX)));
    assert_eq!(cache.resolve(&memory, 0, second), Ok((2, u32::MAX - 1)));
    assert_eq!(cache.resolve(&memory, 0, first), Ok((1, u32::MAX)));
    assert_eq!(memory.owner_reads.get(), 2);
    assert_eq!(cache.resolve(&memory, 0, first), Ok((1, u32::MAX)));
    assert_eq!(cache.resolve(&memory, 0, second), Ok((2, u32::MAX - 1)));
    assert_eq!(memory.owner_reads.get(), 2);
    let mut cache = StateCache::new(2048).unwrap();
    memory.fail.set(true);
    assert_eq!(
        cache.resolve(&memory, 0, second),
        Err("state owner unreadable")
    );
    memory.fail.set(false);
    assert_eq!(cache.resolve(&memory, 0, second), Ok((2, u32::MAX - 1)));
    assert_eq!(cache.resolve(&memory, 0, second), Ok((2, u32::MAX - 1)));
    assert_eq!(memory.owner_reads.get(), 4);
    assert!(std::mem::size_of::<StateCache>() <= 32 * 1024);
}

#[test]
fn state_cache_retains_collisions_beyond_the_former_probe_limit() {
    struct States(std::cell::Cell<usize>);
    impl Memory for States {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            match address & 0xfff {
                0 => destination.copy_from_slice(&(cstate::VTABLE_RVA.0 as u64).to_le_bytes()),
                cstate::ID => destination.copy_from_slice(&((address >> 12) as u32).to_le_bytes()),
                cstate::OWNER => {
                    self.0.set(self.0.get() + 1);
                    destination.copy_from_slice(&123_u32.to_le_bytes());
                }
                _ => return false,
            }
            true
        }
    }
    let mut cache = StateCache::new(2048).unwrap();
    let pointers: Vec<_> = (1_usize..1_000_000)
        .map(|id| id * 0x1000)
        .filter(|&pointer| cache.slot(pointer) == cache.slot(0x1000))
        .take(33)
        .collect();
    assert_eq!(pointers.len(), 33);
    let memory = States(std::cell::Cell::new(0));
    for &pointer in pointers.iter().chain(pointers.iter().rev()) {
        assert_eq!(
            cache.resolve(&memory, 0, pointer),
            Ok(((pointer >> 12) as u32, 123))
        );
    }
    assert_eq!(memory.0.get(), pointers.len());
}

#[test]
fn cache_does_not_claim_to_detect_later_state_changes_but_each_walk_reads_fresh() {
    use std::cell::Cell;
    struct ChangingOwner<'a> {
        bytes: &'a [u8],
        reads: Cell<u32>,
    }
    impl Memory for ChangingOwner<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == 0x13000 + cstate::OWNER {
                self.reads.set(self.reads.get() + 1);
                destination.copy_from_slice(&self.reads.get().to_le_bytes());
                return true;
            }
            SliceMemory {
                base: 0x1000,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }
    let bytes = many_provinces(3);
    let memory = ChangingOwner {
        bytes: &bytes,
        reads: Cell::new(0),
    };
    for expected in [1, 2] {
        let mut rows = vec![Row::default(); 3];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Rows { len: 3 }
        );
        assert!(rows.iter().all(|row| row.owner == expected));
    }
    assert_eq!(memory.reads.get(), 2);
}

fn put(bytes: &mut [u8], offset: usize, data: &[u8]) {
    bytes[offset..offset + data.len()].copy_from_slice(data);
}

fn provinces() -> Vec<u8> {
    let mut bytes = vec![0; 0x1000];
    put(
        &mut bytes,
        cgame_state::PROVINCES,
        &0x1300_u64.to_le_bytes(),
    );
    put(&mut bytes, cgame_state::PROVINCES + 8, &3_u32.to_le_bytes());
    put(
        &mut bytes,
        cgame_state::PROVINCES + 12,
        &3_u32.to_le_bytes(),
    );
    put(&mut bytes, 0x300, &u64::MAX.to_le_bytes());
    put(&mut bytes, 0x308, &0x1400_u64.to_le_bytes());
    put(&mut bytes, 0x310, &0x1600_u64.to_le_bytes());
    for (offset, id, state, controller) in [
        (0x400, 1_u32, 0x1a00_u64, 0xf123_4567_u32),
        (0x600, 2_u32, 0_u64, 0_u32),
    ] {
        put(&mut bytes, offset, &(VTABLE_RVA.0 as u64).to_le_bytes());
        put(&mut bytes, offset + ID, &id.to_le_bytes());
        put(&mut bytes, offset + STATE, &state.to_le_bytes());
        put(&mut bytes, offset + CONTROLLER, &controller.to_le_bytes());
        put_movement(
            &mut bytes,
            offset,
            id,
            0x800 + (id as usize - 1) * 0x100,
            0xb00,
        );
    }
    put(
        &mut bytes,
        0xa00,
        &(cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, 0xa00 + cstate::ID, &99_u32.to_le_bytes());
    put(
        &mut bytes,
        0xa00 + cstate::OWNER,
        &0xfedc_ba98_u32.to_le_bytes(),
    );
    bytes
}

struct FaultMemory<'a> {
    bytes: &'a [u8],
    blocked: std::ops::Range<usize>,
}

impl Memory for FaultMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address < self.blocked.end && address + destination.len() > self.blocked.start {
            destination[0] = 0xff;
            return false;
        }
        SliceMemory {
            base: 0x1000,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }
}

#[test]
fn scattered_fields_preserve_native_values_without_reading_struct_gaps() {
    let bytes = provinces();
    for blocked in [
        0x1408..0x1438,
        0x143c..0x14a4,
        0x14a8..0x14b0,
        0x1a08..0x1a58,
        0x1a5c..0x1ac8,
    ] {
        let memory = FaultMemory {
            bytes: &bytes,
            blocked,
        };
        let mut rows = vec![Row::default(); 2];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            rows,
            [
                Row {
                    id: 1,
                    state: 99,
                    owner: 0xfedc_ba98,
                    controller: 0xf123_4567,
                    victory_points: 0,
                    fort_positive: 0,
                    move_gate: Row::MOVE_NULL_OWNER_AREA,
                    port_sea: 0,
                    fort_bunker: None,
                    controller_area: None,
                    owner_area: None,
                    area_node: None,
                },
                Row {
                    id: 2,
                    state: 0,
                    owner: 0,
                    controller: 0,
                    victory_points: 0,
                    fort_positive: 0,
                    move_gate: 0,
                    port_sea: 0,
                    fort_bunker: None,
                    controller_area: None,
                    owner_area: None,
                    area_node: None,
                },
            ]
        );
    }
}

#[test]
fn the_walk_gathers_both_area_pointers_with_the_movement_fields() {
    let mut bytes = provinces();
    put(
        &mut bytes,
        0x400 + CONTROLLER_AREA,
        &0x5000_u64.to_le_bytes(),
    );
    put(&mut bytes, 0x400 + OWNER_AREA, &0x6000_u64.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut capture = Capture::try_with_capacity(2, 2, 0).unwrap();
    assert_eq!(capture.walk(&memory, 0, 0x1000), Outcome::Rows { len: 2 });
    assert_eq!(
        capture.areas[..2],
        [
            crate::cprovince::controller_areas::Pointers {
                province: 0x1400,
                definition: 0x1800,
                controller: 0x5000,
                owner: 0x6000,
            },
            crate::cprovince::controller_areas::Pointers {
                province: 0x1600,
                definition: 0x1900,
                controller: 0,
                owner: 0,
            },
        ]
    );
    assert_eq!(capture.rows[0].move_gate & Row::MOVE_NULL_OWNER_AREA, 0);
    assert_eq!(capture.rows[0].controller_area, None);
}

#[test]
fn each_unreadable_field_keeps_its_existing_reason() {
    let bytes = provinces();
    for (address, reason) in [
        (0x1400, "province vtable mismatch"),
        (0x14a4, "province id unreadable"),
        (0x14c0, "province state unreadable"),
        (0x1588, "province controller unreadable"),
        (0x1438, "province victory points unreadable"),
        (0x1a00, "state vtable mismatch"),
        (0x1a58, "state id unreadable"),
        (0x1ac8, "state owner unreadable"),
    ] {
        let memory = FaultMemory {
            bytes: &bytes,
            blocked: address..address + 1,
        };
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut vec![Row::default(); 2]),
            Outcome::Unavailable { reason }
        );
    }
}

#[test]
fn current_victory_points_preserve_signed_word_and_transition_to_zero() {
    let mut bytes = provinces();
    put(
        &mut bytes,
        0x400 + VICTORY_POINTS + 4,
        &u32::MAX.to_le_bytes(),
    );
    let mut rows = vec![Row::default(); 2];
    for value in [i32::MAX, i32::MIN, -1, 0] {
        put(&mut bytes, 0x400 + VICTORY_POINTS, &value.to_le_bytes());
        let memory = FaultMemory {
            bytes: &bytes,
            blocked: 0x400 + 0x1000 + VICTORY_POINTS + 4..0x400 + 0x1000 + VICTORY_POINTS + 8,
        };
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].victory_points, value);
        assert_eq!(rows[1].victory_points, 0);
    }
}

#[test]
fn earlier_semantic_mismatches_precede_later_copy_failures() {
    for (offset, value, blocked, reason) in [
        (0x400, 0_u32, 0x1588, "province vtable mismatch"),
        (0x4a4, 3_u32, 0x14c0, "province id differs from slot"),
        (0xa00, 0_u32, 0x1ac8, "state vtable mismatch"),
        (0xa58, 0_u32, 0x1ac8, "non-null state has zero id"),
    ] {
        let mut bytes = provinces();
        put(&mut bytes, offset, &value.to_le_bytes());
        let memory = FaultMemory {
            bytes: &bytes,
            blocked: blocked..blocked + 1,
        };
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut vec![Row::default(); 2]),
            Outcome::Unavailable { reason }
        );
    }
}

#[test]
fn every_province_pointer_and_row_is_checked() {
    let bytes = provinces();
    let memory = FaultMemory {
        bytes: &bytes,
        blocked: 0x1600..0x1608,
    };
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut vec![Row::default(); 2]),
        Outcome::Unavailable {
            reason: "province vtable mismatch"
        }
    );
    let memory = FaultMemory {
        bytes: &bytes,
        blocked: 0x1310..0x1318,
    };
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut vec![Row::default(); 1]),
        Outcome::Unavailable {
            reason: "table pointer unreadable"
        }
    );
}

#[test]
fn address_overflow_keeps_the_original_field_and_semantic_precedence() {
    struct SparseMemory<'a> {
        bytes: &'a [u8],
        fields: &'a [(usize, &'a [u8])],
    }
    impl Memory for SparseMemory<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if let Some((_, source)) = self
                .fields
                .iter()
                .find(|(at, source)| *at == address && source.len() == destination.len())
            {
                destination.copy_from_slice(source);
                true
            } else {
                SliceMemory {
                    base: 0x1000,
                    bytes: self.bytes,
                }
                .copy(address, destination)
            }
        }
    }
    let mut bytes = provinces();
    let pointer = usize::MAX - 0x100;
    put(&mut bytes, 0x308, &(pointer as u64).to_le_bytes());
    let vtable = (VTABLE_RVA.0 as u64).to_le_bytes();
    let no_state = 0_u64.to_le_bytes();
    for (id, reason) in [
        (1_u32, "province address overflow"),
        (2_u32, "province id differs from slot"),
    ] {
        let id = id.to_le_bytes();
        let fields: &[(usize, &[u8])] = &[
            (pointer, &vtable),
            (pointer + ID, &id),
            (pointer + STATE, &no_state),
        ];
        let memory = SparseMemory {
            bytes: &bytes,
            fields,
        };
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut vec![Row::default(); 2]),
            Outcome::Unavailable { reason }
        );
    }
    let mut bytes = provinces();
    let pointer = usize::MAX - 0xa0;
    put(&mut bytes, 0x400 + STATE, &(pointer as u64).to_le_bytes());
    let vtable = (cstate::VTABLE_RVA.0 as u64).to_le_bytes();
    for (id, reason) in [
        (99_u32, "state owner unreadable"),
        (0_u32, "non-null state has zero id"),
    ] {
        let id = id.to_le_bytes();
        let fields: &[(usize, &[u8])] = &[(pointer, &vtable), (pointer + cstate::ID, &id)];
        let memory = SparseMemory {
            bytes: &bytes,
            fields,
        };
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut vec![Row::default(); 2]),
            Outcome::Unavailable { reason }
        );
    }
}

#[test]
fn movement_operands_sum_native_forts() {
    use crate::{cbuilding, cbuilding_template};
    let build = |first: i16, second: i16, weight: i32| {
        let mut bytes = provinces();
        bytes.resize(0x4000, 0);
        put(&mut bytes, 0x5e0, &0x2000_u64.to_le_bytes());
        put(&mut bytes, 0x5e8, &2_u32.to_le_bytes());
        put(&mut bytes, 0x5ec, &2_u32.to_le_bytes());
        put(&mut bytes, 0x1000, &0x2100_u64.to_le_bytes());
        put(&mut bytes, 0x1008, &0x2400_u64.to_le_bytes());
        for (building, healthy) in [(0x1100, first), (0x1400, second)] {
            put(
                &mut bytes,
                building + cbuilding::HEALTHY_LEVEL,
                &healthy.to_le_bytes(),
            );
            put(
                &mut bytes,
                building + cbuilding::TEMPLATE,
                &0x3000_u64.to_le_bytes(),
            );
        }
        put(
            &mut bytes,
            0x2000 + cbuilding_template::MOVEMENT_FORT_WEIGHT,
            &weight.to_le_bytes(),
        );
        bytes
    };
    for (first, second, weight, fort_positive) in
        [(2, -1, 3, 1), (1, -1, 3, 0), (0, 0, 3, 0), (5, 0, 0, 0)]
    {
        let bytes = build(first, second, weight);
        let mut rows = vec![Row::default(); 2];
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000,
                &mut rows
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].fort_positive, fort_positive);
        assert_eq!(rows[1].fort_positive, 0);
    }
}

#[test]
fn fort_scratch_keeps_exact_pointer_field_and_fault_order_across_short_batches() {
    use crate::{cbuilding, cbuilding_template};
    const POINTERS: usize = 0x2000;
    const BUILDINGS: usize = 0x4000;
    const TEMPLATE: usize = 0x18000;
    let weight = TEMPLATE + cbuilding_template::MOVEMENT_FORT_WEIGHT;
    let mut bytes = vec![0_u8; 0x19000];
    for index in 0..65 {
        let building = BUILDINGS + index * 0x200;
        put(
            &mut bytes,
            POINTERS - 0x1000 + index * 8,
            &(building as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            building - 0x1000 + cbuilding::HEALTHY_LEVEL,
            &1_i16.to_le_bytes(),
        );
        put(
            &mut bytes,
            building - 0x1000 + cbuilding::TEMPLATE,
            &(TEMPLATE as u64).to_le_bytes(),
        );
    }
    put(&mut bytes, weight - 0x1000, &3_i32.to_le_bytes());
    let array = |count| Array {
        data: POINTERS,
        count,
    };
    let expected = |count: usize| {
        let mut reads = Vec::new();
        for first in (0..count).step_by(BATCH_ROWS) {
            let end = count.min(first + BATCH_ROWS);
            reads.extend((first..end).map(|index| {
                (
                    POINTERS + index * 8,
                    8,
                    "province building pointer unreadable",
                )
            }));
            for index in first..end {
                let building = BUILDINGS + index * 0x200;
                reads.extend([
                    (
                        building + cbuilding::HEALTHY_LEVEL,
                        2,
                        "province building fields unreadable",
                    ),
                    (
                        building + cbuilding::TEMPLATE,
                        8,
                        "province building fields unreadable",
                    ),
                ]);
            }
            if first == 0 {
                reads.push((weight, 4, "province building fort weight unreadable"));
            }
        }
        reads
    };
    for count in [0, 1, 2, 63, 64, 65] {
        let trace = expected(count);
        let addresses: Vec<_> = trace
            .iter()
            .map(|&(address, len, _)| (address, len))
            .collect();
        let memory = dense_trace(&bytes, None);
        let fort_sum = MovementBatch::new().fort_sum(&memory, array(count as u32));
        assert_eq!(fort_sum, Ok(count as i32 * 3));
        assert_eq!(
            fort_sum.map(|sum| u8::from(sum > 0)),
            Ok(u8::from(count != 0))
        );
        assert_eq!(*memory.reads.borrow(), addresses);
        for (index, &(address, len, reason)) in trace.iter().enumerate() {
            for fault in [address, address + len - 1] {
                let memory = dense_trace(&bytes, Some(fault));
                let mut movement = MovementBatch::new();
                movement.building_pointers.fill(usize::MAX);
                movement.building_indirect.fill(0xff);
                movement.building_fields.fill(0xff);
                assert_eq!(movement.fort_sum(&memory, array(count as u32)), Err(reason));
                assert_eq!(*memory.reads.borrow(), addresses[..=index]);
            }
        }
    }
    let mut movement = MovementBatch::new();
    for count in [65, 1, 0, 2, 64, 1] {
        let memory = dense_trace(&bytes, None);
        let fort_sum = movement.fort_sum(&memory, array(count));
        assert_eq!(fort_sum, Ok(count as i32 * 3));
        assert_eq!(
            fort_sum.map(|sum| u8::from(sum > 0)),
            Ok(u8::from(count != 0))
        );
        let addresses: Vec<_> = expected(count as usize)
            .into_iter()
            .filter(|&(address, _, _)| count == 65 || address != weight)
            .map(|(address, len, _)| (address, len))
            .collect();
        assert_eq!(*memory.reads.borrow(), addresses);
    }
    put(
        &mut bytes,
        BUILDINGS - 0x1000 + cbuilding::HEALTHY_LEVEL,
        &(-1_i16).to_le_bytes(),
    );
    let fort_sum = movement.fort_sum(&dense_trace(&bytes, None), array(1));
    assert_eq!(fort_sum, Ok(-3));
    assert_eq!(fort_sum.map(|sum| u8::from(sum > 0)), Ok(0));
    for (data, count, reason) in [
        (usize::MAX, 2, "province building index invalid"),
        (usize::MAX - 7, 1, "province building pointer unreadable"),
        (0, 1, "province building pointer unreadable"),
    ] {
        let memory = dense_trace(&bytes, None);
        assert_eq!(
            movement.fort_sum(&memory, Array { data, count }),
            Err(reason)
        );
        assert!(memory.reads.borrow().is_empty());
    }
}

#[test]
fn ownership_comes_from_the_state_and_raw_controller_zero_is_preserved() {
    let mut bytes = vec![0_u8; 0x800];
    let put = |bytes: &mut [u8], offset: usize, data: &[u8]| {
        bytes[offset..offset + data.len()].copy_from_slice(data)
    };
    put(&mut bytes, 0x2b0, &0x1300_u64.to_le_bytes());
    put(&mut bytes, 0x2b8, &2_u32.to_le_bytes());
    put(&mut bytes, 0x2bc, &2_u32.to_le_bytes());
    put(&mut bytes, 0x308, &0x1400_u64.to_le_bytes());
    put(&mut bytes, 0x400, &(VTABLE_RVA.0 as u64).to_le_bytes());
    put(&mut bytes, 0x4a4, &1_u32.to_le_bytes());
    put(&mut bytes, 0x4c0, &0x1600_u64.to_le_bytes());
    put(
        &mut bytes,
        0x600,
        &(cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, 0x658, &7_u32.to_le_bytes());
    put(&mut bytes, 0x6c8, &42_u32.to_le_bytes());
    put_movement(&mut bytes, 0x400, 1, 0x100, 0x700);
    let mut rows = vec![Row::default(); 2];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut rows),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(
        rows[0],
        Row {
            id: 1,
            state: 7,
            owner: 42,
            controller: 0,
            victory_points: 0,
            fort_positive: 0,
            move_gate: Row::MOVE_NULL_OWNER_AREA,
            port_sea: 0,
            fort_bunker: None,
            controller_area: None,
            owner_area: None,
            area_node: None,
        }
    );
    bytes[0x400] ^= 1;
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut rows),
        Outcome::Unavailable {
            reason: "province vtable mismatch"
        }
    );
    bytes[0x400] ^= 1;
    bytes[0x308..0x310].fill(0);
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut rows),
        Outcome::Unavailable {
            reason: "province table has no readable provinces"
        }
    );
    bytes[0x2bc..0x2c0].copy_from_slice(&1_u32.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut rows),
        Outcome::Unavailable {
            reason: "province table has no province slots"
        }
    );
}
fn scattered_reference_read(
    batch: &mut ProvinceBatch,
    memory: &impl Memory,
    array: Array,
    first: u32,
    capacity: usize,
) {
    batch.len = ((array.count - first) as usize).min(BATCH_ROWS);
    let addresses = std::array::from_fn(|slot| {
        if slot < batch.len {
            array.element(first + slot as u32, 8).unwrap_or(0)
        } else {
            0
        }
    });
    let mut destinations = batch.pointers.each_mut().map(|bytes| bytes.as_mut_slice());
    for destination in &mut destinations[batch.len..] {
        *destination = &mut [];
    }
    batch.pointer_count = memory
        .copy_scattered(addresses, &mut destinations)
        .min(batch.len);

    let mut remaining = capacity;
    let enabled: [bool; BATCH_ROWS] = std::array::from_fn(|slot| {
        if slot >= batch.pointer_count || batch.pointer(slot) == 0 || remaining == 0 {
            false
        } else {
            remaining -= 1;
            true
        }
    });
    if !enabled.iter().any(|&value| value) {
        batch.field_count = BATCH_ROWS * FIELDS_PER_ROW;
        return;
    }
    let addresses: [usize; BATCH_ROWS * FIELDS_PER_ROW] = std::array::from_fn(|field| {
        let slot = field / FIELDS_PER_ROW;
        if enabled[slot] {
            batch
                .pointer(slot)
                .checked_add([0, ID, STATE, CONTROLLER, VICTORY_POINTS][field % FIELDS_PER_ROW])
                .unwrap_or(0)
        } else {
            0
        }
    });
    let mut destinations = batch
        .fields
        .each_mut()
        .into_iter()
        .enumerate()
        .map(|(field, bytes)| {
            &mut bytes[..if enabled[field / FIELDS_PER_ROW] {
                [8, 4, 8, 4, 4][field % FIELDS_PER_ROW]
            } else {
                0
            }]
        });
    batch.field_count = memory.copy_scattered(
        addresses,
        &mut std::array::from_fn(|_| destinations.next().unwrap()),
    );
}

fn scattered_reference_walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
) -> Outcome {
    let Some(array) = game_state
        .checked_add(cgame_state::PROVINCES)
        .and_then(|p| Array::read(memory, p, 8))
    else {
        return Outcome::Unavailable {
            reason: "province array unreadable or invalid",
        };
    };
    if array.count < 2 {
        return Outcome::Unavailable {
            reason: "province table has no province slots",
        };
    }
    let mut states = StateCache::new(2048).unwrap();
    let mut batch = ProvinceBatch::new();
    let mut movement = MovementBatch::new();
    let mut pointers = [0_usize; BATCH_ROWS];
    let mut ids = [0_u32; BATCH_ROWS];
    let mut fields = [MovementFields::default(); BATCH_ROWS];
    let mut first = 1;
    let mut len = 0;
    let mut unread_rows = 0;
    while first < array.count {
        scattered_reference_read(&mut batch, memory, array, first, output.len() - len);
        let admitted = len;
        let mut staged = 0;
        for slot in 0..batch.pointer_count {
            if batch.pointer(slot) == 0 {
                continue;
            }
            if len == output.len() {
                unread_rows += 1;
                continue;
            }
            match batch.row(memory, base, first + slot as u32, slot, &mut states) {
                Ok(row) => {
                    output[len] = row;
                    pointers[staged] = batch.pointer(slot);
                    ids[staged] = row.id;
                    staged += 1;
                    len += 1;
                }
                Err(reason) => return Outcome::Unavailable { reason },
            }
        }
        if staged != 0
            && let Err(reason) = movement.read(
                memory,
                &pointers[..staged],
                &ids[..staged],
                &mut fields[..staged],
            )
        {
            return Outcome::Unavailable { reason };
        }
        for (row, fields) in output[admitted..len].iter_mut().zip(&fields[..staged]) {
            row.fort_positive = u8::from(fields.fort_sum > 0);
            row.move_gate = fields.move_gate;
            row.port_sea = fields.port_sea;
        }
        if batch.pointer_count < batch.len {
            return Outcome::Unavailable {
                reason: if array
                    .element(first + batch.pointer_count as u32, 8)
                    .is_none()
                {
                    "table address overflow"
                } else {
                    "table pointer unreadable"
                },
            };
        }
        first += batch.len as u32;
    }
    if unread_rows != 0 {
        Outcome::Truncated { len, unread_rows }
    } else if len == 0 {
        Outcome::Unavailable {
            reason: "province table has no readable provinces",
        }
    } else {
        Outcome::Rows { len }
    }
}

struct ConflictMemory {
    bytes: Vec<u8>,
    root: [u8; 8],
}

const REGISTRY_ROOT: usize =
    crate::creference_registry::LOW_ROOTS_RVA.0 + crate::corders_group::REFERENCE_TYPE as usize * 8;

impl Memory for ConflictMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address == REGISTRY_ROOT && destination.len() == 8 {
            destination.copy_from_slice(&self.root);
            return true;
        }
        SliceMemory {
            base: 0x1000,
            bytes: &self.bytes,
        }
        .copy(address, destination)
    }
}

fn conflict_fixture() -> ConflictMemory {
    use crate::{
        cland_border_war_combat,
        cland_border_war_combatant,
        cland_combat,
        corders_group,
        creference_registry as registry,
        msvc_tree_node as node,
        cprovince::definition as province_definition,
        reference_bucket as bucket,
    };
    let mut bytes = many_provinces(3);
    bytes.resize(0x1c000, 0);
    let at = |offset: usize| (offset + 0x1000) as u64;
    let province = 0x1200;
    put(
        &mut bytes,
        0xa100 + province_definition::PORT_SEA,
        &77_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        province + OWNER_AREA,
        &at(0x11000).to_le_bytes(),
    );
    let (head, leaf, combat) = (0x10000, 0x10040, 0x10100);
    put(
        &mut bytes,
        province + BORDER_CONFLICT_TREE,
        &at(head).to_le_bytes(),
    );
    put(
        &mut bytes,
        province + BORDER_CONFLICT_NONZERO_QWORD,
        &1_u64.to_le_bytes(),
    );
    for link in [node::LEFT, node::PARENT, node::RIGHT] {
        put(&mut bytes, head + link, &at(leaf).to_le_bytes());
        put(&mut bytes, leaf + link, &at(head).to_le_bytes());
    }
    bytes[head + node::NIL] = 1;
    put(&mut bytes, leaf + node::PAYLOAD, &at(combat).to_le_bytes());
    put(
        &mut bytes,
        combat,
        &(cland_border_war_combat::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    let (table, entries) = (0x1a000, 0x1a100);
    put(
        &mut bytes,
        table + registry::ENTRIES,
        &at(entries).to_le_bytes(),
    );
    put(&mut bytes, table + registry::COUNT, &2_u32.to_le_bytes());
    put(&mut bytes, table + registry::MASK, &7_u32.to_le_bytes());
    bytes[table + registry::OVERFLOW_DISTANCE] = 5;
    bytes[entries + 13 * bucket::STRIDE + bucket::DISTANCE] = bucket::TERMINAL_DISTANCE;
    for (side, offset, group, id, border_war) in [
        (0x16000, cland_combat::DEFENDER, 0x18000, 57_u32, 1_u64),
        (0x17000, cland_combat::ATTACKER, 0x19000, 58, 0),
    ] {
        put(&mut bytes, combat + offset, &at(side).to_le_bytes());
        put(
            &mut bytes,
            side,
            &(cland_border_war_combatant::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        let handle = (u64::from(id) << 32) | u64::from(corders_group::REFERENCE_TYPE);
        put(
            &mut bytes,
            side + BORDER_WAR_SIDE_GROUP,
            &handle.to_le_bytes(),
        );
        put(
            &mut bytes,
            group,
            &(corders_group::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            group + corders_group::HANDLE,
            &handle.to_le_bytes(),
        );
        put(
            &mut bytes,
            group + corders_group::BORDER_WAR,
            &border_war.to_le_bytes(),
        );
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let cell = entries + ((((hash >> 32) as u32 ^ hash as u32) & 7) as usize) * 0x18;
        bytes[cell + bucket::DISTANCE] = 1;
        put(&mut bytes, cell + bucket::TYPE, &handle.to_le_bytes());
        put(&mut bytes, cell + bucket::OBJECT, &at(group).to_le_bytes());
    }
    ConflictMemory {
        bytes,
        root: at(table).to_le_bytes(),
    }
}

fn conflict_walk(
    memory: &ConflictMemory,
    rows: usize,
    groups: usize,
) -> (Outcome, ConflictWalk, Vec<Row>, Vec<ConflictRow>, Vec<u64>) {
    let mut output = vec![Row::default(); 4];
    let mut conflicts = vec![ConflictRow::default(); rows];
    let mut handles = vec![0; groups];
    let (outcome, walk) = walk_with_conflicts_buffer(memory, 0, 0x1000, crate::walk::Buffer::Growing(&mut output), ConflictBuffers {
            rows: crate::walk::Buffer::Growing(&mut conflicts),
            groups: crate::walk::Buffer::Growing(&mut handles),
        }, crate::walk::Buffer::Count, None);
    (outcome, walk, output, conflicts, handles)
}

#[test]
fn border_conflicts_admit_only_resolved_groups_at_border_war() {
    let memory = conflict_fixture();
    let (outcome, walk, rows, conflicts, handles) = conflict_walk(&memory, 2, 2);
    assert_eq!(outcome, Outcome::Rows { len: 3 });
    assert_eq!(
        (rows[1].move_gate, rows[1].port_sea),
        (Row::MOVE_BORDER_CONFLICT, 77)
    );
    assert_eq!(rows[0].move_gate, Row::MOVE_NULL_OWNER_AREA);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!((walk.groups_len, walk.required_groups), (1, 1));
    assert_eq!(
        conflicts[0],
        ConflictRow {
            province: 2,
            groups_start: 0,
            groups_len: 1,
        }
    );
    assert_eq!(
        handles[0],
        (57_u64 << 32) | u64::from(crate::corders_group::REFERENCE_TYPE)
    );
}

#[test]
fn a_broken_conflict_set_fails_its_table_without_the_provinces() {
    for (offset, value) in [
        (0x1200 + BORDER_CONFLICT_NONZERO_QWORD, 2_u64),
        (0x10000 + crate::msvc_tree_node::NIL, 0),
        (0x10100, 0),
    ] {
        let mut memory = conflict_fixture();
        put(
            &mut memory.bytes,
            offset,
            &value.to_le_bytes()[..if offset == 0x10000 + crate::msvc_tree_node::NIL {
                1
            } else {
                8
            }],
        );
        let (outcome, walk, ..) = conflict_walk(&memory, 2, 2);
        assert_eq!(outcome, Outcome::Rows { len: 3 });
        assert!(
            matches!(walk.outcome, Outcome::Unavailable { .. }),
            "{offset:#x}"
        );
    }
    let mut memory = conflict_fixture();
    memory.root = [0; 8];
    let (_, walk, _, conflicts, _) = conflict_walk(&memory, 2, 2);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(conflicts[0].groups_len, 0);
}

fn dense_trace(bytes: &[u8], blocked: Option<usize>) -> TracedMemory<'_> {
    TracedMemory {
        bytes,
        blocked,
        guards: std::cell::Cell::new(0),
        reads: std::cell::RefCell::new(Vec::new()),
    }
}

#[test]
fn geometry_owned_index_preserves_reads_and_mismatched_id_backlink_faults() {
    let bytes = many_provinces(2);
    let memory = dense_trace(&bytes, None);
    let mut packet = GeometryPacket::with_capacities(GeometryCapacities {
        provinces: 2,
        neighbour_entries: 0,
    });
    packet.begin(&memory, 0, 0x1000).unwrap();
    assert_eq!(packet.acquire_flags(&memory, 1), Ok(1));
    assert_eq!(packet.adjacency(&memory, 1).unwrap().1.count, 0);
    assert_eq!(packet.acquire_definition(&memory, 1), Ok(0xb000));
    assert_eq!(packet.acquire_pointer(&memory, 1), Ok(0x2000));
    assert_eq!(packet.acquire_route_pointer(&memory, 1), Ok(0x2000));
    assert_eq!(packet.acquire_province_id(&memory, 0x2000), Ok(1));
    assert_eq!(
        *memory.reads.borrow(),
        [
            (0x1000 + cgame_state::PROVINCES, 16),
            (0x1308, 8),
            (0x2000, 8),
            (0x2000 + ID, 4),
            (0x2000 + DEFINITION, 8),
            (0xb000 + crate::cprovince::definition::ID, 4),
            (0xb000 + crate::cprovince::definition::TERRAIN_FLAGS, 1),
            (0xb000 + crate::cprovince::definition::NEIGHBOURS, 16),
        ]
    );

    for (alias, blocked, reason) in [
        (false, None, "unit geometry province differs from census"),
        (true, None, "unit geometry province identity mismatch"),
        (
            true,
            Some(0x1310),
            "unit geometry province outside census or unreadable",
        ),
    ] {
        let mut bytes = many_provinces(2);
        put(&mut bytes, 0x1000 + ID, &2_u32.to_le_bytes());
        if alias {
            put(&mut bytes, 0x310, &0x2000_u64.to_le_bytes());
        }
        let memory = dense_trace(&bytes, blocked);
        let mut packet = GeometryPacket::with_capacities(GeometryCapacities {
            provinces: 2,
            neighbour_entries: 0,
        });
        packet.begin(&memory, 0, 0x1000).unwrap();
        assert_eq!(packet.acquire_flags(&memory, 1), Err(reason));
        assert_eq!(packet.acquire_definition(&memory, 1), Err(reason));
        assert_eq!(packet.acquire_pointer(&memory, 1), Err(reason));
        assert_eq!(packet.acquire_route_pointer(&memory, 1), Err(reason));
        assert_eq!(
            *memory.reads.borrow(),
            [
                (0x1000 + cgame_state::PROVINCES, 16),
                (0x1308, 8),
                (0x2000, 8),
                (0x2000 + ID, 4),
                (0x1310, 8),
            ]
        );
        assert_eq!(packet.required_capacities().provinces, 2);
    }

    let mut bytes = many_provinces(2);
    put(&mut bytes, 0x300, &0x2000_u64.to_le_bytes());
    put(&mut bytes, 0x1000 + ID, &0_u32.to_le_bytes());
    let memory = dense_trace(&bytes, None);
    packet.begin(&memory, 0, 0x1000).unwrap();
    assert_eq!(packet.acquire_route_pointer(&memory, 0), Ok(0x2000));
    let reads = memory.reads.borrow().len();
    assert_eq!(
        packet.acquire_province_id(&memory, 0x2000),
        Err("unit geometry province identity mismatch")
    );
    assert_eq!(memory.reads.borrow().len(), reads);
}

#[test]
fn held_province_ids_answer_a_later_pass_of_their_world_without_a_read() {
    let mut packet = GeometryPacket::with_capacities(GeometryCapacities {
        provinces: 2,
        neighbour_entries: 0,
    });
    let bytes = many_provinces(2);
    let memory = dense_trace(&bytes, None);
    packet.begin(&memory, 0, 0x1000).unwrap();
    packet.hold_identities(1);
    assert_eq!(packet.acquire_province_id(&memory, 0x2000), Ok(1));
    assert_eq!(packet.held_counts(), [0, 1]);

    let mut bytes = many_provinces(2);
    put(&mut bytes, 0x1000 + ID, &2_u32.to_le_bytes());
    let memory = dense_trace(&bytes, None);
    packet.begin(&memory, 0, 0x1000).unwrap();
    packet.hold_identities(1);
    assert_eq!(packet.acquire_province_id(&memory, 0x2000), Ok(1));
    assert_eq!(packet.acquire_province_id(&memory, 0x2000), Ok(1));
    assert_eq!(packet.held_counts(), [1, 0]);
    assert_eq!(
        *memory.reads.borrow(),
        [(0x1000 + cgame_state::PROVINCES, 16)]
    );

    for world in [None, Some(2)] {
        let memory = dense_trace(&bytes, None);
        packet.begin(&memory, 0, 0x1000).unwrap();
        if let Some(world) = world {
            packet.hold_identities(world);
        }
        assert_eq!(
            packet.acquire_province_id(&memory, 0x2000),
            Err("unit geometry province differs from census")
        );
        assert_eq!(packet.held_counts(), [0, 0]);
        assert!(memory.reads.borrow().contains(&(0x2000 + ID, 4)));
    }
}

fn compare_dense_walk(bytes: &[u8], blocked: Option<usize>, capacity: usize) {
    let current = dense_trace(bytes, blocked);
    let reference = dense_trace(bytes, blocked);
    let sentinel = Row {
        id: 71,
        state: 72,
        owner: 73,
        controller: 74,
        victory_points: -75,
        ..Row::default()
    };
    let mut rows = vec![sentinel; capacity];
    let mut old_rows = rows.clone();
    assert_eq!(
        walk(&current, 0, 0x1000, &mut rows),
        scattered_reference_walk(&reference, 0, 0x1000, &mut old_rows),
        "capacity {capacity}, fault {blocked:?}"
    );
    assert_eq!(rows, old_rows);
    assert_eq!(*current.reads.borrow(), *reference.reads.borrow());
    assert_eq!(current.guards.get(), reference.guards.get());
}

#[test]
fn dense_records_match_scattered_walk_windows_faults_and_state_order() {
    for count in [1, 2, 63, 64, 65, 67] {
        let bytes = many_provinces(count);
        for capacity in [count, count + 1] {
            compare_dense_walk(&bytes, None, capacity);
        }
        let trace = dense_trace(&bytes, None);
        let _ = scattered_reference_walk(&trace, 0, 0x1000, &mut vec![Row::default(); count]);
        for &(address, width) in trace.reads.borrow().iter() {
            for fault in [address, address + width - 1] {
                compare_dense_walk(&bytes, Some(fault), count);
            }
        }
    }

    let mut bytes = many_provinces(67);
    for slot in [1, 32, 64, 67] {
        put(&mut bytes, 0x300 + slot * 8, &0_u64.to_le_bytes());
    }
    for capacity in [67] {
        compare_dense_walk(&bytes, None, capacity);
        compare_dense_walk(&bytes, Some(0x1300 + 65 * 8), capacity);
    }
    let mut bytes = many_provinces(65);
    put(&mut bytes, 0x12000 + cstate::ID, &0_u32.to_le_bytes());
    compare_dense_walk(&bytes, Some(0x2200 + VICTORY_POINTS), 65);
    let mut bytes = many_provinces(2);
    put(&mut bytes, 0x12000, &0_u64.to_le_bytes());
    compare_dense_walk(&bytes, Some(0x2000 + VICTORY_POINTS), 2);
}

#[test]
fn dense_records_preserve_dirty_backing_empty_tails_and_changing_duplicate_reads() {
    struct Changing<'a> {
        inner: TracedMemory<'a>,
        controllers: std::cell::Cell<u32>,
    }
    impl Memory for Changing<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if !self.inner.copy(address, destination) {
                return false;
            }
            if address == 0x2000 + CONTROLLER && destination.len() == 4 {
                self.controllers.set(self.controllers.get() + 1);
                destination.copy_from_slice(&self.controllers.get().to_le_bytes());
            }
            true
        }
    }
    for count in [0, 1, 2, 63, 64] {
        for capacity in [0, 1, count, count + 1] {
            for hole in [false, true] {
                let mut bytes = many_provinces(count);
                if count > 1 {
                    put(
                        &mut bytes,
                        0x300 + 2 * 8,
                        &(if hole { 0_u64 } else { 0x2000_u64 }).to_le_bytes(),
                    );
                }
                for blocked in [None, Some(0x2000 + CONTROLLER), Some(0x1300 + 8 * count)] {
                    let current = Changing {
                        inner: dense_trace(&bytes, blocked),
                        controllers: std::cell::Cell::new(0),
                    };
                    let reference = Changing {
                        inner: dense_trace(&bytes, blocked),
                        controllers: std::cell::Cell::new(0),
                    };
                    let mut batch = ProvinceBatch::new();
                    let mut old = ProvinceBatch::new();
                    batch.fields.fill([0xa5; 8]);
                    old.fields.fill([0xa5; 8]);
                    batch.pointers.fill([0x5a; 8]);
                    old.pointers.fill([0x5a; 8]);
                    let array = Array {
                        data: 0x1300,
                        count: count as u32 + 1,
                    };
                    batch.read(&current, array, 1, capacity);
                    scattered_reference_read(&mut old, &reference, array, 1, capacity);
                    assert_eq!(batch.fields, old.fields);
                    assert_eq!(batch.pointers, old.pointers);
                    assert_eq!(
                        (batch.pointer_count, batch.field_count, batch.len),
                        (old.pointer_count, old.field_count, old.len)
                    );
                    assert_eq!(
                        *current.inner.reads.borrow(),
                        *reference.inner.reads.borrow()
                    );
                    assert_eq!(current.controllers.get(), reference.controllers.get());
                }
            }
        }
    }
}

#[test]
fn dense_records_preserve_invalid_source_prefix_without_retry() {
    struct AddressMemory {
        pointer: usize,
        reads: std::cell::RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for AddressMemory {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            if address == 0x1308 {
                destination.copy_from_slice(&(self.pointer as u64).to_le_bytes());
                return true;
            }
            if address == self.pointer && destination.len() == 8 {
                destination.copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
                return true;
            }
            destination.fill(0xee);
            false
        }
    }
    for pointer in [0, 0x0000_8000_0000_0000, usize::MAX - 8, usize::MAX - 4] {
        let current = AddressMemory {
            pointer,
            reads: std::cell::RefCell::new(Vec::new()),
        };
        let reference = AddressMemory {
            pointer,
            reads: std::cell::RefCell::new(Vec::new()),
        };
        let mut batch = ProvinceBatch::new();
        let mut old = ProvinceBatch::new();
        batch.fields.fill([0xa5; 8]);
        old.fields.fill([0xa5; 8]);
        let array = Array {
            data: 0x1300,
            count: 2,
        };
        batch.read(&current, array, 1, 1);
        scattered_reference_read(&mut old, &reference, array, 1, 1);
        assert_eq!(batch.fields, old.fields);
        assert_eq!(
            (batch.pointer_count, batch.field_count),
            (old.pointer_count, old.field_count)
        );
        assert_eq!(*current.reads.borrow(), *reference.reads.borrow());
    }
    let current = AddressMemory {
        pointer: 0x2000,
        reads: std::cell::RefCell::new(Vec::new()),
    };
    let reference = AddressMemory {
        pointer: 0x2000,
        reads: std::cell::RefCell::new(Vec::new()),
    };
    let array = Array {
        data: usize::MAX - 4,
        count: 2,
    };
    let mut batch = ProvinceBatch::new();
    let mut old = ProvinceBatch::new();
    batch.read(&current, array, 1, 1);
    scattered_reference_read(&mut old, &reference, array, 1, 1);
    assert_eq!(
        (batch.pointer_count, batch.field_count),
        (old.pointer_count, old.field_count)
    );
    assert_eq!(*current.reads.borrow(), *reference.reads.borrow());
}

#[test]
fn allocation_capacity_overflow_owned_capture_buffers() {
    let oom = std::io::ErrorKind::OutOfMemory;
    assert_eq!(
        crate::country_memory::Cache::try_with_capacity(usize::MAX)
            .err()
            .unwrap()
            .kind(),
        oom
    );
    assert_eq!(
        Capture::try_with_capacity(usize::MAX, 0, 0)
            .err()
            .unwrap()
            .kind(),
        oom
    );
    assert_eq!(
        crate::cname_group_member::GroupKeys::try_with_capacity(usize::MAX, 0)
            .err()
            .unwrap()
            .kind(),
        oom
    );
    assert_eq!(
        crate::ccountry_raid_status::TypeTokens::try_with_definitions(usize::MAX)
            .err()
            .unwrap()
            .kind(),
        oom
    );
    assert_eq!(
        crate::ccharacter::manager::PortraitCensus::try_new(usize::MAX)
            .err()
            .unwrap()
            .kind(),
        oom
    );
    assert_eq!(
        crate::carmy::MovementPacket::try_new(0, usize::MAX)
            .err()
            .unwrap()
            .kind(),
        oom
    );
    let mut geometry = GeometryPacket::try_with_capacities(GeometryCapacities {
        provinces: 2,
        neighbour_entries: 4,
    })
    .unwrap();
    let before = geometry.allocated_bytes();
    assert_eq!(
        geometry
            .grow(GeometryCapacities {
                provinces: usize::MAX,
                neighbour_entries: 4,
            })
            .unwrap_err()
            .kind(),
        oom
    );
    assert_eq!(geometry.allocated_bytes(), before);
    assert_eq!(geometry.capacities.provinces, 2);
}

#[test]
fn province_names_grow_on_first_and_later_mod_sized_samples_without_restarting_reads() {
    let mut rows = Vec::new();
    let mut arena = Vec::new();
    for (count, name_len) in [(5000_usize, 4_usize), (5000, 64), (20000, 64)] {
        let table = 0x3000;
        let objects = table + (count + 1) * 8;
        let stride = (NAME_OVERRIDE + 32).max(ID + 4);
        let heap = objects + count * stride;
        let mut bytes = vec![0_u8; heap + count * (name_len + 1)];
        put(
            &mut bytes,
            cgame_state::PROVINCES,
            &((table + 0x1000) as u64).to_le_bytes(),
        );
        for at in [8, 12] {
            put(
                &mut bytes,
                cgame_state::PROVINCES + at,
                &((count + 1) as u32).to_le_bytes(),
            );
        }
        for index in 0..count {
            let object = objects + index * stride;
            put(
                &mut bytes,
                table + (index + 1) * 8,
                &((object + 0x1000) as u64).to_le_bytes(),
            );
            put(&mut bytes, object, &(VTABLE_RVA.0 as u64).to_le_bytes());
            put(&mut bytes, object + ID, &((index + 1) as u32).to_le_bytes());
            let literal = object + NAME_OVERRIDE;
            if name_len <= 15 {
                put(&mut bytes, literal, &vec![b'N'; name_len]);
                put(&mut bytes, literal + 24, &15_u64.to_le_bytes());
            } else {
                let source = heap + index * (name_len + 1);
                put(
                    &mut bytes,
                    literal,
                    &((source + 0x1000) as u64).to_le_bytes(),
                );
                put(&mut bytes, source, &vec![b'N'; name_len]);
                put(&mut bytes, literal + 24, &(name_len as u64).to_le_bytes());
            }
            put(&mut bytes, literal + 16, &(name_len as u64).to_le_bytes());
        }
        let fixed = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            count_names(&fixed, 0, 0x1000).outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: count as u32
            }
        );
        let memory = TracedMemory {
            bytes: &bytes,
            blocked: None,
            guards: std::cell::Cell::new(0),
            reads: std::cell::RefCell::new(Vec::new()),
        };
        let mut expected_rows = vec![NameRow::default(); count];
        let mut expected_text = vec![0; count * name_len];
        let expected = walk_names(&memory, 0, 0x1000, &mut expected_rows, &mut expected_text);
        let trace = memory.reads.replace(Vec::new());
        let captured = walk_names(&memory, 0, 0x1000, &mut rows, &mut arena);
        assert_eq!(captured, expected);
        assert_eq!(captured.outcome, Outcome::Rows { len: count });
        assert_eq!(&rows[..count], expected_rows.as_slice());
        assert_eq!(&arena[..count * name_len], expected_text.as_slice());
        assert_eq!(*memory.reads.borrow(), trace);
        assert_eq!(
            trace
                .iter()
                .filter(|&&(at, _)| at == 0x1000 + cgame_state::PROVINCES)
                .count(),
            1
        );
    }
}

#[test]
fn default_geometry_has_no_chosen_province_or_neighbour_population() {
    assert_eq!(
        GeometryCapacities::default(),
        GeometryCapacities {
            provinces: 0,
            neighbour_entries: 0
        }
    );
}

const TAKE: Option<[u64; 2]> = Some([3, 4]);

fn carrying() -> Capture {
    let mut capture = Capture::with_capacity(0, 0, 0);
    capture.province_store = Some(Box::new(carry::RowStore::new()));
    capture
}

fn carry_hour(
    capture: &mut Capture,
    memory: &impl Memory,
    take: Option<[u64; 2]>,
) -> (Outcome, carry::Counts) {
    let outcome = capture.walk_carrying(memory, 0, 0x1000, take);
    (outcome, capture.province_store.as_ref().unwrap().counts())
}

fn mark(capture: &mut Capture, provinces: &[u32], states: &[u32]) {
    let (marked, marked_states) = capture.province_store.as_mut().unwrap().marks_mut();
    marked.extend_from_slice(provinces);
    marked_states.extend_from_slice(states);
}

fn slice(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory {
        base: 0x1000,
        bytes,
    }
}

fn assert_full_read(capture: &Capture, memory: &impl Memory, len: usize) {
    let mut full = Capture::with_capacity(0, 0, 0);
    assert_eq!(full.walk(memory, 0, 0x1000), Outcome::Rows { len });
    assert_eq!(capture.rows[..len], full.rows[..len]);
    assert_eq!(capture.areas[..len], full.areas[..len]);
    assert_eq!(capture.conflict_walk, full.conflict_walk);
    let conflicts = full.conflict_walk.outcome.len();
    assert_eq!(capture.conflicts[..conflicts], full.conflicts[..conflicts]);
    let groups = full.conflict_walk.groups_len;
    assert_eq!(capture.groups[..groups], full.groups[..groups]);
}

fn province_at(id: usize) -> usize {
    0x1000 + (id - 1) * 0x200
}

fn carry_fixture() -> Vec<u8> {
    use crate::{cbuilding, cbuilding_template};
    let mut bytes = many_provinces(67);
    bytes.resize(0x20000, 0);
    let at = |offset: usize| (offset + 0x1000) as u64;
    let fortified = province_at(3);
    put(&mut bytes, fortified + STATE, &0_u64.to_le_bytes());
    put(&mut bytes, fortified + CONTROLLER, &7_u32.to_le_bytes());
    let buildings = fortified + MOVEMENT_BUILDINGS;
    put(&mut bytes, buildings, &at(0x14000).to_le_bytes());
    put(&mut bytes, buildings + 8, &1_u32.to_le_bytes());
    put(&mut bytes, buildings + 12, &1_u32.to_le_bytes());
    put(&mut bytes, 0x14000, &at(0x14100).to_le_bytes());
    put(
        &mut bytes,
        0x14100 + cbuilding::HEALTHY_LEVEL,
        &1_i16.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x14100 + cbuilding::TEMPLATE,
        &at(0x14400).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x14400 + cbuilding_template::MOVEMENT_FORT_WEIGHT,
        &1_i32.to_le_bytes(),
    );
    for (state, id, owner, provinces) in [
        (0x12800, 90_u32, 5_u32, [10, 11]),
        (0x12a00, 91, 6, [30, 31]),
    ] {
        put(
            &mut bytes,
            state,
            &(cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, state + cstate::ID, &id.to_le_bytes());
        put(&mut bytes, state + cstate::OWNER, &owner.to_le_bytes());
        for province in provinces {
            put(
                &mut bytes,
                province_at(province) + STATE,
                &at(state).to_le_bytes(),
            );
        }
    }
    bytes
}

fn traced(bytes: &[u8]) -> TracedMemory<'_> {
    TracedMemory {
        bytes,
        blocked: None,
        guards: std::cell::Cell::new(0),
        reads: std::cell::RefCell::new(Vec::new()),
    }
}

#[test]
fn a_quiet_hour_reads_no_carried_province_and_equals_a_full_read() {
    let bytes = carry_fixture();
    let memory = traced(&bytes);
    let mut capture = carrying();
    let (outcome, counts) = carry_hour(&mut capture, &memory, TAKE);
    assert_eq!(outcome, Outcome::Rows { len: 67 });
    assert!(counts.full && counts.read == 67);
    assert!(capture.province_store.as_ref().unwrap().holds());
    assert_eq!(capture.rows[2].fort_positive, 1);
    assert_eq!(capture.rows[2].fort_bunker, Some(0));
    memory.reads.borrow_mut().clear();
    let (outcome, counts) = carry_hour(&mut capture, &memory, TAKE);
    assert_eq!(outcome, Outcome::Rows { len: 67 });
    assert_eq!(
        counts,
        carry::Counts {
            carried: 67,
            refreshed: 1,
            quiet: true,
            ..carry::Counts::default()
        }
    );
    let header = 0x1000 + cgame_state::PROVINCES;
    let table = 0x1300..0x1300 + 68 * 8;
    let fort_state = (0x1000 + province_at(3) + STATE, 8);
    for &(address, len) in memory.reads.borrow().iter() {
        assert!(
            (address >= header && address + len <= header + 16)
                || (table.contains(&address) && address + len <= table.end)
                || (address, len) == fort_state,
            "{address:#x}+{len}"
        );
    }
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&read| read == fort_state)
            .count(),
        1
    );
    assert_eq!(capture.carried_rows(), 67);
    assert_full_read(&capture, &memory, 67);
}

fn modifier_take(capture: &mut Capture, take: Option<u64>) {
    *capture.province_store.as_mut().unwrap().modifier_marks_mut().1 = take;
}

#[test]
fn a_certified_bunker_term_is_kept_until_its_modifier_take_moves() {
    let bytes = carry_fixture();
    let memory = traced(&bytes);
    let mut capture = carrying();
    modifier_take(&mut capture, Some(1));
    carry_hour(&mut capture, &memory, TAKE);
    modifier_take(&mut capture, Some(1));
    memory.reads.borrow_mut().clear();
    let (outcome, counts) = carry_hour(&mut capture, &memory, TAKE);
    assert_eq!(outcome, Outcome::Rows { len: 67 });
    assert_eq!((counts.carried, counts.refreshed), (67, 0));
    assert_eq!(capture.rows[2].fort_bunker, Some(0));
    let header = 0x1000 + cgame_state::PROVINCES;
    let table = 0x1300..0x1300 + 68 * 8;
    let countries = [cgame_state::COUNTRY_LINKS, cgame_state::COUNTRIES].map(|at| 0x1000 + at);
    for &(address, len) in memory.reads.borrow().iter() {
        assert!(
            (address >= header && address + len <= header + 16)
                || (table.contains(&address) && address + len <= table.end)
                || (countries.contains(&address) && len == 16),
            "{address:#x}+{len}"
        );
    }
    assert_full_read(&capture, &memory, 67);
    for take in [Some(2), None] {
        modifier_take(&mut capture, take);
        let (_, counts) = carry_hour(&mut capture, &memory, TAKE);
        assert_eq!((counts.carried, counts.refreshed), (67, 1));
    }
    modifier_take(&mut capture, Some(2));
    assert_eq!(carry_hour(&mut capture, &memory, TAKE).1.refreshed, 1);
    modifier_take(&mut capture, Some(2));
    assert_eq!(carry_hour(&mut capture, &memory, TAKE).1.refreshed, 0);
    assert_full_read(&capture, &memory, 67);
    let mut headless = bytes.clone();
    put(&mut headless, cgame_state::PROVINCES, &[0; 16]);
    mark(&mut capture, &[], &[90]);
    modifier_take(&mut capture, Some(2));
    assert!(matches!(
        carry_hour(&mut capture, &slice(&headless), TAKE).0,
        Outcome::Unavailable { .. }
    ));
    assert!(!capture.province_store.as_ref().unwrap().holds());
    modifier_take(&mut capture, Some(2));
    let (_, counts) = carry_hour(&mut capture, &memory, TAKE);
    assert!(counts.full && counts.read == 67);
    assert_full_read(&capture, &memory, 67);
}

#[test]
fn marked_provinces_states_resets_and_moved_slots_are_read_again() {
    let mut bytes = carry_fixture();
    let mut capture = carrying();
    assert_eq!(
        carry_hour(&mut capture, &slice(&bytes), TAKE).0,
        Outcome::Rows { len: 67 }
    );
    put(
        &mut bytes,
        province_at(5) + CONTROLLER,
        &44_u32.to_le_bytes(),
    );
    put(&mut bytes, 0x12800 + cstate::OWNER, &8_u32.to_le_bytes());
    for id in [30, 31] {
        put(&mut bytes, province_at(id) + STATE, &0_u64.to_le_bytes());
    }
    let moved = bytes[province_at(4)..province_at(4) + 0x200].to_vec();
    put(&mut bytes, 0x16000, &moved);
    put(&mut bytes, 0x16000 + VICTORY_POINTS, &12_i32.to_le_bytes());
    put(&mut bytes, 0x300 + 4 * 8, &0x17000_u64.to_le_bytes());
    let memory = traced(&bytes);
    mark(&mut capture, &[5], &[91, 90]);
    let (outcome, counts) = carry_hour(&mut capture, &memory, TAKE);
    assert_eq!(outcome, Outcome::Rows { len: 67 });
    assert_eq!((counts.read, counts.carried), (6, 61));
    assert!(!counts.quiet && !counts.full);
    assert_eq!(capture.rows[4].controller, 44);
    assert_eq!((capture.rows[9].owner, capture.rows[10].owner), (8, 8));
    assert_eq!((capture.rows[29].state, capture.rows[29].owner), (0, 0));
    assert_eq!(capture.rows[3].victory_points, 12);
    assert_full_read(&capture, &memory, 67);
    let (_, counts) = carry_hour(&mut capture, &memory, TAKE);
    assert_eq!((counts.read, counts.carried), (0, 67));
    assert_full_read(&capture, &memory, 67);
}

#[test]
fn conflicted_provinces_and_their_groups_are_read_every_hour() {
    let mut memory = conflict_fixture();
    let mut capture = carrying();
    assert_eq!(
        carry_hour(&mut capture, &memory, TAKE).0,
        Outcome::Rows { len: 3 }
    );
    assert_eq!(capture.conflict_walk.groups_len, 1);
    put(
        &mut memory.bytes,
        0x19000 + crate::corders_group::BORDER_WAR,
        &1_u64.to_le_bytes(),
    );
    let (outcome, counts) = carry_hour(&mut capture, &memory, TAKE);
    assert_eq!(outcome, Outcome::Rows { len: 3 });
    assert_eq!((counts.read, counts.carried), (1, 2));
    assert_eq!(capture.conflict_walk.groups_len, 2);
    assert_full_read(&capture, &memory, 3);
}

#[test]
fn a_failed_walk_or_an_uncertified_take_holds_nothing() {
    let good = carry_fixture();
    let mut bytes = good.clone();
    let memory = slice(&good);
    let mut capture = carrying();
    carry_hour(&mut capture, &memory, TAKE);
    put(&mut bytes, province_at(10), &0_u64.to_le_bytes());
    mark(&mut capture, &[10], &[]);
    assert_eq!(
        carry_hour(&mut capture, &slice(&bytes), TAKE).0,
        Outcome::Unavailable {
            reason: "province vtable mismatch"
        }
    );
    assert!(!capture.province_store.as_ref().unwrap().holds());
    let (_, counts) = carry_hour(&mut capture, &memory, TAKE);
    assert!(counts.full && counts.read == 67);
    let (_, counts) = carry_hour(&mut capture, &memory, None);
    assert!(counts.full);
    assert!(!capture.province_store.as_ref().unwrap().holds());
    let (_, counts) = carry_hour(&mut capture, &memory, TAKE);
    assert!(counts.full);
    let (_, counts) = carry_hour(&mut capture, &memory, Some([3, 5]));
    assert!(counts.full);
    let (_, counts) = carry_hour(&mut capture, &memory, Some([3, 5]));
    assert_eq!(counts.carried, 67);
    capture.province_store.as_mut().unwrap().force_full = true;
    let (_, counts) = carry_hour(&mut capture, &memory, Some([3, 5]));
    assert!(counts.full);
    assert_full_read(&capture, &memory, 67);
}

#[test]
fn carried_rows_read_their_area_pointers_again_on_request() {
    let mut bytes = carry_fixture();
    let mut capture = carrying();
    carry_hour(&mut capture, &slice(&bytes), TAKE);
    put(
        &mut bytes,
        province_at(5) + CONTROLLER_AREA,
        &0x18000_u64.to_le_bytes(),
    );
    let memory = slice(&bytes);
    carry_hour(&mut capture, &memory, TAKE);
    assert_eq!(capture.areas[4].controller, 0);
    capture
        .refresh_carried_areas(&memory, &|id| id == 5)
        .unwrap();
    assert_eq!(capture.areas[4].controller, 0x18000);
    assert_full_read(&capture, &memory, 67);
    carry_hour(&mut capture, &memory, TAKE);
    assert_eq!(capture.areas[4].controller, 0x18000);
}

#[test]
fn movement_province_reads_state_controller_area_region_and_sea() {
    const BASE: usize = 0x1000;
    const ARRAY: usize = 0x2000;
    const PROVINCE: usize = 0x3000;
    const DEFINITION_AT: usize = 0x3800;
    let mut bytes = vec![0_u8; 0x4000];
    let put = |bytes: &mut [u8], at: usize, value: u64| {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    };
    put(&mut bytes, ARRAY + 2 * 8, PROVINCE as u64);
    put(&mut bytes, PROVINCE, (BASE + VTABLE_RVA.0) as u64);
    bytes[PROVINCE + ID..PROVINCE + ID + 4].copy_from_slice(&2_u32.to_le_bytes());
    put(&mut bytes, PROVINCE + STATE, 0x5100);
    bytes[PROVINCE + CONTROLLER..PROVINCE + CONTROLLER + 4].copy_from_slice(&7_u32.to_le_bytes());
    put(&mut bytes, PROVINCE + CONTROLLER_AREA, 0x5200);
    put(&mut bytes, PROVINCE + STRATEGIC_REGION, 0x5300);
    put(&mut bytes, PROVINCE + DEFINITION, DEFINITION_AT as u64);
    bytes[DEFINITION_AT + crate::cprovince::definition::TERRAIN_FLAGS] =
        crate::cprovince::definition::LAND_FLAG;
    let provinces = Array {
        data: ARRAY,
        count: 3,
    };
    let read = |bytes: &[u8], id: u32| {
        movement_province(&SliceMemory { base: 0, bytes }, BASE, provinces, id)
    };
    let land = MovementProvince {
        state: Ok(0x5100),
        controller: Ok(7),
        controller_area: Ok(0x5200),
        strategic_region: Ok(0x5300),
        terrain_flags: Ok(crate::cprovince::definition::LAND_FLAG),
    };
    assert_eq!(read(&bytes, 2), Ok(land));
    bytes[DEFINITION_AT + crate::cprovince::definition::TERRAIN_FLAGS] = 0;
    assert_eq!(
        read(&bytes, 2),
        Ok(MovementProvince {
            terrain_flags: Ok(0),
            ..land
        })
    );
    struct Fault<'a> {
        bytes: &'a [u8],
        at: usize,
    }
    impl crate::Memory for Fault<'_> {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            if at <= self.at && self.at < at.saturating_add(out.len()) {
                return false;
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(at, out)
        }
    }
    for field in [STATE, CONTROLLER, CONTROLLER_AREA, DEFINITION] {
        let facts = movement_province(
            &Fault {
                bytes: &bytes,
                at: PROVINCE + field,
            },
            BASE,
            provinces,
            2,
        )
        .unwrap();
        assert_eq!(facts.strategic_region, Ok(0x5300));
    }
    assert_eq!(read(&bytes, 1), Err("movement province identity mismatch"));
    assert_eq!(
        read(&bytes, 3),
        Err("movement province outside the province array")
    );
    bytes[PROVINCE + ID] = 5;
    assert_eq!(read(&bytes, 2), Err("movement province identity mismatch"));
}

const MOVEMENT_BASE: usize = 0x1000;

fn movement_world() -> (Vec<u8>, Array, Vec<u32>) {
    const ARRAY: usize = 0x2000;
    const COUNT: u32 = 80;
    const STRIDE: usize = 0x200;
    const PROVINCES: usize = 0x4000;
    const DEFINITION_STRIDE: usize = 0x100;
    const DEFINITIONS: usize = PROVINCES + COUNT as usize * STRIDE;
    const _: () = assert!(crate::cprovince::definition::TERRAIN_FLAGS < DEFINITION_STRIDE);
    let mut bytes = vec![0_u8; DEFINITIONS + COUNT as usize * DEFINITION_STRIDE];
    let put = |bytes: &mut [u8], at: usize, value: u64| {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    };
    let put4 = |bytes: &mut [u8], at: usize, value: u32| {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    };
    for id in 0..COUNT {
        let province = PROVINCES + id as usize * STRIDE;
        let definition = DEFINITIONS + id as usize * DEFINITION_STRIDE;
        if id % 10 == 3 {
            continue;
        }
        put(&mut bytes, ARRAY + id as usize * 8, province as u64);
        let vtable = (MOVEMENT_BASE + VTABLE_RVA.0) as u64 + u64::from(id % 10 == 4);
        put(&mut bytes, province, vtable);
        put4(&mut bytes, province + ID, id + u32::from(id % 10 == 5));
        put(&mut bytes, province + STATE, 0x10_0000 + u64::from(id));
        put4(&mut bytes, province + CONTROLLER, id * 3);
        put(
            &mut bytes,
            province + CONTROLLER_AREA,
            0x20_0000 + u64::from(id),
        );
        put(
            &mut bytes,
            province + STRATEGIC_REGION,
            0x30_0000 + u64::from(id),
        );
        let definition_word = if id % 10 == 6 { 0 } else { definition as u64 };
        put(&mut bytes, province + DEFINITION, definition_word);
        bytes[definition + crate::cprovince::definition::TERRAIN_FLAGS] = id as u8;
    }
    let provinces = Array {
        data: ARRAY,
        count: COUNT - 2,
    };
    (bytes, provinces, (0..COUNT).collect())
}

struct MovementFaults<'a> {
    bytes: &'a [u8],
    fault: Option<usize>,
    once: bool,
    spent: std::cell::Cell<bool>,
    reads: std::cell::RefCell<std::collections::BTreeSet<usize>>,
}

impl<'a> MovementFaults<'a> {
    fn new(bytes: &'a [u8], fault: Option<usize>, once: bool) -> Self {
        Self {
            bytes,
            fault,
            once,
            spent: Default::default(),
            reads: Default::default(),
        }
    }
}

impl Memory for MovementFaults<'_> {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        self.reads
            .borrow_mut()
            .extend(at..at.saturating_add(out.len()));
        if let Some(fault) = self.fault
            && at <= fault
            && fault < at.saturating_add(out.len())
            && !self.spent.get()
        {
            self.spent.set(self.once);
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(at, out)
    }
}

type MovementOutcome = Vec<(u32, Result<MovementProvince, &'static str>)>;

fn batched_movement(memory: &impl Memory, provinces: Array, ids: &[u32]) -> MovementOutcome {
    let mut batched = Vec::new();
    movement_provinces(memory, MOVEMENT_BASE, provinces, ids, |id, facts| {
        batched.push((id, facts))
    });
    batched
}

fn movement_alone(memory: &impl Memory, provinces: Array, ids: &[u32]) -> MovementOutcome {
    ids.iter()
        .map(|&id| (id, movement_province(memory, MOVEMENT_BASE, provinces, id)))
        .collect()
}

#[test]
fn batched_movement_provinces_equal_each_province_read_alone() {
    let (bytes, provinces, ids) = movement_world();
    let clean = MovementFaults::new(&bytes, None, false);
    let batched = batched_movement(&clean, provinces, &ids);
    assert_eq!(batched, movement_alone(&clean, provinces, &ids));
    let mismatch = Err("movement province identity mismatch");
    assert_eq!(
        batched[7],
        (
            7,
            Ok(MovementProvince {
                state: Ok(0x10_0007),
                controller: Ok(21),
                controller_area: Ok(0x20_0007),
                strategic_region: Ok(0x30_0007),
                terrain_flags: Ok(7),
            })
        )
    );
    assert_eq!(batched[73], (73, mismatch));
    assert_eq!(batched[74], (74, mismatch));
    assert_eq!(batched[75], (75, mismatch));
    assert_eq!(
        batched[76].1.unwrap().terrain_flags,
        Err("movement province definition flags unreadable")
    );
    assert_eq!(
        batched[78],
        (78, Err("movement province outside the province array"))
    );
    let touched = clean.reads.into_inner();
    assert!(touched.len() > ids.len() * 40);
    for at in touched {
        let memory = MovementFaults::new(&bytes, Some(at), false);
        assert_eq!(
            batched_movement(&memory, provinces, &ids),
            movement_alone(&memory, provinces, &ids),
            "fault at {at:#x}"
        );
    }
}

#[test]
fn batched_movement_provinces_one_shot_faults_refuse_no_more() {
    fn fields(outcome: MovementOutcome) -> Vec<[Result<u64, &'static str>; 5]> {
        outcome
            .into_iter()
            .map(|(_, facts)| match facts {
                Ok(province) => [
                    province.state.map(|value| value as u64),
                    province.controller.map(u64::from),
                    province.controller_area.map(|value| value as u64),
                    province.strategic_region.map(|value| value as u64),
                    province.terrain_flags.map(u64::from),
                ],
                Err(reason) => [Err(reason); 5],
            })
            .collect()
    }
    let (bytes, provinces, ids) = movement_world();
    let clean = MovementFaults::new(&bytes, None, false);
    let expected = fields(batched_movement(&clean, provinces, &ids));
    let (mut recovered, mut refused) = (0, 0);
    for at in clean.reads.into_inner() {
        let batch = MovementFaults::new(&bytes, Some(at), true);
        let alone = MovementFaults::new(&bytes, Some(at), true);
        let batched = fields(batched_movement(&batch, provinces, &ids));
        let walked = fields(movement_alone(&alone, provinces, &ids));
        for ((batched, walked), expected) in batched.iter().zip(&walked).zip(&expected) {
            for field in 0..5 {
                let batched = batched[field];
                assert!(
                    batched == expected[field] || batched == walked[field],
                    "fault at {at:#x}"
                );
                recovered += usize::from(batched == expected[field] && batched != walked[field]);
                refused += usize::from(batched != expected[field]);
            }
        }
    }
    assert!(recovered > 0 && refused > 0);
}
