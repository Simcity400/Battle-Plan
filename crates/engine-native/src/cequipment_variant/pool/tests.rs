use super::*;
use crate::{cequipment_variant, cgame_state};
use engine_native_memory::memory::SliceMemory;

fn p32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn p64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn array(bytes: &mut [u8], offset: usize, data: usize, count: u32) {
    p64(bytes, offset, data as u64);
    p32(bytes, offset + 8, count);
    p32(bytes, offset + 12, count);
}

fn stockpile_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0xb000];
    array(&mut bytes, 0x1000 + cgame_state::COUNTRIES, 0x2000, 4);
    array(&mut bytes, 0x1000 + cgame_state::COUNTRY_LINKS, 0x2100, 4);
    for (index, pointer) in [0_u64, 0x4000, 0x6000, 0x4000].into_iter().enumerate() {
        p64(&mut bytes, 0x2000 + index * 8, pointer);
    }
    for (index, canonical) in [0_u32, 1, 2, 1].into_iter().enumerate() {
        p32(&mut bytes, 0x2100 + index * 4, canonical);
    }
    for (country, id, manager, data, count) in [
        (0x4000, 3, 0x8000, 0x2200, 1),
        (0x6000, 2, 0x9000, 0x2300, 2),
    ] {
        p64(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
        p32(&mut bytes, country + ccountry::ID, id);
        p64(
            &mut bytes,
            country + ccountry::PRODUCTION_STATUS,
            manager as u64,
        );
        p64(&mut bytes, manager, cproduction_status::VTABLE_RVA.0 as u64);
        p64(
            &mut bytes,
            manager + cproduction_status::COUNTRY,
            country as u64,
        );
        p64(
            &mut bytes,
            manager + cproduction_status::STOCKPILE_POOL,
            VTABLE_RVA.0 as u64,
        );
        array(
            &mut bytes,
            manager + cproduction_status::STOCKPILE,
            data,
            count,
        );
        for ordinal in 0..count as usize {
            p64(&mut bytes, data + ordinal * equipment_entry::STRIDE, 0xa000);
            p64(
                &mut bytes,
                data + ordinal * equipment_entry::STRIDE + 8,
                u64::MAX,
            );
        }
    }
    p64(&mut bytes, 0xa000, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
    p64(&mut bytes, 0xa008, 0xfedc_ba98_0000_0046);
    bytes
}

#[test]
fn stockpiles_authenticate_aliases_and_count_later_countries_after_capacity() {
    let mut bytes = stockpile_fixture();
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut rows = vec![StockpileRow::default(); 4];
    assert_eq!(
        walk_stockpiles_buffer(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut rows)),
        Outcome::Rows { len: 3 }
    );
    assert_eq!(
        (rows[0].country, rows[0].ordinal, rows[0].amount_raw),
        (3, 0, -1)
    );
    assert_eq!((rows[1].country, rows[1].ordinal), (2, 0));
    assert_eq!((rows[2].country, rows[2].ordinal), (2, 1));
    assert_eq!(rows[0].variant_handle, 0xfedc_ba98_0000_0046);
    p64(&mut bytes, 0x2300, 0);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert!(matches!(
        walk_stockpiles_buffer(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut rows)),
        Outcome::Unavailable { .. }
    ));
    p64(&mut bytes, 0x9000 + cproduction_status::COUNTRY, 0x4000);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        walk_stockpiles(&memory, 0, 0x1000, &mut rows),
        Outcome::Unavailable {
            reason: "production status country mismatch"
        }
    );
}

#[test]
fn no_country_census_is_unavailable_but_authenticated_empty_stockpiles_are_empty() {
    let mut bytes = stockpile_fixture();
    array(&mut bytes, 0x8000 + cproduction_status::STOCKPILE, 0, 0);
    array(&mut bytes, 0x9000 + cproduction_status::STOCKPILE, 0, 0);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        walk_stockpiles_buffer(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Count),
        Outcome::Rows { len: 0 }
    );
    bytes[0x2000..0x2020].fill(0);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        walk_stockpiles_buffer(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Count),
        Outcome::Unavailable {
            reason: "stockpile country census contains no countries"
        }
    );
}

