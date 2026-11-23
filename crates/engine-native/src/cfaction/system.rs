use engine_native_parts::cdiplomacy_status_layout as cdiplomacy_status;

use engine_sites::Rva;
use crate::{
    Memory, ccountry,  cfaction, cgame_state, cpdx_array::Array, read,
    walk::Outcome,
};

pub const VTABLE_RVA: Rva = Rva(0x029e_2fd0);

pub const FACTIONS: usize = 0x20;
pub const EXTRACTED_RESOURCES: usize = 0x38;
#[cfg(test)]
pub const FACTION_COUNT: usize = 0x2c;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MemberRow {
    pub country: u32,
    pub faction: u64,
    pub ordinal: u32,
}

pub fn roster(memory: &impl Memory, base: usize, game_state: usize) -> Result<Array, &'static str> {
    let pointer = game_state
        .checked_add(cgame_state::FACTION_SYSTEM)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("faction system unreadable or null")?;
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("faction system vtable mismatch");
    }
    pointer
        .checked_add(FACTIONS)
        .and_then(|p| Array::read(memory, p, 8))
        .ok_or("faction roster unreadable or invalid")
}

pub fn accounts(memory: &impl Memory, base: usize, state: usize) -> Result<Array, &'static str> {
    let system = state
        .checked_add(cgame_state::FACTION_SYSTEM)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value != 0)
        .ok_or("faction account system unavailable")?;
    let vtable =
        engine_sites::at(base, VTABLE_RVA).ok_or("faction account system vtable mismatch")?;
    if read::<u64>(memory, system) != Some(vtable as u64) {
        return Err("faction account system vtable mismatch");
    }
    system
        .checked_add(crate::cfaction_member_status::ACCOUNTS)
        .and_then(|at| Array::read(memory, at, crate::cfaction_member_status::STRIDE))
        .ok_or("faction account array invalid")
}

fn pointer(memory: &impl Memory, array: Array, index: u32) -> Result<usize, &'static str> {
    array
        .element(index, 8)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("faction roster pointer unreadable or null")
}

pub fn faction(
    memory: &impl Memory,
    base: usize,
    roster: Array,
    index: u32,
) -> Result<(usize, u64), &'static str> {
    let address = pointer(memory, roster, index)?;
    let id = cfaction::identity(memory, base, address)?;
    for previous in 0..index {
        let other = pointer(memory, roster, previous)?;
        let other_id = other
            .checked_add(cfaction::REFERENCE)
            .and_then(|p| read::<u64>(memory, p))
            .ok_or("faction handle unreadable")?;
        if other == address || other_id == id {
            return Err("duplicate faction pointer or handle");
        }
    }
    Ok((address, id))
}

pub fn count_members(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_members_buffer(memory, base, game_state, crate::walk::Buffer::Count)
}

pub fn walk_members(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<MemberRow>,
) -> Outcome {
    walk_members_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
    )
}

