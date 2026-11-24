use super::*;
use engine_native::cgame_state;
use engine_native_memory::msvc_string;
use std::collections::BTreeMap;

mod reference;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Fields,
    Wars,
    Totals,
    Popularity,
    Interception,
    Children,
    ChildHeads,
    Ideas,
    Leader,
    Ideology,
    Advisors,
    Statics,
    Dynamic,
    ScaledStatics,
    Matching,
    Emission,
}

thread_local! {
    static PHASE: std::cell::Cell<Phase> = const { std::cell::Cell::new(Phase::Fields) };
}

pub(super) fn set_phase(phase: Phase) {
    PHASE.with(|current| current.set(phase));
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CopyCall {
    phase: Phase,
    operands: Vec<(usize, usize, bool)>,
}

struct Copies<'a> {
    source: &'a World,
    calls: std::cell::RefCell<Vec<CopyCall>>,
    fault: std::cell::Cell<Option<usize>>,
    persistent: bool,
}

impl<'a> Copies<'a> {
    fn new(source: &'a World) -> Self {
        Self {
            source,
            calls: Default::default(),
            fault: Default::default(),
            persistent: false,
        }
    }

    fn operand(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return true;
        }
        let failed = self
            .fault
            .get()
            .is_some_and(|at| at >= address && at - address < destination.len());
        if failed && !self.persistent {
            self.fault.set(None);
        }
        let copied = !failed && self.source.copy(address, destination);
        self.calls.borrow_mut().last_mut().unwrap().operands.push((
            address,
            destination.len(),
            copied,
        ));
        copied
    }

    fn begin(&self) {
        self.calls.borrow_mut().push(CopyCall {
            phase: PHASE.with(|phase| phase.get()),
            operands: Vec::new(),
        });
    }

    fn counts(&self) -> [usize; 16] {
        let mut counts = [0; 16];
        for call in self.calls.borrow().iter() {
            counts[call.phase as usize] += 1;
        }
        counts
    }
}

impl Memory for Copies<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.begin();
        self.operand(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.begin();
        for (index, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if !destination.is_empty()
                && (address == 0
                    || address.checked_add(destination.len()).is_none()
                    || !self.operand(address, destination))
            {
                return index;
            }
        }
        N
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.begin();
        let Some(count) = layout.count(sources, destination.len()) else {
            return 0;
        };
        let mut completed = 0;
        for row in 0..sources.len() {
            for field in layout.fields() {
                if field.len != 0 {
                    let Some(address) = sources
                        .pointer(row)
                        .filter(|&p| p != 0)
                        .and_then(|p| p.checked_add(field.source_offset))
                        .filter(|p| p.checked_add(field.len).is_some())
                    else {
                        return completed;
                    };
                    let start = row * layout.stride() + field.destination_offset;
                    if !self.operand(address, &mut destination[start..start + field.len]) {
                        return completed;
                    }
                }
                completed += 1;
            }
        }
        assert_eq!(completed, count);
        completed
    }
}

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
            self.vtable(child, cmodifier::VTABLE_RVA.0);
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

