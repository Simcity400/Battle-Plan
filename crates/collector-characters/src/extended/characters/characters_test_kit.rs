use super::tests::{
    ADDRESS, CHARACTERS, IMAGE, MANAGER, ROSTER, UNITS, fixture, literal, pointer, put,
};
use super::{
    Buffer, Capacities, Captured, CharactersCopyContext, CharactersCopyReport, CharactersJob, N,
    Roles, copy_characters, walk_family,
};
use engine_native::{
    cadvisor,
    carmy_leader,
    ccharacter,
    ccharacter_manager,
    character_advisor_node,
    cnavy_leader,
    cunit_leader,
    cunit_leader_skill,
    msvc_string,
    msvc_tree_node,
};
use engine_native_politics::{ccountry_leader, character_leader_node};

pub const BASE: usize = IMAGE;
pub const GAME_STATE: usize = ADDRESS;

pub const LEAD_NODE: usize = 0x8100;
pub const ADV_NODE: usize = 0x8200;
pub const HEAP_AREA: usize = 0x8300;
pub const SKILL0: usize = 0x7900;
const LEAD_ROLE: usize = 0x8140;
pub const ADV_ROLE: usize = 0x8260;
const CHAR2: usize = 0x2300;
pub const UNIT2: usize = 0x5200;
const HEAD2L: usize = 0x2500;
const HEAD2A: usize = 0x2530;

pub fn memory(bytes: &[u8]) -> engine_native::memory::SliceMemory<'_> {
    engine_native::memory::SliceMemory {
        base: ADDRESS,
        bytes,
    }
}

fn heap_string(bytes: &mut [u8], header: usize, payload: usize, value: &[u8]) {
    assert!(value.len() > msvc_string::INLINE_CAPACITY as usize);
    put(bytes, header, ((ADDRESS + payload) as u64).to_le_bytes());
    put(bytes, header + 16, (value.len() as u64).to_le_bytes());
    put(bytes, header + 24, (value.len() as u64).to_le_bytes());
    bytes[payload..payload + value.len()].copy_from_slice(value);
    bytes[payload + value.len()] = 0;
}

fn empty_map(bytes: &mut [u8], character: usize, offset: usize, head: usize) {
    pointer(bytes, character + offset, head);
    for link in [0, 8, 16] {
        pointer(bytes, head + link, head);
    }
    bytes[head + msvc_tree_node::NIL] = 1;
}