fn walk_members_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, MemberRow>,
) -> Outcome {
    let result = (|| {
        let roster = roster(memory, base, game_state)?;
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        for index in 0..roster.count {
            let (address, id) = faction(memory, base, roster, index)?;
            let members = address
                .checked_add(cfaction::MEMBERS)
                .and_then(|p| Array::read(memory, p, 8))
                .ok_or("faction members unreadable or invalid")?;
            output.ensure(
                len.checked_add(members.count as usize)
                    .ok_or("faction member count overflow")?,
            )?;
            for ordinal in 0..members.count {
                let member = members
                    .element(ordinal, 8)
                    .and_then(|p| read::<u64>(memory, p))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .ok_or("faction member pointer unreadable or null")?;
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("faction unread member count overflow")?;
                    continue;
                }
                let country_id = member
                    .checked_add(ccountry::ID)
                    .and_then(|p| read::<u32>(memory, p))
                    .ok_or("faction member country id unreadable")?;
                let country = if country_id == 0 {
                    ccountry::read_reserved_root(memory, base, game_state)?
                } else {
                    countries.resolve(country_id)?
                };
                if country.pointer != member {
                    return Err("faction member differs from canonical country object");
                }
                if country_faction(memory, base, member)? != address {
                    return Err("country faction differs from membership roster");
                }
                if output[..len].iter().any(|row| row.country == country.id) {
                    return Err("country has duplicate faction membership");
                }
                output[len] = MemberRow {
                    country: country.id,
                    faction: id,
                    ordinal,
                };
                len += 1;
            }
        }
        if unread_rows != 0 {
            return Ok(Outcome::Truncated { len, unread_rows });
        }
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let address = country_faction(memory, base, country.pointer)?;
            let member = output[..len].iter().find(|row| row.country == country.id);
            match (address, member) {
                (0, None) => {}
                (0, Some(_)) => return Err("faction member country has no faction"),
                (_, None) => return Err("country faction is missing from member roster"),
                (_, Some(member)) => {
                    if cfaction::identity(memory, base, address)? != member.faction {
                        return Err("country faction handle differs from member roster");
                    }
                }
            }
        }
        Ok(Outcome::Rows { len })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn country_faction(
    memory: &impl Memory,
    base: usize,
    country: usize,
) -> Result<usize, &'static str> {
    let diplomacy = country
        .checked_add(ccountry::DIPLOMACY_STATUS)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("country diplomacy unreadable or null")?;
    let expected = engine_sites::at(base, cdiplomacy_status::VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, diplomacy) != Some(expected as u64) {
        return Err("country diplomacy vtable mismatch");
    }
    if diplomacy
        .checked_add(cdiplomacy_status::COUNTRY)
        .and_then(|p| read::<u64>(memory, p))
        != Some(country as u64)
    {
        return Err("country diplomacy owner differs from country");
    }
    diplomacy
        .checked_add(cdiplomacy_status::FACTION)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("country faction pointer unreadable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{memory::SliceMemory, msvc_string};

    const GS: usize = 0x1000;
    const COUNTRIES: usize = 0x2000;
    const LINKS: usize = 0x2100;
    const SYSTEM: usize = 0x2200;
    const FACTION_POINTERS: usize = 0x2300;
    const FIRST: usize = 0x3000;
    const SECOND: usize = 0xb000;
    const TEMPLATE_OBJECT: usize = 0xbc00;
    const TEMPLATE_TOKEN: u32 = 0x4c1a_0042;
    const FIRST_COLOR: [f32; 4] = [81.0 / 255.0, 78.0 / 255.0, 74.0 / 255.0, 1.0];
    const HEAP_NAME: usize = 0x4000;
    const MEMBER_POINTERS: usize = 0x4300;
    const C1: usize = 0x5000;
    const C2: usize = 0x7000;
    const D1: usize = 0x9000;
    const D2: usize = 0xa000;
    const FIRST_ID: u64 = (4_u64 << 32) | 88;
    const SECOND_ID: u64 = 1_u64 << 32;

    fn put32(bytes: &mut [u8], address: usize, value: u32) {
        bytes[address..address + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put64(bytes: &mut [u8], address: usize, value: u64) {
        bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn array(bytes: &mut [u8], address: usize, data: usize, capacity: u32, count: u32) {
        put64(bytes, address, data as u64);
        put32(bytes, address + 8, capacity);
        put32(bytes, address + 12, count);
    }

    fn name(bytes: &mut [u8], faction: usize, value: &[u8]) {
        let string = faction + cfaction::NAME;
        let source = if value.len() <= 15 {
            put64(bytes, string + msvc_string::CAPACITY, 15);
            string
        } else {
            put64(bytes, string, HEAP_NAME as u64);
            put64(bytes, string + msvc_string::CAPACITY, value.len() as u64);
            HEAP_NAME
        };
        put64(bytes, string + msvc_string::LENGTH, value.len() as u64);
        bytes[source..source + value.len()].copy_from_slice(value);
        bytes[source + value.len()] = 0;
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0xc000];
        array(&mut bytes, GS + cgame_state::COUNTRIES, COUNTRIES, 4, 4);
        array(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS, 8, 8);
        for (slot, pointer) in [u64::MAX, C1 as u64, C2 as u64, C1 as u64]
            .into_iter()
            .enumerate()
        {
            put64(&mut bytes, COUNTRIES + slot * 8, pointer);
        }
        for (raw, canonical) in [(1, 1), (2, 2), (3, 1), (7, 1)] {
            put32(&mut bytes, LINKS + raw * 4, canonical);
        }
        for (country, id, diplomacy) in [(C1, 7, D1), (C2, 2, D2)] {
            put64(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
            put32(&mut bytes, country + ccountry::ID, id);
            put64(
                &mut bytes,
                country + ccountry::DIPLOMACY_STATUS,
                diplomacy as u64,
            );
            put64(&mut bytes, diplomacy, cdiplomacy_status::VTABLE_RVA.0 as u64);
            put64(
                &mut bytes,
                diplomacy + cdiplomacy_status::COUNTRY,
                country as u64,
            );
            put64(
                &mut bytes,
                diplomacy + cdiplomacy_status::FACTION,
                FIRST as u64,
            );
        }
        put64(&mut bytes, GS + cgame_state::FACTION_SYSTEM, SYSTEM as u64);
        put64(&mut bytes, SYSTEM, VTABLE_RVA.0 as u64);
        array(&mut bytes, SYSTEM + FACTIONS, FACTION_POINTERS, 4, 2);
        put64(&mut bytes, FACTION_POINTERS, FIRST as u64);
        put64(&mut bytes, FACTION_POINTERS + 8, SECOND as u64);
        put64(&mut bytes, FACTION_POINTERS + 16, u64::MAX);
        for (faction, id, color, overridden) in [
            (FIRST, FIRST_ID, FIRST_COLOR, 0),
            (SECOND, SECOND_ID, [1.0; 4], 1),
        ] {
            put64(&mut bytes, faction, cfaction::VTABLE_RVA.0 as u64);
            put64(&mut bytes, faction + cfaction::REFERENCE, id);
            put64(
                &mut bytes,
                faction + cfaction::COLOR,
                cfaction::COLOR_VTABLE_RVA.0 as u64,
            );
            for (index, component) in color.into_iter().enumerate() {
                put32(
                    &mut bytes,
                    faction + cfaction::COLOR + cfaction::COLOR_COMPONENTS + index * 4,
                    component.to_bits(),
                );
            }
            bytes[faction + cfaction::COLOR_OVERRIDDEN] = overridden;
        }
        put64(
            &mut bytes,
            FIRST + cfaction::TEMPLATE,
            TEMPLATE_OBJECT as u64,
        );
        put64(
            &mut bytes,
            TEMPLATE_OBJECT,
            crate::cfaction_template::VTABLE_RVA.0 as u64,
        );
        put32(
            &mut bytes,
            TEMPLATE_OBJECT + crate::cfaction_template::IDENTIFIER_TOKEN,
            TEMPLATE_TOKEN,
        );
        name(&mut bytes, FIRST, b"Axis");
        name(&mut bytes, SECOND, b"");
        array(&mut bytes, FIRST + cfaction::MEMBERS, MEMBER_POINTERS, 3, 2);
        array(&mut bytes, SECOND + cfaction::MEMBERS, 0, 0, 0);
        put64(&mut bytes, MEMBER_POINTERS, C2 as u64);
        put64(&mut bytes, MEMBER_POINTERS + 8, C1 as u64);
        put64(&mut bytes, MEMBER_POINTERS + 16, u64::MAX);
        bytes
    }

    fn members(bytes: &[u8], output: &mut Vec<MemberRow>) -> Outcome {
        walk_members(&SliceMemory { base: 0, bytes }, 0, GS, output)
    }

    #[test]
    fn reserved_member_uses_actual_root_without_broadening_positive_resolution() {
        const C0: usize = 0xd000;
        const D0: usize = 0xf000;
        let mut bytes = fixture();
        bytes.resize(0x11000, 0);
        put64(&mut bytes, COUNTRIES, C0 as u64);
        put64(&mut bytes, C0, ccountry::VTABLE_RVA.0 as u64);
        put64(&mut bytes, C0 + ccountry::DIPLOMACY_STATUS, D0 as u64);
        put64(&mut bytes, D0, cdiplomacy_status::VTABLE_RVA.0 as u64);
        put64(&mut bytes, D0 + cdiplomacy_status::COUNTRY, C0 as u64);
        put64(&mut bytes, D0 + cdiplomacy_status::FACTION, FIRST as u64);
        put32(&mut bytes, FIRST + cfaction::MEMBER_COUNT, 3);
        put64(&mut bytes, MEMBER_POINTERS + 16, C0 as u64);
        let mut rows = vec![MemberRow::default(); 3];
        assert_eq!(members(&bytes, &mut rows), Outcome::Rows { len: 3 });
        assert_eq!(
            rows[2],
            MemberRow {
                country: 0,
                faction: FIRST_ID,
                ordinal: 2
            }
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert!(
            ccountry::Reader::new(&memory, 0, GS)
                .unwrap()
                .resolve(0)
                .is_err()
        );
        for (address, value, reason) in [
            (COUNTRIES, C2 as u64, "reserved country id is not zero"),
            (
                D0 + cdiplomacy_status::FACTION,
                0,
                "country faction differs from membership roster",
            ),
            (
                D0 + cdiplomacy_status::COUNTRY,
                C1 as u64,
                "country diplomacy owner differs from country",
            ),
        ] {
            let mut bad = bytes.clone();
            put64(&mut bad, address, value);
            assert_eq!(members(&bad, &mut rows), Outcome::Unavailable { reason });
        }
        put32(&mut bytes, FIRST + cfaction::MEMBER_COUNT, 4);
        put32(&mut bytes, FIRST + cfaction::MEMBER_CAPACITY, 4);
        put64(&mut bytes, MEMBER_POINTERS + 24, C0 as u64);
        assert_eq!(
            members(&bytes, &mut vec![MemberRow::default(); 4]),
            Outcome::Unavailable {
                reason: "country has duplicate faction membership",
            }
        );
    }

    fn names(bytes: &[u8], output: &mut Vec<cfaction::Row>, arena: &mut Vec<u8>) -> cfaction::NameWalk {
        cfaction::walk(&SliceMemory { base: 0, bytes }, 0, GS, output, arena)
    }

    #[test]
    fn exact_names_and_ordered_members_preserve_aliases_empty_factions_and_full_handles() {
        let bytes = fixture();
        let mut rows = vec![cfaction::Row::default(); 2];
        let mut arena = vec![0; 4];
        assert_eq!(
            names(&bytes, &mut rows, &mut arena),
            cfaction::NameWalk {
                outcome: Outcome::Rows { len: 2 },
                bytes_len: 4,
                required_bytes: 4,
            }
        );
        assert_eq!(arena, *b"Axis");
        assert_eq!(
            rows,
            [
                cfaction::Row {
                    id: FIRST_ID,
                    name_start: 0,
                    name_len: 4,
                    template_token: Some(TEMPLATE_TOKEN),
                    color: FIRST_COLOR.map(f32::to_bits),
                    color_overridden: 0,
                },
                cfaction::Row {
                    id: SECOND_ID,
                    name_start: 4,
                    name_len: 0,
                    template_token: None,
                    color: [1.0_f32.to_bits(); 4],
                    color_overridden: 1,
                },
            ]
        );
        let mut output = vec![MemberRow::default(); 2];
        assert_eq!(members(&bytes, &mut output), Outcome::Rows { len: 2 });
        assert_eq!(
            output,
            [
                MemberRow {
                    country: 2,
                    faction: FIRST_ID,
                    ordinal: 0
                },
                MemberRow {
                    country: 7,
                    faction: FIRST_ID,
                    ordinal: 1
                },
            ]
        );
    }

    #[test]
    fn names_grow_by_required_bytes_without_shortening_or_utf8_conversion() {
        let mut bytes = fixture();
        let value: Vec<u8> = (0..=255).collect();
        name(&mut bytes, FIRST, &value);
        put64(&mut bytes, FIRST + cfaction::REFERENCE, 9000);
        let mut rows = vec![cfaction::Row::default(); 2];
        let mut enough = vec![0; 128];
        assert_eq!(
            names(&bytes, &mut rows, &mut enough).outcome,
            Outcome::Rows { len: 2 }
        );
        assert_eq!(&enough[..256], value);
        assert_eq!(rows[0].id, 9000);
        assert_eq!(
            cfaction::count(&SliceMemory { base: 0, bytes: &bytes }, 0, GS),
            cfaction::NameWalk {
                outcome: Outcome::Truncated {
                    len: 0,
                    unread_rows: 2
                },
                bytes_len: 0,
                required_bytes: 256,
            }
        );
    }

    #[test]
    fn malformed_names_are_unavailable_even_when_the_row_buffer_is_full() {
        for (at, value) in [
            (FIRST + cfaction::NAME + msvc_string::LENGTH, 16),
            (FIRST + cfaction::NAME + msvc_string::CAPACITY, 14),
            (FIRST + cfaction::NAME + msvc_string::CAPACITY, 16),
        ] {
            let mut bytes = fixture();
            put64(&mut bytes, at, value);
            assert!(matches!(
                names(&bytes, &mut Vec::new(), &mut Vec::new()).outcome,
                Outcome::Unavailable { .. }
            ));
        }
        let mut bytes = fixture();
        bytes[FIRST + cfaction::NAME + 4] = b'!';
        assert!(matches!(
            names(&bytes, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Unavailable { .. }
        ));
        name(&mut bytes, FIRST, &[b'x'; 256]);
        put64(&mut bytes, FIRST + cfaction::NAME, 0);
        assert!(matches!(
            names(&bytes, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn presentation_is_one_record_copy_per_faction_and_one_span_per_template() {
        let counted = |bytes: &[u8]| {
            crate::memory::ReadCounts::take_current();
            let memory = crate::memory::CountedMemory::new(SliceMemory { base: 0, bytes });
            let walk = cfaction::walk(
                &memory,
                0,
                GS,
                &mut vec![cfaction::Row::default(); 2],
                &mut vec![0; 4],
            );
            assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
            crate::memory::ReadCounts::take_current()
        };
        let templated = counted(&fixture());
        let mut bytes = fixture();
        put64(&mut bytes, FIRST + cfaction::TEMPLATE, 0);
        let plain = counted(&bytes);
        assert_eq!(templated.record_calls, 2);
        assert_eq!(plain.record_calls, 2);
        assert_eq!(templated.copy_calls, plain.copy_calls + 1);
        assert_eq!(
            names(
                &bytes[..SECOND + cfaction::COLOR + 8],
                &mut vec![cfaction::Row::default(); 2],
                &mut vec![0; 4]
            )
            .outcome,
            Outcome::Unavailable {
                reason: "faction presentation unreadable"
            }
        );
    }

    #[test]
    fn a_foreign_template_or_colour_is_unavailable() {
        for at in [TEMPLATE_OBJECT, FIRST + cfaction::COLOR] {
            let mut bytes = fixture();
            put64(&mut bytes, at, 0x1234);
            assert!(matches!(
                names(&bytes, &mut vec![cfaction::Row::default(); 2], &mut vec![0; 4]).outcome,
                Outcome::Unavailable { .. }
            ));
        }
    }

    #[test]
    fn members_still_check_later_pointer_slots() {
        let mut bytes = fixture();
        let mut output = vec![MemberRow::default(); 1];
        put64(&mut bytes, MEMBER_POINTERS + 8, 0);
        assert!(matches!(
            members(&bytes, &mut output),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn null_wrong_class_duplicate_and_incomplete_faction_rosters_are_unavailable() {
        for (at, value) in [
            (GS + cgame_state::FACTION_SYSTEM, 0),
            (SYSTEM, 0),
            (FACTION_POINTERS, 0),
            (FIRST, 0),
            (FIRST + cfaction::REFERENCE, 0),
            (FACTION_POINTERS + 8, FIRST as u64),
            (SECOND + cfaction::REFERENCE, FIRST_ID),
        ] {
            let mut bytes = fixture();
            put64(&mut bytes, at, value);
            assert!(
                matches!(
                    names(&bytes, &mut vec![cfaction::Row::default(); 2], &mut vec![0; 4]).outcome,
                    Outcome::Unavailable { .. }
                ),
                "name case {at:x}"
            );
            assert!(
                matches!(
                    members(&bytes, &mut vec![MemberRow::default(); 2]),
                    Outcome::Unavailable { .. }
                ),
                "member case {at:x}"
            );
        }
        let mut bytes = fixture();
        put32(&mut bytes, SYSTEM + FACTION_COUNT, 5);
        assert!(matches!(
            names(&bytes, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn membership_requires_canonical_countries_and_both_directions_of_crosslinks() {
        for (at, value) in [
            (MEMBER_POINTERS, 0),
            (MEMBER_POINTERS + 8, C2 as u64),
            (C2, 0),
            (D2, 0),
            (D2 + cdiplomacy_status::COUNTRY, C1 as u64),
            (D2 + cdiplomacy_status::FACTION, SECOND as u64),
            (D2 + cdiplomacy_status::FACTION, 0),
            (COUNTRIES + 8, C2 as u64),
        ] {
            let mut bytes = fixture();
            put64(&mut bytes, at, value);
            assert!(
                matches!(
                    members(&bytes, &mut vec![MemberRow::default(); 2]),
                    Outcome::Unavailable { .. }
                ),
                "case {at:x}"
            );
        }
        let mut bytes = fixture();
        put32(&mut bytes, FIRST + cfaction::MEMBER_COUNT, 1);
        assert_eq!(
            members(&bytes, &mut vec![MemberRow::default(); 2]),
            Outcome::Unavailable {
                reason: "country faction is missing from member roster",
            }
        );
        put32(&mut bytes, FIRST + cfaction::MEMBER_COUNT, 4);
        assert!(matches!(
            members(&bytes, &mut vec![MemberRow::default(); 4]),
            Outcome::Unavailable { .. }
        ));
        assert_eq!(
            names(&bytes, &mut vec![cfaction::Row::default(); 2], &mut vec![0; 4]).outcome,
            Outcome::Rows { len: 2 }
        );
    }

    #[test]
    fn empty_system_is_valid_only_when_countries_have_no_membership() {
        let mut bytes = fixture();
        array(&mut bytes, SYSTEM + FACTIONS, 0, 0, 0);
        assert_eq!(
            names(&bytes, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Rows { len: 0 }
        );
        assert!(matches!(
            members(&bytes, &mut Vec::new()),
            Outcome::Unavailable { .. }
        ));
        for diplomacy in [D1, D2] {
            put64(&mut bytes, diplomacy + cdiplomacy_status::FACTION, 0);
        }
        assert_eq!(members(&bytes, &mut Vec::new()), Outcome::Rows { len: 0 });
    }
    #[test]
    fn faction_companion_roster_remains_complete_across_an_independent_name_failure() {
        let mut bytes = fixture();
        put64(&mut bytes, FIRST + cfaction::NAME + msvc_string::LENGTH, 16);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut seen = Vec::new();
        let (names, complete) = cfaction::walk_with(
            &memory,
            0,
            GS,
            &mut Vec::new(),
            &mut Vec::new(),
            |pointer, id| seen.push((pointer, id)),
        );
        assert!(matches!(names.outcome, Outcome::Unavailable { .. }));
        assert!(complete);
        assert_eq!(seen, [(FIRST, FIRST_ID), (SECOND, SECOND_ID)]);
        put64(&mut bytes, SECOND, 0);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let (failed, complete) = cfaction::walk_with(
            &memory,
            0,
            GS,
            &mut Vec::new(),
            &mut Vec::new(),
            |_, _| {},
        );
        assert!(!complete);
        assert_eq!(failed.outcome, names.outcome);
    }
}