fn world() -> World {
    let mut w = World::default();
    w.array(GS + cgame_state::COUNTRIES, COUNTRIES, 3);
    w.array(GS + cgame_state::COUNTRY_LINKS, LINKS, 3);
    w.u64(COUNTRIES, 0);
    for (slot, country, diplomacy) in [(1, C1, D1), (2, C2, D2)] {
        w.u64(COUNTRIES + slot * 8, country as u64);
        w.u32(LINKS + slot * 4, slot as u32);
        w.vtable(country, ccountry::VTABLE_RVA.0);
        w.u32(country + ccountry::ID, slot as u32);
        w.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
        w.vtable(diplomacy, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0);
        w.u64(diplomacy + engine_native_parts::cdiplomacy_status_layout::COUNTRY, country as u64);
        w.array(diplomacy + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, 0, 0);
        w.u32(diplomacy + cdiplomacy_status::ENEMY_COUNTRY_COUNT, 0);
        w.put(country + PRIDE_OF_THE_FLEET_SUNK_ACTIVE, &[0]);
        w.i64(
            country + engine_native_memory::country_identity::MANPOWER + engine_native_parts::ccountry_manpower::MOBILIZATION,
            250_000,
        );
        w.u32(diplomacy + engine_native_parts::ccountry_manpower::HOST, 0);
    }
    w.u32(D2 + DIPLOMACY_HOSTING, 0);
    w.array(D1 + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, RELATIONS, 1);
    w.u64(RELATIONS, R1 as u64);
    w.vtable(R1, cwar_relation::VTABLE_RVA.0);
    w.u32(R1 + crelation::TYPE_ID, cwar_relation::TYPE);
    w.u32(R1 + 0xc, 0);
    w.u32(R1 + crelation::FIRST_COUNTRY, 1);
    w.u32(R1 + crelation::SECOND_COUNTRY, 2);
    w.put(R1 + cwar_relation::FIRST_WAS_INSTIGATOR, &[1]);
    w.i64(C1 + ccountry::STABILITY, 50_000);
    w.i64(C1 + ccountry::WAR_SUPPORT, 40_000);
    for (index, value) in [-3277_i64, -1, -2, -3, -4].into_iter().enumerate() {
        w.i64(C1 + PROPAGANDA_STABILITY_PENALTY + index * 8, value);
    }
    w.i64(C1 + COASTAL_PROTECTION_STABILITY, 2_500);
    w.u64(C1 + AIR_STATISTICS, STATS as u64);
    w.u64(STATS + RING, RING_OBJECT as u64);
    w.put(RING_OBJECT, &[0; 0x29]);
    w.u64(RING_OBJECT + RING_DATA, RING_SLOTS as u64);
    w.u32(RING_OBJECT + RING_SIZE, 2);
    w.u32(RING_OBJECT + RING_SIZE + 4, 1);
    w.u64(RING_SLOTS, RING_ROW as u64);
    w.u64(RING_ROW, RING_VALUES as u64);
    w.i64(RING_VALUES + 8, 777);
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
    w.array(C1 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, data, 3);
    w.u64(C1 + ccountry::POLITICAL_STATUS, STATUS as u64);
    w.vtable(STATUS, engine_native_parts::cpolitical_status_layout::VTABLE_RVA.0);
    w.u64(STATUS + engine_native_parts::cpolitical_status_layout::RULING_PARTY, PARTY as u64);
    w.array(STATUS + cpolitical_status::IDEAS, IDEAS, 2);
    for (index, (idea, token, entries)) in [
        (IDEA_A, b"idea_a".as_slice(), [(STABILITY_KEY, 10_000_i64)]),
        (IDEA_B, b"idea_b".as_slice(), [(3, 1)]),
    ]
    .into_iter()
    .enumerate()
    {
        w.u64(IDEAS + index * 8, idea as u64);
        w.vtable(idea, cidea::VTABLE_RVA.0);
        w.string(idea + cidea::TOKEN, token);
        w.modifier(idea + IDEA_MODIFIER, &entries, b"built label");
    }
    w.vtable(PARTY, engine_native_parts::cpolitical_party_layout::VTABLE_RVA.0);
    w.i64(PARTY + cpolitical_party::POPULARITY, 4_500_000);
    w.u64(PARTY + engine_native_parts::cpolitical_party_layout::IDEOLOGY_GROUP, GROUP as u64);
    w.array(PARTY + engine_native_parts::cpolitical_party_layout::COUNTRY_LEADERS, LEADER - 0x100, 1);
    w.u64(LEADER - 0x100, LEADER as u64);
    w.vtable(LEADER, engine_native_parts::ccountry_leader_layout::VTABLE_RVA.0);
    w.u64(LEADER + engine_native_parts::ccountry_leader_layout::CHARACTER, CHARACTER_77 as u64);
    w.modifier(
        LEADER + LEADER_MODIFIER,
        &[(WAR_SUPPORT_KEY, 2_000)],
        b"Leader Name",
    );
    w.vtable(GROUP, cideology_group::VTABLE_RVA.0);
    w.string(GROUP + cideology_group::TOKEN, b"fascism");
    w.modifier(GROUP + IDEOLOGY_GROUP_MODIFIER, &[], b"Fascism");
    for (character, id) in [(CHARACTER_77, 77), (CHARACTER_88, 88)] {
        w.vtable(character, ccharacter::VTABLE_RVA.0);
        w.u32(character + ccharacter::TYPE, ccharacter::REFERENCE_TYPE);
        w.u32(character + ccharacter::ID, id);
    }
    w.u64(C1 + ccountry::CHARACTERS, CHARACTERS as u64);
    w.vtable(CHARACTERS, engine_native::ccountry::rosters::VTABLE_RVA.0);
    w.u64(CHARACTERS + engine_native::ccountry::rosters::OWNER, C1 as u64);
    w.array(
        CHARACTERS + ccountry_characters::APPOINTED_ADVISORS,
        ADVISORS,
        1,
    );
    w.u64(ADVISORS, ADVISOR as u64);
    w.put(ADVISOR, &[0; 0x20]);
    w.vtable(ADVISOR, cadvisor::VTABLE_RVA.0);
    w.u64(ADVISOR + cadvisor::CHARACTER, CHARACTER_88 as u64);
    w.string(ADVISOR + cadvisor::IDEA_TOKEN, b"adv_idea");
    w.modifier(
        ADVISOR + ADVISOR_MODIFIER,
        &[(WAR_SUPPORT_KEY, 10_000)],
        b"Advisor",
    );
    let container = C1 + ccountry::DYNAMIC_MODIFIERS;
    w.array(container + cdynamic_modifier_container::ENTRIES, ENTRY, 1);
    w.put(ENTRY, &[0; dynamic_modifier_entry::STRIDE]);
    w.vtable(ENTRY, dynamic_modifier_entry::VTABLE_RVA.0);
    w.u32(ENTRY + dynamic_modifier_entry::SCOPE_COUNTRY, 1);
    w.u32(ENTRY + dynamic_modifier_entry::DAYS_REMAINING, u32::MAX);
    w.u64(
        ENTRY + dynamic_modifier_entry::DEFINITION,
        DEFINITION as u64,
    );
    w.put(ENTRY + dynamic_modifier_entry::ENABLED, &[1]);
    w.array(ENTRY + dynamic_modifier_entry::VALUES, VALUES, 1);
    w.i64(VALUES, 0);
    w.vtable(DEFINITION, cdynamic_modifier::VTABLE_RVA.0);
    w.string(DEFINITION + cdynamic_modifier::ID, b"dyn_mod");
    w.modifier(DEFINITION + DYNAMIC_CONSTANT, &[(COMMAND_KEY, 100)], b"Dyn");
    w.u32(DEFINITION + cdynamic_modifier::VALUE_COUNT, 1);
    w.u64(DEFINITION + DYNAMIC_VALUE_ROWS, VALUE_ROWS as u64);
    w.u32(VALUE_ROWS, STABILITY_KEY);
    w.u64(BASE + STATIC_DATABASE_RVA.0, DATABASE as u64);
    for slot in STATIC_SLOTS {
        w.u64(DATABASE + slot.offset, 0);
    }
    w.u64(DATABASE + 0x398, AT_WAR as u64);
    w.vtable(AT_WAR, cstatic_modifier::VTABLE_RVA.0);
    w.modifier(AT_WAR, &[(STABILITY_KEY, 5_000)], b"At war");
    w.string(AT_WAR + STATIC_NAME, b"country_is_at_war");
    for (index, rva) in DEFINE_RVAS.into_iter().enumerate() {
        w.i64(BASE + rva.0, 1_000 + index as i64);
    }
    w.u64(GS + cgame_state::WORLD_THREAT, WORLD as u64);
    w.put(WORLD, &[0; 0x40]);
    w.vtable(WORLD, cworld_threat::VTABLE_RVA.0);
    w.i64(WORLD + cworld_threat::TOTAL, 12_345);
    w.u64(
        BASE + localization_root_layout::SINGLETON_RVA.0,
        LANGUAGE_ROOT as u64,
    );
    w.u64(LANGUAGE_ROOT, LANGUAGE as u64);
    w.string(LANGUAGE, b"l_english");
    w.u64(BASE + modifier_definition::TABLE_RVA.0, TABLE as u64);
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
    fallbacks: u64,
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
        0,
        0,
    );
    Walked {
        result,
        input,
        lines,
        bytes,
        fallbacks: scratch.batch_fallbacks(),
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
    let mut w = world();
    w.u64(BASE + localization_root_layout::SINGLETON_RVA.0, 0);
    assert_eq!(
        Context::read(&w, BASE, GS, keys()).unwrap().language.len(),
        0
    );
    let mut w = world();
    w.vtable(AT_WAR, cmodifier::VTABLE_RVA.0);
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

    let mut w = world();
    const INACTIVE_PEACE: usize = 0x3b000;
    const INACTIVE_ATTACHE: usize = 0x3b400;
    const UNWITNESSED_CG: usize = 0x3b800;
    const UNWITNESSED_SCALED: usize = 0x3bc00;
    for (slot_offset, address, name, entries, display) in [
        (
            0x390,
            INACTIVE_PEACE,
            b"country_is_at_peace".as_slice(),
            [(STABILITY_KEY, 15_000), (COMMAND_KEY, 500)].as_slice(),
            b"Decision: X".as_slice(),
        ),
        (
            0x90,
            INACTIVE_ATTACHE,
            b"attache_sent".as_slice(),
            [(STABILITY_KEY, 4_000)].as_slice(),
            b"Mod line A".as_slice(),
        ),
        (
            0x60,
            UNWITNESSED_CG,
            b"lacking_consumer_goods".as_slice(),
            [(STABILITY_KEY, 3_000)].as_slice(),
            b"Mod line B".as_slice(),
        ),
        (
            0x78,
            UNWITNESSED_SCALED,
            b"war_support_good".as_slice(),
            [(STABILITY_KEY, 1)].as_slice(),
            b"Mod line C".as_slice(),
        ),
    ] {
        w.u64(DATABASE + slot_offset, address as u64);
        w.vtable(address, cstatic_modifier::VTABLE_RVA.0);
        w.modifier(address, entries, display);
        w.string(address + STATIC_NAME, name);
    }
    w.children(&[
        (&[(STABILITY_KEY, 10_000)], b"Politics: Idea A"),
        (&[(WAR_SUPPORT_KEY, 2_000)], b"Leader Name"),
        (&[(WAR_SUPPORT_KEY, 10_000)], b"Advisor"),
        (&[(STABILITY_KEY, 4_000)], b"Mod line A"),
        (&[(STABILITY_KEY, 3_000)], b"Mod line B"),
        (
            &[(STABILITY_KEY, 15_000), (COMMAND_KEY, 500)],
            b"Decision: X",
        ),
        (&[(STABILITY_KEY, 5_000)], b"At war"),
        (&[(COMMAND_KEY, 100)], b"Dyn"),
        (&[(STABILITY_KEY, 9_000)], b"Mod line C"),
    ]);
    let walked = walk(&w, 16);
    let CountryWalk { lines, .. } = walked.result.unwrap();
    assert_eq!(lines, Outcome::Rows { len: 10 });
    let observed: Vec<_> = walked.lines[..10]
        .iter()
        .map(|row| {
            (
                row.line,
                row.key,
                row.kind,
                text(&walked, row.identity).to_vec(),
                row.value,
                text(&walked, row.text).to_vec(),
            )
        })
        .collect();
    assert_eq!(
        observed,
        [
            (0, STABILITY_KEY, Kind::Idea, v(b"idea_a"), 10_000, v(b"")),
            (1, WAR_SUPPORT_KEY, Kind::CountryLeader, v(b""), 2_000, v(b"")),
            (2, WAR_SUPPORT_KEY, Kind::Advisor, v(b"adv_idea"), 10_000, v(b"")),
            (3, STABILITY_KEY, Kind::Unpaired, v(b""), 4_000, v(b"Mod line A")),
            (4, STABILITY_KEY, Kind::Unpaired, v(b""), 3_000, v(b"Mod line B")),
            (5, STABILITY_KEY, Kind::Unpaired, v(b""), 15_000, v(b"Decision: X")),
            (5, COMMAND_KEY, Kind::Unpaired, v(b""), 500, v(b"Decision: X")),
            (6, STABILITY_KEY, Kind::Static, v(b"country_is_at_war"), 5_000, v(b"")),
            (7, COMMAND_KEY, Kind::DynamicModifier, v(b"dyn_mod"), 100, v(b"")),
            (8, STABILITY_KEY, Kind::Unpaired, v(b""), 9_000, v(b"Mod line C")),
        ]
    );
}

