use super::*;
use crate::{cgame_state, msvc_string};
use std::collections::BTreeMap;

const BASE: usize = 0x1_4000_0000;
const GS: usize = 0x1000;
const COUNTRIES: usize = 0x2000;
const LINKS: usize = 0x2100;
const C1: usize = 0x10000;
const C2: usize = 0x20000;
const D1: usize = 0x30000;
const D2: usize = 0x30800;
const R1: usize = 0x31000;
const RELATIONS: usize = 0x31800;
const STATUS: usize = 0x32000;
const PARTY: usize = 0x32400;
const LEADER: usize = 0x32800;
const GROUP: usize = 0x33000;
const IDEAS: usize = 0x33800;
const IDEA_A: usize = 0x34000;
const IDEA_B: usize = 0x34400;
const CHARACTER_77: usize = 0x35000;
const CHARACTER_88: usize = 0x35100;
const CHARACTERS: usize = 0x36000;
const ADVISORS: usize = 0x36400;
const ADVISOR: usize = 0x36800;
const ENTRY: usize = 0x37000;
const DEFINITION: usize = 0x37400;
const VALUES: usize = 0x37800;
const VALUE_ROWS: usize = 0x37900;
const STATS: usize = 0x38000;
const RING_OBJECT: usize = 0x38100;
const RING_SLOTS: usize = 0x38200;
const RING_ROW: usize = 0x38300;
const RING_VALUES: usize = 0x38400;
const WORLD: usize = 0x39000;
const LANGUAGE_ROOT: usize = 0x39400;
const LANGUAGE: usize = 0x39500;
const DATABASE: usize = 0x3a000;
const AT_WAR: usize = 0x3a800;
const TABLE: usize = 0x40000;
const CHILDREN: usize = 0x50000;
const CHILD: usize = 0x51000;
const ENTRIES: usize = 0x60000;
const HEAP: usize = 0x70000;

/// A tree child: its sorted entries and display text.
type ChildSpec<'a> = (&'a [(u32, i64)], &'a [u8]);

#[derive(Default)]
struct World {
    bytes: BTreeMap<usize, u8>,
    entries: usize,
    heap: usize,
}

