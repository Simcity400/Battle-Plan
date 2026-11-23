use super::*;
use crate::{cbuilding_template, memory::SliceMemory};
use std::cell::{Cell, RefCell};

struct TracedMemory<'a> {
    inner: SliceMemory<'a>,
    calls: Cell<usize>,
    spans: RefCell<Vec<(usize, usize)>>,
    blocked: Option<usize>,
}

struct FieldOnly<'a, 'b>(&'a TracedMemory<'b>);

impl Memory for FieldOnly<'_, '_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.0.field(address, destination)
    }
}

impl TracedMemory<'_> {
    fn field(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return true;
        }
        self.spans.borrow_mut().push((address, destination.len()));
        if self
            .blocked
            .is_some_and(|blocked| address <= blocked && blocked - address < destination.len())
        {
            destination.fill(0xff);
            return false;
        }
        self.inner.copy(address, destination)
    }
}

impl Memory for TracedMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.field(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        for (index, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if !self.field(address, destination) {
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
        self.calls.set(self.calls.get() + 1);
        FieldOnly(self).copy_records(sources, layout, destination)
    }
}

fn put<T: AsRef<[u8]>>(bytes: &mut [u8], at: usize, value: T) {
    bytes[at..at + value.as_ref().len()].copy_from_slice(value.as_ref());
}
fn vector(bytes: &mut [u8], at: usize, pointer: u64, count: i32) {
    put(bytes, at, pointer.to_le_bytes());
    put(bytes, at + 8, count.to_le_bytes());
    put(bytes, at + 12, count.to_le_bytes());
}
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x3000];
    vector(&mut bytes, cgame_state::STATES, 0x1300, 2);
    vector(&mut bytes, cgame_state::PROVINCES, 0x1320, 3);
    put(&mut bytes, 0x308, 0x1500_u64.to_le_bytes());
    put(&mut bytes, 0x328, 0x2000_u64.to_le_bytes());
    put(&mut bytes, 0x330, 0x2300_u64.to_le_bytes());
    put(
        &mut bytes,
        0x500,
        (0x1000 + cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, 0x500 + cstate::ID, 1_u32.to_le_bytes());
    for (pointer, id, state) in [(0x1000, 1_u32, 0x1500_u64), (0x1300, 2, 0)] {
        put(
            &mut bytes,
            pointer,
            (0x1000 + cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, pointer + cprovince::ID, id.to_le_bytes());
        put(&mut bytes, pointer + cprovince::STATE, state.to_le_bytes());
    }
    for (status, flags, id, pointers, indices) in [
        (0x620, 2_u32, 1_u32, 0x1600, 0x1620),
        (0x1190, 1, 1, 0x1640, 0x1660),
        (0x1490, 1, 2, 0x1680, 0x16a0),
    ] {
        put(
            &mut bytes,
            status,
            (0x1000 + VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, status + FLAGS, flags.to_le_bytes());
        put(&mut bytes, status + ID, id.to_le_bytes());
        put(&mut bytes, status + COUNTRY, 0x8000_0001_u32.to_le_bytes());
        vector(&mut bytes, status + BUILDINGS, 0x1000 + pointers as u64, 1);
        vector(
            &mut bytes,
            status + BUILDING_INDICES,
            0x1000 + indices as u64,
            2,
        );
        put(&mut bytes, indices, (-1_i32).to_le_bytes());
        put(&mut bytes, indices + 4, 0_i32.to_le_bytes());
    }
    for (index, (slot, status)) in [(0x1600, 0x620), (0x1640, 0x1190), (0x1680, 0x1490)]
        .into_iter()
        .enumerate()
    {
        let building = 0x1800 + index * 0x200;
        put(&mut bytes, slot, (0x1000 + building as u64).to_le_bytes());
        put(
            &mut bytes,
            building,
            (0x1000 + cbuilding::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            building + cbuilding::TYPE_ID,
            77_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            building + cbuilding::LEVEL,
            (-2_i16).to_le_bytes(),
        );
        put(
            &mut bytes,
            building + cbuilding::HEALTHY_LEVEL,
            i16::MIN.to_le_bytes(),
        );
        put(
            &mut bytes,
            building + cbuilding::PARTIAL_HEALTH,
            (-5_000_000_009_i64).to_le_bytes(),
        );
        put(
            &mut bytes,
            building + cbuilding::BUILDING_STATUS,
            (0x1000 + status as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            building + cbuilding::TEMPLATE,
            0x3200_u64.to_le_bytes(),
        );
    }
    put(
        &mut bytes,
        0x2200 + cbuilding_template::TYPE_ID,
        77_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x2200 + cbuilding_template::DATABASE_ORDINAL,
        1_u32.to_le_bytes(),
    );
    bytes
}

#[test]
fn signed_building_fields_and_sea_province_membership_survive_exactly() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut states = vec![cbuilding::Row::default(); 2];
    let mut provinces = vec![cbuilding::Row::default(); 3];
    assert_eq!(
        walk(&memory, 0x1000, 0x1000, &mut states, &mut provinces),
        Walk {
            states: Outcome::Rows { len: 1 },
            provinces: Outcome::Rows { len: 2 }
        }
    );
    assert_eq!(states[0].level, -2);
    assert_eq!(states[0].healthy_level, i16::MIN);
    assert_eq!(states[0].partial_health, -5_000_000_009);
    assert_eq!(states[0].country, 0x8000_0001);
    assert_eq!(provinces[0].location, 1);
    assert_eq!(provinces[1].location, 2);
    let mut empty_states = Vec::new();
    let mut short_provinces = vec![cbuilding::Row::default(); 1];
    assert_eq!(
        walk(
            &memory,
            0x1000,
            0x1000,
            &mut empty_states,
            &mut short_provinces
        ),
        Walk {
            states: Outcome::Rows { len: 1 },
            provinces: Outcome::Rows { len: 2 },
        }
    );
    assert_eq!(empty_states, states[..1]);
    assert_eq!(short_provinces, provinces[..2]);
}

#[test]
fn building_growth_follows_readable_rows_instead_of_native_count() {
    const LARGE_COUNT: i32 = 32_768;
    for readable in [0, 1] {
        let mut expected_trace = None;
        for count in [readable + 1, LARGE_COUNT] {
            let mut bytes = fixture();
            vector(&mut bytes, 0x620 + BUILDINGS, 0x2600, count);
            let memory = TracedMemory {
                inner: SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                calls: Cell::new(0),
                spans: RefCell::new(Vec::new()),
                blocked: Some(0x2600 + readable as usize * 8 + 3),
            };
            let mut states = Vec::new();
            let mut provinces = Vec::new();
            assert_eq!(
                walk(&memory, 0x1000, 0x1000, &mut states, &mut provinces),
                Walk {
                    states: Outcome::Unavailable {
                        reason: "building instance pointer unreadable or null"
                    },
                    provinces: Outcome::Rows { len: 2 },
                }
            );
            assert_eq!(states.len(), readable as usize);
            if readable == 0 {
                assert_eq!(states.capacity(), 0);
            } else {
                assert!(states.capacity() < LARGE_COUNT as usize);
                assert_eq!(states[0].building_type, 77);
                assert_eq!(states[0].ordinal, 0);
            }
            let trace = memory.spans.into_inner();
            if let Some(expected) = &expected_trace {
                assert_eq!(&trace, expected);
            } else {
                expected_trace = Some(trace);
            }
        }
    }
}

#[test]
fn bad_building_membership_does_not_hide_other_scope() {
    let mut bytes = fixture();
    let mut states = vec![cbuilding::Row::default(); 2];
    let mut provinces = vec![cbuilding::Row::default(); 3];
    put(
        &mut bytes,
        0x1800 + cbuilding::BUILDING_STATUS,
        0x2190_u64.to_le_bytes(),
    );
    let result = walk(
        &SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        0x1000,
        0x1000,
        &mut states,
        &mut provinces,
    );
    assert_eq!(
        result.states,
        Outcome::Unavailable {
            reason: "building status backlink mismatch"
        }
    );
    assert_eq!(result.provinces, Outcome::Rows { len: 2 });
    put(
        &mut bytes,
        0x1800 + cbuilding::BUILDING_STATUS,
        0x1620_u64.to_le_bytes(),
    );
    put(&mut bytes, 0x1624, (-1_i32).to_le_bytes());
    assert_eq!(
        walk(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0x1000,
            0x1000,
            &mut states,
            &mut provinces
        )
        .states,
        Outcome::Unavailable {
            reason: "building index map differs from pointer vector"
        }
    );
}

#[test]
fn batched_fields_share_only_the_current_root_and_never_accept_partial_fields() {
    let bytes = fixture();
    let memory = TracedMemory {
        inner: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        calls: Cell::new(0),
        spans: RefCell::new(Vec::new()),
        blocked: None,
    };
    let mut states = vec![cbuilding::Row::default(); 2];
    let mut provinces = vec![cbuilding::Row::default(); 3];
    let expected = Walk {
        states: Outcome::Rows { len: 1 },
        provinces: Outcome::Rows { len: 2 },
    };
    assert_eq!(
        walk(&memory, 0x1000, 0x1000, &mut states, &mut provinces),
        expected
    );
    assert_eq!(memory.calls.get(), 16);
    let state_root = (0x1000 + cgame_state::STATES, 16);
    assert_eq!(
        memory
            .spans
            .borrow()
            .iter()
            .filter(|&&span| span == state_root)
            .count(),
        1
    );
    assert!(
        !memory
            .spans
            .borrow()
            .contains(&(0x1500 + cprovince::STATE, 8))
    );
    let exact_fields = memory.spans.borrow().clone();
    for (address, length) in exact_fields {
        let faulty = TracedMemory {
            inner: SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            calls: Cell::new(0),
            spans: RefCell::new(Vec::new()),
            blocked: Some(address + length - 1),
        };
        let result = walk(&faulty, 0x1000, 0x1000, &mut states, &mut provinces);
        assert!(
            matches!(result.states, Outcome::Unavailable { .. })
                || matches!(result.provinces, Outcome::Unavailable { .. }),
            "partial span at {address:x} was accepted: {result:?}"
        );
        let growing = TracedMemory {
            inner: faulty.inner,
            calls: Cell::new(0),
            spans: RefCell::new(Vec::new()),
            blocked: faulty.blocked,
        };
        assert_eq!(
            walk(&growing, 0x1000, 0x1000, &mut Vec::new(), &mut Vec::new()),
            result,
            "growth changed the failure at {address:x}"
        );
        assert_eq!(growing.calls.get(), faulty.calls.get());
        assert_eq!(*growing.spans.borrow(), *faulty.spans.borrow());
    }
    assert_eq!(
        walk(&memory, 0x1000, 0x1000, &mut states, &mut provinces),
        expected
    );
    assert_eq!(memory.calls.get(), 32);
    assert_eq!(
        memory
            .spans
            .borrow()
            .iter()
            .filter(|&&span| span == state_root)
            .count(),
        2
    );
}

#[test]
fn pointer_batches_keep_order_and_fault_precedence_at_the_batch_boundary() {
    let bytes: Vec<u8> = (0..131_u64).flat_map(u64::to_le_bytes).collect();
    let memory = TracedMemory {
        inner: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        calls: Cell::new(0),
        spans: RefCell::new(Vec::new()),
        blocked: Some(0x1000 + 65 * 8 + 3),
    };
    let locations = Array {
        data: 0x1000,
        count: 131,
    };
    let mut visited = Vec::new();
    assert_eq!(
        visit_locations(&memory, locations, |ids, pointers| {
            for (&slot, &pointer) in ids.iter().zip(pointers) {
                assert_eq!(pointer, slot as usize);
                visited.push(slot);
            }
            Ok(())
        }),
        Err("building location pointer unreadable")
    );
    assert_eq!(visited, (1..65).collect::<Vec<_>>());
    assert_eq!(memory.calls.get(), 2);
    assert_eq!(
        visit_locations(&memory, locations, |ids, _| {
            if ids.contains(&2) {
                Err("earlier location invalid")
            } else {
                Ok(())
            }
        }),
        Err("earlier location invalid")
    );
    let clean = TracedMemory {
        blocked: None,
        ..memory
    };
    visited.clear();
    assert_eq!(
        visit_locations(&clean, locations, |ids, _| {
            visited.extend_from_slice(ids);
            Ok(())
        }),
        Ok(())
    );
    assert_eq!(visited, (1..131).collect::<Vec<_>>());
    assert!(!clean.spans.borrow().contains(&(0x1000, 8)));
    assert!(!clean.spans.borrow().contains(&(0x1000 + 131 * 8, 8)));
}

#[test]
fn building_batches_preserve_full_values_and_earlier_dependent_faults() {
    let mut bytes = vec![0; 0x17000];
    for slot in 0..65_usize {
        let building = 0x1000 + slot * 0x200;
        let template = 0xa000 + slot * 0x300;
        put(
            &mut bytes,
            slot * 8,
            (0x1000 + building as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x400 + slot * 4, (slot as i32).to_le_bytes());
        put(
            &mut bytes,
            building,
            (0x1000 + cbuilding::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            building + cbuilding::TYPE_ID,
            (slot as u32 + 42).to_le_bytes(),
        );
        put(
            &mut bytes,
            building + cbuilding::LEVEL,
            (-(slot as i16)).to_le_bytes(),
        );
        put(
            &mut bytes,
            building + cbuilding::HEALTHY_LEVEL,
            (i16::MIN + slot as i16).to_le_bytes(),
        );
        put(
            &mut bytes,
            building + cbuilding::PARTIAL_HEALTH,
            (i64::MIN + slot as i64).to_le_bytes(),
        );
        put(
            &mut bytes,
            building + cbuilding::BUILDING_STATUS,
            0xdead000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            building + cbuilding::TEMPLATE,
            (0x1000 + template as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            template + cbuilding_template::TYPE_ID,
            (slot as u32 + 42).to_le_bytes(),
        );
        put(
            &mut bytes,
            template + cbuilding_template::DATABASE_ORDINAL,
            (slot as u32).to_le_bytes(),
        );
    }
    let buildings = Array {
        data: 0x1000,
        count: 65,
    };
    let indices = Array {
        data: 0x1400,
        count: 65,
    };
    let seed = cbuilding::Row {
        location: 1,
        status_flags: 2,
        country: 0x8000_0001,
        ..cbuilding::Row::default()
    };
    let mut rows = Vec::new();
    let staged = [cbuilding::LocationBuildings {
        buildings,
        status: 0xdead000,
        indices,
        active: Array::default(),
        seed,
        start: 0,
        len: 65,
    }];
    {
        let memory = TracedMemory {
            inner: SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            calls: Cell::new(0),
            spans: RefCell::new(Vec::new()),
            blocked: None,
        };
        assert_eq!(
            cbuilding::read_rows(&memory, 0x1000, &staged, &mut rows),
            Ok(())
        );
        assert_eq!(memory.calls.get(), 8);
        for (slot, row) in rows.iter().enumerate() {
            assert_eq!(
                *row,
                cbuilding::Row {
                    building_type: slot as u32 + 42,
                    ordinal: slot as u32,
                    level: -(slot as i16),
                    healthy_level: i16::MIN + slot as i16,
                    partial_health: i64::MIN + slot as i64,
                    ..seed
                }
            );
        }
    }
    put(&mut bytes, 0x400 + 2 * 4, (-1_i32).to_le_bytes());
    let memory = TracedMemory {
        inner: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        calls: Cell::new(0),
        spans: RefCell::new(Vec::new()),
        blocked: Some(0x1000 + 9 * 8 + 3),
    };
    assert_eq!(
        cbuilding::read_rows(&memory, 0x1000, &staged, &mut rows),
        Err("building index map differs from pointer vector")
    );
    drop(memory);
    put(&mut bytes, 0x400 + 2 * 4, 2_i32.to_le_bytes());
    let memory = TracedMemory {
        inner: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        calls: Cell::new(0),
        spans: RefCell::new(Vec::new()),
        blocked: Some(0x1000 + 0xa000 + 32 * 0x300 + cbuilding_template::DATABASE_ORDINAL + 3),
    };
    assert_eq!(
        cbuilding::read_rows(&memory, 0x1000, &staged, &mut rows),
        Err("building template ordinal unreadable")
    );
}

#[test]
fn a_map_scale_building_census_batches_authentication_and_dependent_stages() {
    const PROVINCES: usize = 130;
    const STATES: usize = 3;
    const PER_PROVINCE: usize = 2;
    const PROVINCE_STRIDE: usize = 0x400;
    const STATE_STRIDE: usize = 0x200;
    const BUILDING_STRIDE: usize = 0x200;
    const TEMPLATE_STRIDE: usize = cbuilding_template::SIZE;
    const PROVINCE_SLOTS: usize = 0x1000;
    const STATE_SLOTS: usize = 0x2000;
    const PROVINCE_BASE: usize = 0x3000;
    const STATE_BASE: usize = PROVINCE_BASE + (PROVINCES + 1) * PROVINCE_STRIDE;
    const VECTOR_BASE: usize = STATE_BASE + (STATES + 1) * STATE_STRIDE;
    const BUILDING_BASE: usize = VECTOR_BASE + (PROVINCES + 1) * 0x100;
    const TEMPLATE_BASE: usize = BUILDING_BASE + (PROVINCES * PER_PROVINCE + 1) * BUILDING_STRIDE;
    let mut bytes = vec![0; TEMPLATE_BASE + (PROVINCES * PER_PROVINCE + 1) * TEMPLATE_STRIDE];
    vector(
        &mut bytes,
        cgame_state::STATES,
        0x1000 + STATE_SLOTS as u64,
        STATES as i32 + 1,
    );
    vector(
        &mut bytes,
        cgame_state::PROVINCES,
        0x1000 + PROVINCE_SLOTS as u64,
        PROVINCES as i32 + 1,
    );
    for state in 1..=STATES {
        let object = STATE_BASE + state * STATE_STRIDE;
        put(
            &mut bytes,
            STATE_SLOTS + state * 8,
            (0x1000 + object as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            object,
            (0x1000 + cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            object + cstate::ID,
            (state as u32).to_le_bytes(),
        );
        let status = object + cstate::BUILDING_STATUS;
        put(
            &mut bytes,
            status,
            (0x1000 + VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, status + FLAGS, STATE_FLAG.to_le_bytes());
        put(&mut bytes, status + ID, (state as u32).to_le_bytes());
    }
    for province in 1..=PROVINCES {
        let object = PROVINCE_BASE + province * PROVINCE_STRIDE;
        let state = (province - 1) % STATES + 1;
        put(
            &mut bytes,
            PROVINCE_SLOTS + province * 8,
            (0x1000 + object as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            object,
            (0x1000 + cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            object + cprovince::ID,
            (province as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            object + cprovince::STATE,
            (0x1000 + (STATE_BASE + state * STATE_STRIDE) as u64).to_le_bytes(),
        );
        let status = object + cprovince::BUILDING_STATUS;
        let pointers = VECTOR_BASE + province * 0x100;
        let indices = pointers + 0x80;
        put(
            &mut bytes,
            status,
            (0x1000 + VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, status + FLAGS, PROVINCE_FLAG.to_le_bytes());
        put(&mut bytes, status + ID, (province as u32).to_le_bytes());
        put(&mut bytes, status + COUNTRY, 7_u32.to_le_bytes());
        vector(
            &mut bytes,
            status + BUILDINGS,
            0x1000 + pointers as u64,
            PER_PROVINCE as i32,
        );
        vector(
            &mut bytes,
            status + BUILDING_INDICES,
            0x1000 + indices as u64,
            PER_PROVINCE as i32,
        );
        for slot in 0..PER_PROVINCE {
            let ordinal = (province - 1) * PER_PROVINCE + slot;
            let building = BUILDING_BASE + ordinal * BUILDING_STRIDE;
            let template = TEMPLATE_BASE + ordinal * TEMPLATE_STRIDE;
            put(
                &mut bytes,
                pointers + slot * 8,
                (0x1000 + building as u64).to_le_bytes(),
            );
            put(&mut bytes, indices + slot * 4, (slot as i32).to_le_bytes());
            put(
                &mut bytes,
                building,
                (0x1000 + cbuilding::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                building + cbuilding::TYPE_ID,
                (slot as u32 + 5).to_le_bytes(),
            );
            put(
                &mut bytes,
                building + cbuilding::LEVEL,
                (province as i16).to_le_bytes(),
            );
            put(
                &mut bytes,
                building + cbuilding::BUILDING_STATUS,
                (0x1000 + status as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                building + cbuilding::TEMPLATE,
                (0x1000 + template as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                template + cbuilding_template::TYPE_ID,
                (slot as u32 + 5).to_le_bytes(),
            );
            put(
                &mut bytes,
                template + cbuilding_template::DATABASE_ORDINAL,
                (slot as u32).to_le_bytes(),
            );
        }
    }
    let source = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let census = crate::read_census::ReadCensus::new(&source);
    let mut states = Vec::new();
    let mut provinces = Vec::new();
    assert_eq!(
        walk(&census, 0x1000, 0x1000, &mut states, &mut provinces),
        Walk {
            states: Outcome::Rows { len: 0 },
            provinces: Outcome::Rows {
                len: PROVINCES * PER_PROVINCE
            }
        }
    );
    assert_eq!(
        provinces[3],
        cbuilding::Row {
            location: 2,
            building_type: 6,
            ordinal: 1,
            status_flags: PROVINCE_FLAG,
            country: 7,
            level: 2,
            healthy_level: 0,
            partial_health: 0,
            active: false,
        }
    );
    let batches = PROVINCES.div_ceil(64);
    let expected =
        2 + 2 * STATES.div_ceil(64) + 4 * batches + 4 * (PROVINCES * PER_PROVINCE).div_ceil(64);
    assert_eq!(census.calls(), expected);
    assert_eq!(census.calls(), 36);
    assert_eq!(
        census.bytes(),
        32 + STATES * (8 + 80)
            + PROVINCES * (8 + 88 + 12 + 8)
            + PROVINCES * PER_PROVINCE * (8 + 40 + 8 + 4)
    );
    assert_eq!(census.bytes(), 30_976);
}

#[test]
fn a_shared_batch_keeps_every_locations_own_backlink_ordinals_and_duplicates() {
    const LOCATIONS: usize = 3;
    const PER_LOCATION: usize = 4;
    let mut bytes = vec![0; 0x9000];
    for location in 0..LOCATIONS {
        for slot in 0..PER_LOCATION {
            let building = 0x2000 + (location * PER_LOCATION + slot) * 0x100;
            let template = 0x6000 + (location * PER_LOCATION + slot) * 0x40;
            put(
                &mut bytes,
                location * 0x100 + slot * 8,
                (0x1000 + building as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                0x800 + location * 0x100 + slot * 4,
                (slot as i32).to_le_bytes(),
            );
            put(
                &mut bytes,
                building,
                (0x1000 + cbuilding::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                building + cbuilding::TYPE_ID,
                (slot as u32 + 7).to_le_bytes(),
            );
            put(
                &mut bytes,
                building + cbuilding::LEVEL,
                (location as i16 + 1).to_le_bytes(),
            );
            put(
                &mut bytes,
                building + cbuilding::BUILDING_STATUS,
                (0xd0000 + location as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                building + cbuilding::TEMPLATE,
                (0x1000 + template as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                template + cbuilding_template::TYPE_ID,
                (slot as u32 + 7).to_le_bytes(),
            );
            put(
                &mut bytes,
                template + cbuilding_template::DATABASE_ORDINAL,
                (slot as u32).to_le_bytes(),
            );
        }
    }
    let mut staged: [cbuilding::LocationBuildings; LOCATIONS] =
        std::array::from_fn(|location| cbuilding::LocationBuildings {
            buildings: Array {
                data: 0x1000 + location * 0x100,
                count: PER_LOCATION as u32,
            },
            status: 0xd0000 + location,
            indices: Array {
                data: 0x1800 + location * 0x100,
                count: PER_LOCATION as u32,
            },
            active: Array::default(),
            seed: cbuilding::Row {
                location: location as u32 + 1,
                ..cbuilding::Row::default()
            },
            start: location * PER_LOCATION,
            len: PER_LOCATION,
        });
    let mut rows = Vec::new();
    let memory = TracedMemory {
        inner: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        calls: Cell::new(0),
        spans: RefCell::new(Vec::new()),
        blocked: None,
    };
    assert_eq!(
        cbuilding::read_rows(&memory, 0x1000, &staged, &mut rows),
        Ok(())
    );
    assert_eq!(memory.calls.get(), 4);
    for location in 0..LOCATIONS {
        for slot in 0..PER_LOCATION {
            assert_eq!(
                rows[location * PER_LOCATION + slot],
                cbuilding::Row {
                    location: location as u32 + 1,
                    building_type: slot as u32 + 7,
                    ordinal: slot as u32,
                    level: location as i16 + 1,
                    ..cbuilding::Row::default()
                }
            );
        }
    }
    drop(memory);
    let building = |location: usize, slot: usize| {
        0x1000 + 0x2000 + (location * PER_LOCATION + slot) as u64 * 0x100
    };
    put(&mut bytes, 0x8000, building(1, 1).to_le_bytes());
    put(&mut bytes, 0x8008, building(1, 3).to_le_bytes());
    staged[1].active = Array {
        data: 0x9000,
        count: 2,
    };
    let listed = |rows: &[cbuilding::Row]| -> Vec<usize> {
        (0..rows.len()).filter(|&row| rows[row].active).collect()
    };
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        cbuilding::read_rows(&memory, 0x1000, &staged, &mut rows),
        Ok(())
    );
    assert_eq!(listed(&rows), [PER_LOCATION + 1, PER_LOCATION + 3]);
    put(&mut bytes, 0x8000, building(1, 3).to_le_bytes());
    put(&mut bytes, 0x8008, building(1, 1).to_le_bytes());
    let unlisted = Err("active building vector is not an ordered subset of the building vector");
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        cbuilding::read_rows(&memory, 0x1000, &staged, &mut rows),
        unlisted
    );
    put(&mut bytes, 0x8000, building(0, 1).to_le_bytes());
    put(&mut bytes, 0x8008, building(1, 3).to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        cbuilding::read_rows(&memory, 0x1000, &staged, &mut rows),
        unlisted
    );
    staged[1].active.count = PER_LOCATION as u32 + 1;
    assert_eq!(
        cbuilding::read_rows(&memory, 0x1000, &staged, &mut rows),
        Err("active building vector exceeds building vector")
    );
    staged[1].active = Array::default();
    put(
        &mut bytes,
        0x2000 + PER_LOCATION * 0x100 + cbuilding::BUILDING_STATUS,
        0xd0000_u64.to_le_bytes(),
    );
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        cbuilding::read_rows(&memory, 0x1000, &staged, &mut rows),
        Err("building status backlink mismatch")
    );
    put(
        &mut bytes,
        0x2000 + PER_LOCATION * 0x100 + cbuilding::BUILDING_STATUS,
        (0xd0000_u64 + 1).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x6000 + (PER_LOCATION + 1) * 0x40 + cbuilding_template::TYPE_ID,
        7_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x2000 + (PER_LOCATION + 1) * 0x100 + cbuilding::TYPE_ID,
        7_u32.to_le_bytes(),
    );
    put(&mut bytes, 0x800 + 0x100 + 4, 0_i32.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        cbuilding::read_rows(&memory, 0x1000, &staged, &mut rows),
        Err("building index map differs from pointer vector")
    );
    put(&mut bytes, 0x800 + 0x100 + 4, 1_i32.to_le_bytes());
    put(
        &mut bytes,
        0x6000 + (PER_LOCATION + 1) * 0x40 + cbuilding_template::DATABASE_ORDINAL,
        1_u32.to_le_bytes(),
    );
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        cbuilding::read_rows(&memory, 0x1000, &staged, &mut rows),
        Err("duplicate building type in location")
    );
}

#[derive(Default)]
struct Held(Vec<Location>);

impl Carry for Held {
    fn plan(&mut self, location: &Location) -> (usize, bool) {
        self.0.push(*location);
        (self.0.len() - 1, false)
    }
    fn carry(&mut self, _: usize, _: &mut [cbuilding::Row]) {}
    fn read(&mut self, _: usize, _: &[cbuilding::Row]) {}
}

fn traced(bytes: &[u8], blocked: Option<usize>) -> TracedMemory<'_> {
    TracedMemory {
        inner: SliceMemory {
            base: 0x1000,
            bytes,
        },
        calls: Cell::new(0),
        spans: RefCell::new(Vec::new()),
        blocked,
    }
}

#[test]
fn roots_are_the_two_location_table_headers() {
    let bytes = fixture();
    let memory = traced(&bytes, None);
    let headers = roots(&memory, 0x1000).unwrap();
    assert_eq!(headers[0], bytes[cgame_state::STATES..][..16]);
    assert_eq!(headers[1], bytes[cgame_state::PROVINCES..][..16]);
    assert_eq!(memory.calls.get(), 1);
    let blocked = traced(&bytes, Some(0x1000 + cgame_state::PROVINCES + 4));
    assert_eq!(roots(&blocked, 0x1000), None);
}

#[test]
fn a_touched_status_refreshes_its_rows_from_the_status_alone() {
    let mut bytes = fixture();
    let memory = traced(&bytes, None);
    let mut states = Vec::new();
    let mut provinces = Vec::new();
    let mut held = Held::default();
    assert_eq!(
        walk_carried(
            &memory,
            0x1000,
            0x1000,
            &mut states,
            &mut provinces,
            &mut held
        ),
        Walk {
            states: Outcome::Rows { len: 1 },
            provinces: Outcome::Rows { len: 2 },
        }
    );
    let [state, first, second] = <[Location; 3]>::try_from(&held.0[..]).unwrap();
    memory.spans.take();
    memory.calls.set(0);
    let mut output = vec![cbuilding::Row::default(); 1];
    let mut fresh = [state; 3];
    assert_eq!(
        refresh(
            &memory,
            0x1000,
            &[second, state, first],
            &mut fresh,
            &mut output
        ),
        Ok(3)
    );
    assert_eq!(fresh, [second, state, first]);
    assert_eq!(output, [provinces[1], states[0], provinces[0]]);
    assert_eq!(memory.calls.get(), 5);
    let locations = [
        (0x1500, cstate::BUILDING_STATUS),
        (0x2000, cprovince::BUILDING_STATUS),
        (0x2300, cprovince::BUILDING_STATUS),
    ];
    for (address, _) in memory.spans.take() {
        assert!(!(0x1000..0x1400).contains(&address), "{address:#x}");
        assert!(
            !locations
                .iter()
                .any(|&(location, status)| (location..location + status).contains(&address)),
            "{address:#x}"
        );
    }
    assert_eq!(refresh(&memory, 0x1000, &[], &mut [], &mut output), Ok(0));
    assert!(memory.spans.take().is_empty());
    drop(memory);

    put(&mut bytes, 0x1c00 + cbuilding::LEVEL, 3_i16.to_le_bytes());
    let memory = traced(&bytes, None);
    assert_eq!(
        refresh(&memory, 0x1000, &[second], &mut fresh[..1], &mut output),
        Ok(1)
    );
    assert_eq!(output[0].level, 3);
    assert_ne!(output[0], provinces[1]);
    let blocked = traced(&bytes, Some(0x1000 + 0x1c00 + cbuilding::LEVEL));
    assert_eq!(
        refresh(
            &blocked,
            0x1000,
            &[state, second],
            &mut fresh[..2],
            &mut output
        ),
        Err("building fields unreadable")
    );
    drop(memory);

    let mut changed = bytes.clone();
    put(&mut changed, 0x1490 + COUNTRY, 7_u32.to_le_bytes());
    let memory = traced(&changed, None);
    assert_eq!(
        refresh(
            &memory,
            0x1000,
            &[state, second],
            &mut fresh[..2],
            &mut output
        ),
        Err("held building status differs from its held read")
    );
    let mut replaced = bytes.clone();
    put(&mut replaced, 0x1490, 0_u64.to_le_bytes());
    let memory = traced(&replaced, None);
    assert_eq!(
        refresh(&memory, 0x1000, &[second], &mut fresh[..1], &mut output),
        Err("held building status vtable unreadable or changed")
    );
}