#[test]
fn witnessed_conditional_statics_attribute_only_when_their_native_predicate_holds() {
    let mut w = world();
    const PEACE: usize = 0x3b000;
    const ATTACHE: usize = 0x3b400;
    const SUNK: usize = 0x3b800;
    const WAR_SCALED: usize = 0x3bc00;
    const ATTACHE_REL: usize = 0x31400;
    for (slot_offset, address, name, entries, display) in [
        (
            0x390,
            PEACE,
            b"country_is_at_peace".as_slice(),
            [(STABILITY_KEY, 2_000)].as_slice(),
            b"At peace".as_slice(),
        ),
        (
            0x90,
            ATTACHE,
            b"attache_sent".as_slice(),
            [(WAR_SUPPORT_KEY, 4_000)].as_slice(),
            b"Attache".as_slice(),
        ),
        (
            0xc0,
            SUNK,
            b"pride_of_the_fleet_sunk_temporary".as_slice(),
            [(WAR_SUPPORT_KEY, -3_000)].as_slice(),
            b"Pride sunk".as_slice(),
        ),
        (
            0x88,
            WAR_SCALED,
            b"war_support_during_war".as_slice(),
            [(STABILITY_KEY, -1)].as_slice(),
            b"War support".as_slice(),
        ),
    ] {
        w.u64(DATABASE + slot_offset, address as u64);
        w.vtable(address, cstatic_modifier::VTABLE_RVA.0);
        w.modifier(address, entries, display);
        w.string(address + STATIC_NAME, name);
    }
    w.array(D1 + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, 0, 0);
    w.children(&[
        (&[(WAR_SUPPORT_KEY, 4_000)], b"Attache"),
        (&[(STABILITY_KEY, 2_000)], b"At peace"),
        (&[(STABILITY_KEY, 5_000)], b"At war"),
        (&[(WAR_SUPPORT_KEY, -3_000)], b"Pride sunk"),
        (&[(STABILITY_KEY, -700)], b"War support"),
    ]);
    let walked = walk(&w, 16);
    assert_eq!(walked.result.unwrap().lines, Outcome::Rows { len: 5 });
    assert_eq!(walked.lines[0].kind, Kind::Unpaired);
    assert_eq!(text(&walked, walked.lines[0].text), b"Attache");
    assert_eq!(walked.lines[1].kind, Kind::Static);
    assert_eq!(text(&walked, walked.lines[1].identity), b"country_is_at_peace");
    assert_eq!(walked.lines[2].kind, Kind::Unpaired);
    assert_eq!(text(&walked, walked.lines[2].text), b"At war");
    assert_eq!(walked.lines[3].kind, Kind::Unpaired);
    assert_eq!(text(&walked, walked.lines[3].text), b"Pride sunk");
    assert_eq!(walked.lines[4].kind, Kind::Unpaired);
    assert_eq!(text(&walked, walked.lines[4].text), b"War support");

    w.u32(D1 + cdiplomacy_status::ENEMY_COUNTRY_COUNT, 1);
    w.put(C1 + PRIDE_OF_THE_FLEET_SUNK_ACTIVE, &[1]);
    w.array(D1 + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, RELATIONS, 1);
    w.u64(RELATIONS, ATTACHE_REL as u64);
    w.vtable(ATTACHE_REL, cattache_relation::VTABLE_RVA.0);
    w.u32(ATTACHE_REL + crelation::TYPE_ID, cattache_relation::TYPE);
    w.u32(ATTACHE_REL + 0xc, 0);
    w.u32(ATTACHE_REL + crelation::FIRST_COUNTRY, 2);
    w.u32(ATTACHE_REL + crelation::SECOND_COUNTRY, 1);
    let walked = walk(&w, 16);
    assert_eq!(walked.result.unwrap().lines, Outcome::Rows { len: 5 });
    assert_eq!(walked.lines[0].kind, Kind::Unpaired);
    assert_eq!(walked.lines[1].kind, Kind::Unpaired);
    assert_eq!(walked.lines[2].kind, Kind::Static);
    assert_eq!(text(&walked, walked.lines[2].identity), b"country_is_at_war");
    assert_eq!(walked.lines[3].kind, Kind::Static);
    assert_eq!(
        text(&walked, walked.lines[3].identity),
        b"pride_of_the_fleet_sunk_temporary"
    );
    assert_eq!(walked.lines[4].kind, Kind::Unpaired);
    assert_eq!(text(&walked, walked.lines[4].text), b"War support");
    assert_eq!(walked.lines[4].value, -700);

    w.u32(ATTACHE_REL + crelation::FIRST_COUNTRY, 1);
    w.u32(ATTACHE_REL + crelation::SECOND_COUNTRY, 2);
    let walked = walk(&w, 16);
    assert_eq!(walked.result.unwrap().lines, Outcome::Rows { len: 5 });
    assert_eq!(walked.lines[0].kind, Kind::Static);
    assert_eq!(text(&walked, walked.lines[0].identity), b"attache_sent");
    assert_eq!(walked.lines[4].kind, Kind::Unpaired);
}

#[test]
fn failed_witness_reads_preserve_stability_and_leave_only_that_static_unpaired() {
    const PEACE: usize = 0x3b000;
    const ATTACHE: usize = 0x3b400;
    const SUNK: usize = 0x3b800;
    const ATTACHE_REL: usize = 0x31400;
    let fixture = || {
        let mut w = world();
        for (slot_offset, address, name, entries, display) in [
            (
                0x390,
                PEACE,
                b"country_is_at_peace".as_slice(),
                [(STABILITY_KEY, 2_000)].as_slice(),
                b"At peace".as_slice(),
            ),
            (
                0x90,
                ATTACHE,
                b"attache_sent".as_slice(),
                [(WAR_SUPPORT_KEY, 4_000)].as_slice(),
                b"Attache".as_slice(),
            ),
            (
                0xc0,
                SUNK,
                b"pride_of_the_fleet_sunk_temporary".as_slice(),
                [(WAR_SUPPORT_KEY, -3_000)].as_slice(),
                b"Pride sunk".as_slice(),
            ),
        ] {
            w.u64(DATABASE + slot_offset, address as u64);
            w.vtable(address, cstatic_modifier::VTABLE_RVA.0);
            w.modifier(address, entries, display);
            w.string(address + STATIC_NAME, name);
        }
        w.children(&[
            (&[(WAR_SUPPORT_KEY, 4_000)], b"Attache"),
            (&[(STABILITY_KEY, 2_000)], b"At peace"),
            (&[(STABILITY_KEY, 5_000)], b"At war"),
            (&[(WAR_SUPPORT_KEY, -3_000)], b"Pride sunk"),
        ]);
        w
    };

    let mut pride_fault = fixture();
    pride_fault
        .bytes
        .remove(&(C1 + PRIDE_OF_THE_FLEET_SUNK_ACTIVE));
    let walked = walk(&pride_fault, 16);
    assert_eq!(walked.result.unwrap().lines, Outcome::Rows { len: 4 });
    assert_eq!(walked.input.stability_raw, 50_000);
    assert_eq!(walked.lines[2].kind, Kind::Static);
    assert_eq!(text(&walked, walked.lines[2].identity), b"country_is_at_war");
    assert_eq!(walked.lines[3].kind, Kind::Unpaired);
    assert_eq!(text(&walked, walked.lines[3].text), b"Pride sunk");

    let mut enemy_fault = fixture();
    enemy_fault.array(D1 + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, 0, 0);
    enemy_fault.put(C1 + PRIDE_OF_THE_FLEET_SUNK_ACTIVE, &[1]);
    enemy_fault
        .bytes
        .remove(&(D1 + cdiplomacy_status::ENEMY_COUNTRY_COUNT));
    let walked = walk(&enemy_fault, 16);
    assert_eq!(walked.result.unwrap().lines, Outcome::Rows { len: 4 });
    assert_eq!(walked.input.stability_raw, 50_000);
    assert_eq!(walked.lines[1].kind, Kind::Unpaired);
    assert_eq!(text(&walked, walked.lines[1].text), b"At peace");
    assert_eq!(walked.lines[2].kind, Kind::Unpaired);
    assert_eq!(text(&walked, walked.lines[2].text), b"At war");
    assert_eq!(walked.lines[3].kind, Kind::Static);
    assert_eq!(
        text(&walked, walked.lines[3].identity),
        b"pride_of_the_fleet_sunk_temporary"
    );

    for bad_link in [false, true] {
        let mut attache_fault = fixture();
        attache_fault.u32(D1 + cdiplomacy_status::ENEMY_COUNTRY_COUNT, 1);
        attache_fault.array(D1 + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, RELATIONS, 1);
        attache_fault.u64(RELATIONS, ATTACHE_REL as u64);
        attache_fault.vtable(
            ATTACHE_REL,
            if bad_link {
                cattache_relation::VTABLE_RVA.0
            } else {
                cwar_relation::VTABLE_RVA.0
            },
        );
        attache_fault.u32(ATTACHE_REL + crelation::TYPE_ID, cattache_relation::TYPE);
        attache_fault.u32(ATTACHE_REL + 0xc, 0);
        attache_fault.u32(
            ATTACHE_REL + crelation::FIRST_COUNTRY,
            if bad_link { 99 } else { 1 },
        );
        attache_fault.u32(ATTACHE_REL + crelation::SECOND_COUNTRY, 2);
        let walked = walk(&attache_fault, 16);
        assert_eq!(walked.result.unwrap().lines, Outcome::Rows { len: 4 });
        assert_eq!(walked.input.stability_raw, 50_000);
        assert_eq!(walked.lines[0].kind, Kind::Unpaired);
        assert_eq!(text(&walked, walked.lines[0].text), b"Attache");
        assert_eq!(walked.lines[2].kind, Kind::Static);
        assert_eq!(text(&walked, walked.lines[2].identity), b"country_is_at_war");
    }

    let mut endpoint_fault = fixture();
    endpoint_fault.u32(D1 + cdiplomacy_status::ENEMY_COUNTRY_COUNT, 1);
    endpoint_fault.array(D1 + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, RELATIONS, 1);
    endpoint_fault.u64(RELATIONS, ATTACHE_REL as u64);
    endpoint_fault.vtable(ATTACHE_REL, cattache_relation::VTABLE_RVA.0);
    endpoint_fault.u32(ATTACHE_REL + crelation::TYPE_ID, cattache_relation::TYPE);
    let walked = walk(&endpoint_fault, 16);
    assert_eq!(walked.result.unwrap().lines, Outcome::Rows { len: 4 });
    assert_eq!(walked.input.stability_raw, 50_000);
    assert_eq!(walked.lines[0].kind, Kind::Unpaired);
    assert_eq!(text(&walked, walked.lines[0].text), b"Attache");
    assert_eq!(walked.lines[2].kind, Kind::Static);
    assert_eq!(text(&walked, walked.lines[2].identity), b"country_is_at_war");
    assert_same_walk(
        &observed_walk(&Copies::new(&endpoint_fault), true, &mut Memo::default(), 16, None),
        &observed_walk(&Copies::new(&endpoint_fault), false, &mut Memo::default(), 16, None),
    );

    let mut withheld_site = fixture();
    withheld_site.u32(D1 + cdiplomacy_status::ENEMY_COUNTRY_COUNT, 1);
    withheld_site.array(D1 + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, RELATIONS, 1);
    withheld_site.u64(RELATIONS, ATTACHE_REL as u64);
    withheld_site.vtable(ATTACHE_REL, cattache_relation::VTABLE_RVA.0);
    withheld_site.u32(ATTACHE_REL + crelation::TYPE_ID, cattache_relation::TYPE);
    withheld_site.u32(ATTACHE_REL + crelation::FIRST_COUNTRY, 1);
    withheld_site.u32(ATTACHE_REL + crelation::SECOND_COUNTRY, 2);
    let _ = engine_sites::take_taint();
    let (old, new, taint) = engine_sites::with_synthetic_withheld(
        cattache_relation::VTABLE_RVA,
        || {
            let old = observed_walk(&Copies::new(&withheld_site), true, &mut Memo::default(), 16, None);
            let new = observed_walk(&Copies::new(&withheld_site), false, &mut Memo::default(), 16, None);
            (old, new, engine_sites::take_taint())
        },
    );
    assert_eq!(taint, None);
    assert_same_walk(&old, &new);
    assert_eq!(new.result.unwrap().lines, Outcome::Rows { len: 4 });
    assert_eq!(new.lines[0].kind, Kind::Unpaired);
    assert_eq!(text(&new, new.lines[0].text), b"Attache");
    assert_eq!(new.lines[2].kind, Kind::Static);
    assert_eq!(text(&new, new.lines[2].identity), b"country_is_at_war");
}

