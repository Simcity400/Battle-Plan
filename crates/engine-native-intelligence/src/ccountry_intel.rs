
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::read_slice,
    read,
    walk::Outcome,
};
use engine_native::ccountry;
use crate::country_intel_row;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029d_bb28);
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00d6_3480);
pub const SIZE: usize = 0xf0;
pub const OWNER: usize = 0x08;
pub const DATA: usize = 0x10;
pub const CAPACITY: usize = 0x18;
pub const COUNT: usize = 0x1c;

pub fn native_rows(memory: &impl Memory, base: usize, game_state: usize) -> Option<usize> {
    let countries = ccountry::Reader::new(memory, base, game_state).ok()?;
    let expected = engine_sites::at(base, VTABLE_RVA)? as u64;
    let reserved = countries.reserved();
    let mut total = 0_usize;
    for slot in 1..countries.count() {
        let Ok(Some(country)) = countries.at(slot) else {
            continue;
        };
        let Some(table) = country
            .pointer
            .checked_add(ccountry::INTEL_TABLE)
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|value| usize::try_from(value).ok())
            .filter(|&table| table != 0)
        else {
            continue;
        };
        if read::<u64>(memory, table) != Some(expected) {
            continue;
        }
        let Some(vector) = table
            .checked_add(DATA)
            .and_then(|address| read::<[u8; 16]>(memory, address))
            .and_then(|header| Array::from_header(header, country_intel_row::STRIDE))
        else {
            continue;
        };
        total = total.saturating_add(vector.count as usize);
    }
    if let Ok((_, count)) = reserved_vector_at(memory, base, reserved, &mut None, &mut |_, _, _| {})
    {
        total = total.saturating_add(count as usize);
    }
    Some(total)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<country_intel_row::Row>,
    scratch: &mut [u8],
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        scratch,
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, country_intel_row::Row>,
    scratch: &mut [u8],
) -> Outcome {
    match ccountry::Reader::new(memory, base, game_state) {
        Ok(countries) => walk_in_reader(
            memory,
            base,
            &countries,
            output.suffix(0),
            scratch,
            &mut |_, _, _| {},
        ),
        Err(reason) => Outcome::Unavailable { reason },
    }
}

fn walk_in_reader(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    mut output: engine_native_memory::walk::Buffer<'_, country_intel_row::Row>,
    scratch: &mut [u8],
    visit: &mut dyn FnMut(u32, usize, usize),
) -> Outcome {
    let result = (|| {
        let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
        if scratch.len() < country_intel_row::STRIDE {
            return Err("country intel scratch holds no record");
        }
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let mut found_country = false;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let table = country
                .pointer
                .checked_add(ccountry::INTEL_TABLE)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("country intel table unreadable")?;
            if table == 0 {
                continue;
            }
            if read::<u64>(memory, table) != Some(expected) {
                return Err("country intel vtable mismatch");
            }
            visit(country.id, table, country.pointer);
            let header = table
                .checked_add(DATA)
                .and_then(|address| read::<[u8; 16]>(memory, address))
                .ok_or("country intel vector header unreadable")?;
            let data = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
                .map_err(|_| "country intel data pointer overflow")?;
            let capacity = i32::from_le_bytes(header[8..12].try_into().unwrap());
            let count = i32::from_le_bytes(header[12..].try_into().unwrap());
            if count <= 0 {
                if count < 0 || capacity < 0 {
                    return Err("country intel vector count or capacity is negative");
                }
                continue;
            }
            if count > capacity {
                return Err("country intel vector count exceeds its capacity");
            }
            let count = count as u32;
            let span = (count as usize)
                .checked_mul(country_intel_row::STRIDE)
                .ok_or("country intel span overflow")?;
            if data == 0 || data.checked_add(span).is_none() {
                return Err("country intel data pointer is null or overflows");
            }
            let (written, unread) =
                copy_rows(memory, country.id, data, count, output.suffix(len), scratch)?;
            len += written;
            unread_rows = unread_rows
                .checked_add(unread)
                .ok_or("country intel unread row count overflow")?;
        }
        if !found_country {
            Err("country table has no readable countries")
        } else if unread_rows == 0 {
            Ok(Outcome::Rows { len })
        } else {
            Ok(Outcome::Truncated { len, unread_rows })
        }
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[derive(Clone, Copy, Debug)]
pub struct ReservedCapture {
    pub outcome: Outcome,
    pub vector_count: Result<u32, &'static str>,
    pub reserved_required: usize,
    pub reserved_reached: bool,
}

#[cfg(test)]
pub fn walk_with_reserved(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<country_intel_row::Row>,
    scratch: &mut [u8],
) -> ReservedCapture {
    walk_with_reserved_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        scratch,
    )
}

#[cfg(test)]
pub fn walk_with_reserved_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: engine_native_memory::walk::Buffer<'_, country_intel_row::Row>,
    scratch: &mut [u8],
) -> ReservedCapture {
    walk_with_reserved_visit(memory, base, game_state, output, scratch, &mut |_, _, _| {})
}

