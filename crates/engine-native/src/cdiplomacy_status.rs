//! CDiplomacyStatus, reached through CCountry's diplomacy pointer.
//! These fields do not establish hourly consistency.

use crate::{Memory, ccountry, cpdx_array::Array, cwar_goal, read, walk::Outcome};

/// CDiplomacyStatus primary vtable.
pub const VTABLE_RVA: usize = 0x029d_e5e8;
/// CDiplomacyStatus constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00d9_6000;
/// CDiplomacyStatus CRelationStatus pointer matrix, indexed by resolved country ordinal.
pub const RELATIONS: usize = 0x08;
/// Relation-matrix signed-i32 capacity in its array header.
#[cfg(test)]
pub const RELATION_CAPACITY: usize = 0x10;
/// Relation-matrix signed-i32 count, sized from country ordinals
/// without deduplicating country rows.
#[cfg(test)]
pub const RELATION_COUNT: usize = 0x14;
/// CDiplomacyStatus owned CRelation pointer vector.
pub const OWNED_RELATIONS: usize = 0x20;
/// CDiplomacyStatus owned-vector signed-i32 capacity.
#[cfg(test)]
pub const OWNED_RELATION_CAPACITY: usize = 0x28;
/// CDiplomacyStatus owned-vector signed-i32 count.
#[cfg(test)]
pub const OWNED_RELATION_COUNT: usize = 0x2c;
/// Owned war-goal pointer array: the goals the country holds and has not yet
/// used in a war. The diplomacy map mode tests the selected country's goals'
/// recipients here.
pub const OWNED_WARGOALS: usize = 0x68;
/// Enemy raw-country-tag array, its count follows the capacity. Transport interdiction uses it;
/// wartime rebuild fills it from opposing per-war lists. Reset clears it. It is not serialized.
pub const ENEMY_COUNTRIES: usize = 0x98;
/// Enemy-country signed count. Nonzero establishes war state before
/// scanning relations in the ordinary mode.
pub const ENEMY_COUNTRY_COUNT: usize = ENEMY_COUNTRIES + 0xc;
/// Wartime-ally raw-tag array, its count follows the capacity. Rebuild adds nonenemy friendly
/// countries and marks their relation status, then removes canonical slots that later wars list as
/// enemies. It is not serialized.
pub const WARTIME_ALLIES: usize = 0xb0;
/// Ordered raw subject references used by faction descriptions.
pub const SUBJECTS: usize = 0x170;
/// Overlord country word, read as i32.
pub const OVERLORD: usize = 0x188;
/// Diplomacy status's owning country pointer, stored by its constructor.
pub const COUNTRY: usize = 0x2d0;
/// CDiplomacyStatus nullable CFaction pointer.
pub const FACTION: usize = 0x2d8;
/// CDiplomacyStatus capitulated byte, D60 accepts exactly 0 or 1; build_profile.rs:757.
pub const CAPITULATED: usize = 0x328;
/// Raw capitulation hour, read as i32.
pub const CAPITULATED_HOUR: usize = 0x338;
/// CDiplomacyStatus CCurrentAutonomyStatus pointer.
pub const CURRENT_AUTONOMY_STATUS: usize = 0x3b8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CapitulationRow {
    pub country: u32,
    pub capitulated: u8,
    pub capitulated_hour: i32,
    /// The owned war goals' recipients ([`cwar_goal::RECIPIENT`]), in vector
    /// order, at `recipients[war_goals_start..war_goals_start + len]`;
    /// `None` when a goal could not be read, which refuses only this list.
    pub war_goals: Option<u32>,
    pub war_goals_start: usize,
}

/// Reads every canonical country's native diplomacy scalars and its owned
/// war goals' recipients into `recipients`. Nullable save dates retain their
/// engine sentinel, and byte flags retain their representation. A missing
/// diplomacy root is unavailable, never a row filled with zeroes; a country
/// whose recipients do not fit a fixed `recipients` is an unread row.
pub fn walk_capitulation(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [CapitulationRow],
    recipients: &mut [u32],
) -> Outcome {
    walk_capitulation_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(recipients),
    )
}

