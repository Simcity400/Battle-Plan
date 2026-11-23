use super::*;
use crate::cgame_state;
use engine_native_memory::memory::SliceMemory;

const MEMORY_BASE: usize = 0x1000;
const MEMORY_END: usize = 0xa000;
const STATUSES: [usize; 2] = [0x6000, 0x7800];
const GROUPS: [usize; 3] = [MILITARY_FACTORIES, NAVAL_FACTORIES, CIVILIAN_FACTORIES];
const UNMAPPED: u64 = 0x2_0000;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - MEMORY_BASE..address - MEMORY_BASE + value.len()].copy_from_slice(value);
}

fn expected_group(status: usize, group: usize) -> FactoryGroup {
    let tag = (status * 3 + group) as i32 + 1;
    let wide = |field: i64| ((tag as i64) << 36) + field;
    FactoryGroup {
        levels: match (status, group) {
            (0, 0) => i32::MIN,
            (0, 1) => i32::MAX,
            (0, 2) => -3,
            _ => tag * 7,
        },
        total_raw: if status == 0 && group == 0 {
            i64::MAX
        } else {
            wide(1)
        },
        controlled_raw: wide(2),
        owned_raw: -wide(3),
        damaged_raw: if status == 1 && group == 1 {
            i64::MIN
        } else {
            wide(4)
        },
        used: tag * 11,
        unavailable: -tag,
        from_subjects: tag * 13,
        to_overlord: tag * 17,
        from_others: if status == 0 && group == 2 {
            i32::MAX
        } else {
            tag * 19
        },
        to_others: if status == 1 && group == 2 {
            i32::MIN
        } else {
            tag * 23
        },
    }
}

fn expected_row(status: usize, country: u32) -> FactoryRow {
    let tag = status as i32 * 100;
    FactoryRow {
        country,
        military: expected_group(status, 0),
        naval: expected_group(status, 1),
        civilian: expected_group(status, 2),
        civilian_license_exports: tag + 29,
        civilian_trade_exports: tag + 31,
        consumer_goods: tag + 37,
        special_projects: tag + 41,
        license_imports: tag + 43,
        trade_imports: tag + 47,
        contract_payments: tag + 53,
    }
}

fn write_group(bytes: &mut [u8], at: usize, group: FactoryGroup) {
    use factory_group::*;
    for (offset, value) in [
        (TOTAL, group.total_raw),
        (CONTROLLED, group.controlled_raw),
        (OWNED, group.owned_raw),
        (DAMAGED, group.damaged_raw),
    ] {
        put(bytes, at + offset, &value.to_le_bytes());
    }
    for (offset, value) in [
        (LEVELS, group.levels),
        (USED, group.used),
        (UNAVAILABLE, group.unavailable),
        (FROM_SUBJECTS, group.from_subjects),
        (TO_OVERLORD, group.to_overlord),
        (FROM_OTHERS, group.from_others),
        (TO_OTHERS, group.to_others),
    ] {
        put(bytes, at + offset, &value.to_le_bytes());
    }
}

