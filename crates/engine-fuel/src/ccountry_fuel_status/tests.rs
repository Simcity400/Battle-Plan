use super::*;
use engine_native::cgame_state;
use engine_native_memory::memory::SliceMemory;

const MEMORY_BASE: usize = 0x1000;
const STATUSES: [(usize, usize, usize); 2] = [(0x7000, 0x7400, 0x7800), (0x7200, 0x7480, 0x8000)];
const UNMAPPED: u64 = 0x2_0000;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - MEMORY_BASE..address - MEMORY_BASE + value.len()].copy_from_slice(value);
}

fn history_value(status: usize, slot: i32, field: usize) -> i64 {
    let value = ((status as i64 + 1) << 40) + (i64::from(slot) + 1) * 16 + field as i64;
    if field % 2 == 1 { -value } else { value }
}

fn ring_sum(status: usize, field: usize) -> i64 {
    (0..HISTORY_RING_ENTRIES as i32)
        .map(|slot| history_value(status, slot, field))
        .sum()
}

fn expected_row(status: usize, country: u32) -> Row {
    let sum = |field| ring_sum(status, field);
    Row {
        country,
        country_instance: Some(0),
        ring_gain_raw: sum(1),
        ring_consumed_raw: sum(2),
        ring_other_raw: sum(6),
        ring_received_raw: [sum(7), sum(8), sum(9)],
        ..Row::default()
    }
}

fn expected_consumer(status: usize, country: u32, index: u32) -> ConsumerRow {
    ConsumerRow {
        country,
        index,
        priority: status as i32 * 10 + index as i32 - 1,
        requested_raw: (1_i64 << 41) + i64::from(index),
        received_raw: -((1_i64 << 39) + status as i64 * 8 + i64::from(index)),
    }
}

fn header(bytes: &mut [u8], address: usize, data: u64, capacity: u32, count: u32) {
    put(bytes, address, &data.to_le_bytes());
    put(bytes, address + 8, &capacity.to_le_bytes());
    put(bytes, address + 12, &count.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x9000];
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
        let (status, entries, ring) = STATUSES[index];
        put(
            &mut bytes,
            country,
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, country + ccountry::ID, &id.to_le_bytes());
        put(
            &mut bytes,
            country + ccountry::FUEL_STATUS,
            &(status as u64).to_le_bytes(),
        );
        put(&mut bytes, status, &(VTABLE_RVA.0 as u64).to_le_bytes());
        put(
            &mut bytes,
            status + COUNTRY,
            &(country as u64).to_le_bytes(),
        );
        header(&mut bytes, status + CONSUMERS, entries as u64, 3, 3);
        header(&mut bytes, status + HISTORY_RING, ring as u64, 24, 24);
        for position in 0..3 {
            let row = expected_consumer(index, id, position);
            let entry = entries + position as usize * consumer::STRIDE;
            put(
                &mut bytes,
                entry + consumer::PRIORITY,
                &row.priority.to_le_bytes(),
            );
            put(
                &mut bytes,
                entry + consumer::REQUESTED,
                &row.requested_raw.to_le_bytes(),
            );
            put(
                &mut bytes,
                entry + consumer::RECEIVED,
                &row.received_raw.to_le_bytes(),
            );
        }
        for slot in -1..HISTORY_RING_ENTRIES as i32 {
            let entry = if slot == -1 {
                status + PENDING_HISTORY
            } else {
                ring + slot as usize * entry::STRIDE
            };
            for field in 0..10 {
                put(
                    &mut bytes,
                    entry + field * 8,
                    &history_value(index, slot, field).to_le_bytes(),
                );
            }
        }
    }
    for (offset, value) in [
        (CURRENT, (1_i64 << 45) + 1),
        (MAXIMUM, (1_i64 << 46) + 3),
        (GAIN, -((1_i64 << 40) + 5)),
        (GAIN_FROM_STATES, 6_440_000),
        (LEND_LEASE_GAIN, -((1_i64 << 38) + 9)),
        (LEND_LEASE_CONSUMPTION, (1_i64 << 37) + 11),
        (COST, (1_i64 << 42) + 7),
        (GAIN_PER_OIL, 360_000),
    ] {
        put(&mut bytes, 0x7000 + offset, &value.to_le_bytes());
    }
    put(
        &mut bytes,
        0x7200 + CURRENT,
        &(-QUANTITY_SCALE).to_le_bytes(),
    );
    bytes
}

struct Captured {
    outcomes: Outcomes,
    rows: Vec<Row>,
    consumers: Vec<ConsumerRow>,
}

fn capture(bytes: &[u8], capacities: [usize; 2]) -> Captured {
    let mut rows = vec![Row::default(); capacities[0]];
    let mut consumers = vec![ConsumerRow::default(); capacities[1]];
    let outcomes = walk(
        &SliceMemory {
            base: MEMORY_BASE,
            bytes,
        },
        0,
        MEMORY_BASE,
        &mut rows,
        &mut consumers,
    );
    Captured {
        outcomes,
        rows,
        consumers,
    }
}