#[test]
fn verified_attache_sender_prevails_over_malformed_attache_relations_in_any_order() {
    const ATTACHE: usize = 0x3b400;
    const GOOD_REL: usize = 0x31400;
    const BAD_REL: usize = 0x31500;

    #[derive(Clone, Copy)]
    enum Fault {
        WrongVtable,
        UnreadableEndpoint,
        BadLink,
    }

    for fault in [Fault::WrongVtable, Fault::UnreadableEndpoint, Fault::BadLink] {
        for (sender, expected_kind) in [(1, Kind::Static), (2, Kind::Unpaired)] {
            for order in [[GOOD_REL, BAD_REL], [BAD_REL, GOOD_REL]] {
                let mut w = world();
                w.u64(DATABASE + 0x90, ATTACHE as u64);
                w.vtable(ATTACHE, cstatic_modifier::VTABLE_RVA.0);
                w.modifier(ATTACHE, &[(WAR_SUPPORT_KEY, 4_000)], b"Attache");
                w.string(ATTACHE + STATIC_NAME, b"attache_sent");
                w.u32(D1 + cdiplomacy_status::ENEMY_COUNTRY_COUNT, 1);
                w.array(D1 + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, RELATIONS, 2);
                w.u64(RELATIONS, order[0] as u64);
                w.u64(RELATIONS + 8, order[1] as u64);

                w.vtable(GOOD_REL, cattache_relation::VTABLE_RVA.0);
                w.u32(GOOD_REL + crelation::TYPE_ID, cattache_relation::TYPE);
                w.u32(GOOD_REL + crelation::FIRST_COUNTRY, sender);
                w.u32(GOOD_REL + crelation::SECOND_COUNTRY, 3 - sender);

                w.vtable(
                    BAD_REL,
                    match fault {
                        Fault::WrongVtable => cwar_relation::VTABLE_RVA.0,
                        Fault::UnreadableEndpoint | Fault::BadLink => {
                            cattache_relation::VTABLE_RVA.0
                        }
                    },
                );
                w.u32(BAD_REL + crelation::TYPE_ID, cattache_relation::TYPE);
                match fault {
                    Fault::WrongVtable => {
                        w.u32(BAD_REL + crelation::FIRST_COUNTRY, 2);
                        w.u32(BAD_REL + crelation::SECOND_COUNTRY, 1);
                    }
                    Fault::UnreadableEndpoint => {}
                    Fault::BadLink => {
                        w.u32(BAD_REL + crelation::FIRST_COUNTRY, 99);
                        w.u32(BAD_REL + crelation::SECOND_COUNTRY, 2);
                    }
                }
                w.children(&[
                    (&[(WAR_SUPPORT_KEY, 4_000)], b"Attache"),
                    (&[(STABILITY_KEY, 5_000)], b"At war"),
                ]);
                let old = observed_walk(&Copies::new(&w), true, &mut Memo::default(), 16, None);
                let new = observed_walk(&Copies::new(&w), false, &mut Memo::default(), 16, None);
                assert_same_walk(&old, &new);
                assert_eq!(new.result.unwrap().lines, Outcome::Rows { len: 2 });
                assert_eq!(new.lines[0].kind, expected_kind);
                if expected_kind == Kind::Static {
                    assert_eq!(text(&new, new.lines[0].identity), b"attache_sent");
                } else {
                    assert_eq!(text(&new, new.lines[0].text), b"Attache");
                }
                assert_eq!(new.lines[1].kind, Kind::Static);
                assert_eq!(text(&new, new.lines[1].identity), b"country_is_at_war");
            }
        }
    }
}

#[test]
fn equal_sources_that_could_each_be_the_child_leave_it_unpaired() {
    let mut w = world();
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
    assert_eq!(walked.lines[1].kind, Kind::CountryLeader);
}

#[test]
fn children_and_sources_without_tracked_values_do_not_move_the_pairing() {
    let mut w = world();
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
    let entries = Array::read(&w, ADVISOR + ADVISOR_MODIFIER + cmodifier::ACCUMULATOR, 16).unwrap();
    w.i64(entries.data + 8, 9_999);
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
    let mut w = world();
    w.children(&[(&[(7, 1), (5, 1)], b"x")]);
    w.array(C1 + MODIFIER_CHILDREN, CHILDREN, 1);
    assert!(walk(&w, 16).result.is_err());
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
    w.bytes.remove(&VALUE_ROWS);
    assert!(walk(&w, 16).result.is_err());
}

#[test]
fn defensive_wars_and_hosted_counterparts_follow_the_engine_predicates() {
    let mut w = world();
    w.put(R1 + cwar_relation::FIRST_WAS_INSTIGATOR, &[0]);
    w.u32(D2 + DIPLOMACY_HOSTING, 1);
    let walked = walk(&w, 16);
    walked.result.unwrap();
    assert!(!walked.input.at_war && !walked.input.offensive_war && walked.input.defensive_war);
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
    w.u64(DATABASE + 0x88, SCALED as u64);
    w.vtable(SCALED, cstatic_modifier::VTABLE_RVA.0);
    w.modifier(SCALED, &source, b"Scaled static");
    w.string(SCALED + STATIC_NAME, b"scaled_static");
    w.children(&[(&child, b"Scaled static")]);
    let walked = walk(&w, 16);
    assert_eq!(walked.result.unwrap().lines, Outcome::Rows { len: 3 });
    assert!(
        walked.lines[..3]
            .iter()
            .all(|row| row.kind == Kind::Unpaired && row.value == 7)
    );
    let source_entries = Array::read(&w, SCALED + cmodifier::ACCUMULATOR, 16).unwrap();
    let child_entries = Array::read(&w, CHILD + cmodifier::ACCUMULATOR, 16).unwrap();
    let child_signature = signature(&w, child_entries, |_, _| {}).unwrap();
    let source_display = SCALED + cmodifier::DISPLAY_NAME;
    let source_header = read::<[u8; 32]>(&w, source_display).unwrap();
    let (len, digest) = display_digest(&w, source_display, source_header).unwrap();
    assert!(
        matches(
            &w,
            &[],
            &Child {
                entries: child_entries,
                signature: child_signature,
                display: (CHILD + cmodifier::DISPLAY_NAME, digest),
                ..Default::default()
            },
            &Candidate {
                kind: Kind::StaticScaled,
                content: Content::Entries(source_entries),
                display: Some((source_display, len, digest)),
                scaled: true,
                tracked: true,
                ..Default::default()
            },
        )
        .unwrap()
    );
}

fn observed_walk(
    copies: &Copies<'_>,
    old: bool,
    memo: &mut Memo,
    capacity: usize,
    fault: Option<usize>,
) -> Walked {
    observed_walk_bytes(copies, old, memo, capacity, 256, fault)
}

