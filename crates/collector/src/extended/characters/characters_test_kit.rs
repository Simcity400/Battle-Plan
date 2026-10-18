use super::tests::{
    ADDRESS, CHARACTERS, IMAGE, MANAGER, ROSTER, UNITS, fixture, literal, pointer, put,
};
use super::{
    Buffer, Capacities, Captured, CharactersCopyContext, CharactersCopyReport, CharactersJob,
    copy_characters_refresh, walk_family,
};
use engine_native::{
    cadvisor, carmy_leader, ccharacter, ccharacter_manager, ccountry_leader,
    character_advisor_node, character_leader_node, cnavy_leader, cunit_leader, cunit_leader_skill,
    msvc_string, msvc_tree_node,
};

pub(crate) const BASE: usize = IMAGE;
pub(crate) const GAME_STATE: usize = ADDRESS;

pub(crate) const LEAD_NODE: usize = 0x8100;
pub(crate) const ADV_NODE: usize = 0x8200;
pub(crate) const HEAP_AREA: usize = 0x8300;
pub(crate) const SKILL0: usize = 0x7900;
const LEAD_ROLE: usize = 0x8140;
pub(crate) const ADV_ROLE: usize = 0x8260;
const CHAR2: usize = 0x2300;
pub(crate) const UNIT2: usize = 0x5200;
const HEAD2L: usize = 0x2500;
const HEAD2A: usize = 0x2530;

pub(crate) fn memory(bytes: &[u8]) -> engine_native::memory::SliceMemory<'_> {
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

/// Base fixture plus one leader role, one heap-keyed advisor role, a
/// valid skill on unit 0, army kind on unit 1, and a third navy character
/// with a heap name.
pub(crate) fn mapped_fixture() -> Vec<u8> {
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
    // Single leader role on character 0 with ideology key 7. The map
    // count lives in the character's map header, not the sentinel.
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
        ((IMAGE + ccountry_leader::VTABLE_RVA) as u64).to_le_bytes(),
    );
    pointer(
        &mut bytes,
        LEAD_ROLE + ccountry_leader::CHARACTER,
        CHARACTERS[0],
    );
    // Single advisor role on character 0 with a 20-byte heap key.
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
        ((IMAGE + cadvisor::VTABLE_RVA) as u64).to_le_bytes(),
    );
    pointer(&mut bytes, ADV_ROLE + cadvisor::CHARACTER, CHARACTERS[0]);
    literal(&mut bytes, ADV_ROLE + cadvisor::IDEA_TOKEN, b"idea");
    literal(&mut bytes, ADV_ROLE + cadvisor::SLOT, b"slot");
    // Valid skill definition on unit 0.
    put(
        &mut bytes,
        SKILL0,
        ((IMAGE + cunit_leader_skill::VTABLE_RVA) as u64).to_le_bytes(),
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
    // Army kind with components on unit 1.
    put(
        &mut bytes,
        UNITS[1],
        ((IMAGE + carmy_leader::VTABLE_RVA) as u64).to_le_bytes(),
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
    // Third character with a heap name and a navy unit.
    put(
        &mut bytes,
        CHAR2,
        ((IMAGE + ccharacter::VTABLE_RVA) as u64).to_le_bytes(),
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
        ((IMAGE + cnavy_leader::VTABLE_RVA) as u64).to_le_bytes(),
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

/// Reference typed walk with identical pooled capacities, outcomes and
/// arenas installed exactly like `capture` installs them.
pub(crate) fn direct(memory: &impl engine_native::Memory, capacities: &Capacities) -> Captured {
    let mut captured = Captured::with_capacities(capacities);
    let results = walk_family(memory, BASE, GAME_STATE, captured.family(), None, None);
    captured.characters.primary(results[0]);
    captured.leaders.primary(results[1]);
    captured.advisors.primary(results[2]);
    captured.units.primary(results[3]);
    captured.traits.primary(results[4]);
    captured
}

/// Capture one owned payload and decode into fresh pooled buffers.
pub(crate) fn split(
    memory: &impl engine_native::Memory,
    capacities: &Capacities,
) -> (Captured, CharactersCopyReport, CharactersJob) {
    let mut captured = Captured::with_capacities(capacities);
    let (trait_rows, trait_bytes) = captured.trait_capture_capacities();
    let ctx = CharactersCopyContext {
        force_full: false,
        base: BASE,
        game_state: GAME_STATE,
        active: [true; 5],
        trait_rows,
        trait_bytes,
    };
    let mut first = CharactersJob::new();
    let report = copy_characters_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let mut ticks = [0u64; 5];
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

pub(crate) fn assert_matches_direct(split: &Captured, direct: &Captured) {
    assert_buffer("characters", &split.characters, &direct.characters);
    assert_buffer("leaders", &split.leaders, &direct.leaders);
    let len = assert_buffer_payload("advisors", &split.advisors, &direct.advisors);
    // The mutation-certified cache packs definitions before fresh numeric
    // caches. Empty spans may therefore point at different arena tails;
    // they address no byte. Preserve every flag, failure and nonempty span.
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