pub fn mapped_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    put(
        &mut bytes,
        MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS + 8,
        3_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS + 12,
        3_u32.to_le_bytes(),
    );
    pointer(&mut bytes, ROSTER + 16, CHAR2);
    put(
        &mut bytes,
        CHARACTERS[0] + ccharacter::LEADER_MAP_HEAD + 8,
        1_u64.to_le_bytes(),
    );
    pointer(&mut bytes, 0x2400, LEAD_NODE);
    pointer(&mut bytes, LEAD_NODE, 0x2400);
    pointer(&mut bytes, LEAD_NODE + 8, 0x2400);
    pointer(&mut bytes, LEAD_NODE + 16, 0x2400);
    bytes[LEAD_NODE + msvc_tree_node::NIL] = 0;
    put(
        &mut bytes,
        LEAD_NODE + character_leader_node::IDEOLOGY,
        7_i32.to_le_bytes(),
    );
    pointer(
        &mut bytes,
        LEAD_NODE + character_leader_node::LEADER,
        LEAD_ROLE,
    );
    put(
        &mut bytes,
        LEAD_ROLE,
        ((IMAGE + ccountry_leader::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    pointer(
        &mut bytes,
        LEAD_ROLE + ccountry_leader::CHARACTER,
        CHARACTERS[0],
    );
    put(
        &mut bytes,
        CHARACTERS[0] + ccharacter::ADVISOR_MAP_HEAD + 8,
        1_u64.to_le_bytes(),
    );
    pointer(&mut bytes, 0x2430, ADV_NODE);
    pointer(&mut bytes, ADV_NODE, 0x2430);
    pointer(&mut bytes, ADV_NODE + 8, 0x2430);
    pointer(&mut bytes, ADV_NODE + 16, 0x2430);
    bytes[ADV_NODE + msvc_tree_node::NIL] = 0;
    pointer(
        &mut bytes,
        ADV_NODE + character_advisor_node::ADVISOR,
        ADV_ROLE,
    );
    heap_string(
        &mut bytes,
        ADV_NODE + character_advisor_node::KEY,
        HEAP_AREA,
        b"twenty-byte-advisor!",
    );
    put(
        &mut bytes,
        ADV_NODE + character_advisor_node::KEY_HASH,
        1234_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        ADV_ROLE,
        ((IMAGE + cadvisor::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    pointer(&mut bytes, ADV_ROLE + cadvisor::CHARACTER, CHARACTERS[0]);
    literal(&mut bytes, ADV_ROLE + cadvisor::IDEA_TOKEN, b"idea");
    literal(&mut bytes, ADV_ROLE + cadvisor::SLOT, b"slot");
    put(
        &mut bytes,
        SKILL0,
        ((IMAGE + cunit_leader_skill::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    bytes[SKILL0 + cunit_leader_skill::VALID] = 1;
    put(
        &mut bytes,
        SKILL0 + cunit_leader_skill::LEVEL,
        3_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        SKILL0 + cunit_leader_skill::LEVEL + 4,
        100_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        SKILL0 + cunit_leader_skill::LEVEL + 8,
        5_i32.to_le_bytes(),
    );
    pointer(&mut bytes, UNITS[0] + cunit_leader::SKILL, SKILL0);
    put(
        &mut bytes,
        UNITS[1],
        ((IMAGE + carmy_leader::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        UNITS[1] + carmy_leader::ATTACK,
        11_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        UNITS[1] + carmy_leader::DEFENSE,
        22_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        UNITS[1] + carmy_leader::PLANNING,
        33_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        UNITS[1] + carmy_leader::LOGISTICS,
        44_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        CHAR2,
        ((IMAGE + ccharacter::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        CHAR2 + ccharacter::TYPE,
        ccharacter::REFERENCE_TYPE.to_le_bytes(),
    );
    put(&mut bytes, CHAR2 + ccharacter::ID, 42_u32.to_le_bytes());
    put(&mut bytes, CHAR2 + ccharacter::COUNTRY, 3_u32.to_le_bytes());
    pointer(&mut bytes, CHAR2 + ccharacter::UNIT_LEADER, UNIT2);
    heap_string(
        &mut bytes,
        CHAR2 + ccharacter::NAME,
        HEAP_AREA + 64,
        b"heap-named-character!",
    );
    literal(&mut bytes, CHAR2 + ccharacter::TOKEN, b"tok");
    empty_map(&mut bytes, CHAR2, ccharacter::LEADER_MAP_HEAD, HEAD2L);
    empty_map(&mut bytes, CHAR2, ccharacter::ADVISOR_MAP_HEAD, HEAD2A);
    put(
        &mut bytes,
        UNIT2,
        ((IMAGE + cnavy_leader::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        UNIT2 + cunit_leader::TYPE,
        cunit_leader::REFERENCE_TYPE.to_le_bytes(),
    );
    put(&mut bytes, UNIT2 + cunit_leader::ID, 42_u32.to_le_bytes());
    put(
        &mut bytes,
        UNIT2 + cunit_leader::CHARACTER_TYPE,
        ccharacter::REFERENCE_TYPE.to_le_bytes(),
    );
    put(
        &mut bytes,
        UNIT2 + cunit_leader::CHARACTER_ID,
        42_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        UNIT2 + cunit_leader::EXPERIENCE,
        999_i64.to_le_bytes(),
    );
    literal(&mut bytes, UNIT2 + cunit_leader::TOKEN, b"n2");
    literal(&mut bytes, UNIT2 + cunit_leader::NAME, b"m2");
    put(
        &mut bytes,
        UNIT2 + cunit_leader::TRAIT_DATA,
        0_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        UNIT2 + cunit_leader::TRAIT_EXPERIENCE_DATA,
        0_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        UNIT2 + cnavy_leader::ATTACK,
        51_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        UNIT2 + cnavy_leader::DEFENSE,
        52_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        UNIT2 + cnavy_leader::MANEUVERING,
        53_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        UNIT2 + cnavy_leader::COORDINATION,
        54_i32.to_le_bytes(),
    );
    bytes
}

pub fn direct(memory: &impl engine_native::Memory, capacities: &Capacities) -> Captured {
    let mut captured = Captured::with_capacities(capacities);
    captured.set_character_activity([true; N]);
    let results = walk_family(memory, BASE, GAME_STATE, captured.family(), None, None);
    captured.characters.primary(results[0]);
    captured.leaders.primary(results[1]);
    captured.advisors.primary(results[2]);
    captured.units.primary(results[3]);
    captured.traits.primary(results[4]);
    captured
}

pub fn split(
    memory: &impl engine_native::Memory,
    capacities: &Capacities,
) -> (Captured, CharactersCopyReport, CharactersJob) {
    let mut captured = Captured::with_capacities(capacities);
    let ctx = CharactersCopyContext {
        force_full: false,
        base: BASE,
        game_state: GAME_STATE,
        active: [true; N],
    };
    let mut first = CharactersJob::new();
    let report = copy_characters(memory, ctx, &mut first, Roles::Independent);
    assert!(report.raw_ok);
    let mut ticks = [0u64; N];
    captured.decode_characters_payload(BASE, &first, &mut ticks);
    (captured, report, first)
}

fn assert_buffer_payload<R>(name: &str, split: &Buffer<R>, direct: &Buffer<R>) -> usize {
    assert_eq!(split.rows.outcome, direct.rows.outcome, "{name} outcome");
    let len = split.rows.outcome.len();
    assert_eq!(direct.rows.outcome.len(), len, "{name} length");
    assert_eq!(
        &split.bytes.first[..split.bytes.len],
        &direct.bytes.first[..direct.bytes.len],
        "{name} arena"
    );
    assert_eq!(
        split.bytes.required, direct.bytes.required,
        "{name} required bytes"
    );
    len
}

fn assert_buffer<R: PartialEq + std::fmt::Debug>(
    name: &str,
    split: &Buffer<R>,
    direct: &Buffer<R>,
) {
    let len = assert_buffer_payload(name, split, direct);
    assert_eq!(
        &split.rows.rows[..len],
        &direct.rows.rows[..len],
        "{name} rows"
    );
}

pub fn assert_matches_direct(split: &Captured, direct: &Captured) {
    assert_buffer("characters", &split.characters, &direct.characters);
    assert_buffer("leaders", &split.leaders, &direct.leaders);
    let len = assert_buffer_payload("advisors", &split.advisors, &direct.advisors);
    let empty_cache = |row: &cadvisor::Row| cadvisor::Row {
        modifier_start: if row.modifier_len == 0 {
            0
        } else {
            row.modifier_start
        },
        ..*row
    };
    assert_eq!(
        split.advisors.rows.rows[..len]
            .iter()
            .map(empty_cache)
            .collect::<Vec<_>>(),
        direct.advisors.rows.rows[..len]
            .iter()
            .map(empty_cache)
            .collect::<Vec<_>>(),
        "advisors rows"
    );
    assert_eq!(split.advisors_table(), direct.advisors_table());
    assert_buffer("units", &split.units, &direct.units);
    assert_buffer("traits", &split.traits, &direct.traits);
}