fn observed_walk_bytes(
    copies: &Copies<'_>,
    old: bool,
    memo: &mut Memo,
    capacity: usize,
    byte_capacity: usize,
    fault: Option<usize>,
) -> Walked {
    let countries = ccountry::Reader::new(copies, BASE, GS).unwrap();
    let country = countries.at(1).unwrap().unwrap();
    let context = Context::read(copies, BASE, GS, keys()).unwrap();
    copies.calls.borrow_mut().clear();
    copies.fault.set(fault);
    let mut scratch = Scratch::vacant();
    let mut input = InputRow::default();
    let mut lines = vec![LineRow::default(); capacity];
    let mut bytes = vec![0; byte_capacity];
    let result = if old {
        reference::walk_country(
            copies,
            BASE,
            &countries,
            country,
            &context,
            &mut scratch,
            &mut input,
            &mut lines,
            &mut bytes,
        )
    } else {
        walk_country_buffers(
            memo,
            copies,
            BASE,
            &countries,
            country,
            &context,
            &mut scratch,
            &mut input,
            Buffer::Growing(&mut lines),
            Buffer::Growing(&mut bytes),
            0,
            0,
            None,
        )
    };
    copies.fault.set(None);
    Walked {
        result,
        input,
        lines,
        bytes,
        fallbacks: scratch.batch_fallbacks(),
    }
}

fn assert_same_walk(old: &Walked, new: &Walked) {
    assert_eq!(new.result, old.result);
    assert_eq!(new.input, old.input);
    assert_eq!(new.lines, old.lines);
    assert_eq!(new.bytes, old.bytes);
}

fn scaled_world() -> World {
    let mut w = world();
    const SUNK: usize = 0x6100_0000;
    w.put(C1 + PRIDE_OF_THE_FLEET_SUNK_ACTIVE, &[1]);
    w.u64(DATABASE + 0xc0, SUNK as u64);
    w.vtable(SUNK, cstatic_modifier::VTABLE_RVA.0);
    w.modifier(SUNK, &[(STABILITY_KEY, 7)], b"Pride sunk");
    w.string(SUNK + STATIC_NAME, b"pride_sunk");
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
        (&[(STABILITY_KEY, 7)], b"Pride sunk"),
        (&[(COMMAND_KEY, 100)], b"Dyn"),
        (&[(STABILITY_KEY, 250)], b"Dyn"),
    ]);
    w
}

fn secondary_idea_world() -> World {
    let mut w = world();
    let idea = IDEA_A + cidea::SECONDARY_BASE;
    w.u64(IDEAS, idea as u64);
    w.vtable(idea, cidea::SECONDARY_VTABLE_RVA.0);
    w.modifier(
        idea + IDEA_MODIFIER,
        &[(STABILITY_KEY, 10_000)],
        b"built label",
    );
    w
}

const PAGED_COUNT: u32 = 130;
const PAGED_DYNAMIC_ENTRIES: usize = 0x6500_2000;
const PAGED_OBJECTS: usize = 0x6600_0000;

fn paged_sources_world() -> World {
    let mut w = scaled_world();
    const COUNT: u32 = PAGED_COUNT;
    const IDEA_POINTERS: usize = 0x6500_0000;
    const ADVISOR_POINTERS: usize = 0x6500_1000;
    const DYNAMIC_ENTRIES: usize = PAGED_DYNAMIC_ENTRIES;
    const OBJECTS: usize = PAGED_OBJECTS;
    let advisor: Vec<_> = w
        .bytes
        .range(ADVISOR..ADVISOR + 0x260)
        .map(|(&at, &byte)| (at - ADVISOR, byte))
        .collect();
    let entry: Vec<_> = w
        .bytes
        .range(ENTRY..ENTRY + dynamic_modifier_entry::STRIDE)
        .map(|(&at, &byte)| (at - ENTRY, byte))
        .collect();
    w.entries = 0x6800_0000 - ENTRIES;
    w.heap = 0x6900_0000 - HEAP;
    w.array(STATUS + cpolitical_status::IDEAS, IDEA_POINTERS, COUNT);
    w.array(
        CHARACTERS + ccountry_characters::APPOINTED_ADVISORS,
        ADVISOR_POINTERS,
        COUNT,
    );
    w.array(
        C1 + ccountry::DYNAMIC_MODIFIERS + cdynamic_modifier_container::ENTRIES,
        DYNAMIC_ENTRIES,
        COUNT,
    );
    for index in 0..COUNT as usize {
        let idea = OBJECTS + index * 0x1000;
        let advisor_object = idea + 0x400;
        w.u64(IDEA_POINTERS + index * 8, idea as u64);
        w.vtable(idea, cidea::VTABLE_RVA.0);
        w.string(idea + cidea::TOKEN, b"paged_idea");
        w.modifier(idea + IDEA_MODIFIER, &[(STABILITY_KEY, 10_000)], b"label");
        w.u64(ADVISOR_POINTERS + index * 8, advisor_object as u64);
        for &(offset, byte) in &advisor {
            w.put(advisor_object + offset, &[byte]);
        }
        w.modifier(
            advisor_object + ADVISOR_MODIFIER,
            &[(WAR_SUPPORT_KEY, 10_000)],
            b"A heap display shared by different advisors",
        );
        for &(offset, byte) in &entry {
            w.put(
                DYNAMIC_ENTRIES + index * dynamic_modifier_entry::STRIDE + offset,
                &[byte],
            );
        }
    }
    for (index, slot) in STATIC_SLOTS.iter().enumerate() {
        let modifier = OBJECTS + COUNT as usize * 0x1000 + index * 0x400;
        w.u64(DATABASE + slot.offset, modifier as u64);
        w.vtable(modifier, cstatic_modifier::VTABLE_RVA.0);
        w.modifier(
            modifier,
            &[(STABILITY_KEY, 1)],
            b"A heap display shared by different statics",
        );
        w.string(modifier + STATIC_NAME, b"paged_static");
    }
    w
}

#[test]
fn stability_batched_source_pages_keep_aliases_and_every_static_slot() {
    let w = paged_sources_world();
    let serial = Copies::new(&w);
    let batched = Copies::new(&w);
    assert_same_walk(
        &observed_walk(&serial, true, &mut Memo::default(), 16, None),
        &observed_walk(&batched, false, &mut Memo::default(), 16, None),
    );
    for phase in [
        Phase::Ideas,
        Phase::Advisors,
        Phase::Statics,
        Phase::Dynamic,
    ] {
        assert!(
            batched.counts()[phase as usize] < serial.counts()[phase as usize],
            "{phase:?} was not batched"
        );
    }
    type HeadSpan = fn(usize) -> (usize, usize);
    let heads: [(Phase, HeadSpan, usize); 3] = [
        (
            Phase::Ideas,
            |index| (PAGED_OBJECTS + index * 0x1000, 8),
            32,
        ),
        (
            Phase::Advisors,
            |index| (PAGED_OBJECTS + index * 0x1000 + 0x400, 24),
            22,
        ),
        (
            Phase::Dynamic,
            |index| {
                (
                    PAGED_DYNAMIC_ENTRIES + index * dynamic_modifier_entry::STRIDE,
                    56,
                )
            },
            64,
        ),
    ];
    for (phase, head, maximum) in heads {
        let expected: Vec<_> = (0..PAGED_COUNT as usize).map(head).collect();
        let calls = batched.calls.borrow();
        let mut gathered = Vec::new();
        let mut most = 0;
        for call in calls.iter().filter(|call| call.phase == phase) {
            assert!(call.operands.len() <= 64);
            let before = gathered.len();
            gathered.extend(
                call.operands
                    .iter()
                    .map(|&(address, len, _)| (address, len))
                    .filter(|span| expected.contains(span)),
            );
            most = most.max(gathered.len() - before);
        }
        gathered.sort_unstable();
        assert_eq!(gathered, expected, "{phase:?} heads");
        assert_eq!(most, maximum, "{phase:?} chunk geometry");
    }
}

