use super::*;
use engine_native_memory::memory::SliceMemory;

#[test]
fn timed_ideas_keep_stored_days_order_and_live_demand() {
    let mut world = ccountry::synthetic::World::new(BASE, 4, 0x1000);
    for country in 1..=4 {
        let owner = world.object(country);
        let status = world.alloc(COUNTRY + 8);
        let ideas = world.alloc(TIMED_IDEA_STRIDE * 2);
        let idea = world.alloc(cidea::TOKEN + msvc_string::STRIDE);
        world.u64(owner + ccountry::POLITICAL_STATUS, status as u64);
        world.u64(status, (BASE + VTABLE_RVA.0) as u64);
        world.u64(status + COUNTRY, owner as u64);
        world.array(status + TIMED_IDEAS, ideas, 2);
        world.u64(idea, (BASE + cidea::VTABLE_RVA.0) as u64);
        world.put(idea + cidea::TOKEN, b"idea\xff");
        world.u64(idea + cidea::TOKEN + 16, 5);
        world.u64(idea + cidea::TOKEN + 24, 15);
        for (ordinal, days) in [i32::MIN, 17].into_iter().enumerate() {
            let entry = ideas + ordinal * TIMED_IDEA_STRIDE;
            world.u64(entry, (BASE + TIMED_IDEA_VTABLE_RVA.0) as u64);
            world.u64(entry + 8, idea as u64);
            world.put(entry + 16, &days.to_le_bytes());
        }
    }
    assert_eq!(
        timed_idea_demand(&world.memory(), BASE, world.game_state()),
        Ok(IdeaDemand { rows: 8, bytes: 40 })
    );
    let mut rows = Vec::new();
    let mut tokens = Vec::new();
    let walk = walk_timed_ideas(
        &world.memory(),
        BASE,
        world.game_state(),
        &mut rows,
        &mut tokens,
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 8 });
    for (index, row) in rows[..8].iter().enumerate() {
        assert_eq!(
            (row.country, row.ordinal),
            (index as u32 / 2 + 1, index as u32 % 2)
        );
        assert_eq!(row.days, [i32::MIN, 17][index % 2]);
        assert_eq!(
            &tokens[row.token_start..row.token_start + row.token_len],
            b"idea\xff"
        );
    }
    for country in 1..=4 {
        let status = engine_native_memory::read::<u64>(
            &world.memory(),
            world.object(country) + ccountry::POLITICAL_STATUS,
        )
        .unwrap() as usize;
        let old = engine_native_memory::read::<u64>(&world.memory(), status + TIMED_IDEAS).unwrap() as usize;
        let idea = engine_native_memory::read::<u64>(&world.memory(), old + 8).unwrap();
        let entries = world.alloc(10 * TIMED_IDEA_STRIDE);
        world.array(status + TIMED_IDEAS, entries, 10);
        for ordinal in 0..10 {
            let entry = entries + ordinal * TIMED_IDEA_STRIDE;
            world.u64(entry, (BASE + TIMED_IDEA_VTABLE_RVA.0) as u64);
            world.u64(entry + 8, idea);
            world.put(entry + 16, &(-(ordinal as i32)).to_le_bytes());
        }
    }
    let later = walk_timed_ideas(
        &world.memory(),
        BASE,
        world.game_state(),
        &mut rows,
        &mut tokens,
    );
    assert_eq!(later.outcome, Outcome::Rows { len: 40 });
    assert_eq!(later.bytes_len, 200);
    assert_eq!(later.required_bytes, 200);
    for (index, row) in rows[..40].iter().enumerate() {
        assert_eq!(
            (row.country, row.ordinal),
            (index as u32 / 10 + 1, index as u32 % 10)
        );
        assert_eq!(row.days, -(index as i32 % 10));
        assert_eq!(
            &tokens[row.token_start..row.token_start + row.token_len],
            b"idea\xff"
        );
    }
    let status = engine_native_memory::read::<u64>(
        &world.memory(),
        world.object(1) + ccountry::POLITICAL_STATUS,
    )
    .unwrap() as usize;
    let array = engine_native_memory::read::<u64>(&world.memory(), status + TIMED_IDEAS).unwrap() as usize;
    world.u64(array, 0);
    assert_eq!(
        walk_timed_ideas(
            &world.memory(),
            BASE,
            world.game_state(),
            &mut rows,
            &mut tokens
        )
        .outcome,
        Outcome::Unavailable {
            reason: "timed idea entry vtable mismatch"
        }
    );
}

