//! CNuke entries are inline in CCountry's two-type array, not pointers to rows.
//! The raw amount is a signed i64 progress quantity; it is not the ready count.

use crate::{Memory, ccountry, cpdx_array::Array, walk::Outcome};

/// Nuke primary vtable.
pub const VTABLE_RVA: usize = 0x027e_7270;
/// CNuke inline array stride.
pub const STRIDE: usize = 0x48;
/// CNuke owning CCountry pointer.
pub const COUNTRY: usize = 0x10;
/// CNuke signed i64 raw amount.
pub const AMOUNT: usize = 0x18;
/// CNuke type u32.
pub const TYPE: usize = 0x38;
/// Ready nuke count, signed i32; reset and increments are independent of amount.
pub const READY: usize = 0x3c;
/// The country nuke array can have any valid count. Loading clears it and appends a default object
/// per saved entry without checking the total. The persistent name token is set by ordinary
/// construction; loading uses the base default.
pub const SERIAL_TYPE: usize = 8;
/// Normal-construction nuke name token, shared with its parent array.
pub const SERIAL_TYPE_VALUE: u32 = 0x34cd;
/// Loaded-construction nuke name token from the persistent-base default.
pub const LOADED_SERIAL_TYPE_VALUE: u32 = 0x165;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Authenticated owner's raw country reference, including engine aliases.
    pub country: u32,
    /// Native type ordinal, not a collector-assigned row identifier.
    pub nuke_type: u32,
    pub amount_raw: i64,
    /// This hour's ready launch count. Post-hour queue consumption retains it;
    /// it is not an independently derived available-stockpile count.
    pub ready: i32,
}

/// Copies the two inline CNuke entries for every canonical country. The row key
/// is (country, nuke_type); amount and ready retain their native signed widths.
/// No totals or progress-to-ready conversion are substituted for either field.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, rows: &mut [Row]) -> Outcome {
    walk_buffer(memory, base, game_state, crate::walk::Buffer::Fixed(rows))
}

pub fn walk_growing(
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
    let vtable = base
        .checked_add(VTABLE_RVA)
        .ok_or("nuke vtable address overflow")? as u64;
    let mut found_country = false;
    let mut len = 0_usize;
    let mut unread_rows = 0_u32;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        found_country = true;
        let array = country
            .pointer
            .checked_add(ccountry::NUKES)
            .and_then(|address| Array::read(memory, address, STRIDE))
            .ok_or("country nuke array unreadable or invalid")?;
        rows.ensure(
            len.checked_add(array.count as usize)
                .ok_or("nuke row count overflow")?,
        )?;
        for nuke_type in 0..array.count {
            if len == rows.len() {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("nuke row count overflow")?;
                continue;
            }
            let object = array
                .element(nuke_type, STRIDE)
                .ok_or("nuke address overflow")?;
            rows[len] = read_row(
                memory,
                object,
                vtable,
                country.pointer,
                country.id,
                nuke_type,
            )?;
            len += 1;
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

    /// Types written by the fixture; the reader accepts any native count.
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
        // Slot 3 aliases slot 1, and the native owner id is the alias.
        u64_at(&mut bytes, COUNTRIES + 8, OWNER as u64);
        u64_at(&mut bytes, COUNTRIES + 24, OWNER as u64);
        u32_at(&mut bytes, LINKS + 4, 1);
        u32_at(&mut bytes, LINKS + 12, 1);
        u64_at(&mut bytes, OWNER, ccountry::VTABLE_RVA as u64);
        u32_at(&mut bytes, OWNER + ccountry::ID, 3);
        array(&mut bytes, OWNER + ccountry::NUKES, NUKES, TYPE_COUNT);
        for index in 0..TYPE_COUNT {
            let at = NUKES + index as usize * STRIDE;
            u64_at(&mut bytes, at, VTABLE_RVA as u64);
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

    fn capture(bytes: &[u8], output: &mut [Row]) -> Outcome {
        walk(&SliceMemory { base: 0, bytes }, 0, GAME, output)
    }

    #[test]
    fn raw_signed_values_and_alias_owners_survive_without_totals_or_rounding() {
        let bytes = fixture();
        let mut rows = [Row::default(); 4];
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
            capture(&bytes, &mut rows[..1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(
            capture(&bytes, &mut []),
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
        let mut rows = [Row::default(); 2];
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
                capture(&bytes, &mut [Row::default(); 2]),
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
                capture(&bytes, &mut [Row::default(); 2]),
                Outcome::Unavailable { .. }
            ));
        }
        let bytes = fixture();
        assert!(matches!(
            capture(
                &bytes[..NUKES + STRIDE + READY + 3],
                &mut [Row::default(); 2]
            ),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn any_native_type_count_emits_exactly_the_entries_that_exist() {
        // A save-loaded array can hold any count; zero is a readable empty.
        let mut bytes = fixture();
        u32_at(&mut bytes, OWNER + ccountry::NUKES + 12, 0);
        assert_eq!(
            capture(&bytes, &mut [Row::default(); 2]),
            Outcome::Rows { len: 0 }
        );
        // A third entry is read and authenticated like the first two.
        let mut bytes = fixture();
        array(&mut bytes, OWNER + ccountry::NUKES, NUKES, 3);
        let at = NUKES + 2 * STRIDE;
        u64_at(&mut bytes, at, VTABLE_RVA as u64);
        u32_at(&mut bytes, at + SERIAL_TYPE, LOADED_SERIAL_TYPE_VALUE);
        u64_at(&mut bytes, at + COUNTRY, OWNER as u64);
        u64_at(&mut bytes, at + AMOUNT, 7);
        u32_at(&mut bytes, at + TYPE, 2);
        u32_at(&mut bytes, at + READY, 1);
        let mut rows = [Row::default(); 3];
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
        assert_eq!(
            capture(&bytes, &mut rows[..2]),
            Outcome::Truncated {
                len: 2,
                unread_rows: 1
            }
        );
    }

    #[test]
    fn absent_roster_is_not_an_empty_stockpile_and_overflow_does_not_read_a_suffix() {
        let mut bytes = fixture();
        u64_at(&mut bytes, COUNTRIES + 8, 0);
        u64_at(&mut bytes, COUNTRIES + 24, 0);
        assert!(matches!(
            capture(&bytes, &mut []),
            Outcome::Unavailable { .. }
        ));
        let bytes = fixture();
        assert_eq!(
            capture(&bytes[..NUKES + STRIDE], &mut [Row::default(); 1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
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
