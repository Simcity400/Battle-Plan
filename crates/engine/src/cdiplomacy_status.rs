//! CDiplomacyStatus, reached through CCountry's diplomacy pointer.
//! These fields do not establish hourly consistency.

use crate::{Memory, ccountry, read, walk::Outcome};

/// CDiplomacyStatus primary vtable.
pub const VTABLE_RVA: usize = 0x0294_b1f8;
/// CDiplomacyStatus constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00d1_e540;
/// CDiplomacyStatus CRelationStatus pointer matrix, indexed by resolved country ordinal.
pub const RELATIONS: usize = 0x08;
/// Relation-matrix signed-i32 capacity in its array header.
pub const RELATION_CAPACITY: usize = 0x10;
/// Relation-matrix signed-i32 count, sized from country ordinals
/// without deduplicating country rows.
pub const RELATION_COUNT: usize = 0x14;
/// CDiplomacyStatus owned CRelation pointer vector.
pub const OWNED_RELATIONS: usize = 0x20;
/// CDiplomacyStatus owned-vector signed-i32 capacity.
pub const OWNED_RELATION_CAPACITY: usize = 0x28;
/// CDiplomacyStatus owned-vector signed-i32 count.
pub const OWNED_RELATION_COUNT: usize = 0x2c;
/// Owned war-goal pointer array.
pub const OWNED_WARGOALS: usize = 0x68;
/// Enemy raw-country-tag array, count +0xA4. Transport interdiction
/// uses it; wartime rebuild fills it from opposing per-war lists. Reset
/// clears it. It is not serialized.
pub const ENEMY_COUNTRIES: usize = 0x98;
/// Enemy-country signed count. Nonzero establishes war state before
/// scanning relations in the ordinary mode.
pub const ENEMY_COUNTRY_COUNT: usize = ENEMY_COUNTRIES + 0xc;
/// Wartime-ally raw-tag array, count +0xBC. Rebuild adds nonenemy friendly
/// countries and marks their relation status, then removes canonical slots
/// that later wars list as enemies. It is not serialized.
pub const WARTIME_ALLIES: usize = 0xb0;
/// Ordered raw subject references used by faction descriptions.
pub const SUBJECTS: usize = 0x170;
/// Overlord country word, read as i32.
pub const OVERLORD: usize = 0x188;
/// CDiplomacyStatus signed i64 legitimacy /100000.
pub const LEGITIMACY: usize = 0x1b0;
/// CDiplomacyStatus signed i32 exile army leader count.
pub const EXILE_ARMY_LEADERS: usize = 0x1b8;
/// CDiplomacyStatus owning CCountry pointer.
pub const COUNTRY: usize = 0x288;
/// CDiplomacyStatus nullable CFaction pointer.
pub const FACTION: usize = 0x290;
/// Raw last-surrender hour, read as i32.
pub const LAST_SURRENDER_HOUR: usize = 0x2b8;
/// Raw faction-join hour, read as i32.
pub const FACTION_JOIN_HOUR: usize = 0x2d0;
/// Capitulated byte, accepting exactly 0 or 1.
pub const CAPITULATED: usize = 0x2e0;
/// Raw capitulation hour, read as i32.
pub const CAPITULATED_HOUR: usize = 0x2f0;
/// CDiplomacyStatus vector at +0x300, distinct from CRelationStatus opinion.
pub const AUXILIARY_VECTOR: usize = 0x300;
/// Diplomacy +0x300 vector count; element semantics remain unproven.
pub const AUXILIARY_VECTOR_COUNT: usize = 0x30c;
/// CDiplomacyStatus CCurrentAutonomyStatus pointer.
pub const CURRENT_AUTONOMY_STATUS: usize = 0x350;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CapitulationRow {
    pub country: u32,
    pub capitulated: u8,
    pub capitulated_hour: i32,
    pub last_surrender_hour: i32,
    pub faction_join_hour: i32,
    pub legitimacy_raw: i64,
    pub exile_army_leaders: i32,
}

/// Reads every canonical country's native diplomacy scalars. Nullable save dates
/// retain their engine sentinel, and byte flags retain their representation.
/// A missing diplomacy root is unavailable, never a row filled with zeroes.
pub fn walk_capitulation(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [CapitulationRow],
) -> Outcome {
    walk_capitulation_buffer(memory, base, game_state, crate::walk::Buffer::Fixed(output))
}

