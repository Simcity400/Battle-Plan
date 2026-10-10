//! CCryptology, owned by CIntelligenceAgency.

use crate::{
    Memory, ccountry, ccountry_decryption_state as state, cintelligence_agency as agency,
    cpdx_array::Array, memory::Raw, read, walk::Outcome,
};

/// CCryptology primary vtable.
pub const VTABLE_RVA: usize = 0x029a_6540;
/// CCryptology inline CCountryDecryptionState array data pointer.
pub const DECRYPTION_STATES: usize = 0x28;
/// Decryption-state signed i32 count, incremented after each 0x38-byte inline row.
pub const DECRYPTION_STATE_COUNT: usize = 0x34;
/// CCryptology decryption-state capacity i32.
pub const DECRYPTION_STATE_CAPACITY: usize = 0x30;
/// CCryptology owning CCountry pointer.
pub const OWNER_COUNTRY: usize = 0x08;
/// CCryptology container Save.
pub const SAVE_RVA: usize = 0x013e_3d30;
/// CCryptology container Load.
pub const LOAD_RVA: usize = 0x013e_3950;
/// Decryption targets token, emitted only for nonzero state count.
pub const TOKEN_TARGETS: u32 = 0x3d75;
/// Intelligence-source token, emitted only when present.
pub const TOKEN_INTEL_SOURCE: u32 = 0x4b34;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub owner_country: u32,
    pub target_country: u32,
    pub amount_raw: i64,
    pub bonus_days: i32,
    pub bonus_active: u8,
    pub decrypting: u8,
}

/// Reads each authenticated country's inline decryption states without allocation.
/// Zero progress and inactive states are retained; days-to-decrypt is derived by
/// the engine and is deliberately not substituted for a native stored field.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut [Row]) -> Outcome {
    walk_buffer(memory, base, game_state, crate::walk::Buffer::Fixed(output))
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: crate::walk::Buffer<'_, Row>,
) -> Outcome {
    walk_inner(memory, base, game_state, output)
        .unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
) -> Result<Outcome, &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut found_country = false;
    let mut len = 0_usize;
    let mut unread_rows = 0_u32;
    let mut seen = crate::slot_set::SlotSet::new();
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        found_country = true;
        let agency_pointer = pointer(memory, country.pointer, ccountry::INTELLIGENCE_AGENCY)?;
        if agency_pointer == 0 {
            continue;
        }
        authenticate(memory, base, agency_pointer, agency::VTABLE_RVA)?;
        if pointer(memory, agency_pointer, agency::OWNER_COUNTRY)? != country.pointer {
            return Err("cryptology agency owner mismatch");
        }
        let crypto = pointer(memory, agency_pointer, agency::CRYPTOLOGY)?;
        if crypto == 0 {
            continue;
        }
        authenticate(memory, base, crypto, VTABLE_RVA)?;
        if pointer(memory, crypto, OWNER_COUNTRY)? != country.pointer {
            return Err("cryptology owner mismatch");
        }
        let states = crypto
            .checked_add(DECRYPTION_STATES)
            .and_then(|address| Array::read(memory, address, state::STRIDE))
            .ok_or("cryptology state array unreadable or invalid")?;
        output.ensure(
            len.checked_add(states.count as usize)
                .ok_or("cryptology row demand overflow")?,
        )?;
        let owner_start = len;
        for index in 0..states.count {
            if len == output.len() {
                unread_rows = unread_rows
                    .checked_add(states.count - index)
                    .ok_or("cryptology unread row count overflow")?;
                break;
            }
            let at = states
                .element(index, state::STRIDE)
                .ok_or("cryptology row address overflow")?;
            let mut row = read_state(memory, base, at)?;
            let target = countries.resolve(row.target_country)?;
            // The engine maintains one state per canonical target. The set
            // answers in O(1); its exact fallback uses the unpublished owner
            // word as scratch until this owner's walk finishes.
            let slot = target.canonical_slot;
            let earlier = &output[owner_start..len];
            if seen.insert(slot, || {
                Ok::<_, &'static str>(earlier.iter().any(|p| p.owner_country == slot))
            })? {
                return Err("duplicate cryptology target");
            }
            row.owner_country = target.canonical_slot;
            output[len] = row;
            len += 1;
        }
        seen.clear();
        for row in &mut output[owner_start..len] {
            row.owner_country = country.id;
        }
    }
    if !found_country {
        Err("country table has no readable countries")
    } else if unread_rows != 0 {
        Ok(Outcome::Truncated { len, unread_rows })
    } else {
        Ok(Outcome::Rows { len })
    }
}

fn read_state(memory: &impl Memory, base: usize, at: usize) -> Result<Row, &'static str> {
    // This 32-byte prefix is within the proven 0x38-byte inline object. Reading
    // it once keeps all six fields and the vtable in one guarded native copy.
    let bytes = read::<[u8; 32]>(memory, at).ok_or("cryptology state unreadable")?;
    let expected = base
        .checked_add(state::VTABLE_RVA)
        .ok_or("module address overflow")?;
    if u64::from_le_bytes(bytes[0..8].try_into().unwrap()) != expected as u64 {
        return Err("cryptology state vtable mismatch");
    }
    Ok(Row {
        owner_country: 0,
        target_country: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        bonus_days: i32::from_le_bytes(bytes[12..16].try_into().unwrap()),
        bonus_active: bytes[state::BONUS_ACTIVE],
        decrypting: bytes[state::DECRYPTING],
        amount_raw: i64::from_le_bytes(bytes[24..32].try_into().unwrap()),
    })
}