const BASE: usize = 0x140000000;
const COUNTRY_POINTER: usize = 0x1000;
const STATUS: usize = 0x3000;

fn put<const N: usize>(bytes: &mut [u8], address: usize, value: [u8; N]) {
    bytes[address - COUNTRY_POINTER..address - COUNTRY_POINTER + N].copy_from_slice(&value);
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x6000];
    put(
        &mut bytes,
        COUNTRY_POINTER,
        ((BASE + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY_POINTER + ccountry::POLITICAL_STATUS,
        (STATUS as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        COUNTRY_POINTER + ccountry::CAPITAL,
        (-17_i32).to_le_bytes(),
    );
    put(
        &mut bytes,
        STATUS,
        ((BASE + VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        STATUS + COUNTRY,
        (COUNTRY_POINTER as u64).to_le_bytes(),
    );
    put(&mut bytes, STATUS + LAST_ELECTION, (-400_i32).to_le_bytes());
    put(
        &mut bytes,
        STATUS + ELECTION_FREQUENCY,
        48_i32.to_le_bytes(),
    );
    put(&mut bytes, STATUS + ELECTIONS_ALLOWED, [1]);
    bytes
}

fn array(bytes: &mut [u8], at: usize, data: usize, capacity: u32, count: u32) {
    put(bytes, at, (data as u64).to_le_bytes());
    put(bytes, at + 8, capacity.to_le_bytes());
    put(bytes, at + 12, count.to_le_bytes());
}

fn inline(bytes: &mut [u8], at: usize, token: &[u8]) {
    bytes[at - COUNTRY_POINTER..at - COUNTRY_POINTER + token.len()].copy_from_slice(token);
    put(
        bytes,
        at + msvc_string::LENGTH,
        (token.len() as u64).to_le_bytes(),
    );
    put(bytes, at + msvc_string::CAPACITY, 15_u64.to_le_bytes());
}

fn party(bytes: &mut [u8], index: u32, token: &[u8]) {
    let pointer = 0x4000 + index as usize * 0x100;
    let ideology = 0x5000 + index as usize * 0x100;
    let id = index * 11;
    put(
        bytes,
        0x3800 + index as usize * 8,
        (pointer as u64).to_le_bytes(),
    );
    put(
        bytes,
        pointer,
        ((BASE + cpolitical_party::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(bytes, pointer + cpolitical_party::ID, id.to_le_bytes());
    put(
        bytes,
        pointer + cpolitical_party::IDEOLOGY_GROUP,
        (ideology as u64).to_le_bytes(),
    );
    put(
        bytes,
        pointer + cpolitical_party::POPULARITY,
        (-(1_i64 << 40) + i64::from(index)).to_le_bytes(),
    );
    put(
        bytes,
        pointer + cpolitical_party::TAG,
        (100 + index).to_le_bytes(),
    );
    put(bytes, pointer + cpolitical_party::NAME_DEFAULT, [1]);
    inline(bytes, pointer + cpolitical_party::NAME, b"");
    inline(bytes, pointer + cpolitical_party::LONG_NAME, b"");
    put(
        bytes,
        ideology,
        ((BASE + cideology_group::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(bytes, ideology + cpolitical_party::ID, id.to_le_bytes());
    inline(bytes, ideology + cideology_group::TOKEN, token);
}

#[test]
fn native_roster_count_and_precision_survive_without_vanilla_projection() {
    let mut bytes = fixture();
    array(&mut bytes, STATUS + PARTIES, 0x3800, 7, 5);
    let tokens = [b"a?".as_slice(), b"b!", b"c/", b"d+", &[0xff, 0, 0x81]];
    for (index, token) in tokens.iter().enumerate() {
        party(&mut bytes, index as u32, token);
    }
    put(&mut bytes, STATUS + RULING_PARTY, 0x4400_u64.to_le_bytes());
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes: &bytes,
    };
    let reader = Reader::new(&memory, BASE, COUNTRY_POINTER).unwrap();
    let row = reader.row().unwrap();
    assert_eq!(row.last_election, -400);
    assert_eq!(row.capital, -17);
    assert_eq!(row.ruling_party, Some(44));
    let mut rows = vec![cpolitical_party::Row::default(); 8];
    let mut arena = vec![0; 32];
    let walked = reader.parties(&mut rows, 0, &mut arena, 0);
    assert_eq!(walked.outcome, Outcome::Rows { len: 5 });
    assert_eq!(walked.required_bytes, 11);
    for (index, expected) in tokens.iter().enumerate() {
        let row = rows[index];
        assert_eq!(
            &arena[row.token_start..row.token_start + row.token_len],
            *expected
        );
        assert_eq!(row.popularity, -(1_i64 << 40) + index as i64);
        assert_eq!(row.is_ruling, u8::from(index == 4));
        assert_eq!(row.tag, 100 + index as u32);
        assert_eq!(row.name_default, 1);
        assert_eq!((row.name_len, row.long_name_len), (0, 0));
    }
}

#[test]
fn renamed_party_keys_are_copied_exactly_and_count_toward_the_arena() {
    let mut bytes = fixture();
    array(&mut bytes, STATUS + PARTIES, 0x3800, 2, 2);
    party(&mut bytes, 0, b"x");
    party(&mut bytes, 1, b"y");
    let long = b"AAA_mod_party_long_key";
    put(&mut bytes, 0x4000 + cpolitical_party::NAME_DEFAULT, [0]);
    inline(
        &mut bytes,
        0x4000 + cpolitical_party::NAME,
        b"AAA_mod_party",
    );
    let at = 0x4000 + cpolitical_party::LONG_NAME;
    put(&mut bytes, at, 0x6000_u64.to_le_bytes());
    put(
        &mut bytes,
        at + msvc_string::LENGTH,
        (long.len() as u64).to_le_bytes(),
    );
    put(&mut bytes, at + msvc_string::CAPACITY, 31_u64.to_le_bytes());
    bytes[0x6000 - COUNTRY_POINTER..][..long.len()].copy_from_slice(long);
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes: &bytes,
    };
    let reader = Reader::new(&memory, BASE, COUNTRY_POINTER).unwrap();
    let mut rows = vec![cpolitical_party::Row::default(); 2];
    let mut arena = vec![0; 64];
    let walked = reader.parties(&mut rows, 0, &mut arena, 0);
    assert_eq!(walked.outcome, Outcome::Rows { len: 2 });
    assert_eq!(walked.required_bytes, 2 + 13 + long.len());
    fn text(arena: &[u8], start: usize, len: usize) -> &[u8] {
        &arena[start..start + len]
    }
    assert_eq!(rows[0].name_default, 0);
    assert_eq!(
        text(&arena, rows[0].name_start, rows[0].name_len),
        b"AAA_mod_party"
    );
    assert_eq!(
        text(&arena, rows[0].long_name_start, rows[0].long_name_len),
        long.as_slice()
    );
    assert_eq!(text(&arena, rows[1].token_start, rows[1].token_len), b"y");
    assert_eq!(rows[1].name_default, 1);
    put(&mut bytes, 0x4000 + cpolitical_party::NAME_DEFAULT, [2]);
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes: &bytes,
    };
    let reader = Reader::new(&memory, BASE, COUNTRY_POINTER).unwrap();
    assert_eq!(
        reader.parties(&mut rows, 0, &mut arena, 0).outcome,
        Outcome::Unavailable {
            reason: "political party default name flag is not a boolean"
        }
    );
}

#[test]
fn authenticated_empty_and_unreadable_state_are_distinct_per_table() {
    let mut bytes = fixture();
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes: &bytes,
    };
    let reader = Reader::new(&memory, BASE, COUNTRY_POINTER).unwrap();
    assert_eq!(reader.row().unwrap().ruling_party, None);
    assert_eq!(
        reader.parties(&mut Vec::new(), 0, &mut Vec::new(), 0).outcome,
        Outcome::Rows { len: 0 }
    );
    assert_eq!(
        reader.ideas(&mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Rows { len: 0 }
    );
    put(&mut bytes, STATUS + ELECTIONS_ALLOWED, [2]);
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes: &bytes,
    };
    let reader = Reader::new(&memory, BASE, COUNTRY_POINTER).unwrap();
    assert!(reader.row().unwrap().has_elections);
    assert_eq!(
        reader.ideas(&mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Rows { len: 0 }
    );
    array(&mut bytes, STATUS + PARTIES, 0x3800, 1, 1);
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes: &bytes,
    };
    let reader = Reader::new(&memory, BASE, COUNTRY_POINTER).unwrap();
    assert!(matches!(
        reader.parties(&mut Vec::new(), 0, &mut Vec::new(), 0).outcome,
        Outcome::Unavailable { .. }
    ));
    put(&mut bytes, STATUS + COUNTRY, 0_u64.to_le_bytes());
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes: &bytes,
    };
    assert!(Reader::new(&memory, BASE, COUNTRY_POINTER).is_err());
}

#[test]
fn active_idea_secondary_base_and_heap_tokens_are_copied_exactly() {
    let mut bytes = fixture();
    array(&mut bytes, STATUS + IDEAS, 0x3800, 2, 2);
    put(&mut bytes, 0x3800, 0x4010_u64.to_le_bytes());
    put(&mut bytes, 0x3808, 0x4100_u64.to_le_bytes());
    for pointer in [0x4000, 0x4100] {
        put(
            &mut bytes,
            pointer,
            ((BASE + cidea::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer + cidea::SECONDARY_BASE,
            ((BASE + cidea::SECONDARY_VTABLE_RVA.0) as u64).to_le_bytes(),
        );
    }
    inline(&mut bytes, 0x4000 + cidea::TOKEN, b"idea_one");
    let token = b"arbitrary_mod_idea_with_no_fixed_buffer_limit";
    put(&mut bytes, 0x4100 + cidea::TOKEN, 0x5000_u64.to_le_bytes());
    put(
        &mut bytes,
        0x4100 + cidea::TOKEN + msvc_string::LENGTH,
        (token.len() as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x4100 + cidea::TOKEN + msvc_string::CAPACITY,
        63_u64.to_le_bytes(),
    );
    bytes[0x4000..0x4000 + token.len()].copy_from_slice(token);
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes: &bytes,
    };
    let reader = Reader::new(&memory, BASE, COUNTRY_POINTER).unwrap();
    let mut rows = vec![IdeaRow::default(); 2];
    let mut arena = vec![0; 128];
    let walk = reader.ideas(&mut rows, &mut arena);
    assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
    assert_eq!(&arena[..8], b"idea_one");
    assert_eq!(&arena[rows[1].token_start..walk.bytes_len], token);
    assert_eq!(rows[1].ordinal, 1);
    put(&mut bytes, 0x4000, 0_u64.to_le_bytes());
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes: &bytes,
    };
    let reader = Reader::new(&memory, BASE, COUNTRY_POINTER).unwrap();
    assert_eq!(
        reader.ideas(&mut rows, &mut arena).outcome,
        Outcome::Unavailable {
            reason: "active idea primary vtable mismatch"
        }
    );
}

const LEADERS: [usize; 3] = [0x6100, 0x6300, 0x6500];
const CHARACTERS: [(usize, u32); 2] = [(0x6800, 1482), (0x6900, 1451)];

fn leader_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    array(&mut bytes, STATUS + PARTIES, 0x3800, 2, 2);
    party(&mut bytes, 0, b"a");
    party(&mut bytes, 1, b"b");
    array(
        &mut bytes,
        0x4000 + cpolitical_party::COUNTRY_LEADERS,
        0x6000,
        2,
        2,
    );
    array(
        &mut bytes,
        0x4100 + cpolitical_party::COUNTRY_LEADERS,
        0x6040,
        1,
        1,
    );
    for (slot, leader) in [
        (0x6000, LEADERS[0]),
        (0x6008, LEADERS[1]),
        (0x6040, LEADERS[2]),
    ] {
        put(&mut bytes, slot, (leader as u64).to_le_bytes());
        put(
            &mut bytes,
            leader,
            ((BASE + ccountry_leader::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            leader + ccountry_leader::LEADER_ID,
            (-1_i32).to_le_bytes(),
        );
    }
    put(
        &mut bytes,
        LEADERS[2] + ccountry_leader::LEADER_ID,
        7_i32.to_le_bytes(),
    );
    for (leader, (character, id)) in [(LEADERS[0], CHARACTERS[0]), (LEADERS[2], CHARACTERS[1])] {
        put(
            &mut bytes,
            leader + ccountry_leader::CHARACTER,
            (character as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            character,
            ((BASE + ccharacter::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            character + ccharacter::TYPE,
            ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        put(&mut bytes, character + ccharacter::ID, id.to_le_bytes());
    }
    bytes.resize(0x6400, 0);
    for (character, head, key, leader) in [
        (CHARACTERS[0].0, 0x7000, 5_i32, LEADERS[0]),
        (CHARACTERS[1].0, 0x7100, -11, LEADERS[2]),
    ] {
        let node = head + 0x40;
        put(
            &mut bytes,
            character + ccharacter::LEADER_MAP_HEAD,
            (head as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            character + ccharacter::LEADER_MAP_SIZE,
            1_u64.to_le_bytes(),
        );
        for offset in [0, 8, 16] {
            put(&mut bytes, head + offset, (node as u64).to_le_bytes());
            put(&mut bytes, node + offset, (head as u64).to_le_bytes());
        }
        put(&mut bytes, head + 0x19, [1]);
        put(&mut bytes, node + 0x20, key.to_le_bytes());
        put(&mut bytes, node + 0x28, (leader as u64).to_le_bytes());
    }
    put(&mut bytes, LEADERS[0] + 0x140, 999_u64.to_le_bytes());
    put(
        &mut bytes,
        CHARACTERS[0].0 + ccharacter::LEADER_MAP_SIZE,
        2_u64.to_le_bytes(),
    );
    put(&mut bytes, 0x7000, 0x7080_u64.to_le_bytes());
    put(&mut bytes, 0x7040, 0x7080_u64.to_le_bytes());
    for (offset, pointer) in [(0, 0x7000_u64), (8, 0x7040), (16, 0x7000)] {
        put(&mut bytes, 0x7080 + offset, pointer.to_le_bytes());
    }
    put(&mut bytes, 0x7080 + 0x20, (-17_i32).to_le_bytes());
    put(&mut bytes, 0x7080 + 0x28, 0x7300_u64.to_le_bytes());
    bytes
}

fn walk_leaders(bytes: &[u8], rows: usize) -> (Outcome, Vec<cpolitical_party::LeaderRow>) {
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes,
    };
    let reader = Reader::new(&memory, BASE, COUNTRY_POINTER).unwrap();
    let mut output = vec![cpolitical_party::LeaderRow::default(); rows];
    let outcome = reader.party_leaders(9, &mut output);
    (outcome, output)
}

#[test]
fn party_leaders_keep_party_and_vector_order_null_parents_and_ids() {
    let (outcome, rows) = walk_leaders(&leader_fixture(), 4);
    assert_eq!(outcome, Outcome::Rows { len: 3 });
    let row = |party, ordinal, character, leader_id| cpolitical_party::LeaderRow {
        country: 9,
        party,
        ordinal,
        character,
        role: character.map(|_| if party == 0 { 5 } else { -11 }),
        leader_id,
    };
    assert_eq!(
        rows[..3],
        [
            row(0, 0, Some(1482), -1),
            row(0, 1, None, -1),
            row(11, 0, Some(1451), 7),
        ]
    );
    let mut bytes = leader_fixture();
    array(
        &mut bytes,
        0x4000 + cpolitical_party::COUNTRY_LEADERS,
        0,
        0,
        0,
    );
    array(
        &mut bytes,
        0x4100 + cpolitical_party::COUNTRY_LEADERS,
        0,
        0,
        0,
    );
    assert_eq!(walk_leaders(&bytes, 4).0, Outcome::Rows { len: 0 });
    let mut bytes = leader_fixture();
    array(&mut bytes, STATUS + PARTIES, 0, 0, 0);
    assert_eq!(walk_leaders(&bytes, 4).0, Outcome::Rows { len: 0 });
}

#[test]
fn the_party_leader_count_probe_counts_the_live_total_without_reading_a_leader() {
    let mut bytes = leader_fixture();
    put(&mut bytes, LEADERS[1], 0_u64.to_le_bytes());
    let memory = SliceMemory {
        base: COUNTRY_POINTER,
        bytes: &bytes,
    };
    let reader = Reader::new(&memory, BASE, COUNTRY_POINTER).unwrap();
    assert_eq!(
        reader.party_leaders_buffer(9, engine_native_memory::walk::Buffer::Count),
        Outcome::Truncated {
            len: 0,
            unread_rows: 3
        }
    );
}

#[test]
fn unauthenticated_party_leaders_fail_the_table() {
    for (address, value) in [
        (0x6008, 0_u64),
        (LEADERS[2], 7),
        (CHARACTERS[1].0, 7),
        (0x7040 + 0x28, 0x7300),
        (0x7080 + 0x28, LEADERS[0] as u64),
        (0x7080 + 0x20, 6),
        (0x7080 + 8, 0x9_0000),
        (0x4100, 7),
        (0x4000 + cpolitical_party::COUNTRY_LEADERS, 0x9_0000),
    ] {
        let mut bytes = leader_fixture();
        put(&mut bytes, address, value.to_le_bytes());
        let (outcome, _) = walk_leaders(&bytes, 4);
        assert!(
            matches!(outcome, Outcome::Unavailable { .. }),
            "{address:#x}: {outcome:?}"
        );
    }
    let mut bytes = leader_fixture();
    put(
        &mut bytes,
        CHARACTERS[0].0 + ccharacter::TYPE,
        74_u32.to_le_bytes(),
    );
    assert!(matches!(
        walk_leaders(&bytes, 4).0,
        Outcome::Unavailable { .. }
    ));
}
