use super::*;

pub fn character_shadow_fixture(value: u8) -> Captured {
    let capacities = Capacities {
        rows: [2, 2, 2, 1, 2, 0],
        bytes: [8, 0, 8, 0, 8, 0],
    };
    let mut captured = Captured::with_capacities(&capacities);
    for index in 0..2 {
        captured.characters.rows.rows[index] = ccharacter::Row {
            id: (index + 3) as u32,
            country: 7,
            name_token_id: -9,
            roster: index as u8,
            ordinal: index as u32,
            name_start: index * 3,
            name_len: 2,
            token_start: index * 3 + 2,
            token_len: 1,
            ..Default::default()
        };
        captured.leaders.rows.rows[index] = ccountry_leader::Row {
            character: (index + 3) as u32,
            country: u32::from(value),
            ideology: -7,
            ..Default::default()
        };
        captured.advisors.rows.rows[index] = cadvisor::Row {
            character: (index + 3) as u32,
            country: 7,
            ordinal: 0,
            key_start: index * 3,
            key_len: 1,
            idea_start: index * 3 + 1,
            idea_len: 1,
            slot_start: index * 3 + 2,
            slot_len: 1,
            ..Default::default()
        };
        captured.traits.rows.rows[index] = cunit_leader::TraitRow {
            leader_id: (index + 3) as u32,
            ordinal: 0,
            token_id: -5,
            definition_ordinal: -1,
            token_start: index * 3,
            token_len: 3,
        };
    }
    for bytes in [
        &mut captured.characters.bytes,
        &mut captured.advisors.bytes,
        &mut captured.traits.bytes,
    ] {
        bytes.first[..6].copy_from_slice(&[value, 0, 0xff, b'b', b'c', b'd']);
        bytes.len = 6;
        bytes.required = 6;
    }
    captured.characters.rows.outcome = Outcome::Rows { len: 2 };
    captured.leaders.rows.outcome = Outcome::Rows { len: 2 };
    captured.advisors.rows.outcome = Outcome::Rows { len: 2 };
    captured.traits.rows.outcome = Outcome::Rows { len: 2 };
    captured.units.rows.outcome = Outcome::Rows { len: 0 };
    captured
}