impl Memory for World {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        for (index, byte) in destination.iter_mut().enumerate() {
            let Some(value) = self.bytes.get(&(address + index)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}

impl World {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        self.bytes
            .extend(bytes.iter().enumerate().map(|(i, &b)| (address + i, b)));
    }
    fn u64(&mut self, address: usize, value: u64) {
        self.put(address, &value.to_le_bytes());
    }
    fn u32(&mut self, address: usize, value: u32) {
        self.put(address, &value.to_le_bytes());
    }
    fn i64(&mut self, address: usize, value: i64) {
        self.put(address, &value.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.u64(address, data as u64);
        self.u32(address + 8, count);
        self.u32(address + 12, count);
    }
    fn string(&mut self, address: usize, value: &[u8]) {
        let mut storage = [0_u8; 16];
        let capacity = if value.len() <= 15 {
            storage[..value.len()].copy_from_slice(value);
            15_u64
        } else {
            let heap = HEAP + self.heap;
            self.heap += value.len() + 1;
            storage[..8].copy_from_slice(&(heap as u64).to_le_bytes());
            self.put(heap, value);
            self.put(heap + value.len(), &[0]);
            value.len() as u64 | 31
        };
        self.put(address, &storage);
        self.u64(address + msvc_string::LENGTH, value.len() as u64);
        self.u64(address + msvc_string::CAPACITY, capacity);
    }
    /// A CModifier at `address` with sorted entries and a display string.
    fn modifier(&mut self, address: usize, entries: &[(u32, i64)], display: &[u8]) {
        let data = ENTRIES + self.entries;
        self.entries += 0x200.max(entries.len() * modifier_entry::STRIDE);
        for (index, (key, value)) in entries.iter().enumerate() {
            self.u32(data + index * 16, *key);
            self.u32(data + index * 16 + 4, 0);
            self.i64(data + index * 16 + 8, *value);
        }
        self.array(address + cmodifier::ACCUMULATOR, data, entries.len() as u32);
        self.string(address + cmodifier::DISPLAY_NAME, display);
    }
    fn vtable(&mut self, address: usize, rva: usize) {
        self.u64(address, (BASE + rva) as u64);
    }
    fn children(&mut self, children: &[ChildSpec<'_>]) {
        self.array(C1 + MODIFIER_CHILDREN, CHILDREN, children.len() as u32);
        for (index, (entries, display)) in children.iter().enumerate() {
            let child = CHILD + index * 0x100;
            self.u64(CHILDREN + index * 8, child as u64);
            self.vtable(child, cmodifier::VTABLE_RVA);
            self.modifier(child, entries, display);
        }
    }
}

const STABILITY_KEY: u32 = 5;
const WAR_SUPPORT_KEY: u32 = 7;
const COMMAND_KEY: u32 = 9;

fn keys() -> Keys {
    let mut keys = [None; TRACKED];
    keys[0] = Some(STABILITY_KEY);
    keys[5] = Some(WAR_SUPPORT_KEY);
    keys[8] = Some(COMMAND_KEY);
    Keys {
        keys,
        table: Array {
            data: TABLE,
            count: 16,
        },
    }
}

/// Country 1 at an offensive war with country 2. Its tree has, in order: an
/// idea, the leader, a decision this reader does not replay, an advisor, the
/// at-war static and a dynamic modifier's constant part.
fn world() -> World {
    let mut w = World::default();
    // Countries and links.
    w.array(GS + cgame_state::COUNTRIES, COUNTRIES, 3);
    w.array(GS + cgame_state::COUNTRY_LINKS, LINKS, 3);
    w.u64(COUNTRIES, 0);
    for (slot, country, diplomacy) in [(1, C1, D1), (2, C2, D2)] {
        w.u64(COUNTRIES + slot * 8, country as u64);
        w.u32(LINKS + slot * 4, slot as u32);
        w.vtable(country, ccountry::VTABLE_RVA);
        w.u32(country + ccountry::ID, slot as u32);
        w.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
        w.vtable(diplomacy, cdiplomacy_status::VTABLE_RVA);
        w.u64(diplomacy + cdiplomacy_status::COUNTRY, country as u64);
        w.array(diplomacy + cdiplomacy_status::OWNED_RELATIONS, 0, 0);
        w.u32(diplomacy + DIPLOMACY_WAR_COUNTER, 0);
    }
    w.u32(D2 + DIPLOMACY_HOSTING, 0);
    w.array(D1 + cdiplomacy_status::OWNED_RELATIONS, RELATIONS, 1);
    w.u64(RELATIONS, R1 as u64);
    w.vtable(R1, cwar_relation::VTABLE_RVA);
    w.u32(R1 + crelation::TYPE_ID, cwar_relation::TYPE);
    w.u32(R1 + 0xc, 0);
    w.u32(R1 + crelation::FIRST_COUNTRY, 1);
    w.u32(R1 + crelation::SECOND_COUNTRY, 2);
    w.put(R1 + cwar_relation::FIRST_WAS_INSTIGATOR, &[1]);
    // Scalars.
    w.i64(C1 + ccountry::STABILITY, 50_000);
    w.i64(C1 + ccountry::WAR_SUPPORT, 40_000);
    for (index, value) in [-3277_i64, -1, -2, -3, -4].into_iter().enumerate() {
        w.i64(C1 + PROPAGANDA_STABILITY_PENALTY + index * 8, value);
    }
    w.i64(C1 + COASTAL_PROTECTION_STABILITY, 2_500);
    // Interception ring: one slot, column 1 = 777.
    w.u64(C1 + AIR_STATISTICS, STATS as u64);
    w.u64(STATS + RING, RING_OBJECT as u64);
    w.put(RING_OBJECT, &[0; 0x29]);
    w.u64(RING_OBJECT + RING_DATA, RING_SLOTS as u64);
    w.u32(RING_OBJECT + RING_SIZE, 2);
    w.u32(RING_OBJECT + RING_SIZE + 4, 1);
    w.u64(RING_SLOTS, RING_ROW as u64);
    w.u64(RING_ROW, RING_VALUES as u64);
    w.i64(RING_VALUES + 8, 777);
    // Root accumulator.
    let data = ENTRIES + 0x8000;
    for (index, (key, value)) in [
        (STABILITY_KEY, 30_000_i64),
        (WAR_SUPPORT_KEY, 12_000),
        (COMMAND_KEY, 600),
    ]
    .into_iter()
    .enumerate()
    {
        w.u32(data + index * 16, key);
        w.u32(data + index * 16 + 4, 0);
        w.i64(data + index * 16 + 8, value);
    }
    w.array(C1 + ccountry::MODIFIER_ACCUMULATOR, data, 3);
    // Politics: two ideas, the ruling party, its leader and ideology group.
    w.u64(C1 + ccountry::POLITICAL_STATUS, STATUS as u64);
    w.vtable(STATUS, cpolitical_status::VTABLE_RVA);
    w.u64(STATUS + cpolitical_status::RULING_PARTY, PARTY as u64);
    w.array(STATUS + cpolitical_status::IDEAS, IDEAS, 2);
    for (index, (idea, token, entries)) in [
        (IDEA_A, b"idea_a".as_slice(), [(STABILITY_KEY, 10_000_i64)]),
        (IDEA_B, b"idea_b".as_slice(), [(3, 1)]),
    ]
    .into_iter()
    .enumerate()
    {
        w.u64(IDEAS + index * 8, idea as u64);
        w.vtable(idea, cidea::VTABLE_RVA);
        w.string(idea + cidea::TOKEN, token);
        w.modifier(idea + IDEA_MODIFIER, &entries, b"built label");
    }
    w.vtable(PARTY, cpolitical_party::VTABLE_RVA);
    w.i64(PARTY + cpolitical_party::POPULARITY, 4_500_000);
    w.u64(PARTY + cpolitical_party::IDEOLOGY_GROUP, GROUP as u64);
    w.array(PARTY + cpolitical_party::COUNTRY_LEADERS, LEADER - 0x100, 1);
    w.u64(LEADER - 0x100, LEADER as u64);
    w.vtable(LEADER, ccountry_leader::VTABLE_RVA);
    w.u64(LEADER + ccountry_leader::CHARACTER, CHARACTER_77 as u64);
    w.modifier(
        LEADER + LEADER_MODIFIER,
        &[(WAR_SUPPORT_KEY, 2_000)],
        b"Leader Name",
    );
    w.vtable(GROUP, cideology_group::VTABLE_RVA);
    w.string(GROUP + cideology_group::TOKEN, b"fascism");
    w.modifier(GROUP + IDEOLOGY_GROUP_MODIFIER, &[], b"Fascism");
    for (character, id) in [(CHARACTER_77, 77), (CHARACTER_88, 88)] {
        w.vtable(character, ccharacter::VTABLE_RVA);
        w.u32(character + ccharacter::TYPE, ccharacter::REFERENCE_TYPE);
        w.u32(character + ccharacter::ID, id);
    }
    // One appointed advisor.
    w.u64(C1 + ccountry::CHARACTERS, CHARACTERS as u64);
    w.vtable(CHARACTERS, ccountry_characters::VTABLE_RVA);
    w.u64(CHARACTERS + ccountry_characters::OWNER, C1 as u64);
    w.array(
        CHARACTERS + ccountry_characters::APPOINTED_ADVISORS,
        ADVISORS,
        1,
    );
    w.u64(ADVISORS, ADVISOR as u64);
    w.put(ADVISOR, &[0; 0x20]);
    w.vtable(ADVISOR, cadvisor::VTABLE_RVA);
    w.u64(ADVISOR + cadvisor::CHARACTER, CHARACTER_88 as u64);
    w.string(ADVISOR + cadvisor::IDEA_TOKEN, b"adv_idea");
    w.modifier(
        ADVISOR + ADVISOR_MODIFIER,
        &[(WAR_SUPPORT_KEY, 10_000)],
        b"Advisor",
    );
    // One enabled dynamic modifier whose evaluated value is zero.
    let container = C1 + ccountry::DYNAMIC_MODIFIERS;
    w.array(container + cdynamic_modifier_container::ENTRIES, ENTRY, 1);
    w.put(ENTRY, &[0; dynamic_modifier_entry::STRIDE]);
    w.vtable(ENTRY, dynamic_modifier_entry::VTABLE_RVA);
    w.u32(ENTRY + dynamic_modifier_entry::SCOPE_COUNTRY, 1);
    w.u32(ENTRY + dynamic_modifier_entry::DAYS_REMAINING, u32::MAX);
    w.u64(
        ENTRY + dynamic_modifier_entry::DEFINITION,
        DEFINITION as u64,
    );
    w.put(ENTRY + dynamic_modifier_entry::ENABLED, &[1]);
    w.array(ENTRY + dynamic_modifier_entry::VALUES, VALUES, 1);
    w.i64(VALUES, 0);
    w.vtable(DEFINITION, cdynamic_modifier::VTABLE_RVA);
    w.string(DEFINITION + cdynamic_modifier::ID, b"dyn_mod");
    w.modifier(DEFINITION + DYNAMIC_CONSTANT, &[(COMMAND_KEY, 100)], b"Dyn");
    w.u32(DEFINITION + cdynamic_modifier::VALUE_COUNT, 1);
    w.u64(DEFINITION + DYNAMIC_VALUE_ROWS, VALUE_ROWS as u64);
    w.u32(VALUE_ROWS, STABILITY_KEY);
    // Statics: only the at-war slot is bound.
    w.u64(BASE + STATIC_DATABASE_RVA, DATABASE as u64);
    for slot in STATIC_SLOTS {
        w.u64(DATABASE + slot.offset, 0);
    }
    w.u64(DATABASE + 0x398, AT_WAR as u64);
    w.vtable(AT_WAR, cstatic_modifier::VTABLE_RVA);
    w.modifier(AT_WAR, &[(STABILITY_KEY, 5_000)], b"At war");
    w.string(AT_WAR + STATIC_NAME, b"country_is_at_war");
    // Globals: defines, world tension, language, modifier metadata.
    for (index, rva) in DEFINE_RVAS.into_iter().enumerate() {
        w.i64(BASE + rva, 1_000 + index as i64);
    }
    w.u64(GS + cgame_state::WORLD_THREAT, WORLD as u64);
    w.put(WORLD, &[0; 0x40]);
    w.vtable(WORLD, cworld_threat::VTABLE_RVA);
    w.i64(WORLD + cworld_threat::TOTAL, 12_345);
    w.u64(
        BASE + localization_root_layout::SINGLETON_RVA,
        LANGUAGE_ROOT as u64,
    );
    w.u64(LANGUAGE_ROOT, LANGUAGE as u64);
    w.string(LANGUAGE, b"l_english");
    w.u64(BASE + modifier_definition::TABLE_RVA, TABLE as u64);
    for key in [STABILITY_KEY, WAR_SUPPORT_KEY, COMMAND_KEY] {
        let row = TABLE + key as usize * modifier_definition::STRIDE;
        w.u32(row + modifier_definition::FEATURE_SELECTOR, 0);
        w.u32(row + modifier_definition::FLAGS, 3);
    }
    w.children(&[
        (&[(STABILITY_KEY, 10_000)], b"Politics: Idea A"),
        (&[(WAR_SUPPORT_KEY, 2_000)], b"Leader Name"),
        (
            &[(STABILITY_KEY, 15_000), (COMMAND_KEY, 500)],
            b"Decision: X",
        ),
        (&[(WAR_SUPPORT_KEY, 10_000)], b"Advisor"),
        (&[(STABILITY_KEY, 5_000)], b"At war"),
        (&[(COMMAND_KEY, 100)], b"Dyn"),
    ]);
    w
}

struct Walked {
    result: Result<CountryWalk, &'static str>,
    input: InputRow,
    lines: Vec<LineRow>,
    bytes: Vec<u8>,
}

fn walk(w: &World, capacity: usize) -> Walked {
    let countries = ccountry::Reader::new(w, BASE, GS).unwrap();
    let country = countries.at(1).unwrap().unwrap();
    let context = Context::read(w, BASE, GS, keys()).unwrap();
    let mut scratch = Scratch::new();
    let mut input = InputRow::default();
    let mut lines = vec![LineRow::default(); capacity];
    let mut bytes = vec![0; 256];
    let result = walk_country(
        w,
        BASE,
        &countries,
        country,
        &context,
        &mut scratch,
        &mut input,
        &mut lines,
        &mut bytes,
    );
    Walked {
        result,
        input,
        lines,
        bytes,
    }
}

fn text(walked: &Walked, span: Span) -> &[u8] {
    &walked.bytes[span.start..span.start + span.len]
}

#[test]
fn context_reads_defines_tension_language_and_statics() {
    let w = world();
    let context = Context::read(&w, BASE, GS, keys()).unwrap();
    assert_eq!(context.defines[0], 1_000);
    assert_eq!(context.defines[DEFINES - 1], 1_011);
    assert_eq!(context.world_tension_raw, 12_345);
    assert_eq!(context.language.as_slice(), b"l_english");
    // A null language root is the getter's null result, not a failure.
    let mut w = world();
    w.u64(BASE + localization_root_layout::SINGLETON_RVA, 0);
    assert_eq!(
        Context::read(&w, BASE, GS, keys()).unwrap().language.len(),
        0
    );
    // A bound static with another vtable is refused.
    let mut w = world();
    w.vtable(AT_WAR, cmodifier::VTABLE_RVA);
    assert!(Context::read(&w, BASE, GS, keys()).is_err());
}

#[test]
fn inputs_copy_every_operand_exactly() {
    let walked = walk(&world(), 16);
    walked.result.unwrap();
    let input = walked.input;
    assert_eq!(
        (input.country, input.stability_raw, input.war_support_raw),
        (1, 50_000, 40_000)
    );
    assert!(input.at_war && input.offensive_war && !input.defensive_war);
    assert_eq!(input.ruling_party_popularity_raw, 4_500_000);
    assert_eq!(
        [
            input.propaganda_stability_q15,
            input.bombing_q15,
            input.heroes_q15,
            input.convoys_q15,
            input.propaganda_war_support_q15
        ],
        [-3277, -1, -2, -3, -4]
    );
    assert_eq!(input.coastal_protection_stability_raw, 2_500);
    assert_eq!(input.interception_sum_raw, 777);
    assert_eq!(input.children, 6);
    // max_command_power's total (600) is CountryCommandPower's: not recorded here.
    let mut totals = [None; TOTALS];
    totals[0] = Some(30_000);
    totals[5] = Some(12_000);
    assert_eq!(input.totals, totals);
}

#[test]
fn children_pair_with_their_sources_and_the_rest_keep_their_text() {
    let walked = walk(&world(), 16);
    let CountryWalk { lines, .. } = walked.result.unwrap();
    assert_eq!(lines, Outcome::Rows { len: 7 });
    let rows: Vec<_> = walked.lines[..7]
        .iter()
        .map(|row| {
            (
                row.line,
                row.key,
                row.kind,
                text(&walked, row.identity).to_vec(),
                row.identity_id,
                row.value,
                text(&walked, row.text).to_vec(),
            )
        })
        .collect();
    let v = |bytes: &[u8]| bytes.to_vec();
    assert_eq!(
        rows,
        [
            (
                0,
                STABILITY_KEY,
                Kind::Idea,
                v(b"idea_a"),
                0,
                10_000,
                v(b"")
            ),
            (
                1,
                WAR_SUPPORT_KEY,
                Kind::CountryLeader,
                v(b""),
                77,
                2_000,
                v(b"")
            ),
            (
                2,
                STABILITY_KEY,
                Kind::Unpaired,
                v(b""),
                0,
                15_000,
                v(b"Decision: X")
            ),
            (
                2,
                COMMAND_KEY,
                Kind::Unpaired,
                v(b""),
                0,
                500,
                v(b"Decision: X")
            ),
            (
                3,
                WAR_SUPPORT_KEY,
                Kind::Advisor,
                v(b"adv_idea"),
                88,
                10_000,
                v(b"")
            ),
            (
                4,
                STABILITY_KEY,
                Kind::Static,
                v(b"country_is_at_war"),
                0,
                5_000,
                v(b"")
            ),
            (
                5,
                COMMAND_KEY,
                Kind::DynamicModifier,
                v(b"dyn_mod"),
                1,
                100,
                v(b"")
            ),
        ]
    );
    // The lines of each tracked key sum to its root total.
    for (key, total) in [
        (STABILITY_KEY, 30_000),
        (WAR_SUPPORT_KEY, 12_000),
        (COMMAND_KEY, 600),
    ] {
        let sum: i64 = walked.lines[..7]
            .iter()
            .filter(|row| row.key == key)
            .map(|row| row.value)
            .sum();
        assert_eq!(sum, total, "key {key}");
    }
}

#[test]
fn equal_sources_that_could_each_be_the_child_leave_it_unpaired() {
    let mut w = world();
    // Idea B now carries the same entries as idea A: the one idea child
    // could be either, so it keeps its text.
    let data = ENTRIES + 0x9000;
    w.u32(data, STABILITY_KEY);
    w.u32(data + 4, 0);
    w.i64(data + 8, 10_000);
    w.array(IDEA_B + IDEA_MODIFIER + cmodifier::ACCUMULATOR, data, 1);
    let walked = walk(&w, 16);
    walked.result.unwrap();
    assert_eq!(walked.lines[0].line, 0);
    assert_eq!(walked.lines[0].kind, Kind::Unpaired);
    assert_eq!(text(&walked, walked.lines[0].text), b"Politics: Idea A");
    // The leader still pairs: its display text proves it.
    assert_eq!(walked.lines[1].kind, Kind::CountryLeader);
}

#[test]
fn children_and_sources_without_tracked_values_do_not_move_the_pairing() {
    let mut w = world();
    // Idea B has no entries (a tooltip-only or equipment-only idea), and an
    // unreplayed source (a faction child) adds a child with none either. Paired
    // with each other they moved the backward bound below the leader.
    w.array(
        IDEA_B + IDEA_MODIFIER + cmodifier::ACCUMULATOR,
        ENTRIES + 0x9000,
        0,
    );
    w.children(&[
        (&[(STABILITY_KEY, 10_000)], b"Politics: Idea A"),
        (&[(WAR_SUPPORT_KEY, 2_000)], b"Leader Name"),
        (&[(3, 0)], b"Faction"),
        (
            &[(STABILITY_KEY, 15_000), (COMMAND_KEY, 500)],
            b"Decision: X",
        ),
        (&[(WAR_SUPPORT_KEY, 10_000)], b"Advisor"),
        (&[(STABILITY_KEY, 5_000)], b"At war"),
        (&[(COMMAND_KEY, 100)], b"Dyn"),
    ]);
    let walked = walk(&w, 16);
    walked.result.unwrap();
    let kinds: Vec<_> = walked.lines[..7]
        .iter()
        .map(|row| (row.line, row.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            (0, Kind::Idea),
            (1, Kind::CountryLeader),
            (3, Kind::Unpaired),
            (3, Kind::Unpaired),
            (4, Kind::Advisor),
            (5, Kind::Static),
            (6, Kind::DynamicModifier),
        ]
    );
}

#[test]
fn a_display_mismatch_or_changed_value_is_not_paired() {
    let mut w = world();
    // The advisor's source value moved after the recalculation.
    let entries = Array::read(&w, ADVISOR + ADVISOR_MODIFIER + cmodifier::ACCUMULATOR, 16).unwrap();
    w.i64(entries.data + 8, 9_999);
    // The leader's display differs.
    w.string(
        LEADER + LEADER_MODIFIER + cmodifier::DISPLAY_NAME,
        b"Other Name",
    );
    let walked = walk(&w, 16);
    walked.result.unwrap();
    let kinds: Vec<_> = walked.lines[..7]
        .iter()
        .map(|row| (row.line, row.kind))
        .collect();
    assert_eq!(kinds[1], (1, Kind::Unpaired));
    assert_eq!(kinds[4], (3, Kind::Unpaired));
    assert_eq!(kinds[5], (4, Kind::Static));
}

#[test]
fn lines_that_do_not_fit_are_counted_not_partially_written() {
    let walked = walk(&world(), 3);
    let walk = walked.result.unwrap();
    assert_eq!(
        walk.lines,
        Outcome::Truncated {
            len: 0,
            unread_rows: 7
        }
    );
    assert_eq!(walk.bytes_len, 0);
    assert!(walk.required_bytes >= b"idea_a".len() + 2 * b"Decision: X".len());
}

#[test]
fn every_short_read_fails_the_country() {
    for address in [
        CHILDREN + 8,
        CHILD + 0x100 + cmodifier::DISPLAY_NAME,
        R1 + cwar_relation::FIRST_WAS_INSTIGATOR,
        RING_VALUES + 8,
        PARTY + cpolitical_party::POPULARITY,
        VALUES,
        ADVISOR + cadvisor::IDEA_TOKEN,
    ] {
        let mut w = world();
        w.bytes.remove(&address);
        assert!(walk(&w, 16).result.is_err(), "{address:#x}");
    }
    // Unsorted child entries are not a layout this reader knows.
    let mut w = world();
    w.children(&[(&[(7, 1), (5, 1)], b"x")]);
    w.array(C1 + MODIFIER_CHILDREN, CHILDREN, 1);
    assert!(walk(&w, 16).result.is_err());
}

#[test]
fn the_fingerprint_follows_bases_totals_and_tree_shape() {
    let w = world();
    let keys = keys();
    let first = fingerprint_country(&w, BASE, C1, &keys).unwrap();
    assert_eq!(fingerprint_country(&w, BASE, C1, &keys), Ok(first));
    for change in [
        (C1 + ccountry::STABILITY, 1_i64),
        (C1 + COASTAL_PROTECTION_STABILITY, 7),
        (PARTY + cpolitical_party::POPULARITY, 1),
        (ENTRIES + 0x8000 + 8, 1),
    ] {
        let mut w = world();
        w.i64(change.0, change.1);
        assert_ne!(fingerprint_country(&w, BASE, C1, &keys), Ok(first));
    }
    let mut w = world();
    w.array(C1 + MODIFIER_CHILDREN, CHILDREN, 5);
    assert_ne!(fingerprint_country(&w, BASE, C1, &keys), Ok(first));
    // A source replaced by another with the same tracked values: an idea, the
    // leader or an advisor.
    for (slot, other) in [
        (IDEAS + 8, IDEA_A),
        (LEADER - 0x100, ADVISOR),
        (ADVISORS, LEADER),
    ] {
        let mut w = world();
        w.u64(slot, other as u64);
        assert_ne!(fingerprint_country(&w, BASE, C1, &keys), Ok(first));
    }
    // A dynamic modifier replaced in place (same array header) or disabled.
    let mut w = world();
    w.u64(
        ENTRY + dynamic_modifier_entry::DEFINITION,
        (DEFINITION + 0x10) as u64,
    );
    assert_ne!(fingerprint_country(&w, BASE, C1, &keys), Ok(first));
    let mut w = world();
    w.put(ENTRY + dynamic_modifier_entry::ENABLED, &[0]);
    assert_ne!(fingerprint_country(&w, BASE, C1, &keys), Ok(first));
    let mut w = world();
    w.bytes.remove(&(D1 + DIPLOMACY_WAR_COUNTER));
    assert!(fingerprint_country(&w, BASE, C1, &keys).is_err());
}

#[test]
fn a_dynamic_modifiers_evaluated_values_pair_as_their_own_line() {
    let mut w = world();
    w.i64(VALUES, 250);
    w.children(&[
        (&[(STABILITY_KEY, 10_000)], b"Politics: Idea A"),
        (&[(WAR_SUPPORT_KEY, 2_000)], b"Leader Name"),
        (
            &[(STABILITY_KEY, 15_000), (COMMAND_KEY, 500)],
            b"Decision: X",
        ),
        (&[(WAR_SUPPORT_KEY, 10_000)], b"Advisor"),
        (&[(STABILITY_KEY, 5_000)], b"At war"),
        (&[(COMMAND_KEY, 100)], b"Dyn"),
        (&[(STABILITY_KEY, 250)], b"Dyn"),
    ]);
    let walked = walk(&w, 16);
    assert_eq!(walked.result.unwrap().lines, Outcome::Rows { len: 8 });
    let last = walked.lines[7];
    assert_eq!(
        (last.line, last.kind, last.identity_id, last.value),
        (6, Kind::DynamicModifierValues, 1, 250)
    );
    assert_eq!(text(&walked, last.identity), b"dyn_mod");
    // Another source's child with the same value but its own label (a
    // landmark) never takes the evaluated part.
    w.children(&[
        (&[(STABILITY_KEY, 10_000)], b"Politics: Idea A"),
        (&[(WAR_SUPPORT_KEY, 2_000)], b"Leader Name"),
        (
            &[(STABILITY_KEY, 15_000), (COMMAND_KEY, 500)],
            b"Decision: X",
        ),
        (&[(WAR_SUPPORT_KEY, 10_000)], b"Advisor"),
        (&[(STABILITY_KEY, 250)], b"A Landmark"),
        (&[(STABILITY_KEY, 5_000)], b"At war"),
        (&[(COMMAND_KEY, 100)], b"Dyn"),
        (&[(STABILITY_KEY, 250)], b"Dyn"),
    ]);
    let walked = walk(&w, 16);
    walked.result.unwrap();
    let kinds: Vec<_> = walked.lines[..9]
        .iter()
        .map(|row| (row.line, row.kind))
        .collect();
    assert_eq!(kinds[5], (4, Kind::Unpaired));
    assert_eq!(kinds[6], (5, Kind::Static));
    assert_eq!(kinds[8], (7, Kind::DynamicModifierValues));
    // A value row key that is unreadable fails once the value is nonzero.
    w.bytes.remove(&VALUE_ROWS);
    assert!(walk(&w, 16).result.is_err());
}

#[test]
fn defensive_wars_and_hosted_counterparts_follow_the_engine_predicates() {
    let mut w = world();
    // Country 2 instigated: this country defends. The counterpart hosts
    // (diplomacy +0x1a8 >= 1), so IsAtWar(mode 1) is false.
    w.put(R1 + cwar_relation::FIRST_WAS_INSTIGATOR, &[0]);
    w.u32(D2 + DIPLOMACY_HOSTING, 1);
    let walked = walk(&w, 16);
    walked.result.unwrap();
    assert!(!walked.input.at_war && !walked.input.offensive_war && walked.input.defensive_war);
}

#[test]
fn growing_stability_storage_captures_five_thousand_modifier_children() {
    let mut w = world();
    const POINTERS: usize = 0x1000_0000;
    const OBJECTS: usize = 0x2000_0000;
    const COUNT: usize = 5_000;
    let display = b"A mod-defined stability modifier with a retained display name beyond the old per-country text estimate";
    w.entries = 0x3000_0000 - ENTRIES;
    w.heap = 0x4000_0000 - HEAP;
    w.array(C1 + MODIFIER_CHILDREN, POINTERS, COUNT as u32);
    for index in 0..COUNT {
        let child = OBJECTS + index * 0x100;
        w.u64(POINTERS + index * 8, child as u64);
        w.vtable(child, cmodifier::VTABLE_RVA);
        w.modifier(
            child,
            &[
                (STABILITY_KEY, 100),
                (WAR_SUPPORT_KEY, 200),
                (COMMAND_KEY, 300),
            ],
            display,
        );
    }
    let countries = ccountry::Reader::new(&w, BASE, GS).unwrap();
    let country = countries.at(1).unwrap().unwrap();
    let context = Context::read(&w, BASE, GS, keys()).unwrap();
    let mut scratch = Scratch::vacant();
    let mut input = InputRow::default();
    let required = walk_country(
        &w,
        BASE,
        &countries,
        country,
        &context,
        &mut scratch,
        &mut input,
        &mut [],
        &mut [],
    )
    .unwrap();
    assert_eq!(
        required.lines,
        Outcome::Truncated {
            len: 0,
            unread_rows: (COUNT * 3) as u32
        }
    );
    assert_eq!(required.required_bytes, COUNT * display.len());
    let mut lines = Vec::new();
    let mut bytes = Vec::new();
    let captured = walk_country_growing(
        &w,
        BASE,
        &countries,
        country,
        &context,
        &mut scratch,
        &mut input,
        &mut lines,
        &mut bytes,
        0,
        0,
    )
    .unwrap();
    assert_eq!(input.children, COUNT as u32);
    assert_eq!(captured.lines, Outcome::Rows { len: COUNT * 3 });
    assert_eq!(captured.bytes_len, required.required_bytes);
    for (index, rows) in lines[..COUNT * 3].chunks_exact(3).enumerate() {
        assert!(
            rows.iter()
                .all(|row| row.line == index as u32 && row.kind == Kind::Unpaired)
        );
        assert_eq!(
            rows.iter()
                .map(|row| (row.key, row.value))
                .collect::<Vec<_>>(),
            [
                (STABILITY_KEY, 100),
                (WAR_SUPPORT_KEY, 200),
                (COMMAND_KEY, 300)
            ]
        );
        for row in rows {
            assert_eq!(
                &bytes[row.text.start..row.text.start + row.text.len],
                display
            );
        }
    }
    assert!(scratch.children.len() >= COUNT);
}

#[test]
fn long_display_names_pair_only_after_exact_payload_comparison() {
    let mut w = world();
    let display = vec![b'X'; 8_197];
    w.string(LEADER + LEADER_MODIFIER + cmodifier::DISPLAY_NAME, &display);
    w.string(CHILD + 0x100 + cmodifier::DISPLAY_NAME, &display);
    let walked = walk(&w, 16);
    assert_eq!(walked.result.unwrap().lines, Outcome::Rows { len: 7 });
    assert_eq!(walked.lines[1].kind, Kind::CountryLeader);
    // Even a forced digest collision cannot replace exact comparison.
    let address = LEADER + LEADER_MODIFIER + cmodifier::DISPLAY_NAME;
    let header = read::<[u8; 32]>(&w, address).unwrap();
    let (len, digest) = display_digest(&w, address, header).unwrap();
    let child_entries = Array::read(&w, CHILD + 0x100 + cmodifier::ACCUMULATOR, 16).unwrap();
    let source_entries =
        Array::read(&w, LEADER + LEADER_MODIFIER + cmodifier::ACCUMULATOR, 16).unwrap();
    let source_signature = signature(&w, source_entries, |_, _| {}).unwrap();
    let child_address = CHILD + 0x100 + cmodifier::DISPLAY_NAME;
    let mut different = display.clone();
    different[8_192] = b'Y';
    w.string(child_address, &different);
    assert!(
        !matches(
            &w,
            &[],
            &Child {
                entries: child_entries,
                signature: source_signature,
                display: (child_address, digest),
                ..Default::default()
            },
            &Candidate {
                content: Content::Entries(source_entries),
                signature: source_signature,
                display: Some((address, len, digest)),
                ..Default::default()
            }
        )
        .unwrap()
    );
}

#[test]
fn native_interception_ring_has_no_collector_population_cap() {
    let mut w = world();
    const SLOTS: usize = 0x6000_0000;
    const COUNT: u32 = 5_000;
    w.u64(RING_OBJECT + RING_DATA, SLOTS as u64);
    w.u32(RING_OBJECT + RING_SIZE, COUNT);
    w.u32(RING_OBJECT + RING_SIZE + 4, COUNT);
    for slot in 0..COUNT as usize {
        w.u64(SLOTS + slot * 8, RING_ROW as u64);
    }
    assert_eq!(interception(&w, C1), Ok(i64::from(COUNT) * 777));
}

#[test]
fn active_language_preserves_the_native_string_without_a_size_cap() {
    let mut w = world();
    let language = vec![b'L'; 4_097];
    w.string(LANGUAGE, &language);
    assert_eq!(
        Context::read(&w, BASE, GS, keys()).unwrap().language,
        language
    );
}

#[test]
fn scaled_static_pairing_has_no_nonzero_key_population_cap() {
    let mut w = world();
    const SCALED: usize = 0x6100_0000;
    let source: Vec<_> = (1..=128).map(|key| (key, 1_i64)).collect();
    let child: Vec<_> = (1..=128).map(|key| (key, 7_i64)).collect();
    w.entries = 0x6200_0000 - ENTRIES;
    w.u64(DATABASE + 0x78, SCALED as u64);
    w.vtable(SCALED, cstatic_modifier::VTABLE_RVA);
    w.modifier(SCALED, &source, b"Scaled static");
    w.string(SCALED + STATIC_NAME, b"scaled_static");
    w.children(&[(&child, b"Scaled static")]);
    let walked = walk(&w, 16);
    assert_eq!(walked.result.unwrap().lines, Outcome::Rows { len: 3 });
    assert!(
        walked.lines[..3]
            .iter()
            .all(|row| row.kind == Kind::StaticScaled && row.value == 7)
    );
}