#[test]
fn stability_batched_later_pages_null_slots_and_duplicate_objects_keep_serial_results() {
    const IDEAS: usize = 0x6500_0000;
    const ADVISORS: usize = 0x6500_1000;
    const OBJECTS: usize = 0x6600_0000;
    for advisors in [false, true] {
        let slots = if advisors { ADVISORS } else { IDEAS };
        let offset = if advisors { 0x400 } else { 0 };
        let modifier = if advisors {
            ADVISOR_MODIFIER
        } else {
            IDEA_MODIFIER
        };
        for invalid_prefix in [false, true] {
            let mut w = paged_sources_world();
            w.u64(slots + 64 * 8, 0);
            if invalid_prefix {
                w.u64(OBJECTS + 63 * 0x1000 + offset, 0);
            }
            let old = observed_walk(&Copies::new(&w), true, &mut Memo::default(), 16, None);
            let new = observed_walk(&Copies::new(&w), false, &mut Memo::default(), 16, None);
            assert_same_walk(&old, &new);
            assert!(new.result.is_err());
            assert_eq!(new.fallbacks, 0);
        }
        for duplicate in [false, true] {
            for null in [false, true] {
                let mut w = paged_sources_world();
                if duplicate {
                    w.u64(slots + 64 * 8, (OBJECTS + offset) as u64);
                }
                if null {
                    w.u64(slots + 66 * 8, 0);
                }
                let baseline =
                    observed_walk(&Copies::new(&w), true, &mut Memo::default(), 16, None);
                let healthy =
                    observed_walk(&Copies::new(&w), false, &mut Memo::default(), 16, None);
                assert_same_walk(&baseline, &healthy);
                assert_eq!(healthy.fallbacks, 0);
                if !null {
                    let countries = ccountry::Reader::new(&w, BASE, GS).unwrap();
                    let country = countries.at(1).unwrap().unwrap();
                    let context = Context::read(&w, BASE, GS, keys()).unwrap();
                    let mut scratch = Scratch::vacant();
                    let mut local = Memo::default();
                    let mut shared = Memo::default();
                    let memory = Observed::new(&w, &mut local, &mut shared);
                    let visited = candidates(
                        &memory,
                        BASE,
                        country,
                        &context,
                        StaticWitnesses {
                            attache_sent: Some(false),
                            at_war_mode0: Some(true),
                            pride_sunk: Some(true),
                        },
                        &mut scratch,
                    )
                    .unwrap();
                    assert_eq!(visited, 130 * 4 + 2 + 2);
                    if duplicate {
                        let first = if advisors { 132 } else { 0 };
                        assert_eq!(
                            scratch.candidates[first].identity,
                            scratch.candidates[first + 64].identity
                        );
                    }
                }
                let head = OBJECTS + 65 * 0x1000 + offset + modifier + cmodifier::ACCUMULATOR;
                for address in head..head + 16 {
                    for persistent in [false, true] {
                        let serial = Copies {
                            persistent,
                            ..Copies::new(&w)
                        };
                        let batched = Copies {
                            persistent,
                            ..Copies::new(&w)
                        };
                        let old =
                            observed_walk(&serial, true, &mut Memo::default(), 16, Some(address));
                        let new =
                            observed_walk(&batched, false, &mut Memo::default(), 16, Some(address));
                        assert_eq!(new.fallbacks, 1);
                        if persistent {
                            assert_same_walk(&old, &new);
                        } else {
                            assert!(old.result.is_err());
                            assert_same_walk(&baseline, &new);
                        }
                        let failures = batched
                            .calls
                            .borrow()
                            .iter()
                            .flat_map(|call| &call.operands)
                            .filter(|&&(at, len, copied)| {
                                !copied && at <= address && address - at < len
                            })
                            .count();
                        assert_eq!(failures, if persistent { 2 } else { 1 });
                    }
                }
                if null {
                    w.u64(OBJECTS + 63 * 0x1000 + offset, 0);
                    assert_same_walk(
                        &observed_walk(&Copies::new(&w), true, &mut Memo::default(), 16, None),
                        &observed_walk(&Copies::new(&w), false, &mut Memo::default(), 16, None),
                    );
                }
            }
        }
    }
}

#[test]
fn stability_copy_census_keeps_first_observations_and_cuts_repeated_copies() {
    let expected = [
        (
            [2, 17, 4, 5, 6, 10, 1, 11, 8, 3, 9, 3, 12, 0, 36, 7],
            [2, 17, 4, 5, 6, 3, 1, 5, 6, 0, 7, 2, 6, 0, 0, 5],
        ),
        (
            [2, 17, 4, 5, 6, 12, 1, 11, 8, 3, 9, 6, 13, 0, 50, 9],
            [2, 17, 4, 5, 6, 3, 1, 5, 6, 0, 7, 2, 7, 0, 0, 6],
        ),
    ];
    for (w, (before, after)) in [world(), scaled_world()].into_iter().zip(expected) {
        let old_copies = Copies::new(&w);
        let new_copies = Copies::new(&w);
        let old = observed_walk(&old_copies, true, &mut Memo::default(), 16, None);
        let new = observed_walk(&new_copies, false, &mut Memo::default(), 16, None);
        assert_same_walk(&old, &new);
        assert_eq!(
            new.input.manpower,
            engine_native_parts::ccountry_manpower::Row {
                ratio: Some(250_000),
                host: Some(0),
                ..Default::default()
            }
        );
        assert_eq!(operand_spans(&new_copies), operand_spans(&old_copies));
        assert_eq!(new.fallbacks, 0);
        assert_eq!(old_copies.counts(), before);
        assert_eq!(new_copies.counts(), after);
        eprintln!(
            "stability copy census: before {:?}, after {:?}",
            old_copies.counts(),
            new_copies.counts()
        );
    }
}

#[test]
fn manpower_refusals_preserve_stability_and_the_other_scalar() {
    let w = world();
    for (address, expected) in [
        (
            C1 + engine_native_memory::country_identity::MANPOWER + engine_native_parts::ccountry_manpower::MOBILIZATION,
            engine_native_parts::ccountry_manpower::Row {
                host: Some(0),
                ..Default::default()
            },
        ),
        (
            D1 + engine_native_parts::ccountry_manpower::HOST,
            engine_native_parts::ccountry_manpower::Row {
                ratio: Some(250_000),
                ..Default::default()
            },
        ),
    ] {
        let serial = Copies {
            persistent: true,
            ..Copies::new(&w)
        };
        let batched = Copies {
            persistent: true,
            ..Copies::new(&w)
        };
        let old = observed_walk(&serial, true, &mut Memo::default(), 16, Some(address));
        let new = observed_walk(&batched, false, &mut Memo::default(), 16, Some(address));
        assert_same_walk(&old, &new);
        assert!(new.result.is_ok());
        assert_eq!(new.input.manpower, expected);
        assert_eq!(new.input.stability_raw, 50_000);
    }
}

#[test]
fn stability_copy_faults_at_every_old_operand_byte_keep_the_old_outcome_and_reason() {
    for w in [world(), scaled_world(), secondary_idea_world()] {
        let baseline = Copies::new(&w);
        let walked = observed_walk(&baseline, true, &mut Memo::default(), 16, None);
        assert!(walked.result.is_ok());
        let addresses: std::collections::BTreeSet<_> = baseline
            .calls
            .borrow()
            .iter()
            .flat_map(|call| &call.operands)
            .flat_map(|&(address, len, _)| address..address + len)
            .collect();
        for persistent in [false, true] {
            for &address in &addresses {
                let old_copies = Copies {
                    persistent,
                    ..Copies::new(&w)
                };
                let new_copies = Copies {
                    persistent,
                    ..Copies::new(&w)
                };
                let old = observed_walk(&old_copies, true, &mut Memo::default(), 16, Some(address));
                let new =
                    observed_walk(&new_copies, false, &mut Memo::default(), 16, Some(address));
                assert!(new.fallbacks <= 1, "fault {address:#x} retried a batch");
                if persistent || old.result.is_ok() {
                    assert_same_walk(&old, &new);
                } else if new.result.is_ok() {
                    assert_same_walk(&walked, &new);
                } else {
                    assert!(
                        old.result.is_err(),
                        "one-shot fault {address:#x} refused more data"
                    );
                }
            }
        }
        for capacity in [0, 3] {
            assert_same_walk(
                &observed_walk(&Copies::new(&w), true, &mut Memo::default(), capacity, None),
                &observed_walk(
                    &Copies::new(&w),
                    false,
                    &mut Memo::default(),
                    capacity,
                    None,
                ),
            );
        }
        for byte_capacity in [0, 5, 24, 48] {
            assert_same_walk(
                &observed_walk_bytes(
                    &Copies::new(&w),
                    true,
                    &mut Memo::default(),
                    16,
                    byte_capacity,
                    None,
                ),
                &observed_walk_bytes(
                    &Copies::new(&w),
                    false,
                    &mut Memo::default(),
                    16,
                    byte_capacity,
                    None,
                ),
            );
        }
    }
}

#[test]
fn stability_shared_static_observations_expire_with_each_hour() {
    let mut w = scaled_world();
    let mut memo = Memo::default();
    let first = Copies::new(&w);
    let first_walk = observed_walk(&first, false, &mut memo, 16, None);
    assert!(first_walk.result.is_ok());
    let next = Copies::new(&w);
    let next_walk = observed_walk(&next, false, &mut memo, 16, None);
    assert_same_walk(&first_walk, &next_walk);
    assert!(next.calls.borrow().len() < first.calls.borrow().len());
    w.string(AT_WAR + STATIC_NAME, b"changed_static_name");
    memo.clear();
    let next_hour = Copies::new(&w);
    let captured = observed_walk(&next_hour, false, &mut memo, 16, None);
    assert_same_walk(
        &observed_walk(&Copies::new(&w), true, &mut Memo::default(), 16, None),
        &captured,
    );
    assert_eq!(
        text(&captured, captured.lines[5].identity),
        b"changed_static_name"
    );
}