const FULL: [usize; 2] = [3, 8];

fn complete(fuel: usize, consumers: usize) -> Outcomes {
    Outcomes {
        fuel: Outcome::Rows { len: fuel },
        consumers: Outcome::Rows { len: consumers },
    }
}

#[test]
fn full_read_preserves_native_values_ring_sums_alias_identity_and_dormant_sentinel() {
    let captured = capture(&fixture(), FULL);
    assert_eq!(captured.outcomes, complete(2, 6));
    assert_eq!(
        captured.rows[0],
        Row {
            current_raw: (1_i64 << 45) + 1,
            maximum_raw: (1_i64 << 46) + 3,
            lend_lease_gain_raw: -((1_i64 << 38) + 9),
            ..expected_row(0, 3)
        }
    );
    assert_eq!(
        captured.rows[1],
        Row {
            current_raw: -32_768,
            ..expected_row(1, 2)
        }
    );
    for (status, country) in [(0, 3), (1, 2)] {
        for index in 0..3 {
            assert_eq!(
                captured.consumers[status * 3 + index as usize],
                expected_consumer(status, country, index)
            );
        }
    }
}

#[test]
fn the_count_probe_counts_whole_countries_without_reading_their_entries() {
    let mut bytes = fixture();
    put(&mut bytes, 0x7200 + CONSUMERS, &UNMAPPED.to_le_bytes());
    put(&mut bytes, 0x7200 + HISTORY_RING, &UNMAPPED.to_le_bytes());
    assert_eq!(
        walk_buffer(
            &SliceMemory {
                base: MEMORY_BASE,
                bytes: &bytes,
            },
            0,
            MEMORY_BASE,
            engine_native_memory::walk::Buffer::Count,
            engine_native_memory::walk::Buffer::Count,
        ),
        Outcomes {
            fuel: Outcome::Truncated {
                len: 0,
                unread_rows: 2
            },
            consumers: Outcome::Truncated {
                len: 0,
                unread_rows: 6
            },
        }
    );
    assert_eq!(capture(&fixture(), [0; 2]).outcomes, complete(2, 6));
}

#[test]
fn unreadable_or_foreign_history_ring_withholds_only_the_fuel_rows() {
    for (status, value, reason) in [
        (0x7200, UNMAPPED, "fuel history ring unreadable"),
        (0x7000, UNMAPPED, "fuel history ring unreadable"),
        (
            0x7200,
            0,
            "fuel history ring header is not the constructed 24-entry ring",
        ),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, status + HISTORY_RING, &value.to_le_bytes());
        let captured = capture(&bytes, FULL);
        assert_eq!(
            captured.outcomes,
            Outcomes {
                fuel: Outcome::Unavailable { reason },
                ..complete(2, 6)
            }
        );
        assert_eq!(captured.consumers[5], expected_consumer(1, 2, 2));
    }
    for (capacity, count) in [(24, 23), (23, 24), (25, 25)] {
        let mut bytes = fixture();
        header(&mut bytes, 0x7200 + HISTORY_RING, 0x8000, capacity, count);
        assert_eq!(
            capture(&bytes, FULL).outcomes.fuel,
            Outcome::Unavailable {
                reason: "fuel history ring header is not the constructed 24-entry ring"
            }
        );
    }
    let bytes = fixture();
    assert_eq!(
        capture(&bytes[..0x8780 - MEMORY_BASE - 1], FULL).outcomes,
        Outcomes {
            fuel: Outcome::Unavailable {
                reason: "fuel history ring unreadable"
            },
            ..complete(2, 6)
        }
    );
}

#[test]
fn ring_sums_are_checked_in_both_directions() {
    for (field, first, second) in [
        (entry::GAIN, i64::MAX, i64::MAX),
        (entry::CONSUMED, i64::MIN, i64::MIN),
        (entry::RECEIVED + 16, i64::MAX, i64::MAX),
    ] {
        let mut bytes = fixture();
        for (slot, value) in [(5, first), (6, second)] {
            put(
                &mut bytes,
                0x8000 + slot * entry::STRIDE + field,
                &value.to_le_bytes(),
            );
        }
        let captured = capture(&bytes, FULL);
        assert_eq!(
            captured.outcomes,
            Outcomes {
                fuel: Outcome::Unavailable {
                    reason: "fuel history ring sum exceeds i64"
                },
                ..complete(2, 6)
            },
            "field {field:#x}"
        );
    }
    for (field, values, expected) in [
        (entry::GAIN, [i64::MAX, 1, -1], i64::MAX),
        (entry::CONSUMED, [i64::MIN, -1, 1], i64::MIN),
        (
            entry::RECEIVED + 8,
            [i64::MAX, i64::MAX, -i64::MAX],
            i64::MAX,
        ),
    ] {
        let mut bytes = fixture();
        for slot in 0..24 {
            let value = values.get(slot).copied().unwrap_or(0);
            put(
                &mut bytes,
                0x7800 + slot * entry::STRIDE + field,
                &value.to_le_bytes(),
            );
        }
        let captured = capture(&bytes, FULL);
        assert_eq!(captured.outcomes, complete(2, 6), "field {field:#x}");
        let row = captured.rows[0];
        let sum = match field {
            entry::GAIN => row.ring_gain_raw,
            entry::CONSUMED => row.ring_consumed_raw,
            _ => row.ring_received_raw[1],
        };
        assert_eq!(sum, expected, "field {field:#x}");
    }
    let mut bytes = fixture();
    for slot in 0..24 {
        let value = if slot % 2 == 0 { i64::MAX } else { -i64::MAX };
        put(
            &mut bytes,
            0x7800 + slot * entry::STRIDE + entry::OTHER,
            &value.to_le_bytes(),
        );
    }
    let captured = capture(&bytes, FULL);
    assert_eq!(captured.outcomes, complete(2, 6));
    assert_eq!(captured.rows[0].ring_other_raw, 0);
    let mut bytes = fixture();
    for slot in [5, 6] {
        put(
            &mut bytes,
            0x7800 + slot * entry::STRIDE + entry::REQUESTED,
            &i64::MAX.to_le_bytes(),
        );
    }
    assert_eq!(capture(&bytes, FULL).outcomes, complete(2, 6));
}