fn write_status(bytes: &mut [u8], status: usize, country: usize, row: FactoryRow) {
    bytes[status + MILITARY_FACTORIES - MEMORY_BASE
        ..status + MILITARY_FACTORIES + FACTORY_SPAN - MEMORY_BASE]
        .fill(0xa5);
    put(bytes, status, &(VTABLE_RVA.0 as u64).to_le_bytes());
    put(bytes, status + COUNTRY, &(country as u64).to_le_bytes());
    for (at, group) in GROUPS
        .into_iter()
        .zip([row.military, row.naval, row.civilian])
    {
        write_group(bytes, status + at, group);
    }
    let civilian = status + CIVILIAN_FACTORIES;
    put(
        bytes,
        civilian + factory_group::LICENSE_EXPORTS,
        &row.civilian_license_exports.to_le_bytes(),
    );
    put(
        bytes,
        civilian + factory_group::TRADE_EXPORTS,
        &row.civilian_trade_exports.to_le_bytes(),
    );
    for (&(offset, vtable), allocated) in CONSUMERS.iter().zip([
        row.consumer_goods,
        row.special_projects,
        row.license_imports,
        row.trade_imports,
        row.contract_payments,
    ]) {
        put(bytes, status + offset, &(vtable.0 as u64).to_le_bytes());
        put(
            bytes,
            status + offset + civ_factory_consumer::ALLOCATED,
            &allocated.to_le_bytes(),
        );
        put(
            bytes,
            status + offset + civ_factory_consumer::AVAILABLE,
            &(-allocated).to_le_bytes(),
        );
    }
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; MEMORY_END - MEMORY_BASE];
    for (header, data) in [
        (cgame_state::COUNTRIES, 0x1800_u64),
        (cgame_state::COUNTRY_LINKS, 0x1900),
    ] {
        put(&mut bytes, MEMORY_BASE + header, &data.to_le_bytes());
        put(&mut bytes, MEMORY_BASE + header + 8, &4_u32.to_le_bytes());
        put(&mut bytes, MEMORY_BASE + header + 12, &4_u32.to_le_bytes());
    }
    for (slot, pointer, canonical) in [(1, 0x3000_u64, 1_u32), (2, 0x5000, 2), (3, 0x3000, 1)] {
        put(&mut bytes, 0x1800 + slot * 8, &pointer.to_le_bytes());
        put(&mut bytes, 0x1900 + slot * 4, &canonical.to_le_bytes());
    }
    for (index, (country, id)) in [(0x3000, 3_u32), (0x5000, 2)].into_iter().enumerate() {
        put(
            &mut bytes,
            country,
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, country + ccountry::ID, &id.to_le_bytes());
        put(
            &mut bytes,
            country + ccountry::PRODUCTION_STATUS,
            &(STATUSES[index] as u64).to_le_bytes(),
        );
        write_status(
            &mut bytes,
            STATUSES[index],
            country,
            expected_row(index, id),
        );
    }
    bytes
}

fn capture(bytes: &[u8], capacity: usize) -> (Outcome, Vec<FactoryRow>) {
    let mut rows = vec![FactoryRow::default(); capacity];
    let outcome = walk_factories(
        &SliceMemory {
            base: MEMORY_BASE,
            bytes,
        },
        0,
        MEMORY_BASE,
        &mut rows,
    );
    (outcome, rows)
}

fn unavailable(reason: &'static str) -> Outcome {
    Outcome::Unavailable { reason }
}

#[test]
fn full_read_preserves_every_stored_count_once_per_canonical_country() {
    let (outcome, rows) = capture(&fixture(), 3);
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    assert_eq!(rows[0], expected_row(0, 3));
    assert_eq!(rows[1], expected_row(1, 2));
    assert_eq!(rows[2], FactoryRow::default());
}

#[test]
fn the_count_probe_counts_whole_countries_without_reading_their_statuses() {
    let mut bytes = fixture();
    put(
        &mut bytes,
        0x5000 + ccountry::PRODUCTION_STATUS,
        &UNMAPPED.to_le_bytes(),
    );
    assert_eq!(
        walk_factories_buffer(
            &SliceMemory {
                base: MEMORY_BASE,
                bytes: &bytes,
            },
            0,
            MEMORY_BASE,
            Buffer::Count,
        ),
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
    assert_eq!(capture(&fixture(), 2).0, Outcome::Rows { len: 2 });
}

#[test]
fn a_status_that_cannot_be_read_or_authenticated_withholds_the_table() {
    let second = STATUSES[1];
    let pointer = 0x5000 + ccountry::PRODUCTION_STATUS;
    for (name, address, value, reason) in [
        (
            "null status",
            pointer,
            0,
            "production status pointer unreadable or null",
        ),
        (
            "unmapped status",
            pointer,
            UNMAPPED,
            "production status vtable mismatch",
        ),
        (
            "foreign vtable",
            second,
            0x1234,
            "production status vtable mismatch",
        ),
        (
            "another country's status",
            second + COUNTRY,
            0x3000,
            "production status country mismatch",
        ),
        (
            "moved trade consumer",
            second + TRADE,
            0x5678,
            "civilian factory consumer vtable mismatch",
        ),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, address, &value.to_le_bytes());
        assert_eq!(capture(&bytes, 4).0, unavailable(reason), "{name}");
    }
}