pub fn walk_with_reserved_visit(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, country_intel_row::Row>,
    scratch: &mut [u8],
    visit: &mut dyn FnMut(u32, usize, usize),
) -> ReservedCapture {
    let countries = match ccountry::Reader::new(memory, base, game_state) {
        Ok(countries) => countries,
        Err(reason) => {
            return ReservedCapture {
                outcome: Outcome::Unavailable { reason },
                vector_count: Err("reserved country intel prerequisite positive table incomplete"),
                reserved_required: 0,
                reserved_reached: false,
            };
        }
    };
    let positive = walk_in_reader(memory, base, &countries, output.suffix(0), scratch, visit);
    let Outcome::Rows { len } = positive else {
        return ReservedCapture {
            outcome: positive,
            vector_count: Err("reserved country intel prerequisite positive table incomplete"),
            reserved_required: 0,
            reserved_reached: false,
        };
    };
    let mut known_count = None;
    let mut written = 0;
    let mut required = 0;
    let vector_count = (|| {
        let (data, count) =
            reserved_vector_at(memory, base, countries.reserved(), &mut known_count, visit)?;
        if count == 0 {
            return Ok(0);
        }
        let (stored, unread) = copy_rows(memory, 0, data, count, output.suffix(len), scratch)?;
        required = len
            .checked_add(stored)
            .and_then(|n| n.checked_add(unread as usize))
            .ok_or("reserved country intel row demand overflow")?;
        if unread != 0 {
            return Err("reserved country intel row arena too small");
        }
        written = stored;
        Ok(count)
    })();
    if required == 0 {
        required = known_count.map_or(0, |count| len.saturating_add(count as usize));
    }
    ReservedCapture {
        outcome: if vector_count.is_ok() {
            Outcome::Rows { len: len + written }
        } else {
            positive
        },
        vector_count,
        reserved_required: required,
        reserved_reached: true,
    }
}

fn reserved_vector_at(
    memory: &impl Memory,
    base: usize,
    country: Result<ccountry::Country, &'static str>,
    known_count: &mut Option<u32>,
    visit: &mut dyn FnMut(u32, usize, usize),
) -> Result<(usize, u32), &'static str> {
    let country = country?;
    let table = country
        .pointer
        .checked_add(ccountry::INTEL_TABLE)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|pointer| *pointer != 0)
        .ok_or("reserved country intel table unreadable or null")?;
    let identity =
        read::<[u8; 12]>(memory, table).ok_or("reserved country intel identity unreadable")?;
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected {
        return Err("reserved country intel vtable mismatch");
    }
    if u32::from_le_bytes(identity[8..].try_into().unwrap()) != 0 {
        return Err("reserved country intel owner mismatch");
    }
    visit(0, table, country.pointer);
    let header = table
        .checked_add(DATA)
        .and_then(|address| read::<[u8; 16]>(memory, address))
        .ok_or("reserved country intel vector header unreadable")?;
    let capacity = i32::from_le_bytes(header[8..12].try_into().unwrap());
    let count = i32::from_le_bytes(header[12..].try_into().unwrap());
    if count < 0 || capacity < 0 || count > capacity {
        return Err("reserved country intel vector count or capacity invalid");
    }
    let count = count as u32;
    if count == 0 {
        *known_count = Some(0);
        return Ok((0, 0));
    }
    let data = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        .map_err(|_| "reserved country intel data pointer overflow")?;
    let span = (count as usize)
        .checked_mul(country_intel_row::STRIDE)
        .ok_or("reserved country intel span overflow")?;
    if data == 0 || data.checked_add(span).is_none() {
        return Err("reserved country intel data pointer is null or overflows");
    }
    *known_count = Some(count);
    Ok((data, count))
}