pub fn walk_capitulation_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<CapitulationRow>,
    recipients: &mut Vec<u32>,
) -> Outcome {
    walk_capitulation_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(recipients),
    )
}

pub fn walk_capitulation_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, CapitulationRow>,
    mut recipients: crate::walk::Buffer<'_, u32>,
) -> Outcome {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        output.ensure(countries.count() as usize)?;
        let mut len = 0;
        let mut used = 0_usize;
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
            let (mut row, diplomacy) = read_capitulation(memory, base, country)?;
            match war_goals(memory, base, diplomacy, &mut recipients, used)? {
                Some(goals) => {
                    row.war_goals_start = used;
                    row.war_goals = goals;
                    used += goals.unwrap_or(0) as usize;
                }
                None => {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("capitulation unread row count overflow")?;
                    continue;
                }
            }
            output[len] = row;
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

/// A diplomacy status's owned war goals' recipients into `recipients[used..]`.
/// `Ok(None)` when a fixed buffer is too short; `Ok(Some(None))` when a goal
/// does not read, which refuses only this country's list.
fn war_goals(
    memory: &impl Memory,
    base: usize,
    diplomacy: usize,
    recipients: &mut crate::walk::Buffer<'_, u32>,
    used: usize,
) -> Result<Option<Option<u32>>, &'static str> {
    let Some(goals) = diplomacy
        .checked_add(OWNED_WARGOALS)
        .and_then(|at| Array::read(memory, at, 8))
    else {
        return Ok(Some(None));
    };
    let count = goals.count as usize;
    let end = used
        .checked_add(count)
        .ok_or("war goal recipient count overflow")?;
    if !recipients.ensure(end)? {
        return Ok(None);
    }
    let vtable = base
        .checked_add(cwar_goal::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    for ordinal in 0..goals.count {
        let recipient = goals
            .element(ordinal, 8)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|goal| usize::try_from(goal).ok())
            .filter(|&goal| goal != 0 && read::<u64>(memory, goal) == Some(vtable))
            .and_then(|goal| goal.checked_add(cwar_goal::RECIPIENT))
            .and_then(|at| read::<u32>(memory, at));
        let Some(recipient) = recipient else {
            return Ok(Some(None));
        };
        recipients[used + ordinal as usize] = recipient;
    }
    Ok(Some(Some(goals.count)))
}