#[test]
fn stability_failed_observations_are_retained_without_partial_bytes_or_retry() {
    let w = world();
    let copies = Copies::new(&w);
    copies.fault.set(Some(AT_WAR + cmodifier::ACCUMULATOR));
    let mut local = Memo::default();
    let mut shared = Memo::default();
    let memory = Observed::new(&copies, &mut local, &mut shared);
    let mut bytes = [0; 16];
    assert!(
        !memory
            .shared()
            .copy(AT_WAR + cmodifier::ACCUMULATOR, &mut bytes)
    );
    assert!(!memory.copy(AT_WAR + cmodifier::ACCUMULATOR, &mut bytes));
    assert_eq!(copies.calls.borrow().len(), 1);
}

#[test]
fn stability_batched_failure_falls_back_once_without_installing_a_partial_prefix() {
    for persistent in [false, true] {
        let w = world();
        let copies = Copies {
            persistent,
            ..Copies::new(&w)
        };
        let first = IDEA_A + IDEA_MODIFIER + cmodifier::ACCUMULATOR;
        let second = IDEA_B + IDEA_MODIFIER + cmodifier::ACCUMULATOR;
        copies.fault.set(Some(second));
        let mut local = Memo::default();
        let mut shared = Memo::default();
        let memory = Observed::new(&copies, &mut local, &mut shared);
        memory.gather([(first, 16), (second, 16)], false);
        assert_eq!(memory.fallbacks(), 1);
        memory.gather([(first, 16), (second, 16)], false);
        assert_eq!(copies.calls.borrow().len(), 1);
        let mut bytes = [0; 16];
        assert!(memory.copy(first, &mut bytes));
        assert_eq!(memory.copy(second, &mut bytes), !persistent);
        assert_eq!(copies.calls.borrow().len(), 3);
        assert_eq!(memory.copy(second, &mut bytes), !persistent);
        assert_eq!(copies.calls.borrow().len(), 3);
        assert_eq!(memory.fallbacks(), 1);
        memory.gather([(first, 16), (second, 16)], false);
        assert_eq!(copies.calls.borrow().len(), 3);
    }
}

fn scaled_countries(count: u32) -> World {
    let mut w = scaled_world();
    const COUNTRY_OBJECTS: usize = 0x1000_0000;
    const DIPLOMACY_OBJECTS: usize = 0x2000_0000;
    const CHARACTER_OBJECTS: usize = 0x3000_0000;
    const COUNTRY_SLOTS: usize = 0x4000_0000;
    const COUNTRY_LINKS: usize = 0x4100_0000;
    let fields: Vec<_> = w
        .bytes
        .range(C1..C1 + 0x1700)
        .map(|(&p, &b)| (p - C1, b))
        .collect();
    w.array(GS + cgame_state::COUNTRIES, COUNTRY_SLOTS, count + 1);
    w.array(GS + cgame_state::COUNTRY_LINKS, COUNTRY_LINKS, count + 1);
    for id in 1..=count {
        let country = COUNTRY_OBJECTS + id as usize * 0x2000;
        let diplomacy = DIPLOMACY_OBJECTS + id as usize * 0x200;
        let characters = CHARACTER_OBJECTS + id as usize * 0x100;
        for &(offset, byte) in &fields {
            w.put(country + offset, &[byte]);
        }
        w.u64(COUNTRY_SLOTS + id as usize * 8, country as u64);
        w.u32(COUNTRY_LINKS + id as usize * 4, id);
        w.u32(country + ccountry::ID, id);
        w.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
        w.vtable(diplomacy, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0);
        w.u64(diplomacy + engine_native_parts::cdiplomacy_status_layout::COUNTRY, country as u64);
        w.array(diplomacy + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, 0, 0);
        w.u32(diplomacy + cdiplomacy_status::ENEMY_COUNTRY_COUNT, 1);
        w.u32(diplomacy + engine_native_parts::ccountry_manpower::HOST, 0);
        w.u64(country + ccountry::CHARACTERS, characters as u64);
        w.vtable(characters, engine_native::ccountry::rosters::VTABLE_RVA.0);
        w.u64(characters + engine_native::ccountry::rosters::OWNER, country as u64);
        w.array(
            characters + ccountry_characters::APPOINTED_ADVISORS,
            ADVISORS,
            1,
        );
    }
    w
}

#[test]
fn stability_batched_copies_match_serial() {
    const COUNT: u32 = 4;
    let w = scaled_countries(COUNT);
    let copies = Copies::new(&w);
    let countries = ccountry::Reader::new(&copies, BASE, GS).unwrap();
    let context = Context::read(&copies, BASE, GS, keys()).unwrap();
    let mut memo = Memo::default();
    let mut old_scratch = Scratch::vacant();
    let mut new_scratch = Scratch::vacant();
    let mut before = 0;
    let mut after = 0;
    for slot in 1..=COUNT {
        let country = countries.at(slot).unwrap().unwrap();
        let mut old_input = InputRow::default();
        let mut new_input = InputRow::default();
        let mut old_lines = vec![LineRow::default(); 16];
        let mut new_lines = old_lines.clone();
        let mut old_bytes = vec![0; 256];
        let mut new_bytes = old_bytes.clone();
        copies.calls.borrow_mut().clear();
        let old = reference::walk_country(
            &copies,
            BASE,
            &countries,
            country,
            &context,
            &mut old_scratch,
            &mut old_input,
            &mut old_lines,
            &mut old_bytes,
        );
        assert_eq!(
            copies.counts(),
            [2, 3, 4, 5, 6, 12, 1, 11, 8, 3, 9, 6, 13, 0, 50, 9]
        );
        before += copies.calls.borrow().len();
        copies.calls.borrow_mut().clear();
        let new = walk_country_memo(
            &copies,
            BASE,
            &countries,
            country,
            &context,
            &mut memo,
            &mut new_scratch,
            &mut new_input,
            &mut new_lines,
            &mut new_bytes,
            0,
            0,
        );
        assert_eq!(new, old);
        assert!(new.is_ok());
        assert_eq!(new_input, old_input);
        assert_eq!(new_lines, old_lines);
        assert_eq!(new_bytes, old_bytes);
        assert_eq!(
            copies.counts(),
            if slot == 1 {
                [2, 3, 4, 5, 6, 3, 1, 5, 6, 0, 7, 2, 7, 0, 0, 6]
            } else {
                [2, 3, 4, 5, 6, 3, 1, 5, 6, 0, 7, 0, 7, 0, 0, 2]
            }
        );
        assert_eq!(new_scratch.batch_fallbacks(), 0);
        after += copies.calls.borrow().len();
    }
    assert_eq!(before, COUNT as usize * 142);
    assert_eq!(after, 57 + (COUNT as usize - 1) * 51);
}

#[test]
fn stability_copy_memo_preserves_the_changed_children_consistency_reread() {
    struct ChangedChildren<'a> {
        source: &'a World,
        reads: std::cell::Cell<usize>,
    }
    impl Memory for ChangedChildren<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if !self.source.copy(address, destination) {
                return false;
            }
            if address == C1 + MODIFIER_CHILDREN && destination.len() == 16 {
                self.reads.set(self.reads.get() + 1);
                if self.reads.get() == 2 {
                    destination[12..16].copy_from_slice(&5_u32.to_le_bytes());
                }
            }
            true
        }
    }
    let w = world();
    for old in [false, true] {
        let memory = ChangedChildren {
            source: &w,
            reads: Default::default(),
        };
        let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
        let country = countries.at(1).unwrap().unwrap();
        let context = Context::read(&memory, BASE, GS, keys()).unwrap();
        let mut scratch = Scratch::vacant();
        let result = if old {
            reference::walk_country(
                &memory,
                BASE,
                &countries,
                country,
                &context,
                &mut scratch,
                &mut InputRow::default(),
                &mut Vec::new(),
                &mut Vec::new(),
            )
        } else {
            walk_country(
                &memory,
                BASE,
                &countries,
                country,
                &context,
                &mut scratch,
                &mut InputRow::default(),
                &mut Vec::new(),
                &mut Vec::new(),
                0,
                0,
            )
        };
        assert_eq!(result, Err("country modifier tree changed during the walk"));
        assert_eq!(memory.reads.get(), 2);
    }
}

fn country_walk(
    copies: &Copies<'_>,
    memo: Option<&mut Memo>,
    slot: u32,
    fault: Option<usize>,
) -> Result<CountryWalk, &'static str> {
    let armed = copies.fault.replace(None);
    let countries = ccountry::Reader::new(copies, BASE, GS).unwrap();
    let country = countries.at(slot).unwrap().unwrap();
    let context = Context::read(copies, BASE, GS, keys()).unwrap();
    copies.calls.borrow_mut().clear();
    copies.fault.set(fault.or(armed));
    let mut scratch = Scratch::vacant();
    let mut input = InputRow::default();
    let mut lines = vec![LineRow::default(); 16];
    let mut bytes = vec![0; 256];
    match memo {
        None => reference::walk_country(
            copies,
            BASE,
            &countries,
            country,
            &context,
            &mut scratch,
            &mut input,
            &mut lines,
            &mut bytes,
        ),
        Some(memo) => walk_country_memo(
            copies,
            BASE,
            &countries,
            country,
            &context,
            memo,
            &mut scratch,
            &mut input,
            &mut lines,
            &mut bytes,
            0,
            0,
        ),
    }
}