#[test]
fn a_short_factory_span_is_a_failure_not_zero_counts() {
    let mut bytes = fixture();
    let status = MEMORY_END - MILITARY_FACTORIES - FACTORY_SPAN + 8;
    put(&mut bytes, status, &(VTABLE_RVA.0 as u64).to_le_bytes());
    put(&mut bytes, status + COUNTRY, &0x5000_u64.to_le_bytes());
    put(
        &mut bytes,
        0x5000 + ccountry::PRODUCTION_STATUS,
        &(status as u64).to_le_bytes(),
    );
    assert_eq!(
        capture(&bytes, 4).0,
        unavailable("production status factory totals unreadable")
    );
}

#[test]
fn a_country_table_without_countries_is_unavailable() {
    let mut bytes = fixture();
    for slot in 1..4 {
        put(&mut bytes, 0x1800 + slot * 8, &0_u64.to_le_bytes());
    }
    assert_eq!(
        capture(&bytes, 4).0,
        unavailable("country table has no readable countries")
    );
}

fn population(count: usize) -> Vec<u8> {
    let pointer_data = 0x3000;
    let link_data = pointer_data + (count + 1) * 8;
    let country_data = (link_data + (count + 1) * 4).next_multiple_of(16);
    let country_stride = (ccountry::PRODUCTION_STATUS + 8).next_multiple_of(16);
    let status_data = country_data + count * country_stride;
    let status_stride = 0x500;
    let mut bytes = vec![0; status_data + count * status_stride - MEMORY_BASE];
    for (at, data) in [
        (cgame_state::COUNTRIES, pointer_data),
        (cgame_state::COUNTRY_LINKS, link_data),
    ] {
        put(&mut bytes, MEMORY_BASE + at, &(data as u64).to_le_bytes());
        put(
            &mut bytes,
            MEMORY_BASE + at + 8,
            &((count + 1) as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            MEMORY_BASE + at + 12,
            &((count + 1) as u32).to_le_bytes(),
        );
    }
    for index in 0..count {
        let slot = index + 1;
        let country = country_data + index * country_stride;
        let status = status_data + index * status_stride;
        put(
            &mut bytes,
            pointer_data + slot * 8,
            &(country as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            link_data + slot * 4,
            &(slot as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            country,
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            country + ccountry::ID,
            &(slot as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            country + ccountry::PRODUCTION_STATUS,
            &(status as u64).to_le_bytes(),
        );
        write_status(
            &mut bytes,
            status,
            country,
            expected_row(index, slot as u32),
        );
    }
    bytes
}

#[test]
fn growing_factory_storage_does_not_hide_an_original_status_fault() {
    let mut bytes = fixture();
    put(
        &mut bytes,
        0x5000 + ccountry::PRODUCTION_STATUS,
        &UNMAPPED.to_le_bytes(),
    );
    assert_eq!(
        walk_factories(
            &SliceMemory {
                base: MEMORY_BASE,
                bytes: &bytes
            },
            0,
            MEMORY_BASE,
            &mut Vec::new()
        ),
        Outcome::Unavailable {
            reason: "production status vtable mismatch"
        }
    );
}

fn serial(memory: &impl Memory, count: usize) -> Outcome {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, 0, MEMORY_BASE)?;
        let mut vtables = [0; 6];
        for (vtable, rva) in vtables
            .iter_mut()
            .zip(std::iter::once(VTABLE_RVA).chain(CONSUMERS.iter().map(|&(_, rva)| rva)))
        {
            *vtable = rva.0;
        }
        let mut len = 0;
        for slot in 1..countries.count() {
            if let Some(country) = countries.at(slot)? {
                read_factories(memory, &vtables, country)?;
                len += 1;
            }
        }
        Ok(Outcome::Rows { len })
    })();
    let _ = count;
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn faulty<'a>(bytes: &'a [u8], at: &[usize], once: bool) -> engine_native_memory::batch_fallback::Faulty<'a> {
    engine_native_memory::batch_fallback::Faulty::new(
        SliceMemory {
            base: MEMORY_BASE,
            bytes,
        },
        at.iter().map(|&address| address..address + 1).collect(),
        once,
    )
}

fn addresses(bytes: &[u8], index: usize) -> (usize, usize, usize) {
    let memory = SliceMemory {
        base: MEMORY_BASE,
        bytes,
    };
    let countries = ccountry::Reader::new(&memory, 0, MEMORY_BASE).unwrap();
    let country = countries.at(index as u32 + 1).unwrap().unwrap().pointer;
    let pointer = country + ccountry::PRODUCTION_STATUS;
    let status = u64::from_le_bytes(bytes[pointer - MEMORY_BASE..][..8].try_into().unwrap());
    (country, pointer, status as usize)
}