fn read_capitulation(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
) -> Result<(CapitulationRow, usize), &'static str> {
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
    // The retired last surrender, faction join, legitimacy and exile leader count are not read, so
    // they never refuse the row.
    let mut backlink = [0; 8];
    let mut capitulated = [0; 1];
    let mut capitulation = [0; 4];
    if memory.copy_scattered(
        [at(COUNTRY)?, at(CAPITULATED)?, at(CAPITULATED_HOUR)?],
        &mut [&mut backlink, &mut capitulated, &mut capitulation],
    ) != 3
    {
        return Err("capitulation field unreadable");
    }
    if u64::from_le_bytes(backlink) != country.pointer as u64 {
        return Err("capitulation diplomacy country backlink mismatch");
    }
    Ok((
        CapitulationRow {
            country: country.id,
            capitulated: capitulated[0],
            capitulated_hour: i32::from_le_bytes(capitulation),
            ..CapitulationRow::default()
        },
        diplomacy,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cgame_state, memory::SliceMemory};

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
        }
        bytes
    }

    fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    #[test]
    fn native_precision_flags_dates_and_country_alias_are_preserved() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            FIRST_DIPLOMACY + CAPITULATED_HOUR,
            &i32::MIN.to_le_bytes(),
        );
        bytes[FIRST_DIPLOMACY + CAPITULATED] = 0x80;
        let mut rows = [CapitulationRow::default(); 4];
        assert_eq!(
            walk_capitulation(&memory(&bytes), BASE, GAME, &mut rows, &mut []),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            rows[0],
            CapitulationRow {
                country: 3,
                capitulated: 0x80,
                capitulated_hour: i32::MIN,
                war_goals: Some(0),
                war_goals_start: 0,
            }
        );
        assert_eq!(rows[1].country, 2);
    }

    #[test]
    fn unreadable_retired_fields_never_refuse_the_row() {
        // Legitimacy and the exile leader count (12 bytes), the last surrender and the faction
        // join.
        struct Holes<'a>(SliceMemory<'a>);
        impl Memory for Holes<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let end = address + output.len();
                ![(0x1f8, 12), (0x300, 4), (0x318, 4)]
                    .iter()
                    .any(|&(offset, len)| {
                        let start = FIRST_DIPLOMACY + offset;
                        address < start + len && start < end
                    })
                    && self.0.copy(address, output)
            }
        }
        let bytes = fixture();
        let mut rows = [CapitulationRow::default(); 2];
        assert_eq!(
            walk_capitulation(&Holes(memory(&bytes)), BASE, GAME, &mut rows, &mut []),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].country, 3);
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
            walk_capitulation(&memory(&bytes), BASE, GAME, &mut rows, &mut []),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(rows[0].country, 3);
        assert_eq!(
            walk_capitulation(&memory(&bytes), BASE, GAME, &mut [], &mut []),
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
                walk_capitulation(&memory(&broken), BASE, GAME, &mut rows, &mut []),
                Outcome::Unavailable { .. }
            ));
        }
        let broken = &valid[..SECOND_DIPLOMACY + CAPITULATED_HOUR + 3];
        assert!(matches!(
            walk_capitulation(&memory(broken), BASE, GAME, &mut rows, &mut []),
            Outcome::Unavailable {
                reason: "capitulation field unreadable"
            }
        ));
    }

    /// Each country's owned war goals' recipients, in vector order; a goal
    /// that does not authenticate refuses only its country's list, and a
    /// fixed recipient buffer that is too short leaves the row unread. The
    /// old row held no goals.
    #[test]
    fn owned_war_goal_recipients_follow_the_vector() {
        let mut bytes = fixture();
        bytes.resize(0xb000, 0);
        let goals = 0xa000;
        put(
            &mut bytes,
            FIRST_DIPLOMACY + OWNED_WARGOALS,
            &(goals as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            FIRST_DIPLOMACY + OWNED_WARGOALS + 8,
            &2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            FIRST_DIPLOMACY + OWNED_WARGOALS + 12,
            &2_u32.to_le_bytes(),
        );
        for (index, (goal, recipient)) in
            [(0xa100_usize, 7_u32), (0xa200, 9)].into_iter().enumerate()
        {
            put(&mut bytes, goals + index * 8, &(goal as u64).to_le_bytes());
            put(
                &mut bytes,
                goal,
                &((BASE + cwar_goal::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                goal + cwar_goal::RECIPIENT,
                &recipient.to_le_bytes(),
            );
        }
        let mut rows = [CapitulationRow::default(); 2];
        let mut recipients = [0_u32; 2];
        assert_eq!(
            walk_capitulation(&memory(&bytes), BASE, GAME, &mut rows, &mut recipients),
            Outcome::Rows { len: 2 }
        );
        assert_eq!((rows[0].war_goals, rows[0].war_goals_start), (Some(2), 0));
        assert_eq!(recipients, [7, 9]);
        assert_eq!((rows[1].war_goals, rows[1].war_goals_start), (Some(0), 2));
        // Too short a fixed buffer leaves the country unread.
        assert_eq!(
            walk_capitulation(&memory(&bytes), BASE, GAME, &mut rows, &mut [0; 1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        // A goal that is not a war goal refuses only that country's list.
        put(&mut bytes, 0xa200, &0_u64.to_le_bytes());
        assert_eq!(
            walk_capitulation(&memory(&bytes), BASE, GAME, &mut rows, &mut recipients),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].war_goals, None);
        assert_eq!(rows[1].war_goals, Some(0));
    }
}
