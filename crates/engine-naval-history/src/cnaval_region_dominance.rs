
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x02a0_1090);
pub const REGION: usize = 0x08;
pub const ENTRIES: usize = 0x40;
pub const ENTRY_COUNT: usize = 0x4c;

use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice, walk::Outcome};
use crate::{cdominance_values as values, naval_dominance_entry as entry};
use engine_native::cgame_state;
use engine_native_parts::cstrategic_region;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub region: u32,
    pub region_slot: u32,
    pub country: u32,
    pub current: i64,
    pub base_target: i64,
    pub target: i64,
    pub individual_ratio: i64,
    pub previous: i64,
    pub decline_from: i64,
}

impl Row {
    pub fn key(self) -> u64 {
        (u64::from(self.region) << 32) | u64::from(self.country)
    }
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    match walk_inner(memory, base, game_state, output) {
        Ok(outcome) => outcome,
        Err(reason) => Outcome::Unavailable { reason },
    }
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Result<Outcome, &'static str> {
    let regions = game_state
        .checked_add(cgame_state::STRATEGIC_REGIONS)
        .and_then(|address| Array::read(memory, address, 8))
        .filter(|array| array.count != 0)
        .ok_or("naval supremacy region roster unreadable or invalid")?;
    let region_vtable = engine_sites::at(base, cstrategic_region::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let secondary_vtable = engine_sites::at(base, cstrategic_region::SECONDARY_VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let dominance_vtable = engine_sites::at(base, VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let values_vtable = engine_sites::at(base, values::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let mut len = 0_usize;
    let mut unread_rows = 0_u32;
    const ROWS_PER_COPY: usize = 32;
    let mut buffer = [0_u8; ROWS_PER_COPY * entry::STRIDE];
    for ordinal in 1..regions.count {
        let pointer = regions
            .element(ordinal, 8)
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("naval supremacy region pointer unreadable")?;
        if pointer == 0 {
            continue;
        }
        let mut vtables = [0; 16];
        let mut identity = [0; cstrategic_region::RUNTIME_SLOT - cstrategic_region::ID + 4];
        let mut dominance = [0; 8];
        let copied = memory.copy_scattered(
            [
                pointer,
                pointer.checked_add(cstrategic_region::ID).unwrap_or(0),
                pointer
                    .checked_add(cstrategic_region::NAVAL_DOMINANCE)
                    .unwrap_or(0),
            ],
            &mut [&mut vtables, &mut identity, &mut dominance],
        );
        if copied != 3 {
            return Err("naval supremacy region unreadable");
        }
        if u64_at(&vtables, 0) != region_vtable
            || u64_at(&vtables, cstrategic_region::SECONDARY) != secondary_vtable
        {
            return Err("naval supremacy region vtable mismatch");
        }
        let region = u32_at(&identity, 0);
        let region_slot = u32_at(
            &identity,
            cstrategic_region::RUNTIME_SLOT - cstrategic_region::ID,
        );
        if region_slot != ordinal {
            return Err("naval supremacy region roster identity mismatch");
        }
        let dominance = usize::try_from(u64::from_le_bytes(dominance))
            .ok()
            .filter(|&address| address != 0)
            .ok_or("naval supremacy container null or invalid")?;
        let owner =
            read::<[u8; 16]>(memory, dominance).ok_or("naval supremacy container unreadable")?;
        if u64_at(&owner, 0) != dominance_vtable || u64_at(&owner, REGION) != pointer as u64 {
            return Err("naval supremacy container identity mismatch");
        }
        let entries = dominance
            .checked_add(ENTRIES)
            .and_then(|address| Array::read(memory, address, entry::STRIDE))
            .ok_or("naval supremacy values vector unreadable or invalid")?;
        output.ensure(
            len.checked_add(entries.count as usize)
                .ok_or("naval supremacy row count overflow")?,
        )?;
        let available = output.len().saturating_sub(len).min(entries.count as usize);
        unread_rows = unread_rows
            .checked_add(entries.count - available as u32)
            .ok_or("naval supremacy row count overflow")?;
        let mut first = 0;
        while first < available {
            let count = (available - first).min(ROWS_PER_COPY);
            let address = entries
                .element(first as u32, entry::STRIDE)
                .ok_or("naval supremacy values address overflow")?;
            if !read_slice(memory, address, &mut buffer[..count * entry::STRIDE]) {
                return Err("naval supremacy values unreadable");
            }
            for raw in buffer[..count * entry::STRIDE].chunks_exact(entry::STRIDE) {
                let country = u32_at(raw, entry::COUNTRY_KEY);
                let object = &raw[entry::VALUES..];
                if u64_at(object, 0) != values_vtable
                    || u32_at(object, values::COUNTRY) != country
                    || u64_at(object, values::REGION) != pointer as u64
                {
                    return Err("naval supremacy value identity mismatch");
                }
                output[len] = Row {
                    region,
                    region_slot,
                    country,
                    current: u64_at(object, values::CURRENT) as i64,
                    base_target: u64_at(object, values::BASE_TARGET) as i64,
                    target: u64_at(object, values::TARGET) as i64,
                    individual_ratio: u64_at(object, values::INDIVIDUAL_RATIO) as i64,
                    previous: u64_at(object, values::PREVIOUS) as i64,
                    decline_from: u64_at(object, values::DECLINE_FROM) as i64,
                };
                len += 1;
            }
            first += count;
        }
    }
    Ok(if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    })
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("fixed span"))
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("fixed span"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const ROOT: usize = 0x1000;
    const REGION_OBJECT: usize = 0x1500;
    const DOMINANCE: usize = 0x1800;
    const ROWS: usize = 0x1900;

    fn put32(bytes: &mut [u8], address: usize, value: u32) {
        bytes[address - ROOT..address - ROOT + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put64(bytes: &mut [u8], address: usize, value: u64) {
        bytes[address - ROOT..address - ROOT + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn array(bytes: &mut [u8], address: usize, data: usize, capacity: u32, count: u32) {
        put64(bytes, address, data as u64);
        put32(bytes, address + 8, capacity);
        put32(bytes, address + 12, count);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x1000];
        array(
            &mut bytes,
            ROOT + cgame_state::STRATEGIC_REGIONS,
            0x1400,
            4,
            4,
        );
        put64(&mut bytes, 0x1408, REGION_OBJECT as u64);
        put64(
            &mut bytes,
            REGION_OBJECT,
            cstrategic_region::VTABLE_RVA.0 as u64,
        );
        put64(
            &mut bytes,
            REGION_OBJECT + cstrategic_region::SECONDARY,
            cstrategic_region::SECONDARY_VTABLE_RVA.0 as u64,
        );
        put32(
            &mut bytes,
            REGION_OBJECT + cstrategic_region::ID,
            0x8765_4321,
        );
        put32(
            &mut bytes,
            REGION_OBJECT + cstrategic_region::RUNTIME_SLOT,
            1,
        );
        put64(
            &mut bytes,
            REGION_OBJECT + cstrategic_region::NAVAL_DOMINANCE,
            DOMINANCE as u64,
        );
        put64(&mut bytes, DOMINANCE, VTABLE_RVA.0 as u64);
        put64(&mut bytes, DOMINANCE + REGION, REGION_OBJECT as u64);
        array(&mut bytes, DOMINANCE + ENTRIES, ROWS, 7, 2);
        for index in 0..2 {
            let outer = ROWS + index * entry::STRIDE;
            let object = outer + entry::VALUES;
            let country = if index == 0 { u32::MAX } else { 1 };
            put32(&mut bytes, outer + entry::COUNTRY_KEY, country);
            put64(&mut bytes, object, values::VTABLE_RVA.0 as u64);
            put32(&mut bytes, object + values::COUNTRY, country);
            put64(&mut bytes, object + values::REGION, REGION_OBJECT as u64);
            for (offset, value) in [
                (values::CURRENT, i64::MIN + 1),
                (values::BASE_TARGET, i64::MAX - 1),
                (values::TARGET, 0x1_1234_5678),
                (values::INDIVIDUAL_RATIO, -100_001),
                (values::PREVIOUS, -0x1_1234_5678),
                (values::DECLINE_FROM, 123_456_789),
            ] {
                put64(&mut bytes, object + offset, value as u64);
            }
        }
        bytes
    }

    fn sample(bytes: &[u8], rows: &mut Vec<Row>) -> Outcome {
        walk(&SliceMemory { base: ROOT, bytes }, 0, ROOT, rows)
    }

    #[test]
    fn daily_cache_preserves_signed_qwords_and_distinct_authored_region_and_slot() {
        let bytes = fixture();
        let mut rows = vec![Row::default(); 2];
        assert_eq!(sample(&bytes, &mut rows), Outcome::Rows { len: 2 });
        assert_eq!(
            rows[0],
            Row {
                region: 0x8765_4321,
                region_slot: 1,
                country: u32::MAX,
                current: i64::MIN + 1,
                base_target: i64::MAX - 1,
                target: 0x1_1234_5678,
                individual_ratio: -100_001,
                previous: -0x1_1234_5678,
                decline_from: 123_456_789,
            }
        );
        assert_eq!(rows[0].key(), 0x8765_4321_ffff_ffff);
        assert_eq!(rows[1].country, 1);
    }

    #[test]
    fn capacity_counts_remaining_regions_without_reading_uncaptured_values() {
        let mut bytes = fixture();
        let second_region = 0x1700;
        let copy = bytes[REGION_OBJECT - ROOT..REGION_OBJECT - ROOT + 0xf0].to_vec();
        bytes[second_region - ROOT..second_region - ROOT + 0xf0].copy_from_slice(&copy);
        put64(&mut bytes, 0x1418, second_region as u64);
        put32(&mut bytes, second_region + cstrategic_region::ID, 12);
        put32(
            &mut bytes,
            second_region + cstrategic_region::RUNTIME_SLOT,
            3,
        );
        put64(
            &mut bytes,
            second_region + cstrategic_region::NAVAL_DOMINANCE,
            0x1b00,
        );
        put64(&mut bytes, 0x1b00, VTABLE_RVA.0 as u64);
        put64(&mut bytes, 0x1b00 + REGION, second_region as u64);
        array(&mut bytes, 0x1b00 + ENTRIES, 0x3000, 3, 3);
        assert_eq!(
            count(&SliceMemory { base: ROOT, bytes: &bytes }, 0, ROOT),
            Outcome::Truncated {
                len: 0,
                unread_rows: 5
            }
        );
        assert!(matches!(
            sample(&bytes, &mut vec![Row::default(); 5]),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn missing_bytes_invalid_counts_and_wrong_native_owners_are_unavailable() {
        let original = fixture();
        for (address, value) in [
            (DOMINANCE + REGION, 0_u64),
            (ROWS + entry::VALUES, 0),
            (ROWS + entry::VALUES + values::REGION, 0),
        ] {
            let mut bytes = original.clone();
            put64(&mut bytes, address, value);
            assert!(matches!(
                sample(&bytes, &mut vec![Row::default(); 2]),
                Outcome::Unavailable { .. }
            ));
        }
        for (address, value) in [
            (ROWS + entry::VALUES + values::COUNTRY, 0),
            (REGION_OBJECT + cstrategic_region::RUNTIME_SLOT, 2),
            (DOMINANCE + ENTRY_COUNT, u32::MAX),
            (ROOT + cgame_state::STRATEGIC_REGIONS_COUNT, 0),
        ] {
            let mut bytes = original.clone();
            put32(&mut bytes, address, value);
            assert!(matches!(
                sample(&bytes, &mut vec![Row::default(); 2]),
                Outcome::Unavailable { .. }
            ));
        }
        assert!(matches!(
            sample(
                &original[..ROWS - ROOT + 2 * entry::STRIDE - 1],
                &mut vec![Row::default(); 2]
            ),
            Outcome::Unavailable { .. }
        ));
        let mut empty = original;
        put32(&mut empty, DOMINANCE + ENTRY_COUNT, 0);
        assert_eq!(sample(&empty, &mut Vec::new()), Outcome::Rows { len: 0 });
    }

    #[test]
    fn growing_supremacy_captures_mod_populations_on_first_and_later_samples() {
        let mut bytes = fixture();
        let prototype = bytes[ROWS - ROOT..ROWS - ROOT + entry::STRIDE].to_vec();
        let data = 0x10000;
        let mut output = Vec::new();
        for count in [5_000_u32, 12_000] {
            bytes.resize(data - ROOT + count as usize * entry::STRIDE, 0);
            array(&mut bytes, DOMINANCE + ENTRIES, data, count, count);
            for index in 0..count as usize {
                let at = data + index * entry::STRIDE;
                bytes[at - ROOT..at - ROOT + entry::STRIDE].copy_from_slice(&prototype);
                put32(&mut bytes, at + entry::COUNTRY_KEY, index as u32 + 1);
                put32(
                    &mut bytes,
                    at + entry::VALUES + values::COUNTRY,
                    index as u32 + 1,
                );
            }
            let memory = SliceMemory {
                base: ROOT,
                bytes: &bytes,
            };
            assert_eq!(
                walk(&memory, 0, ROOT, &mut output),
                Outcome::Rows {
                    len: count as usize
                }
            );
            assert_eq!(output[count as usize - 1].country, count);
        }
    }
}