struct Counted<'a> {
    memory: SliceMemory<'a>,
    calls: std::cell::Cell<usize>,
}
impl Memory for Counted<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.memory.copy(address, output)
    }
    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        self.memory.copy_records(sources, layout, destination)
    }
}

#[test]
fn batched_factories_equal_the_serial_walk() {
    let count = 130;

    let bytes = population(count);
    let source = SliceMemory {
        base: MEMORY_BASE,
        bytes: &bytes,
    };
    assert_eq!(serial(&source, count), Outcome::Rows { len: count });
    let mut cache = engine_native_intelligence::country_memory::Cache::with_capacity(1 << 16);
    let view = cache.prepare(&source, 0, MEMORY_BASE);
    let counted = Counted {
        memory: SliceMemory {
            base: MEMORY_BASE,
            bytes: &bytes,
        },
        calls: std::cell::Cell::new(0),
    };
    let mut rows = Vec::new();
    let mut plain = Vec::new();
    assert_eq!(
        walk_factories(&view, 0, MEMORY_BASE, &mut rows),
        Outcome::Rows { len: count }
    );
    assert_eq!(
        walk_factories(&counted, 0, MEMORY_BASE, &mut plain),
        Outcome::Rows { len: count }
    );
    for (index, row) in rows[..count].iter().enumerate() {
        assert_eq!(*row, expected_row(index, index as u32 + 1));
    }
    assert_eq!(plain[..count], rows[..count]);
    let serial_calls = Counted {
        memory: SliceMemory {
            base: MEMORY_BASE,
            bytes: &bytes,
        },
        calls: std::cell::Cell::new(0),
    };
    assert_eq!(serial(&serial_calls, count), Outcome::Rows { len: count });
    assert_eq!(
        counted.calls.get() + 4 * count,
        serial_calls.calls.get() + 2 * count.div_ceil(BATCH)
    );
}

#[test]
fn a_persistent_fault_gives_the_serial_result() {
    let count = 100;
    let mut bytes = population(count);
    let (_, pointer, status) = addresses(&bytes, 40);
    let (_, _, earlier) = addresses(&bytes, 44);
    let (later, _, _) = addresses(&bytes, 50);
    for persistent in [
        vec![status + MILITARY_FACTORIES + 8],
        vec![status],
        vec![pointer],
        vec![earlier + COUNTRY, later],
    ] {
        let memory = faulty(&bytes, &persistent, false);
        let before = engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::CountryFactories);
        let mut rows = Vec::new();
        let outcome = walk_factories(&memory, 0, MEMORY_BASE, &mut rows);
        assert!(
            engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::CountryFactories) > before,
            "{persistent:x?}"
        );
        assert!(
            matches!(outcome, Outcome::Unavailable { .. }),
            "{persistent:x?}"
        );
        assert_eq!(outcome, serial(&memory, count), "{persistent:x?}");
    }
    put(&mut bytes, pointer, &0_u64.to_le_bytes());
    let memory = faulty(&bytes, &[], false);
    let before = engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::CountryFactories);
    assert_eq!(
        walk_factories(&memory, 0, MEMORY_BASE, &mut Vec::new()),
        serial(&memory, count)
    );
    assert!(engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::CountryFactories) > before);
}

#[test]
fn a_one_shot_fault_refuses_no_more_than_the_serial_walk() {
    let count = 100;
    let bytes = population(count);
    let (_, pointer, status) = addresses(&bytes, 40);
    for once in [pointer, status, status + MILITARY_FACTORIES + 8] {
        let before = engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::CountryFactories);
        let memory = faulty(&bytes, &[once], true);
        let mut rows = Vec::new();
        assert_eq!(
            walk_factories(&memory, 0, MEMORY_BASE, &mut rows),
            Outcome::Rows { len: count },
            "{once:#x}"
        );
        for (index, row) in rows[..count].iter().enumerate() {
            assert_eq!(*row, expected_row(index, index as u32 + 1));
        }
        assert!(
            engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::CountryFactories) > before
        );
    }
}
