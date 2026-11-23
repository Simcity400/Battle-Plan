
use crate::{
    Memory, ccountry,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    walk::Outcome,
};
use engine_sites::Rva;

const COUNTRY_BATCH: usize = 64;
const NUKE_HEADER_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: ccountry::NUKES,
    destination_offset: 0,
    len: 16,
}];
const NUKE_HEADER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&NUKE_HEADER_FIELDS, 16).expect("valid nuke header layout");
const PAIR_BYTES: usize = STRIDE * 2;
const NUKE_PAIR_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: PAIR_BYTES,
}];
const NUKE_PAIR_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&NUKE_PAIR_FIELDS, PAIR_BYTES).expect("valid nuke pair layout");

pub const VTABLE_RVA: Rva = Rva(0x0286_4640);
pub const STRIDE: usize = 0x48;
pub const COUNTRY: usize = 0x10;
pub const AMOUNT: usize = 0x18;
pub const TYPE: usize = 0x38;
pub const READY: usize = 0x3c;
pub const SERIAL_TYPE: usize = 8;
pub const SERIAL_TYPE_VALUE: u32 = 0x34cd;
pub const LOADED_SERIAL_TYPE_VALUE: u32 = 0x165;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub nuke_type: u32,
    pub amount_raw: i64,
    pub ready: i32,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(memory, base, game_state, crate::walk::Buffer::Growing(rows))
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: crate::walk::Buffer<'_, Row>,
) -> Outcome {
    match walk_rows(memory, base, game_state, rows) {
        Ok(outcome) => outcome,
        Err(reason) => Outcome::Unavailable { reason },
    }
}

fn walk_rows(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut rows: crate::walk::Buffer<'_, Row>,
) -> Result<Outcome, &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let vtable = engine_sites::at(base, VTABLE_RVA).ok_or("nuke vtable address overflow")? as u64;
    let mut found_country = false;
    let mut len = 0_usize;
    let mut unread_rows = 0_u32;
    let mut batch_countries = [ccountry::Country {
        id: 0,
        canonical_slot: 0,
        pointer: 0,
    }; COUNTRY_BATCH];
    let mut batch_ptrs = [0_usize; COUNTRY_BATCH];
    let mut batch_headers = [0_u8; COUNTRY_BATCH * 16];
    let mut batch_arrays = [Array::default(); COUNTRY_BATCH];
    let mut pair_ptrs = [0_usize; COUNTRY_BATCH];
    let mut pair_slots = [0_usize; COUNTRY_BATCH];
    let mut pair_bytes = [0_u8; COUNTRY_BATCH * PAIR_BYTES];
    let mut slot = 1_u32;
    while slot < countries.count() {
        let mut n = 0_usize;
        let mut all_safe = true;
        while slot < countries.count() && n < COUNTRY_BATCH {
            let cur = slot;
            slot += 1;
            let Some(country) = countries.at(cur)? else {
                continue;
            };
            found_country = true;
            batch_countries[n] = country;
            batch_ptrs[n] = country.pointer;
            all_safe &= country.pointer.checked_add(ccountry::NUKES + 16).is_some();
            n += 1;
        }
        if n == 0 {
            break;
        }
        let headers_copied = if all_safe {
            memory.copy_records(
                RecordSources::Pointers(&batch_ptrs[..n]),
                &NUKE_HEADER_LAYOUT,
                &mut batch_headers[..n * 16],
            )
        } else {
            0
        };
        let mut pair_count = 0_usize;
        for i in 0..n {
            let array = if i < headers_copied {
                Array::from_header(
                    batch_headers[i * 16..(i + 1) * 16].try_into().unwrap(),
                    STRIDE,
                )
                .ok_or("country nuke array unreadable or invalid")?
            } else {
                batch_countries[i]
                    .pointer
                    .checked_add(ccountry::NUKES)
                    .and_then(|address| Array::read(memory, address, STRIDE))
                    .ok_or("country nuke array unreadable or invalid")?
            };
            batch_arrays[i] = array;
            if array.count == 2
                && array.data != 0
                && array.data.checked_add(PAIR_BYTES).is_some()
            {
                pair_ptrs[pair_count] = array.data;
                pair_slots[pair_count] = i;
                pair_count += 1;
            }
        }
        let pairs_copied = if pair_count > 0 {
            memory.copy_records(
                RecordSources::Pointers(&pair_ptrs[..pair_count]),
                &NUKE_PAIR_LAYOUT,
                &mut pair_bytes[..pair_count * PAIR_BYTES],
            )
        } else {
            0
        };
        let mut pair_index = [usize::MAX; COUNTRY_BATCH];
        for k in 0..pairs_copied {
            pair_index[pair_slots[k]] = k;
        }
        for i in 0..n {
            let country = batch_countries[i];
            let array = batch_arrays[i];
            rows.ensure(
                len.checked_add(array.count as usize)
                    .ok_or("nuke row count overflow")?,
            )?;
            let k = pair_index[i];
            for nuke_type in 0..array.count {
                if len == rows.len() {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("nuke row count overflow")?;
                    continue;
                }
                rows[len] = if k != usize::MAX {
                    let off = k * PAIR_BYTES + nuke_type as usize * STRIDE;
                    decode_row(
                        &pair_bytes[off..off + STRIDE],
                        vtable,
                        country.pointer,
                        country.id,
                        nuke_type,
                    )?
                } else {
                    let object = array
                        .element(nuke_type, STRIDE)
                        .ok_or("nuke address overflow")?;
                    read_row(
                        memory,
                        object,
                        vtable,
                        country.pointer,
                        country.id,
                        nuke_type,
                    )?
                };
                len += 1;
            }
        }
    }
    if !found_country {
        return Err("nuke country roster contains no countries");
    }
    Ok(if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    })
}