fn field<T: Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|address| read(memory, address))
        .ok_or("cryptology field unreadable or address overflow")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .map_err(|_| "cryptology pointer overflow")
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    rva: usize,
) -> Result<(), &'static str> {
    let expected = base.checked_add(rva).ok_or("module address overflow")?;
    if read::<u64>(memory, pointer) == Some(expected as u64) {
        Ok(())
    } else {
        Err("cryptology object vtable unreadable or mismatched")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cgame_state, memory::SliceMemory};

    const GS: usize = 0x1000;
    const COUNTRIES: usize = 0x3000;
    const LINKS: usize = 0x3100;
    const COUNTRY: [usize; 3] = [0x4000, 0x6000, 0x8000];
    const AGENCY: usize = 0xa000;
    const CRYPTO: usize = 0xb000;
    const STATES: usize = 0xc000;

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn header(bytes: &mut [u8], at: usize, data: usize, capacity: u32, count: u32) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, capacity);
        p32(bytes, at + 12, count);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0xd000];
        header(&mut bytes, GS + cgame_state::COUNTRIES, COUNTRIES, 5, 5);
        header(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS, 5, 5);
        for (index, pointer) in [usize::MAX, COUNTRY[0], COUNTRY[1], COUNTRY[2], COUNTRY[0]]
            .into_iter()
            .enumerate()
        {
            p64(&mut bytes, COUNTRIES + index * 8, pointer as u64);
        }
        for (index, slot) in [0, 1, 2, 3, 1].into_iter().enumerate() {
            p32(&mut bytes, LINKS + index * 4, slot);
        }
        for (index, country) in COUNTRY.into_iter().enumerate() {
            p64(&mut bytes, country, ccountry::VTABLE_RVA as u64);
            p32(&mut bytes, country + ccountry::ID, [4, 2, 3][index]);
        }
        p64(
            &mut bytes,
            COUNTRY[0] + ccountry::INTELLIGENCE_AGENCY,
            AGENCY as u64,
        );
        p64(&mut bytes, AGENCY, agency::VTABLE_RVA as u64);
        p64(
            &mut bytes,
            AGENCY + agency::OWNER_COUNTRY,
            COUNTRY[0] as u64,
        );
        p64(&mut bytes, AGENCY + agency::CRYPTOLOGY, CRYPTO as u64);
        p64(&mut bytes, CRYPTO, VTABLE_RVA as u64);
        p64(&mut bytes, CRYPTO + OWNER_COUNTRY, COUNTRY[0] as u64);
        // A spare, uninitialized capacity slot must never become a row.
        header(&mut bytes, CRYPTO + DECRYPTION_STATES, STATES, 3, 2);
        for (index, target) in [2, 3].into_iter().enumerate() {
            let at = STATES + index * state::STRIDE;
            p64(&mut bytes, at, state::VTABLE_RVA as u64);
            p32(&mut bytes, at + state::TARGET_COUNTRY, target);
            p32(&mut bytes, at + state::BONUS_DAYS, (-1_i32) as u32);
            bytes[at + state::BONUS_ACTIVE] = 7;
            bytes[at + state::DECRYPTING] = 255;
        }
        p64(
            &mut bytes,
            STATES + state::STRIDE + state::AMOUNT,
            0x1_2345_6789,
        );
        bytes
    }

    fn run(bytes: &[u8], output: &mut [Row]) -> Outcome {
        walk(&SliceMemory { base: 0, bytes }, 0, GS, output)
    }

    #[test]
    fn one_owner_decrypting_5000_countries_is_checked_for_duplicates_in_full() {
        use crate::ccountry::synthetic::World;
        const COUNT: u32 = 5_000;
        let mut world = World::new(0, COUNT, ccountry::INTELLIGENCE_AGENCY + 8);
        let owner = world.object(1);
        let agency_at = world.alloc(0x100);
        let crypto = world.alloc(0x100);
        let states = world.alloc((COUNT as usize - 1) * state::STRIDE);
        world.u64(owner + ccountry::INTELLIGENCE_AGENCY, agency_at as u64);
        world.u64(agency_at, agency::VTABLE_RVA as u64);
        world.u64(agency_at + agency::OWNER_COUNTRY, owner as u64);
        world.u64(agency_at + agency::CRYPTOLOGY, crypto as u64);
        world.u64(crypto, VTABLE_RVA as u64);
        world.u64(crypto + OWNER_COUNTRY, owner as u64);
        world.array(crypto + DECRYPTION_STATES, states, COUNT - 1);
        for index in 0..COUNT as usize - 1 {
            let at = states + index * state::STRIDE;
            world.u64(at, state::VTABLE_RVA as u64);
            world.u32(at + state::TARGET_COUNTRY, index as u32 + 2);
        }
        let mut rows = vec![Row::default(); COUNT as usize];
        let run =
            |world: &World, rows: &mut [Row]| walk(&world.memory(), 0, world.game_state(), rows);
        assert_eq!(
            run(&world, &mut rows),
            Outcome::Rows {
                len: COUNT as usize - 1
            }
        );
        assert!(
            rows[..COUNT as usize - 1]
                .iter()
                .enumerate()
                .all(
                    |(index, row)| row.owner_country == 1 && row.target_country == index as u32 + 2
                )
        );
        let mut growing = Vec::new();
        world.array(crypto + DECRYPTION_STATES, states, 1);
        assert_eq!(
            walk_growing(&world.memory(), 0, world.game_state(), &mut growing),
            Outcome::Rows { len: 1 }
        );
        world.array(crypto + DECRYPTION_STATES, states, COUNT - 1);
        assert_eq!(
            walk_growing(&world.memory(), 0, world.game_state(), &mut growing),
            Outcome::Rows {
                len: COUNT as usize - 1
            }
        );
        assert_eq!(&growing[..COUNT as usize - 1], &rows[..COUNT as usize - 1]);
        // The last state repeats the first target.
        let last = states + (COUNT as usize - 2) * state::STRIDE;
        world.u32(last + state::TARGET_COUNTRY, 2);
        assert_eq!(
            run(&world, &mut rows),
            Outcome::Unavailable {
                reason: "duplicate cryptology target"
            }
        );
    }

    #[test]
    fn native_values_alias_owner_zero_progress_and_live_count_are_preserved() {
        let bytes = fixture();
        let mut rows = [Row::default(); 3];
        assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: 2 });
        assert_eq!(
            rows[0],
            Row {
                owner_country: 4,
                target_country: 2,
                amount_raw: 0,
                bonus_days: -1,
                bonus_active: 7,
                decrypting: 255,
            }
        );
        assert_eq!(rows[1].owner_country, 4);
        assert_eq!(rows[1].target_country, 3);
        assert_eq!(rows[1].amount_raw, 0x1_2345_6789);
    }

    #[test]
    fn absent_manager_and_empty_array_are_measured_empty() {
        let original = fixture();
        let mut rows = [Row::default(); 2];
        let mut bytes = original.clone();
        p64(&mut bytes, AGENCY + agency::CRYPTOLOGY, 0);
        assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: 0 });
        let mut bytes = original.clone();
        p64(&mut bytes, COUNTRY[0] + ccountry::INTELLIGENCE_AGENCY, 0);
        assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: 0 });
        let mut bytes = original;
        header(&mut bytes, CRYPTO + DECRYPTION_STATES, 0, 0, 0);
        assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: 0 });
    }

    #[test]
    fn corrupt_headers_objects_and_country_links_are_unavailable() {
        let original = fixture();
        let mut rows = [Row::default(); 3];
        for (at, value) in [
            (CRYPTO + DECRYPTION_STATE_COUNT, 4),
            (CRYPTO + DECRYPTION_STATE_COUNT, u32::MAX),
            (CRYPTO + DECRYPTION_STATE_CAPACITY, u32::MAX),
            (STATES + state::TARGET_COUNTRY, 0),
            (STATES + state::TARGET_COUNTRY, 8),
        ] {
            let mut bytes = original.clone();
            p32(&mut bytes, at, value);
            assert!(matches!(
                run(&bytes, &mut rows),
                Outcome::Unavailable { .. }
            ));
        }
        for (at, value) in [
            (AGENCY, 1),
            (CRYPTO, 1),
            (STATES, 1),
            (AGENCY + agency::OWNER_COUNTRY, COUNTRY[1] as u64),
            (CRYPTO + OWNER_COUNTRY, COUNTRY[1] as u64),
            (CRYPTO + DECRYPTION_STATES, u64::MAX - 4),
        ] {
            let mut bytes = original.clone();
            p64(&mut bytes, at, value);
            assert!(matches!(
                run(&bytes, &mut rows),
                Outcome::Unavailable { .. }
            ));
        }
    }

    #[test]
    fn repeated_target_aliases_fail_and_capacity_reports_omitted_rows() {
        let original = fixture();
        let mut rows = [Row::default(); 2];
        let mut bytes = original.clone();
        p32(&mut bytes, STATES + state::TARGET_COUNTRY, 1);
        p32(
            &mut bytes,
            STATES + state::STRIDE + state::TARGET_COUNTRY,
            4,
        );
        assert_eq!(
            run(&bytes, &mut rows),
            Outcome::Unavailable {
                reason: "duplicate cryptology target"
            }
        );
        assert_eq!(
            run(&original, &mut rows[..1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(rows[0].owner_country, 4);
        assert_eq!(
            run(&original, &mut []),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
    }

    #[test]
    fn unreadable_live_row_is_not_silently_omitted() {
        let bytes = fixture();
        assert!(matches!(
            run(
                &bytes[..STATES + state::STRIDE + 31],
                &mut [Row::default(); 2]
            ),
            Outcome::Unavailable {
                reason: "cryptology state unreadable"
            }
        ));
    }
}