#[test]
fn division_dependencies_and_later_pool_headers_are_preserved() {
    let mut bytes = stockpile_fixture();
    let sources = [
        DivisionSource {
            pointer: 0x3000,
            division: 0x8000_0001_0000_1269,
            template: 0xffff_0001_0000_00c3,
            country: 3,
        },
        DivisionSource {
            pointer: 0x3500,
            division: 0x8000_0002_0000_1269,
            template: 0,
            country: 2,
        },
    ];
    for (source, data, count) in [(sources[0], 0x2200, 1), (sources[1], 0x2300, 2)] {
        let pool = source.pointer + carmy::EQUIPMENT_POOL;
        p64(&mut bytes, pool, VTABLE_RVA.0 as u64);
        array(&mut bytes, pool + ITEMS, data, count);
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut rows = vec![DivisionRow::default(); 3];
    assert_eq!(
        walk_divisions(&memory, 0, &sources, &mut rows),
        Outcome::Rows { len: 3 }
    );
    assert_eq!(
        (rows[0].division, rows[0].template, rows[0].country),
        (sources[0].division, sources[0].template, 3)
    );
    assert_eq!(
        (rows[2].division, rows[2].ordinal),
        (sources[1].division, 1)
    );
    assert_eq!(
        walk_divisions_where(
            &memory,
            0,
            &sources,
            |source| source.division >> 32 > 0x8000_0001,
            &mut rows
        ),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        (rows[0].division, rows[1].ordinal),
        (sources[1].division, 1)
    );
    p32(
        &mut bytes,
        sources[1].pointer + carmy::EQUIPMENT_POOL + COUNT,
        u32::MAX,
    );
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        walk_divisions(&memory, 0, &sources, &mut rows),
        Outcome::Unavailable {
            reason: "equipment pool array invalid or unreadable"
        }
    );
}

#[test]
fn filtered_worker_division_equipment_grows_for_mod_pools_without_roster_growth() {
    const COUNTRIES: usize = 50;
    const DATA: usize = 0xc000;
    let sources: Vec<_> = (0..COUNTRIES)
        .map(|index| DivisionSource {
            pointer: 0x3000,
            division: ((index as u64 + 1) << 32) | 0x1269,
            country: index as u32 + 1,
            ..Default::default()
        })
        .collect();
    let pool = sources[0].pointer + carmy::EQUIPMENT_POOL;
    let mut bytes = stockpile_fixture();
    bytes.resize(DATA + 17 * equipment_entry::STRIDE, 0);
    p64(&mut bytes, pool, VTABLE_RVA.0 as u64);
    for ordinal in 0..17 {
        p64(&mut bytes, DATA + ordinal * equipment_entry::STRIDE, 0xa000);
        p64(
            &mut bytes,
            DATA + ordinal * equipment_entry::STRIDE + 8,
            ordinal as u64,
        );
    }
    let mut rows = vec![DivisionRow::default(); 1];
    for entries in [3, 17] {
        array(&mut bytes, pool + ITEMS, DATA, entries);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk_divisions_where(&memory, 0, &sources, |_| true, &mut rows),
            Outcome::Rows {
                len: COUNTRIES * entries as usize
            }
        );
        let last = &rows[COUNTRIES * entries as usize - 1];
        assert_eq!(
            (last.country, last.ordinal),
            (COUNTRIES as u32, entries - 1)
        );
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let rejected = DivisionSource {
        pointer: usize::MAX,
        ..Default::default()
    };
    assert_eq!(
        walk_divisions_where(&memory, 0, &[rejected], |_| false, &mut rows),
        Outcome::Rows { len: 0 }
    );
}

fn bank(entries: usize) -> Vec<u8> {
    const POOL: usize = 0x3000;
    const DATA: usize = 0x4000;
    const VARIANT: usize = 0x8000;
    let mut bytes = vec![0_u8; VARIANT + (entries + 1) * 0x40];
    p64(&mut bytes, POOL + carmy::EQUIPMENT_POOL, VTABLE_RVA.0 as u64);
    array(
        &mut bytes,
        POOL + carmy::EQUIPMENT_POOL + ITEMS,
        DATA,
        entries as u32,
    );
    for entry in 0..entries {
        let variant = VARIANT + entry * 0x40;
        let slot = DATA + entry * equipment_entry::STRIDE;
        p64(&mut bytes, slot + equipment_entry::VARIANT, variant as u64);
        p64(
            &mut bytes,
            slot + equipment_entry::AMOUNT,
            (i64::MIN + entry as i64) as u64,
        );
        p64(&mut bytes, variant, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
        p64(
            &mut bytes,
            variant + 8,
            ((entry as u64 + 1) << 32) | u64::from(cequipment_variant::REFERENCE_TYPE),
        );
    }
    bytes
}

#[test]
fn a_mid_batch_bank_fault_keeps_its_reason_and_stops_before_later_entries() {
    const DATA: usize = 0x4000;
    const VARIANT: usize = 0x8000;
    struct Refuse<'a> {
        memory: SliceMemory<'a>,
        address: usize,
    }
    impl Memory for Refuse<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address <= self.address && self.address - address < destination.len() {
                return false;
            }
            self.memory.copy(address, destination)
        }
    }
    let sources = [DivisionSource {
        pointer: 0x3000,
        division: 5,
        template: 6,
        country: 7,
    }];
    let bytes = bank(3);
    let mut rows = vec![DivisionRow::default(); 3];
    assert_eq!(
        walk_divisions(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            &sources,
            &mut rows
        ),
        Outcome::Rows { len: 3 }
    );
    for (address, reason) in [
        (
            DATA + equipment_entry::STRIDE + equipment_entry::AMOUNT,
            "equipment entry unreadable",
        ),
        (VARIANT + 0x40 + 8, "variant identity unreadable"),
    ] {
        let memory = Refuse {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            address,
        };
        assert_eq!(
            walk_divisions(&memory, 0, &sources, &mut rows),
            Outcome::Unavailable { reason },
            "refused {address:#x}"
        );
    }
    let mut broken = bank(3);
    p64(&mut broken, VARIANT + 0x40, 0);
    assert_eq!(
        walk_divisions(
            &SliceMemory {
                base: 0,
                bytes: &broken
            },
            0,
            &sources,
            &mut rows
        ),
        Outcome::Unavailable {
            reason: "variant vtable or reference type mismatch"
        }
    );
}