#[test]
fn invalid_or_unreadable_consumer_array_fails_only_consumers() {
    for (capacity, count, reason) in [
        (2, 3, "fuel consumer array header invalid"),
        (
            4,
            4,
            "fuel consumer count exceeds the engine's three consumers",
        ),
    ] {
        let mut bytes = fixture();
        header(&mut bytes, 0x7000 + CONSUMERS, 0x7400, capacity, count);
        let captured = capture(&bytes, FULL);
        assert_eq!(
            captured.outcomes,
            Outcomes {
                consumers: Outcome::Unavailable { reason },
                ..complete(2, 6)
            }
        );
    }
    let mut bytes = fixture();
    put(&mut bytes, 0x7000 + CONSUMERS, &UNMAPPED.to_le_bytes());
    let captured = capture(&bytes, FULL);
    assert_eq!(
        captured.outcomes,
        Outcomes {
            consumers: Outcome::Unavailable {
                reason: "fuel consumer entries unreadable"
            },
            ..complete(2, 6)
        }
    );
    assert_eq!(captured.rows[0].ring_consumed_raw, ring_sum(0, 2));
    let mut bytes = fixture();
    header(&mut bytes, 0x7200 + CONSUMERS, 0, 0, 0);
    assert_eq!(capture(&bytes, FULL).outcomes, complete(2, 3));
}

#[test]
fn missing_wrong_or_partially_read_status_fails_both_tables() {
    for (address, value, reason) in [
        (
            0x3000 + ccountry::FUEL_STATUS,
            0_u64,
            "fuel status pointer unreadable or null",
        ),
        (0x7000, 7, "fuel status vtable mismatch"),
        (0x7000 + COUNTRY, 0x5000, "fuel status country mismatch"),
        (0x7200 + COUNTRY, 0x3000, "fuel status country mismatch"),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, address, &value.to_le_bytes());
        assert_eq!(
            capture(&bytes, FULL).outcomes,
            Outcomes::unavailable(reason)
        );
    }
    let bytes = fixture();
    for (end, reason) in [
        (0x7058, "fuel status quantities unreadable"),
        (0x70c0, "fuel status history ring header unreadable"),
        (0x70e8, "fuel status country pointer unreadable"),
    ] {
        assert_eq!(
            capture(&bytes[..end - MEMORY_BASE - 1], FULL).outcomes,
            Outcomes::unavailable(reason)
        );
    }
}

#[test]
fn a_later_walk_failure_keeps_each_tables_first_failure() {
    let pointer = "fuel status pointer unreadable or null";
    for (header, first) in [
        (0x7000 + HISTORY_RING, "fuel history ring unreadable"),
        (0x7000 + CONSUMERS, "fuel consumer entries unreadable"),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, header, &UNMAPPED.to_le_bytes());
        put(
            &mut bytes,
            0x5000 + ccountry::FUEL_STATUS,
            &0_u64.to_le_bytes(),
        );
        let expected = if header == 0x7000 + HISTORY_RING {
            Outcomes {
                fuel: Outcome::Unavailable { reason: first },
                consumers: Outcome::Unavailable { reason: pointer },
            }
        } else {
            Outcomes {
                fuel: Outcome::Unavailable { reason: pointer },
                consumers: Outcome::Unavailable { reason: first },
            }
        };
        assert_eq!(capture(&bytes, FULL).outcomes, expected, "{first}");
    }
    let mut bytes = fixture();
    put(
        &mut bytes,
        0x5000 + ccountry::FUEL_STATUS,
        &0_u64.to_le_bytes(),
    );
    assert_eq!(
        capture(&bytes, FULL).outcomes,
        Outcomes::unavailable(pointer)
    );
}