fn operand_spans(copies: &Copies<'_>) -> std::collections::BTreeSet<(usize, usize)> {
    copies
        .calls
        .borrow()
        .iter()
        .flat_map(|call| call.operands.iter().map(|&(at, len, _)| (at, len)))
        .collect()
}

#[test]
fn stability_shared_static_faults_match_the_old_walk_and_are_never_retried() {
    let w = scaled_countries(3);
    let probe = Copies::new(&w);
    let mut memo = Memo::default();
    assert!(country_walk(&probe, Some(&mut memo), 1, None).is_ok());
    let first = operand_spans(&probe);
    assert!(country_walk(&probe, Some(&mut memo), 2, None).is_ok());
    let second = operand_spans(&probe);
    let old = Copies::new(&w);
    assert!(country_walk(&old, None, 2, None).is_ok());
    let shared: Vec<_> = operand_spans(&old)
        .difference(&second)
        .copied()
        .filter(|span| first.contains(span))
        .collect();
    assert!(!shared.is_empty(), "later countries reuse shared spans");
    let mut retained = 0;
    for &(address, len) in &shared {
        let old = Copies {
            persistent: true,
            ..Copies::new(&w)
        };
        let new = Copies {
            persistent: true,
            ..Copies::new(&w)
        };
        let mut memo = Memo::default();
        for slot in 1..=3 {
            assert_eq!(
                country_walk(&new, Some(&mut memo), slot, Some(address)),
                country_walk(&old, None, slot, Some(address)),
                "persistent fault {address:#x}, country {slot}"
            );
        }
        let new = Copies::new(&w);
        let mut memo = Memo::default();
        let failure = country_walk(&new, Some(&mut memo), 1, Some(address));
        let serial = country_walk(&Copies::new(&w), None, 1, Some(address));
        assert!(
            failure.is_ok() || serial.is_err(),
            "one-shot fault {address:#x} refused more data"
        );
        if failure.is_ok() {
            assert!(country_walk(&new, Some(&mut memo), 2, None).is_ok());
            continue;
        }
        let failed: Vec<_> = new
            .calls
            .borrow()
            .iter()
            .flat_map(|call| call.operands.iter().copied())
            .filter(|operand| !operand.2)
            .collect();
        if failed != [(address, len, false)] {
            continue;
        }
        retained += 1;
        assert!(failure.is_err());
        assert_eq!(
            country_walk(&new, Some(&mut memo), 2, None),
            failure,
            "the failed shared observation {address:#x} is kept for the hour"
        );
        assert!(
            new.calls
                .borrow()
                .iter()
                .flat_map(|call| call.operands.iter())
                .all(|&(at, _, _)| at != address),
            "{address:#x} is not read again"
        );
    }
    assert!(retained != 0, "a shared failure was retained");
}

#[cfg(feature = "alloc-fault-injection")]
struct Counted<'a> {
    source: &'a World,
    calls: std::cell::Cell<usize>,
}

#[cfg(feature = "alloc-fault-injection")]
impl Memory for Counted<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.source.copy(address, destination)
    }
}

#[cfg(feature = "alloc-fault-injection")]
#[test]
#[ignore = "run alone: allocation hooks are process-global"]
fn stability_observation_retention_failure_keeps_the_acquired_read() {
    let w = world();
    let counted = Counted {
        source: &w,
        calls: std::cell::Cell::new(0),
    };
    let mut local = Memo::default();
    let mut shared = Memo::default();
    let memory = Observed::new(&counted, &mut local, &mut shared);
    let mut bytes = [0; 16];
    crate::allocation::test_support::with_allocations_denied(|| {
        assert!(memory.copy(AT_WAR + cmodifier::ACCUMULATOR, &mut bytes));
        assert!(
            memory
                .shared()
                .copy(AT_WAR + cmodifier::ACCUMULATOR, &mut bytes)
        );
    });
    assert_eq!(counted.calls.get(), 2);
}

#[cfg(feature = "alloc-fault-injection")]
#[test]
#[ignore = "run alone: allocation hooks are process-global"]
fn stability_batched_retention_failure_reads_nothing_before_the_serial_reader() {
    let w = scaled_world();
    let counted = Counted {
        source: &w,
        calls: std::cell::Cell::new(0),
    };
    let mut local = Memo::default();
    let mut shared = Memo::default();
    let memory = Observed::new(&counted, &mut local, &mut shared);
    let address = AT_WAR + cmodifier::ACCUMULATOR;
    crate::allocation::test_support::with_allocations_denied(|| {
        batched::sources(&memory, &[AT_WAR, 0x6100_0000], true, true);
        batched::objects(&memory, &[IDEA_A, IDEA_B], 8, IDEA_MODIFIER, false);
        assert_eq!(read::<u64>(&memory.prepared(), IDEA_A), None);
        assert_eq!(counted.calls.get(), 0);
        assert!(memory.shared().copy(address, &mut [0; 16]));
        assert_eq!(counted.calls.get(), 1);
        assert_eq!(memory.fallbacks(), 0);
    });
}

#[cfg(feature = "alloc-fault-injection")]
#[test]
#[ignore = "run alone: allocation hooks are process-global"]
fn stability_denied_candidate_preparation_matches_serial_acquisitions() {
    let w = scaled_world();
    let countries = ccountry::Reader::new(&w, BASE, GS).unwrap();
    let country = countries.at(1).unwrap().unwrap();
    let context = Context::read(&w, BASE, GS, keys()).unwrap();
    let demand = Demand {
        children: 8,
        candidates: 16,
        values: 8,
        ..Demand::default()
    };
    let mut old_scratch = Scratch::with_demand(demand).unwrap();
    let mut new_scratch = Scratch::with_demand(demand).unwrap();
    let serial = Counted {
        source: &w,
        calls: std::cell::Cell::new(0),
    };
    let counted = Counted {
        source: &w,
        calls: std::cell::Cell::new(0),
    };
    let old_countries = ccountry::Reader::new(&serial, BASE, GS).unwrap();
    let new_countries = ccountry::Reader::new(&counted, BASE, GS).unwrap();
    serial.calls.set(0);
    counted.calls.set(0);
    let mut old_input = InputRow::default();
    let mut new_input = InputRow::default();
    let mut old_lines = [LineRow::default(); 16];
    let mut new_lines = old_lines;
    let mut old_bytes = [0; 256];
    let mut new_bytes = old_bytes;
    crate::allocation::test_support::with_allocations_denied(|| {
        let old = reference::walk_country(
            &serial,
            BASE,
            &old_countries,
            country,
            &context,
            &mut old_scratch,
            &mut old_input,
            &mut old_lines,
            &mut old_bytes,
        );
        let new = walk_country(
            &counted,
            BASE,
            &new_countries,
            country,
            &context,
            &mut new_scratch,
            &mut new_input,
            &mut new_lines,
            &mut new_bytes,
            0,
            0,
        );
        assert_eq!(new, old);
        assert!(new.is_ok());
        assert_eq!(new_input, old_input);
        assert_eq!(new_lines, old_lines);
        assert_eq!(new_bytes, old_bytes);
        assert_eq!(counted.calls.get(), serial.calls.get());
        assert_eq!(new_scratch.batch_fallbacks(), 0);
    });
}

#[test]
fn manpower_without_certificate_preserves_stability_and_reads_no_new_words() {
    let mut world = world();
    let country = ccountry::Reader::new(&world, BASE, GS)
        .unwrap()
        .at(1)
        .unwrap()
        .unwrap();
    let ratio = country.pointer + engine_native_memory::country_identity::MANPOWER + engine_native_parts::ccountry_manpower::MOBILIZATION;
    world.i64(ratio, 250_000);
    let mut context = Context::read(&world, BASE, GS, keys()).unwrap();
    context.manpower_enabled = false;
    struct NoManpower<'a> {
        world: &'a World,
        ratio: usize,
    }
    impl Memory for NoManpower<'_> {
        fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
            assert_ne!(address, self.ratio, "uncertified recruitment read");
            self.world.copy(address, bytes)
        }
    }
    let guarded = NoManpower {
        world: &world,
        ratio,
    };
    let countries = ccountry::Reader::new(&guarded, BASE, GS).unwrap();
    let (input, _) = inputs(&guarded, BASE, &countries, country, &context).unwrap();
    assert_eq!(input.manpower, Default::default());
    assert_eq!(input.stability_raw, 50_000);
    context.manpower_enabled = true;
    let countries = ccountry::Reader::new(&world, BASE, GS).unwrap();
    assert_eq!(
        inputs(&world, BASE, &countries, country, &context)
            .unwrap()
            .0
            .manpower
            .ratio,
        Some(250_000)
    );
}