#[test]
fn a_bank_census_costs_two_guarded_gathers_per_batch_not_two_per_entry() {
    const ENTRIES: usize = 130;
    const POOL: usize = 0x3000;
    const DATA: usize = 0x4000;
    const VARIANT: usize = 0x8000;
    let mut bytes = vec![0_u8; VARIANT + (ENTRIES + 1) * 0x40];
    p64(&mut bytes, POOL + carmy::EQUIPMENT_POOL, VTABLE_RVA.0 as u64);
    array(
        &mut bytes,
        POOL + carmy::EQUIPMENT_POOL + ITEMS,
        DATA,
        ENTRIES as u32,
    );
    for entry in 0..ENTRIES {
        let variant = VARIANT + entry * 0x40;
        let slot = DATA + entry * equipment_entry::STRIDE;
        p64(&mut bytes, slot + equipment_entry::VARIANT, variant as u64);
        p64(
            &mut bytes,
            slot + equipment_entry::AMOUNT,
            (i64::MIN + entry as i64) as u64,
        );
        p64(&mut bytes, variant, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
        p64(
            &mut bytes,
            variant + 8,
            ((entry as u64 + 1) << 32) | u64::from(cequipment_variant::REFERENCE_TYPE),
        );
    }
    let source = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let census = engine_native_memory::read_census::ReadCensus::new(&source);
    let sources = [DivisionSource {
        pointer: POOL,
        division: 7,
        template: 9,
        country: 11,
    }];
    let mut rows = vec![DivisionRow::default(); ENTRIES];
    assert_eq!(
        walk_divisions(&census, 0, &sources, &mut rows),
        Outcome::Rows { len: ENTRIES }
    );
    assert_eq!(rows[129].ordinal, 129);
    assert_eq!(rows[129].amount_raw, i64::MIN + 129);
    assert_eq!(rows[129].variant_handle >> 32, 130);
    assert_eq!(census.calls(), 2 + 2 * ENTRIES.div_ceil(64));
    assert_eq!(
        census.bytes(),
        24 + ENTRIES * (equipment_entry::STRIDE + 16)
    );
}

#[test]
fn empty_pool_requires_the_correct_object_and_bounded_header() {
    let mut bytes = [0_u8; 0x40];
    bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(walk(&memory, 0, 0x1000, &mut []), Outcome::Rows { len: 0 });
    assert_eq!(
        entries(&memory, 1, 0x1000),
        Err("equipment pool vtable mismatch")
    );
    bytes[COUNT..COUNT + 4].copy_from_slice(&1_u32.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        entries(&memory, 0, 0x1000),
        Err("equipment pool array invalid or unreadable")
    );
    bytes[COUNT..COUNT + 4].fill(0);
    bytes[CAPACITY..CAPACITY + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert!(entries(&memory, 0, 0x1000).is_err());
}

#[test]
fn model_variant_hints_cover_only_positive_exact_canonical_targets() {
    let hints = |pointer: usize, amount: i64| {
        let mut entry = [0; 16];
        p64(&mut entry, 0, pointer as u64);
        p64(&mut entry, 8, amount as u64);
        let mut targets = Vec::new();
        let valid = hint_model_variant(&entry, |address, len| targets.push((address, len)));
        (valid, targets)
    };
    for amount in [i64::MIN, -1, 0] {
        assert_eq!(hints(usize::MAX, amount), (true, vec![]));
    }
    for pointer in [1, 0x1000, 0x1003] {
        assert_eq!(
            hints(pointer, i64::MAX),
            (
                true,
                vec![
                    (pointer, 16),
                    (pointer + cequipment_variant::CLASSIFIER_FLAGS, 8),
                ]
            )
        );
    }
    for pointer in [
        0,
        usize::MAX,
        usize::MAX - 15,
        0x0000_8000_0000_0000,
        0x0000_7fff_ffff_fff8,
    ] {
        assert_eq!(hints(pointer, 1), (false, vec![]));
    }
    for pointer in [
        usize::MAX - cequipment_variant::CLASSIFIER_FLAGS + 1,
        usize::MAX - cequipment_variant::CLASSIFIER_FLAGS - 3,
        0x0000_7fff_ffff_ffff - cequipment_variant::CLASSIFIER_FLAGS - 3,
    ] {
        assert_eq!(hints(pointer, 1), (false, vec![(pointer, 16)]));
    }
}
