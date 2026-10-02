use super::*;
use crate::{cgame_state, memory::SliceMemory};

const MEMORY_BASE: usize = 0x1000;
const MEMORY_END: usize = 0xa000;
/// Production statuses of the two fixture countries.
const STATUSES: [usize; 2] = [0x6000, 0x7800];
const GROUPS: [usize; 3] = [MILITARY_FACTORIES, NAVAL_FACTORIES, CIVILIAN_FACTORIES];
const UNMAPPED: u64 = 0x2_0000;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - MEMORY_BASE..address - MEMORY_BASE + value.len()].copy_from_slice(value);
}

/// Distinct values per status and group, with signed and extreme values so a
/// swapped or narrowed field shows.
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
    // Every byte of the span starts as junk, so an unread field cannot pass
    // as a zero.
    bytes[status + MILITARY_FACTORIES - MEMORY_BASE
        ..status + MILITARY_FACTORIES + FACTORY_SPAN - MEMORY_BASE]
        .fill(0xa5);
    put(bytes, status, &(VTABLE_RVA as u64).to_le_bytes());
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
        put(bytes, status + offset, &(vtable as u64).to_le_bytes());
        put(
            bytes,
            status + offset + civ_factory_consumer::ALLOCATED,
            &allocated.to_le_bytes(),
        );
        // The pre-allocation count is not recorded.
        put(
            bytes,
            status + offset + civ_factory_consumer::AVAILABLE,
            &(-allocated).to_le_bytes(),
        );
    }
}

/// Canonical slots 1 and 2, and slot 3 aliasing slot 1's object.
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
            &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
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
fn truncation_counts_whole_countries_without_reading_their_statuses() {
    let mut bytes = fixture();
    // The second country's status is never read while its row cannot fit.
    put(
        &mut bytes,
        0x5000 + ccountry::PRODUCTION_STATUS,
        &UNMAPPED.to_le_bytes(),
    );
    let (outcome, rows) = capture(&bytes, 1);
    assert_eq!(
        outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert_eq!(rows[0], expected_row(0, 3));
    assert_eq!(
        capture(&fixture(), 0).0,
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
    // Each case overwrites one qword of the second country.
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
    // The vtable and owner are mapped, the factory span runs past the end.
    let status = MEMORY_END - MILITARY_FACTORIES - FACTORY_SPAN + 8;
    put(&mut bytes, status, &(VTABLE_RVA as u64).to_le_bytes());
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