pub fn walk_capitulation_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<CapitulationRow>,
) -> Outcome {
    walk_capitulation_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
    )
}

pub fn walk_capitulation_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, CapitulationRow>,
) -> Outcome {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        output.ensure(countries.count() as usize)?;
        let mut len = 0;
        let mut unread_rows = 0_u32;
        let mut found_country = false;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            if len == output.len() {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("capitulation unread row count overflow")?;
                continue;
            }
            output[len] = read_capitulation(memory, base, country)?;
            len += 1;
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

fn read_capitulation(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
) -> Result<CapitulationRow, &'static str> {
    let diplomacy = country
        .pointer
        .checked_add(ccountry::DIPLOMACY_STATUS)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("capitulation diplomacy root unreadable or null")?;
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, diplomacy) != Some(expected as u64) {
        return Err("capitulation diplomacy vtable mismatch");
    }
    let at = |offset| {
        diplomacy
            .checked_add(offset)
            .ok_or("capitulation address overflow")
    };
    let mut backlink = [0; 8];
    let mut legitimacy_and_leaders = [0; 12];
    let mut surrender = [0; 4];
    let mut faction_join = [0; 4];
    let mut capitulated = [0; 1];
    let mut capitulation = [0; 4];
    if memory.copy_scattered(
        [
            at(COUNTRY)?,
            at(LEGITIMACY)?,
            at(LAST_SURRENDER_HOUR)?,
            at(FACTION_JOIN_HOUR)?,
            at(CAPITULATED)?,
            at(CAPITULATED_HOUR)?,
        ],
        &mut [
            &mut backlink,
            &mut legitimacy_and_leaders,
            &mut surrender,
            &mut faction_join,
            &mut capitulated,
            &mut capitulation,
        ],
    ) != 6
    {
        return Err("capitulation field unreadable");
    }
    if u64::from_le_bytes(backlink) != country.pointer as u64 {
        return Err("capitulation diplomacy country backlink mismatch");
    }
    Ok(CapitulationRow {
        country: country.id,
        capitulated: capitulated[0],
        capitulated_hour: i32::from_le_bytes(capitulation),
        last_surrender_hour: i32::from_le_bytes(surrender),
        faction_join_hour: i32::from_le_bytes(faction_join),
        legitimacy_raw: i64::from_le_bytes(legitimacy_and_leaders[..8].try_into().unwrap()),
        exile_army_leaders: i32::from_le_bytes(legitimacy_and_leaders[8..].try_into().unwrap()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cgame_date, cgame_state, memory::SliceMemory};

    const BASE: usize = 0x0001_4000_0000;
    const GAME: usize = 0x1000;
    const FIRST: usize = 0x4000;
    const SECOND: usize = 0x6000;
    const FIRST_DIPLOMACY: usize = 0x8000;
    const SECOND_DIPLOMACY: usize = 0x9000;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address..address + value.len()].copy_from_slice(value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0xa000];
        for (offset, data, count) in [
            (cgame_state::COUNTRIES, 0x2000_u64, 4_u32),
            (cgame_state::COUNTRY_LINKS, 0x2100, 4),
        ] {
            put(&mut bytes, GAME + offset, &data.to_le_bytes());
            put(&mut bytes, GAME + offset + 8, &count.to_le_bytes());
            put(&mut bytes, GAME + offset + 12, &count.to_le_bytes());
        }
        for (index, pointer) in [0_u64, FIRST as u64, SECOND as u64, FIRST as u64]
            .into_iter()
            .enumerate()
        {
            put(&mut bytes, 0x2000 + index * 8, &pointer.to_le_bytes());
        }
        for (index, canonical) in [0_u32, 1, 2, 1].into_iter().enumerate() {
            put(&mut bytes, 0x2100 + index * 4, &canonical.to_le_bytes());
        }
        for (pointer, diplomacy, id) in [
            (FIRST, FIRST_DIPLOMACY, 3_u32),
            (SECOND, SECOND_DIPLOMACY, 2_u32),
        ] {
            put(
                &mut bytes,
                pointer,
                &((BASE + ccountry::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(&mut bytes, pointer + ccountry::ID, &id.to_le_bytes());
            put(
                &mut bytes,
                pointer + ccountry::DIPLOMACY_STATUS,
                &(diplomacy as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                diplomacy,
                &((BASE + VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                diplomacy + COUNTRY,
                &(pointer as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                diplomacy + LAST_SURRENDER_HOUR,
                &cgame_date::NULL_RAW_HOUR.to_le_bytes(),
            );
        }
        bytes
    }

    fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    #[test]
    fn growing_capitulation_captures_all_5000_countries_from_empty_rows() {
        use crate::ccountry::synthetic::World;
        const COUNT: u32 = 5_000;
        let mut world = World::new(BASE, COUNT, ccountry::DIPLOMACY_STATUS + 8);
        for slot in 1..=COUNT {
            let country = world.object(slot);
            let diplomacy = world.alloc(0x400);
            world.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
            world.u64(diplomacy, (BASE + VTABLE_RVA) as u64);
            world.u64(diplomacy + COUNTRY, country as u64);
            world.u64(diplomacy + LEGITIMACY, u64::from(slot));
            world.u32(diplomacy + EXILE_ARMY_LEADERS, slot);
        }
        let mut expected = vec![CapitulationRow::default(); COUNT as usize];
        let outcome = walk_capitulation(&world.memory(), BASE, world.game_state(), &mut expected);
        assert_eq!(
            outcome,
            Outcome::Rows {
                len: COUNT as usize
            }
        );
        let mut rows = Vec::new();
        assert_eq!(
            walk_capitulation_growing(&world.memory(), BASE, world.game_state(), &mut rows),
            outcome
        );
        assert_eq!(&rows[..COUNT as usize], expected.as_slice());
        assert_eq!(rows[COUNT as usize - 1].country, COUNT);
    }

    #[test]
    fn native_precision_flags_dates_and_country_alias_are_preserved() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            FIRST_DIPLOMACY + LEGITIMACY,
            &0x1_0000_0001_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            FIRST_DIPLOMACY + EXILE_ARMY_LEADERS,
            &(-7_i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            FIRST_DIPLOMACY + CAPITULATED_HOUR,
            &i32::MIN.to_le_bytes(),
        );
        put(
            &mut bytes,
            FIRST_DIPLOMACY + FACTION_JOIN_HOUR,
            &i32::MAX.to_le_bytes(),
        );
        bytes[FIRST_DIPLOMACY + CAPITULATED] = 0x80;
        let mut rows = [CapitulationRow::default(); 4];
        assert_eq!(
            walk_capitulation(&memory(&bytes), BASE, GAME, &mut rows),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            rows[0],
            CapitulationRow {
                country: 3,
                capitulated: 0x80,
                capitulated_hour: i32::MIN,
                last_surrender_hour: cgame_date::NULL_RAW_HOUR,
                faction_join_hour: i32::MAX,
                legitimacy_raw: 0x1_0000_0001,
                exile_army_leaders: -7,
            }
        );
        assert_eq!(rows[1].country, 2);
    }

    #[test]
    fn capacity_counts_canonical_rows_without_reading_omitted_fields() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            SECOND + ccountry::DIPLOMACY_STATUS,
            &0_u64.to_le_bytes(),
        );
        let mut rows = [CapitulationRow::default(); 1];
        assert_eq!(
            walk_capitulation(&memory(&bytes), BASE, GAME, &mut rows),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(rows[0].country, 3);
        assert_eq!(
            walk_capitulation(&memory(&bytes), BASE, GAME, &mut []),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
    }

    #[test]
    fn missing_roots_backlink_and_late_field_faults_are_unavailable() {
        let valid = fixture();
        let mut rows = [CapitulationRow::default(); 2];
        for address in [
            SECOND + ccountry::DIPLOMACY_STATUS,
            SECOND_DIPLOMACY,
            SECOND_DIPLOMACY + COUNTRY,
        ] {
            let mut broken = valid.clone();
            put(&mut broken, address, &0_u64.to_le_bytes());
            assert!(matches!(
                walk_capitulation(&memory(&broken), BASE, GAME, &mut rows),
                Outcome::Unavailable { .. }
            ));
        }
        let broken = &valid[..SECOND_DIPLOMACY + CAPITULATED_HOUR + 3];
        assert!(matches!(
            walk_capitulation(&memory(broken), BASE, GAME, &mut rows),
            Outcome::Unavailable {
                reason: "capitulation field unreadable"
            }
        ));
    }
}