fn copy_rows(
    memory: &impl Memory,
    owner: u32,
    data: usize,
    count: u32,
    mut output: engine_native_memory::walk::Buffer<'_, country_intel_row::Row>,
    scratch: &mut [u8],
) -> Result<(usize, u32), &'static str> {
    let batch = scratch.len() / country_intel_row::STRIDE;
    if batch == 0 {
        return Err("country intel scratch holds no record");
    }
    let (mut len, mut unread, mut index) = (0_usize, 0_u32, 0);
    while index < count as usize {
        let chunk = (count as usize - index).min(batch);
        let address = data
            .checked_add(index * country_intel_row::STRIDE)
            .ok_or("country intel record address overflow")?;
        let span = &mut scratch[..chunk * country_intel_row::STRIDE];
        if !read_slice(memory, address, span) {
            return Err("country intel record batch unreadable");
        }
        for (offset, record) in span.chunks_exact(country_intel_row::STRIDE).enumerate() {
            let at = index + offset;
            let row =
                country_intel_row::Row::from_bytes(owner, at as u32, record.try_into().unwrap());
            if at + 1 != count as usize && row.is_zero() {
                continue;
            }
            output.ensure(
                len.checked_add(1)
                    .ok_or("country intel sparse row demand overflow")?,
            )?;
            if len == output.len() {
                unread = unread
                    .checked_add(1)
                    .ok_or("country intel unread row count overflow")?;
            } else {
                output[len] = row;
                len += 1;
            }
        }
        index += chunk;
    }
    Ok((len, unread))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use engine_native_memory::memory::SliceMemory;

    const BASE: usize = 0;
    const GS: usize = 0x1000;
    const COUNTRIES: [usize; 2] = [0x4000, 0x5000];
    const TABLES: [usize; 2] = [0x8000, 0x9000];
    const RECORDS: [usize; 2] = [0xa000, 0xb000];

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn lanes(bytes: &mut [u8], data: usize, index: usize, values: [i64; 4]) {
        for (lane, value) in values.into_iter().enumerate() {
            p64(
                bytes,
                data + index * country_intel_row::STRIDE + lane * 8,
                value as u64,
            );
        }
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0xc000];
        p64(&mut bytes, GS + cgame_state::COUNTRIES, 0x3000);
        p32(&mut bytes, GS + cgame_state::COUNTRIES + 8, 3);
        p32(&mut bytes, GS + cgame_state::COUNTRIES + 12, 3);
        for (index, pointer) in [0_u64, COUNTRIES[0] as u64, COUNTRIES[1] as u64]
            .into_iter()
            .enumerate()
        {
            p64(&mut bytes, 0x3000 + index * 8, pointer);
        }
        p64(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x3100);
        p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 8, 3);
        p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 12, 3);
        for (index, value) in [0_u32, 1, 2].into_iter().enumerate() {
            p32(&mut bytes, 0x3100 + index * 4, value);
        }
        for owner in 0..2 {
            let country = COUNTRIES[owner];
            let table = TABLES[owner];
            p64(&mut bytes, country, (BASE + ccountry::VTABLE_RVA.0) as u64);
            p32(&mut bytes, country + ccountry::ID, owner as u32 + 1);
            p64(&mut bytes, country + ccountry::INTEL_TABLE, table as u64);
            p64(&mut bytes, table, (BASE + VTABLE_RVA.0) as u64);
            p64(&mut bytes, table + DATA, RECORDS[owner] as u64);
        }
        p32(&mut bytes, TABLES[0] + CAPACITY, 4);
        p32(&mut bytes, TABLES[0] + COUNT, 3);
        p32(&mut bytes, TABLES[1] + CAPACITY, 8);
        p32(&mut bytes, TABLES[1] + COUNT, 0);
        lanes(&mut bytes, RECORDS[0], 0, [0, 0, 0, 0]);
        lanes(
            &mut bytes,
            RECORDS[0],
            1,
            [0, -1_500_000, -1_000_000, -1_500_000],
        );
        lanes(
            &mut bytes,
            RECORDS[0],
            2,
            [3_331_020, i64::MIN, i64::MAX, 813_062],
        );
        bytes
    }

    fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    type Case<'a> = (&'a str, &'a dyn Fn(&mut Vec<u8>));

    fn run(bytes: &[u8], rows: usize) -> (Outcome, Vec<country_intel_row::Row>) {
        let mut output = vec![country_intel_row::Row::default(); rows];
        let mut scratch = vec![0_u8; 2 * country_intel_row::STRIDE];
        let outcome = walk(&memory(bytes), BASE, GS, &mut output, &mut scratch);
        output.truncate(outcome.len());
        (outcome, output)
    }

    #[test]
    fn intel_sources_roots_share_the_existing_census_even_for_empty_headline_vectors() {
        let bytes = fixture();
        let mut output = Vec::new();
        let mut scratch = [0; 64];
        let mut roots = Vec::new();
        let result = walk_with_reserved_visit(
            &memory(&bytes),
            BASE,
            GS,
            engine_native_memory::walk::Buffer::Growing(&mut output),
            &mut scratch,
            &mut |owner, pointer, _| roots.push((owner, pointer)),
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 2 });
        assert_eq!(roots, [(1, TABLES[0]), (2, TABLES[1])]);
        assert!(result.vector_count.is_err());
    }

    #[test]
    fn growing_sparse_rows_use_current_headers_with_unchanged_country_count() {
        let mut bytes = fixture();
        let mut rows = Vec::new();
        let mut scratch = [0; 2 * country_intel_row::STRIDE];
        let expected = run(&bytes, 8);
        assert_eq!(
            walk(&memory(&bytes), BASE, GS, &mut rows, &mut scratch),
            expected.0
        );
        assert_eq!(&rows[..expected.0.len()], expected.1.as_slice());
        const COUNT: usize = 130;
        const DATA: usize = 0x10000;
        bytes.resize(DATA + COUNT * country_intel_row::STRIDE, 0);
        p64(&mut bytes, TABLES[0] + super::DATA, DATA as u64);
        p32(&mut bytes, TABLES[0] + CAPACITY, COUNT as u32);
        p32(&mut bytes, TABLES[0] + super::COUNT, COUNT as u32);
        for index in 0..COUNT {
            lanes(
                &mut bytes,
                DATA,
                index,
                [index as i64, -(index as i64), i64::MIN, i64::MAX],
            );
        }
        let expected = run(&bytes, COUNT);
        assert_eq!(expected.0, Outcome::Rows { len: COUNT });
        assert_eq!(
            walk(&memory(&bytes), BASE, GS, &mut rows, &mut scratch),
            expected.0
        );
        assert_eq!(&rows[..COUNT], expected.1.as_slice());
        assert_eq!(rows[COUNT - 1].target_index, COUNT as u32 - 1);
    }

    #[test]
    fn growing_reserved_rows_append_from_empty_storage_without_a_second_capture() {
        let bytes = reserved_fixture(5);
        let expected = run_reserved(&bytes, 8);
        let mut rows = Vec::new();
        let mut scratch = [0; 2 * country_intel_row::STRIDE];
        let actual = walk_with_reserved(&memory(&bytes), BASE, GS, &mut rows, &mut scratch);
        assert_eq!(actual.outcome, expected.0.outcome);
        assert_eq!(actual.vector_count, expected.0.vector_count);
        assert_eq!(actual.reserved_required, expected.0.reserved_required);
        assert_eq!(&rows[..actual.outcome.len()], expected.1.as_slice());
    }

    #[test]
    fn nonzero_records_keep_their_native_index_and_all_signed_lanes() {
        let (outcome, rows) = run(&fixture(), 8);
        assert_eq!(outcome, Outcome::Rows { len: 2 });
        assert_eq!(
            rows[0],
            country_intel_row::Row {
                owner_country: 1,
                target_index: 1,
                army: -1_500_000,
                air: -1_500_000,
                civilian: 0,
                navy: -1_000_000,
            }
        );
        assert_eq!(
            rows[1],
            country_intel_row::Row {
                owner_country: 1,
                target_index: 2,
                army: i64::MIN,
                air: 813_062,
                civilian: 3_331_020,
                navy: i64::MAX,
            }
        );
        let mut bytes = fixture();
        lanes(&mut bytes, RECORDS[0], 1, [5, 0, -5, 0]);
        let (_, rows) = run(&bytes, 8);
        assert_eq!(
            rows.iter().map(|row| row.target_index).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn a_null_table_or_a_zero_count_contributes_no_rows() {
        let mut bytes = fixture();
        p64(&mut bytes, COUNTRIES[0] + ccountry::INTEL_TABLE, 0);
        assert_eq!(run(&bytes, 8).0, Outcome::Rows { len: 0 });
        let mut bytes = fixture();
        p32(&mut bytes, TABLES[0] + COUNT, 0);
        assert_eq!(run(&bytes, 8).0, Outcome::Rows { len: 0 });
    }

    #[test]
    fn changed_headers_and_bad_identities_are_named_failures() {
        let valid = fixture();
        let cases: [Case<'_>; 5] = [
            ("country intel vtable mismatch", &|bytes: &mut Vec<u8>| {
                p64(bytes, TABLES[0], 0);
            }),
            (
                "country intel vector count exceeds its capacity",
                &|bytes: &mut Vec<u8>| p32(bytes, TABLES[0] + CAPACITY, 1),
            ),
            (
                "country intel vector count or capacity is negative",
                &|bytes: &mut Vec<u8>| p32(bytes, TABLES[0] + COUNT, u32::MAX),
            ),
            (
                "country intel data pointer is null or overflows",
                &|bytes: &mut Vec<u8>| p64(bytes, TABLES[0] + DATA, 0),
            ),
            (
                "country intel data pointer is null or overflows",
                &|bytes: &mut Vec<u8>| p64(bytes, TABLES[0] + DATA, u64::MAX),
            ),
        ];
        for (reason, break_it) in cases {
            let mut bytes = valid.clone();
            break_it(&mut bytes);
            assert_eq!(
                run(&bytes, 8).0,
                Outcome::Unavailable { reason },
                "{reason}"
            );
        }
    }

    struct FailAt<'a> {
        bytes: &'a [u8],
        address: usize,
    }

    impl Memory for FailAt<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            address != self.address
                && SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, destination)
        }
    }

    #[test]
    fn an_unreadable_table_pointer_or_vector_header_is_named_and_never_partial() {
        let bytes = fixture();
        for (address, reason) in [
            (
                COUNTRIES[1] + ccountry::INTEL_TABLE,
                "country intel table unreadable",
            ),
            (TABLES[0] + DATA, "country intel vector header unreadable"),
        ] {
            let memory = FailAt {
                bytes: &bytes,
                address,
            };
            let mut output = vec![country_intel_row::Row::default(); 8];
            let mut scratch = [0_u8; 4 * country_intel_row::STRIDE];
            assert_eq!(
                walk(&memory, BASE, GS, &mut output, &mut scratch),
                Outcome::Unavailable { reason },
                "{reason}"
            );
        }
    }

    #[test]
    fn a_short_record_span_or_scratch_refuses_the_table() {
        let bytes = fixture();
        let cut = RECORDS[0] + 2 * country_intel_row::STRIDE + 8;
        assert_eq!(
            run(&bytes[..cut], 8).0,
            Outcome::Unavailable {
                reason: "country intel record batch unreadable"
            }
        );
        let mut output = vec![country_intel_row::Row::default(); 8];
        assert_eq!(
            walk(&memory(&bytes), BASE, GS, &mut output, &mut []),
            Outcome::Unavailable {
                reason: "country intel scratch holds no record"
            }
        );
    }

    #[test]
    fn one_bulk_copy_per_scratch_fill_preserves_native_order() {
        let bytes = fixture();
        let mut output = vec![country_intel_row::Row::default(); 8];
        let mut scratch = [0_u8; country_intel_row::STRIDE];
        let outcome = walk(&memory(&bytes), BASE, GS, &mut output, &mut scratch);
        assert_eq!(outcome, Outcome::Rows { len: 2 });
        assert_eq!(
            output[..2]
                .iter()
                .map(|row| row.target_index)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn the_last_record_is_kept_even_when_its_lanes_are_zero() {
        let mut bytes = fixture();
        lanes(&mut bytes, RECORDS[0], 1, [0, 0, 0, 0]);
        lanes(&mut bytes, RECORDS[0], 2, [0, 0, 0, 0]);
        let (outcome, rows) = run(&bytes, 8);
        assert_eq!(outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            rows[0],
            country_intel_row::Row {
                owner_country: 1,
                target_index: 2,
                ..Default::default()
            }
        );
        lanes(&mut bytes, RECORDS[0], 0, [0, 0, 0, i64::MIN]);
        let (_, rows) = run(&bytes, 8);
        assert_eq!(
            rows.iter().map(|row| row.target_index).collect::<Vec<_>>(),
            vec![0, 2]
        );
    }

    #[test]
    fn native_rows_keeps_counts_beyond_the_registry_without_reading_payloads() {
        let mut bytes = fixture();
        p32(&mut bytes, TABLES[0] + CAPACITY, 12);
        p32(&mut bytes, TABLES[0] + COUNT, 12);
        for ordinal in 0..12 {
            lanes(
                &mut bytes,
                RECORDS[0],
                ordinal,
                [0, ordinal as i64 + 1, 0, 0],
            );
        }
        assert_eq!(
            native_rows(&memory(&bytes[..RECORDS[0]]), BASE, GS),
            Some(12)
        );
        let (outcome, rows) = run(&bytes, 12);
        assert_eq!(outcome, Outcome::Rows { len: 12 });
        assert_eq!(rows.last().unwrap().target_index, 11);
        p32(&mut bytes, TABLES[0] + CAPACITY, 11);
        assert_eq!(
            native_rows(&memory(&bytes[..RECORDS[0]]), BASE, GS),
            Some(0)
        );
    }

    #[test]
    fn native_rows_skips_null_tables_and_refuses_an_unreadable_census() {
        let bytes = fixture();
        let (outcome, rows) = run(&bytes, 64);
        let Outcome::Rows { len } = outcome else {
            panic!("fixture walk failed: {outcome:?}");
        };
        let hint = native_rows(&memory(&bytes), BASE, GS).unwrap();
        assert!(hint >= len && hint >= rows[..len].len());
        assert_eq!(native_rows(&memory(&bytes[..0x10]), BASE, GS), None);
    }

    const RESERVED_COUNTRY: usize = 0x6000;
    const RESERVED_TABLE: usize = 0x9200;
    const RESERVED_RECORDS: usize = 0xb100;

    fn reserved_fixture(count: u32) -> Vec<u8> {
        let mut bytes = fixture();
        p64(&mut bytes, 0x3000, RESERVED_COUNTRY as u64);
        p64(
            &mut bytes,
            RESERVED_COUNTRY,
            (BASE + ccountry::VTABLE_RVA.0) as u64,
        );
        p32(&mut bytes, RESERVED_COUNTRY + ccountry::ID, 0);
        p64(
            &mut bytes,
            RESERVED_COUNTRY + ccountry::INTEL_TABLE,
            RESERVED_TABLE as u64,
        );
        p64(&mut bytes, RESERVED_TABLE, (BASE + VTABLE_RVA.0) as u64);
        p32(&mut bytes, RESERVED_TABLE + OWNER, 0);
        p64(&mut bytes, RESERVED_TABLE + DATA, RESERVED_RECORDS as u64);
        p32(&mut bytes, RESERVED_TABLE + CAPACITY, count);
        p32(&mut bytes, RESERVED_TABLE + COUNT, count);
        lanes(
            &mut bytes,
            RESERVED_RECORDS,
            0,
            [7, -11, i64::MIN, i64::MAX],
        );
        bytes
    }

    fn run_reserved(
        bytes: &[u8],
        capacity: usize,
    ) -> (ReservedCapture, Vec<country_intel_row::Row>) {
        let mut rows = vec![country_intel_row::Row::default(); capacity];
        let mut scratch = [0_u8; 2 * country_intel_row::STRIDE];
        let result = walk_with_reserved(&memory(bytes), BASE, GS, &mut rows, &mut scratch);
        rows.truncate(result.outcome.len());
        (result, rows)
    }

    #[test]
    fn reserved_vector_appends_exact_signed_row_zero_and_final_zero_sentinel() {
        let bytes = reserved_fixture(5);
        let positive = run(&bytes, 8).1;
        let (capture, rows) = run_reserved(&bytes, 8);
        assert_eq!(capture.outcome, Outcome::Rows { len: 4 });
        assert_eq!(capture.vector_count, Ok(5));
        assert!(capture.reserved_reached);
        assert_eq!(&rows[..positive.len()], positive.as_slice());
        assert_eq!((rows[2].owner_country, rows[2].target_index), (0, 0));
        assert_eq!(
            (rows[2].civilian, rows[2].army, rows[2].navy, rows[2].air),
            (7, -11, i64::MIN, i64::MAX)
        );
        assert_eq!(
            rows[3],
            country_intel_row::Row {
                owner_country: 0,
                target_index: 4,
                ..Default::default()
            }
        );
        assert_eq!(capture.reserved_required, 4);
    }

    #[test]
    fn reserved_empty_is_admitted_but_null_or_wrong_identity_preserves_only_positives() {
        let mut empty = reserved_fixture(0);
        p64(&mut empty, RESERVED_TABLE + DATA, 0);
        let (capture, rows) = run_reserved(&empty, 8);
        assert_eq!(capture.vector_count, Ok(0));
        assert_eq!(capture.outcome, Outcome::Rows { len: 2 });
        assert!(rows.iter().all(|row| row.owner_country > 0));
        let corruptions: [Case<'_>; 5] = [
            ("null", &|bytes| {
                p64(bytes, RESERVED_COUNTRY + ccountry::INTEL_TABLE, 0)
            }),
            ("vtable", &|bytes| p64(bytes, RESERVED_TABLE, 1)),
            ("owner", &|bytes| p32(bytes, RESERVED_TABLE + OWNER, 1)),
            ("negative", &|bytes| {
                p32(bytes, RESERVED_TABLE + COUNT, u32::MAX)
            }),
            ("capacity", &|bytes| {
                p32(bytes, RESERVED_TABLE + CAPACITY, 1)
            }),
        ];
        for (name, corrupt) in corruptions {
            let mut bytes = reserved_fixture(3);
            corrupt(&mut bytes);
            let positive = run(&bytes, 8);
            let (capture, rows) = run_reserved(&bytes, 8);
            assert!(capture.vector_count.is_err(), "{name}");
            assert_eq!((capture.outcome, rows), positive, "{name}");
        }
    }

    struct ReservedMemory<'a> {
        bytes: &'a [u8],
        root: Result<ccountry::Country, &'static str>,
        fail_once: Option<usize>,
        faults: std::cell::Cell<usize>,
        root_reads: std::cell::Cell<usize>,
    }

    impl Memory for ReservedMemory<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == RESERVED_COUNTRY {
                self.root_reads.set(self.root_reads.get() + 1);
            }
            if self.fail_once == Some(address) {
                let previous = self.faults.get();
                self.faults.set(previous + 1);
                if previous == 0 {
                    return false;
                }
            }
            memory(self.bytes).copy(address, destination)
        }

        fn authenticated_reserved_country(
            &self,
            base: usize,
            state: usize,
        ) -> Option<Result<ccountry::Country, &'static str>> {
            assert_eq!((base, state), (BASE, GS));
            Some(self.root)
        }
    }

    fn cached_reserved(bytes: &[u8]) -> ReservedMemory<'_> {
        ReservedMemory {
            bytes,
            root: Ok(ccountry::Country {
                pointer: RESERVED_COUNTRY,
                id: 0,
                canonical_slot: 0,
            }),
            fail_once: None,
            faults: std::cell::Cell::new(0),
            root_reads: std::cell::Cell::new(0),
        }
    }

    #[test]
    fn reserved_cached_root_result_is_reused_for_faults_and_physical_aliases() {
        let mut bytes = reserved_fixture(3);
        p64(&mut bytes, 0x3010, RESERVED_COUNTRY as u64);
        p64(&mut bytes, RESERVED_COUNTRY, 1);
        let memory = cached_reserved(&bytes);
        let mut rows = vec![country_intel_row::Row::default(); 8];
        let mut scratch = [0_u8; 64];
        let capture = walk_with_reserved(&memory, BASE, GS, &mut rows, &mut scratch);
        assert_eq!(capture.vector_count, Ok(3));
        assert_eq!(memory.root_reads.get(), 0);
        let bytes = reserved_fixture(3);
        let mut memory = cached_reserved(&bytes);
        memory.root = Err("first cached reserved root fault");
        let capture = walk_with_reserved(&memory, BASE, GS, &mut rows, &mut scratch);
        assert_eq!(
            capture.vector_count,
            Err("first cached reserved root fault")
        );
        assert_eq!(capture.outcome, Outcome::Rows { len: 2 });
        assert_eq!(memory.root_reads.get(), 0);
    }

    #[test]
    fn reserved_body_fault_is_not_retried_and_retains_full_known_header_floor() {
        let bytes = reserved_fixture(5);
        let mut memory = cached_reserved(&bytes);
        memory.fail_once = Some(RESERVED_RECORDS + 2 * country_intel_row::STRIDE);
        let mut rows = vec![country_intel_row::Row::default(); 8];
        let mut scratch = [0_u8; 64];
        let capture = walk_with_reserved(&memory, BASE, GS, &mut rows, &mut scratch);
        assert_eq!(capture.outcome, Outcome::Rows { len: 2 });
        assert!(capture.vector_count.is_err());
        assert_eq!(capture.reserved_required, 7);
        assert_eq!(memory.faults.get(), 1);
    }

    #[test]
    fn reserved_invalid_data_or_span_adds_no_capture_or_hint_pressure() {
        for data in [0, u64::MAX - country_intel_row::STRIDE as u64] {
            let mut bytes = reserved_fixture(5);
            p64(&mut bytes, RESERVED_TABLE + DATA, data);
            let positive = run(&bytes, 8);
            let (capture, rows) = run_reserved(&bytes, 8);
            assert_eq!((capture.outcome, rows), positive);
            assert!(capture.vector_count.is_err());
            assert_eq!(capture.reserved_required, 0);
            assert_eq!(native_rows(&memory(&bytes), BASE, GS), Some(3));
            p64(&mut bytes, RESERVED_TABLE + DATA, RESERVED_RECORDS as u64);
            assert_eq!(native_rows(&memory(&bytes), BASE, GS), Some(8));
        }
    }

    #[test]
    fn native_hint_uses_one_sticky_root_and_never_retries_a_failed_country_header() {
        struct HintMemory<'a> {
            bytes: &'a [u8],
            fail_once: usize,
            attempts: std::cell::Cell<usize>,
        }
        impl Memory for HintMemory<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if address == self.fail_once {
                    let previous = self.attempts.get();
                    self.attempts.set(previous + 1);
                    if previous == 0 {
                        return false;
                    }
                }
                memory(self.bytes).copy(address, destination)
            }
        }
        let mut bytes = reserved_fixture(3);
        p64(&mut bytes, 0x3010, RESERVED_COUNTRY as u64);
        for (address, expected) in [
            (GS + cgame_state::COUNTRIES, None),
            (RESERVED_COUNTRY, Some(3)),
        ] {
            let memory = HintMemory {
                bytes: &bytes,
                fail_once: address,
                attempts: std::cell::Cell::new(0),
            };
            assert_eq!(native_rows(&memory, BASE, GS), expected);
            assert_eq!(memory.attempts.get(), 1);
        }
    }

    #[test]
    fn reserved_suffix_reuses_positive_reader_headers_and_a_failed_root_attempt() {
        struct ObservedMemory<'a> {
            bytes: &'a [u8],
            observed: usize,
            fail_first: bool,
            attempts: std::cell::Cell<usize>,
        }
        impl Memory for ObservedMemory<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if address == self.observed {
                    let previous = self.attempts.get();
                    self.attempts.set(previous + 1);
                    if self.fail_first && previous == 0 {
                        return false;
                    }
                }
                memory(self.bytes).copy(address, destination)
            }
        }
        let bytes = reserved_fixture(3);
        for address in [GS + cgame_state::COUNTRIES, GS + cgame_state::COUNTRY_LINKS] {
            let memory = ObservedMemory {
                bytes: &bytes,
                observed: address,
                fail_first: false,
                attempts: std::cell::Cell::new(0),
            };
            let mut rows = vec![country_intel_row::Row::default(); 8];
            let mut scratch = [0_u8; 64];
            let capture = walk_with_reserved(&memory, BASE, GS, &mut rows, &mut scratch);
            assert_eq!(capture.vector_count, Ok(3));
            assert_eq!(memory.attempts.get(), 1);
        }
        let memory = ObservedMemory {
            bytes: &bytes,
            observed: RESERVED_COUNTRY,
            fail_first: true,
            attempts: std::cell::Cell::new(0),
        };
        let mut rows = vec![country_intel_row::Row::default(); 8];
        let mut scratch = [0_u8; 64];
        let capture = walk_with_reserved(&memory, BASE, GS, &mut rows, &mut scratch);
        assert_eq!(capture.outcome, Outcome::Rows { len: 2 });
        assert!(capture.vector_count.is_err());
        assert_eq!(capture.reserved_required, 0);
        assert_eq!(memory.attempts.get(), 1);
    }

    #[test]
    fn failed_selected_identity_survives_prepare_roster_view_and_native_hint() {
        struct FailOnce<'a> {
            bytes: &'a [u8],
            fault: usize,
            attempts: std::cell::Cell<usize>,
        }
        impl Memory for FailOnce<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if address == self.fault {
                    let previous = self.attempts.get();
                    self.attempts.set(previous + 1);
                    if previous == 0 {
                        return false;
                    }
                }
                memory(self.bytes).copy(address, destination)
            }
        }

        let mut bytes = reserved_fixture(3);
        p64(&mut bytes, 0x3010, RESERVED_COUNTRY as u64);
        let positive = run(&reserved_fixture(3), 8).1;
        for (fault, reason) in [
            (
                RESERVED_COUNTRY,
                "reserved country object identity mismatch",
            ),
            (
                RESERVED_COUNTRY + ccountry::ID,
                "reserved country id is not zero",
            ),
        ] {
            let source = FailOnce {
                bytes: &bytes,
                fault,
                attempts: std::cell::Cell::new(0),
            };
            let mut cache = crate::country_memory::Cache::with_capacity(128);
            cache.prepare(&source, BASE, GS);
            let view = cache.roster_view(&source);
            assert_eq!(
                view.seed_status(),
                crate::country_memory::SeedStatus::Unavailable { reason }
            );
            assert!(view.authenticated_countries(BASE, GS).is_none());
            let observation = view
                .authenticated_reserved_country_observation(BASE, GS)
                .unwrap();
            assert_eq!(observation.result(), Err(reason));
            assert_eq!(
                observation.physical_alias(RESERVED_COUNTRY),
                Some(Err(reason))
            );
            assert_eq!(observation.physical_alias(COUNTRIES[0]), None);
            assert_eq!(native_rows(&view, BASE, GS), Some(3));

            let reader = ccountry::Reader::new(&view, BASE, GS).unwrap();
            assert_eq!(reader.reserved(), Err(reason));
            assert!(reader.at(1).unwrap().is_some());
            assert_eq!(reader.at(2), Err(reason));
            let mut rows = vec![country_intel_row::Row::default(); 8];
            let mut scratch = [0_u8; 64];
            assert_eq!(
                walk_in_reader(
                    &view,
                    BASE,
                    &reader,
                    engine_native_memory::walk::Buffer::Growing(&mut rows),
                    &mut scratch,
                    &mut |_, _, _| {},
                ),
                Outcome::Unavailable { reason }
            );
            assert_eq!(&rows[..positive.len()], positive.as_slice());
            assert_eq!(source.attempts.get(), 1);

            let next = cache.prepare(&source, BASE, GS);
            assert_eq!(next.seed_status(), crate::country_memory::SeedStatus::Ready);
            assert!(
                next.authenticated_reserved_country_observation(BASE, GS)
                    .unwrap()
                    .result()
                    .is_ok()
            );
            assert_eq!(source.attempts.get(), 2);
        }
    }
}