fn decode_row(
    entry: &[u8],
    vtable: u64,
    owner: usize,
    country: u32,
    nuke_type: u32,
) -> Result<Row, &'static str> {
    if u64::from_le_bytes(entry[..8].try_into().unwrap()) != vtable {
        return Err("nuke vtable mismatch");
    }
    if !matches!(
        u32::from_le_bytes(entry[SERIAL_TYPE..SERIAL_TYPE + 4].try_into().unwrap()),
        SERIAL_TYPE_VALUE | LOADED_SERIAL_TYPE_VALUE
    ) {
        return Err("nuke serialized type mismatch");
    }
    if u64::from_le_bytes(entry[COUNTRY..COUNTRY + 8].try_into().unwrap()) != owner as u64 {
        return Err("nuke owner mismatch");
    }
    if u32::from_le_bytes(entry[TYPE..TYPE + 4].try_into().unwrap()) != nuke_type {
        return Err("nuke type does not match native array ordinal");
    }
    Ok(Row {
        country,
        nuke_type,
        amount_raw: i64::from_le_bytes(entry[AMOUNT..AMOUNT + 8].try_into().unwrap()),
        ready: i32::from_le_bytes(entry[READY..READY + 4].try_into().unwrap()),
    })
}

fn read_row(
    memory: &impl Memory,
    object: usize,
    vtable: u64,
    owner: usize,
    country: u32,
    nuke_type: u32,
) -> Result<Row, &'static str> {
    let owner_address = object.checked_add(COUNTRY).ok_or("nuke address overflow")?;
    let type_address = object.checked_add(TYPE).ok_or("nuke address overflow")?;
    let mut identity = [0; 12];
    let mut owner_amount = [0; 16];
    let mut type_ready = [0; 8];
    let completed = memory.copy_scattered(
        [object, owner_address, type_address],
        &mut [&mut identity, &mut owner_amount, &mut type_ready],
    );
    if completed == 0 {
        return Err("nuke identity unreadable");
    }
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != vtable {
        return Err("nuke vtable mismatch");
    }
    if !matches!(
        u32::from_le_bytes(identity[SERIAL_TYPE..].try_into().unwrap()),
        SERIAL_TYPE_VALUE | LOADED_SERIAL_TYPE_VALUE
    ) {
        return Err("nuke serialized type mismatch");
    }
    if completed < 2 {
        return Err("nuke owner or amount unreadable");
    }
    if u64::from_le_bytes(owner_amount[..8].try_into().unwrap()) != owner as u64 {
        return Err("nuke owner mismatch");
    }
    if completed < 3 {
        return Err("nuke type or ready count unreadable");
    }
    if u32::from_le_bytes(type_ready[..4].try_into().unwrap()) != nuke_type {
        return Err("nuke type does not match native array ordinal");
    }
    Ok(Row {
        country,
        nuke_type,
        amount_raw: i64::from_le_bytes(owner_amount[8..].try_into().unwrap()),
        ready: i32::from_le_bytes(type_ready[4..].try_into().unwrap()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cgame_state, memory::SliceMemory};

    const TYPE_COUNT: u32 = 2;
    const GAME: usize = 0x1000;
    const COUNTRIES: usize = 0x2000;
    const LINKS: usize = 0x2100;
    const OWNER: usize = 0x3000;
    const NUKES: usize = 0x5000;

    fn u32_at(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn u64_at(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        u64_at(bytes, at, data as u64);
        u32_at(bytes, at + 8, count);
        u32_at(bytes, at + 12, count);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x6000];
        array(&mut bytes, GAME + cgame_state::COUNTRIES, COUNTRIES, 4);
        array(&mut bytes, GAME + cgame_state::COUNTRY_LINKS, LINKS, 4);
        u64_at(&mut bytes, COUNTRIES + 8, OWNER as u64);
        u64_at(&mut bytes, COUNTRIES + 24, OWNER as u64);
        u32_at(&mut bytes, LINKS + 4, 1);
        u32_at(&mut bytes, LINKS + 12, 1);
        u64_at(&mut bytes, OWNER, ccountry::VTABLE_RVA.0 as u64);
        u32_at(&mut bytes, OWNER + ccountry::ID, 3);
        array(&mut bytes, OWNER + ccountry::NUKES, NUKES, TYPE_COUNT);
        for index in 0..TYPE_COUNT {
            let at = NUKES + index as usize * STRIDE;
            u64_at(&mut bytes, at, VTABLE_RVA.0 as u64);
            u32_at(&mut bytes, at + SERIAL_TYPE, SERIAL_TYPE_VALUE);
            u64_at(&mut bytes, at + COUNTRY, OWNER as u64);
            u64_at(
                &mut bytes,
                at + AMOUNT,
                [i64::MIN, i64::MAX][index as usize] as u64,
            );
            u32_at(&mut bytes, at + TYPE, index);
            u32_at(
                &mut bytes,
                at + READY,
                [i32::MIN, i32::MAX][index as usize] as u32,
            );
        }
        bytes
    }

    fn capture(bytes: &[u8], output: &mut Vec<Row>) -> Outcome {
        walk(&SliceMemory { base: 0, bytes }, 0, GAME, output)
    }

    fn count(bytes: &[u8]) -> Outcome {
        walk_buffer(&SliceMemory { base: 0, bytes }, 0, GAME, crate::walk::Buffer::Count)
    }

    #[test]
    fn raw_signed_values_and_alias_owners_survive_without_totals_or_rounding() {
        let bytes = fixture();
        let mut rows = vec![Row::default(); 4];
        assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 2 });
        assert_eq!(
            rows[0],
            Row {
                country: 3,
                nuke_type: 0,
                amount_raw: i64::MIN,
                ready: i32::MIN
            }
        );
        assert_eq!(
            rows[1],
            Row {
                country: 3,
                nuke_type: 1,
                amount_raw: i64::MAX,
                ready: i32::MAX
            }
        );
        assert_eq!(
            count(&bytes),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
    }

    #[test]
    fn save_constructed_entries_keep_the_base_persistent_name_token() {
        let mut bytes = fixture();
        for index in 0..TYPE_COUNT {
            u32_at(
                &mut bytes,
                NUKES + index as usize * STRIDE + SERIAL_TYPE,
                LOADED_SERIAL_TYPE_VALUE,
            );
        }
        let mut rows = vec![Row::default(); 2];
        assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 2 });
        assert_eq!((rows[0].amount_raw, rows[1].ready), (i64::MIN, i32::MAX));
    }

    #[test]
    fn incomplete_roots_and_wrong_inline_authentication_are_unavailable() {
        for (at, value) in [
            (OWNER + ccountry::NUKES + 8, 1),
            (OWNER + ccountry::NUKES + 8, u32::MAX),
            (NUKES + STRIDE + SERIAL_TYPE, 0),
            (NUKES + STRIDE + TYPE, 0),
        ] {
            let mut bytes = fixture();
            u32_at(&mut bytes, at, value);
            assert!(matches!(
                capture(&bytes, &mut vec![Row::default(); 2]),
                Outcome::Unavailable { .. }
            ));
        }
        for at in [
            OWNER + ccountry::NUKES,
            NUKES + STRIDE,
            NUKES + STRIDE + COUNTRY,
        ] {
            let mut bytes = fixture();
            u64_at(&mut bytes, at, 0);
            assert!(matches!(
                capture(&bytes, &mut vec![Row::default(); 2]),
                Outcome::Unavailable { .. }
            ));
        }
        let bytes = fixture();
        assert!(matches!(
            capture(
                &bytes[..NUKES + STRIDE + READY + 3],
                &mut vec![Row::default(); 2]
            ),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn any_native_type_count_emits_exactly_the_entries_that_exist() {
        let mut bytes = fixture();
        u32_at(&mut bytes, OWNER + ccountry::NUKES + 12, 0);
        assert_eq!(
            capture(&bytes, &mut vec![Row::default(); 2]),
            Outcome::Rows { len: 0 }
        );
        let mut bytes = fixture();
        array(&mut bytes, OWNER + ccountry::NUKES, NUKES, 3);
        let at = NUKES + 2 * STRIDE;
        u64_at(&mut bytes, at, VTABLE_RVA.0 as u64);
        u32_at(&mut bytes, at + SERIAL_TYPE, LOADED_SERIAL_TYPE_VALUE);
        u64_at(&mut bytes, at + COUNTRY, OWNER as u64);
        u64_at(&mut bytes, at + AMOUNT, 7);
        u32_at(&mut bytes, at + TYPE, 2);
        u32_at(&mut bytes, at + READY, 1);
        let mut rows = vec![Row::default(); 3];
        assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 3 });
        assert_eq!(
            rows[2],
            Row {
                country: 3,
                nuke_type: 2,
                amount_raw: 7,
                ready: 1
            }
        );
    }

    #[test]
    fn absent_roster_is_not_an_empty_stockpile_and_overflow_does_not_read_a_suffix() {
        let mut bytes = fixture();
        u64_at(&mut bytes, COUNTRIES + 8, 0);
        u64_at(&mut bytes, COUNTRIES + 24, 0);
        assert!(matches!(
            count(&bytes),
            Outcome::Unavailable { .. }
        ));
        let bytes = fixture();
        assert_eq!(
            read_row(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                usize::MAX - 8,
                0,
                OWNER,
                3,
                0
            ),
            Err("nuke address overflow")
        );
    }
}
