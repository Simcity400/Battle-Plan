
use super::{
    ADVISOR_MODIFIER, AIR_STATISTICS, Context, DEFINE_RVAS, DEFINES, DIPLOMACY_HOSTING,
    DYNAMIC_CONSTANT, DYNAMIC_VALUE_ROW_STRIDE, DYNAMIC_VALUE_ROWS, IDEA_MODIFIER,
    IDEOLOGY_GROUP_MODIFIER, InputRow, Keys, Kind, LEADER_MODIFIER, LineRow, MODIFIER_CHILDREN,
    Memo, PRIDE_OF_THE_FLEET_SUNK_ACTIVE, RING, RING_DATA, RING_SIZE, STATIC_DATABASE_RVA,
    STATIC_NAME, STATIC_SLOTS, Scratch, TRACKED, walk_country_memo,
};
use engine_native::{
    cadvisor,
    ccharacter,
    ccountry,
    cgame_state,
    crelation,
    cwar_relation,
};
use engine_native_parts::{
    cattache_relation,
    cdynamic_modifier,
    cidea,
    cideology_group,
    cstatic_modifier,
    modifier_definition,
    modifier_entry,
};
use engine_native_politics::{ccountry_characters, cpolitical_party, cpolitical_status};
use crate::{
    cdynamic_modifier_container,
    cmodifier,
    cworld_threat,
    dynamic_modifier_entry,
    localization_root_layout,
};
use engine_native_intelligence::country_memory::{Cache, CountryMemory, SeedStatus};
use engine_native_memory::{
    cpdx_array::Array,
    memory::LiveMemory,
    msvc_string,
    read_counts::{CountedMemory, ReadCounts},
    walk::Outcome,
};
use std::{hint::black_box, time::Instant};

const RUNS: usize = 20;
const CHILDREN: usize = 100;
const IDEAS: usize = 15;
const ADVISORS: usize = 3;
const DYNAMICS: usize = 10;
const LINES: usize = CHILDREN * 2;
const STABILITY_KEY: u32 = 1;
const WAR_SUPPORT_KEY: u32 = 6;
const UNTRACKED_KEY: u32 = TRACKED as u32 + 1;
const IMAGE: usize = (localization_root_layout::SINGLETON_RVA.0 + 8 + 15) & !15;
const ACTIVE_STATICS: [usize; 3] = [0, 2, 3];
const ACTIVE_SCALED: [usize; 0] = [];

struct Arena {
    bytes: Vec<u8>,
    pointers: Vec<usize>,
    objects: Vec<(usize, usize)>,
}

struct FrozenArena(Box<[u8]>);

impl Arena {
    fn new() -> Self {
        Self {
            bytes: vec![0; IMAGE],
            pointers: Vec::new(),
            objects: vec![(0, IMAGE)],
        }
    }

    fn alloc(&mut self, len: usize) -> usize {
        let at = (self.bytes.len() + 15) & !15;
        self.bytes.resize(at.checked_add(len).unwrap(), 0);
        self.objects.push((at, at + len));
        at
    }

    fn put(&mut self, address: usize, bytes: &[u8]) {
        debug_assert!(
            {
                let index = self.objects.partition_point(|&(start, _)| start <= address);
                index != 0 && address + bytes.len() <= self.objects[index - 1].1
            },
            "fixture write {address:#x}+{} leaves its object",
            bytes.len()
        );
        self.bytes[address..address + bytes.len()].copy_from_slice(bytes);
    }

    fn u8(&mut self, address: usize, value: u8) {
        self.put(address, &[value]);
    }

    fn u32(&mut self, address: usize, value: u32) {
        self.put(address, &value.to_le_bytes());
    }

    fn i64(&mut self, address: usize, value: i64) {
        self.put(address, &value.to_le_bytes());
    }

    fn pointer(&mut self, address: usize, target: usize) {
        self.put(address, &(target as u64).to_le_bytes());
        self.pointers.push(address);
    }

    fn vtable(&mut self, address: usize, rva: usize) {
        self.pointer(address, rva);
    }

    fn array(&mut self, address: usize, data: usize, count: usize) {
        self.pointer(address, data);
        self.u32(address + 8, u32::try_from(count).unwrap());
        self.u32(address + 12, u32::try_from(count).unwrap());
    }

    fn string(&mut self, address: usize, value: &[u8]) {
        if value.len() <= msvc_string::INLINE_CAPACITY as usize {
            self.put(address, value);
            self.i64(address + msvc_string::CAPACITY, 15);
        } else {
            let heap = self.alloc(value.len() + 1);
            self.put(heap, value);
            self.pointer(address, heap);
            self.i64(address + msvc_string::CAPACITY, (value.len() | 31) as i64);
        }
        self.i64(address + msvc_string::LENGTH, value.len() as i64);
    }

    fn entries(&mut self, entries: &[(u32, i64)]) -> usize {
        let data = self.alloc(entries.len() * modifier_entry::STRIDE);
        for (index, &(key, value)) in entries.iter().enumerate() {
            let row = data + index * modifier_entry::STRIDE;
            self.u32(row, key);
            self.i64(row + 8, value);
        }
        data
    }

    fn modifier(&mut self, address: usize, spec: &ChildSpec) {
        let data = self.entries(&spec.entries);
        self.array(address + cmodifier::ACCUMULATOR, data, spec.entries.len());
        self.string(address + cmodifier::DISPLAY_NAME, spec.display.as_bytes());
    }

    fn character(&mut self, id: u32) -> usize {
        let character = self.alloc(16);
        self.vtable(character, ccharacter::VTABLE_RVA.0);
        self.u32(character + ccharacter::TYPE, ccharacter::REFERENCE_TYPE);
        self.u32(character + ccharacter::ID, id);
        character
    }

    fn freeze(self) -> FrozenArena {
        let mut bytes = self.bytes.into_boxed_slice();
        let base = bytes.as_ptr() as usize;
        for address in self.pointers {
            let target = u64::from_le_bytes(bytes[address..address + 8].try_into().unwrap());
            if target != 0 {
                let target = usize::try_from(target).unwrap();
                assert!(target < bytes.len());
                bytes[address..address + 8]
                    .copy_from_slice(&(base.checked_add(target).unwrap() as u64).to_le_bytes());
            }
        }
        FrozenArena(bytes)
    }
}

impl FrozenArena {
    fn base(&self) -> usize {
        self.0.as_ptr() as usize
    }
}

#[derive(Clone)]
struct ChildSpec {
    entries: [(u32, i64); 3],
    display: String,
}

impl ChildSpec {
    fn new(value: i64, display: String) -> Self {
        assert!(display.len() > msvc_string::INLINE_CAPACITY as usize);
        Self {
            entries: [
                (STABILITY_KEY, value),
                (WAR_SUPPORT_KEY, -value),
                (UNTRACKED_KEY, value + 3),
            ],
            display,
        }
    }
}

struct Fixture {
    arena: FrozenArena,
    game_state: usize,
    keys: Keys,
    countries: Vec<engine_native_memory::country_identity::Country>,
}

impl Fixture {
    fn new(n: usize) -> Self {
        let mut arena = Arena::new();
        let game_state = arena.alloc(0x1000);
        let slots = arena.alloc((n + 1) * 8);
        let links = arena.alloc((n + 1) * 4);
        arena.array(game_state + cgame_state::COUNTRIES, slots, n + 1);
        arena.array(game_state + cgame_state::COUNTRY_LINKS, links, n + 1);

        let table_count = UNTRACKED_KEY as usize + 1;
        let table = arena.alloc(table_count * modifier_definition::STRIDE);
        arena.array(modifier_definition::TABLE_RVA.0, table, table_count);
        for key in 1..=UNTRACKED_KEY {
            let row = table + key as usize * modifier_definition::STRIDE;
            arena.u32(row + modifier_definition::FLAGS, 3);
            arena.u32(row + modifier_definition::FEATURE_SELECTOR, 0);
        }
        assert_eq!(DEFINES, DEFINE_RVAS.len());
        for (index, rva) in DEFINE_RVAS.into_iter().enumerate() {
            arena.i64(rva.0, 1_000 + index as i64);
        }
        let world = arena.alloc(0x40);
        arena.pointer(game_state + cgame_state::WORLD_THREAT, world);
        arena.vtable(world, cworld_threat::VTABLE_RVA.0);
        arena.i64(world + cworld_threat::TOTAL, 12_345);
        let language_root = arena.alloc(8);
        let language = arena.alloc(msvc_string::STRIDE);
        arena.pointer(localization_root_layout::SINGLETON_RVA.0, language_root);
        arena.pointer(
            language_root + localization_root_layout::LANGUAGE_STRING,
            language,
        );
        arena.string(language, b"synthetic_language");

        let database = arena.alloc(0x400);
        arena.pointer(STATIC_DATABASE_RVA.0, database);
        let statics: Vec<_> = STATIC_SLOTS
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                let spec = ChildSpec::new(
                    200 + index as i64,
                    format!("synthetic shared static display {index:02}"),
                );
                let object = arena.alloc(STATIC_NAME + msvc_string::STRIDE);
                arena.vtable(object, cstatic_modifier::VTABLE_RVA.0);
                arena.pointer(database + slot.offset, object);
                arena.modifier(object, &spec);
                arena.string(
                    object + STATIC_NAME,
                    format!("synthetic_static_{index:02}").as_bytes(),
                );
                spec
            })
            .collect();

        let mut countries = Vec::with_capacity(n);
        for slot in 1..=n {
            let country = arena.alloc(0x1700);
            arena.pointer(slots + slot * 8, country);
            arena.u32(links + slot * 4, slot as u32);
            arena.vtable(country, ccountry::VTABLE_RVA.0);
            arena.u32(country + ccountry::ID, slot as u32);
            arena.i64(country + ccountry::STABILITY, 50_000);
            arena.i64(country + ccountry::WAR_SUPPORT, 40_000);
            arena.i64(country + super::PROPAGANDA_STABILITY_PENALTY, -3277);
            arena.i64(country + super::COASTAL_PROTECTION_STABILITY, 2500);
            arena.u8(country + PRIDE_OF_THE_FLEET_SUNK_ACTIVE, 1);

            let diplomacy = arena.alloc(engine_native_parts::cdiplomacy_status_layout::COUNTRY + 8);
            arena.pointer(country + ccountry::DIPLOMACY_STATUS, diplomacy);
            arena.vtable(diplomacy, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0);
            arena.pointer(diplomacy + engine_native_parts::cdiplomacy_status_layout::COUNTRY, country);
            arena.u32(diplomacy + DIPLOMACY_HOSTING, 0);
            let relation = arena.alloc(cwar_relation::FIRST_WAS_INSTIGATOR + 1);
            let attache = arena.alloc(0x18);
            let owned = arena.alloc(16);
            arena.pointer(owned, relation);
            arena.pointer(owned + 8, attache);
            arena.array(diplomacy + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, owned, 2);
            arena.vtable(relation, cwar_relation::VTABLE_RVA.0);
            arena.u32(relation + crelation::TYPE_ID, cwar_relation::TYPE);
            arena.u32(relation + crelation::FIRST_COUNTRY, slot as u32);
            arena.u32(relation + crelation::SECOND_COUNTRY, (slot % n + 1) as u32);
            arena.u8(relation + cwar_relation::FIRST_WAS_INSTIGATOR, 1);
            arena.vtable(attache, cattache_relation::VTABLE_RVA.0);
            arena.u32(attache + crelation::TYPE_ID, cattache_relation::TYPE);
            arena.u32(attache + crelation::FIRST_COUNTRY, slot as u32);
            arena.u32(attache + crelation::SECOND_COUNTRY, (slot % n + 1) as u32);

            let stats = arena.alloc(RING + 8);
            let ring = arena.alloc(RING_SIZE + 9);
            let ring_slots = arena.alloc(8 * 8);
            arena.pointer(country + AIR_STATISTICS, stats);
            arena.pointer(stats + RING, ring);
            arena.pointer(ring + RING_DATA, ring_slots);
            arena.u32(ring + RING_SIZE, 8);
            arena.u32(ring + RING_SIZE + 4, 8);
            for index in 0..8 {
                let row = arena.alloc(8);
                let values = arena.alloc(16);
                arena.pointer(ring_slots + index * 8, row);
                arena.pointer(row, values);
                arena.i64(values + 8, 777 + index as i64);
            }

            let mut children = Vec::with_capacity(CHILDREN);
            let status = arena.alloc(0x100);
            let party = arena.alloc(0x100);
            let ideas = arena.alloc(IDEAS * 8);
            arena.pointer(country + ccountry::POLITICAL_STATUS, status);
            arena.vtable(status, engine_native_parts::cpolitical_status_layout::VTABLE_RVA.0);
            arena.pointer(status + engine_native_parts::cpolitical_status_layout::COUNTRY, country);
            arena.pointer(status + engine_native_parts::cpolitical_status_layout::RULING_PARTY, party);
            arena.array(status + cpolitical_status::IDEAS, ideas, IDEAS);
            arena.vtable(party, engine_native_parts::cpolitical_party_layout::VTABLE_RVA.0);
            arena.i64(party + cpolitical_party::POPULARITY, 4_500_000);
            for index in 0..IDEAS {
                let mut spec = ChildSpec::new(
                    index as i64 + 1,
                    format!("synthetic idea display {slot:04} {index:02}"),
                );
                let idea = arena.alloc(IDEA_MODIFIER + cmodifier::DISPLAY_NAME + 32);
                arena.pointer(ideas + index * 8, idea);
                arena.vtable(idea, cidea::VTABLE_RVA.0);
                arena.string(
                    idea + cidea::TOKEN,
                    format!("synthetic_idea_{slot:04}_{index:02}").as_bytes(),
                );
                arena.modifier(idea + IDEA_MODIFIER, &spec);
                spec.display = format!("synthetic politics idea label {slot:04} {index:02}");
                children.push(spec);
            }

            let leader = arena.alloc(LEADER_MODIFIER + cmodifier::DISPLAY_NAME + 32);
            let leaders = arena.alloc(8);
            let character = arena.character(slot as u32 * 4);
            let spec = ChildSpec::new(16, format!("synthetic leader display {slot:04}"));
            arena.vtable(leader, engine_native_parts::ccountry_leader_layout::VTABLE_RVA.0);
            arena.pointer(leader + engine_native_parts::ccountry_leader_layout::CHARACTER, character);
            arena.pointer(leaders, leader);
            arena.array(party + engine_native_parts::cpolitical_party_layout::COUNTRY_LEADERS, leaders, 1);
            arena.modifier(leader + LEADER_MODIFIER, &spec);
            children.push(spec);

            let group = arena.alloc(IDEOLOGY_GROUP_MODIFIER + cmodifier::DISPLAY_NAME + 32);
            let spec = ChildSpec::new(17, format!("synthetic ideology display {slot:04}"));
            arena.vtable(group, cideology_group::VTABLE_RVA.0);
            arena.pointer(party + engine_native_parts::cpolitical_party_layout::IDEOLOGY_GROUP, group);
            arena.string(
                group + cideology_group::TOKEN,
                format!("synthetic_ideology_{slot:04}").as_bytes(),
            );
            arena.modifier(group + IDEOLOGY_GROUP_MODIFIER, &spec);
            children.push(spec);

            Self::unpaired(&mut children, slot, 10);
            let characters = arena.alloc(ccountry_characters::APPOINTED_ADVISORS + 16);
            let advisors = arena.alloc(ADVISORS * 8);
            arena.pointer(country + ccountry::CHARACTERS, characters);
            arena.vtable(characters, engine_native::ccountry::rosters::VTABLE_RVA.0);
            arena.pointer(characters + engine_native::ccountry::rosters::OWNER, country);
            arena.array(
                characters + ccountry_characters::APPOINTED_ADVISORS,
                advisors,
                ADVISORS,
            );
            for index in 0..ADVISORS {
                let advisor = arena.alloc(ADVISOR_MODIFIER + cmodifier::DISPLAY_NAME + 32);
                let character = arena.character(slot as u32 * 4 + index as u32 + 1);
                let spec = ChildSpec::new(
                    28 + index as i64,
                    format!("synthetic advisor display {slot:04} {index:02}"),
                );
                arena.pointer(advisors + index * 8, advisor);
                arena.vtable(advisor, cadvisor::VTABLE_RVA.0);
                arena.pointer(advisor + cadvisor::CHARACTER, character);
                arena.string(
                    advisor + cadvisor::IDEA_TOKEN,
                    format!("synthetic_advisor_{slot:04}_{index:02}").as_bytes(),
                );
                arena.modifier(advisor + ADVISOR_MODIFIER, &spec);
                children.push(spec);
            }
            for index in ACTIVE_STATICS {
                children.push(statics[index].clone());
            }
            Self::unpaired(&mut children, slot, 20);

            let entries = arena.alloc(DYNAMICS * dynamic_modifier_entry::STRIDE);
            arena.array(
                country + ccountry::DYNAMIC_MODIFIERS + cdynamic_modifier_container::ENTRIES,
                entries,
                DYNAMICS,
            );
            for index in 0..DYNAMICS {
                let entry = entries + index * dynamic_modifier_entry::STRIDE;
                let definition = arena.alloc(cdynamic_modifier::VALUE_COUNT + 4);
                let rows = arena.alloc(3 * DYNAMIC_VALUE_ROW_STRIDE);
                let values = arena.alloc(3 * 8);
                let display = format!("synthetic dynamic display {slot:04} {index:02}");
                let constant = ChildSpec::new(1_000 + index as i64 * 2, display.clone());
                let evaluated = ChildSpec::new(1_001 + index as i64 * 2, display);
                arena.vtable(entry, dynamic_modifier_entry::VTABLE_RVA.0);
                arena.u32(entry + dynamic_modifier_entry::SCOPE_COUNTRY, slot as u32);
                arena.u32(entry + dynamic_modifier_entry::DAYS_REMAINING, u32::MAX);
                arena.pointer(entry + dynamic_modifier_entry::DEFINITION, definition);
                arena.u8(entry + dynamic_modifier_entry::ENABLED, 1);
                arena.array(entry + dynamic_modifier_entry::VALUES, values, 3);
                arena.vtable(definition, cdynamic_modifier::VTABLE_RVA.0);
                arena.string(
                    definition + cdynamic_modifier::ID,
                    format!("synthetic_dynamic_{slot:04}_{index:02}").as_bytes(),
                );
                arena.modifier(definition + DYNAMIC_CONSTANT, &constant);
                arena.pointer(definition + DYNAMIC_VALUE_ROWS, rows);
                arena.u32(definition + cdynamic_modifier::VALUE_COUNT, 3);
                for (value_index, &(key, value)) in evaluated.entries.iter().enumerate() {
                    arena.u32(rows + value_index * DYNAMIC_VALUE_ROW_STRIDE, key);
                    arena.i64(values + value_index * 8, value);
                }
                children.push(constant);
                children.push(evaluated);
            }
            for index in ACTIVE_SCALED {
                let mut spec = statics[index].clone();
                for (_, value) in &mut spec.entries {
                    *value *= 2;
                }
                children.push(spec);
            }
            let remainder = CHILDREN - children.len();
            Self::unpaired(&mut children, slot, remainder);
            assert_eq!(children.len(), CHILDREN);
            let child_pointers = arena.alloc(CHILDREN * 8);
            arena.array(country + MODIFIER_CHILDREN, child_pointers, CHILDREN);
            let mut totals = [(STABILITY_KEY, 0), (WAR_SUPPORT_KEY, 0), (UNTRACKED_KEY, 0)];
            for (index, spec) in children.iter().enumerate() {
                let child = arena.alloc(cmodifier::DISPLAY_NAME + 32);
                arena.pointer(child_pointers + index * 8, child);
                arena.vtable(child, cmodifier::VTABLE_RVA.0);
                arena.modifier(child, spec);
                for ((_, total), (_, value)) in totals.iter_mut().zip(spec.entries) {
                    *total += value;
                }
            }
            let root_entries = arena.entries(&totals);
            arena.array(
                country + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
                root_entries,
                totals.len(),
            );
            countries.push(engine_native_memory::country_identity::Country {
                pointer: country,
                canonical_slot: slot as u32,
                id: slot as u32,
            });
        }
        let arena = arena.freeze();
        let base = arena.base();
        for country in &mut countries {
            country.pointer += base;
        }
        Self {
            game_state: base + game_state,
            keys: Keys {
                keys: std::array::from_fn(|index| Some(index as u32 + 1)),
                table: Array {
                    data: base + table,
                    count: table_count as u32,
                },
            },
            arena,
            countries,
        }
    }

    fn unpaired(children: &mut Vec<ChildSpec>, slot: usize, count: usize) {
        for _ in 0..count {
            let index = children.len();
            children.push(ChildSpec::new(
                10_000 + index as i64,
                format!("synthetic unreplayed display {slot:04} {index:03}"),
            ));
        }
    }
}

#[derive(Clone, Copy)]
enum Mode {
    HourShared,
    FreshPerCountry,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::HourShared => "hour-shared",
            Self::FreshPerCountry => "fresh-per-country",
        }
    }
}

#[derive(Default)]
struct Retained {
    local_bytes: usize,
    local_spans: usize,
    static_bytes: usize,
    static_spans: usize,
}

struct Run {
    mode: Mode,
    memo: Memo,
    scratch: Scratch,
    inputs: Vec<InputRow>,
    lines: Vec<LineRow>,
    bytes: Vec<u8>,
    lines_len: usize,
    bytes_len: usize,
    retained: Retained,
}

type WorkerMemory<'a> = CountryMemory<'a, CountedMemory<LiveMemory>, false>;

impl Run {
    fn new(mode: Mode, countries: usize) -> Self {
        Self {
            mode,
            memo: Memo::default(),
            scratch: Scratch::vacant(),
            inputs: vec![InputRow::default(); countries],
            lines: Vec::new(),
            bytes: Vec::new(),
            lines_len: 0,
            bytes_len: 0,
            retained: Retained::default(),
        }
    }

    fn hour<'memory>(
        &mut self,
        fixture: &Fixture,
        memory: &WorkerMemory<'memory>,
        countries: &ccountry::Reader<'_, WorkerMemory<'memory>>,
        context: &Context,
        observe_retention: bool,
    ) {
        self.memo.clear();
        self.lines_len = 0;
        self.bytes_len = 0;
        self.retained = Retained::default();
        for (index, &country) in fixture.countries.iter().enumerate() {
            let mut fresh = match self.mode {
                Mode::HourShared => None,
                Mode::FreshPerCountry => Some(Memo::default()),
            };
            let memo = fresh.as_mut().unwrap_or(&mut self.memo);
            let before = if observe_retention {
                memo.retained()
            } else {
                (0, 0)
            };
            let walked = walk_country_memo(
                memory,
                fixture.arena.base(),
                countries,
                country,
                context,
                memo,
                &mut self.scratch,
                &mut self.inputs[index],
                &mut self.lines,
                &mut self.bytes,
                self.lines_len,
                self.bytes_len,
            )
            .unwrap();
            assert_eq!(walked.lines, Outcome::Rows { len: LINES });
            assert_eq!(self.scratch.batch_fallbacks(), 0);
            self.lines_len += walked.lines.len();
            self.bytes_len += walked.bytes_len;
            if observe_retention {
                let (bytes, spans) = self.scratch.observations.retained();
                self.retained.local_bytes += bytes;
                self.retained.local_spans += spans;
                let (bytes, spans) = memo.retained();
                self.retained.static_bytes += bytes - before.0;
                self.retained.static_spans += spans - before.1;
            }
        }
        black_box((
            &self.inputs,
            &self.lines[..self.lines_len],
            &self.bytes[..self.bytes_len],
        ));
    }

    fn validate(&self) {
        assert_eq!(self.lines_len, self.inputs.len() * LINES);
        for input in &self.inputs {
            assert_eq!(input.children, CHILDREN as u32);
            assert!(input.at_war && input.offensive_war && !input.defensive_war);
            assert_eq!(input.interception_sum_raw, 8 * 777 + 28);
        }
        let mut kinds = [0; 9];
        for row in &self.lines[..LINES] {
            let index = match row.kind {
                Kind::Unpaired => 0,
                Kind::Idea => 1,
                Kind::CountryLeader => 2,
                Kind::Ideology => 3,
                Kind::Advisor => 4,
                Kind::Static => 5,
                Kind::StaticScaled => 6,
                Kind::DynamicModifier => 7,
                Kind::DynamicModifierValues => 8,
            };
            kinds[index] += 1;
        }
        assert_eq!(kinds, [114, 30, 2, 2, 6, 6, 0, 20, 20]);
    }

    fn report(&self, n: usize, times: &mut [u128], reads: ReadCounts) {
        times.sort_unstable();
        let per_country = |total: usize| total as f64 / n as f64;
        let (shared_bytes, shared_spans) = self.memo.retained();
        println!(
            "BENCH | CountryStability | {n} countries | {} | {:.0} ns/country | \
             local retained {:.1} bytes/country {:.1} spans/country | \
             static retained {:.3} bytes/country {:.3} spans/country | \
             shared end {shared_bytes} bytes {shared_spans} spans | \
             local memo allocated {} bytes | shared memo allocated {} bytes | \
             native {:.1} copies/country {:.1} scattered/country {:.1} records/country \
             {:.1} spans/country {:.1} bytes/country",
            self.mode.name(),
            times[times.len() / 2] as f64 / n as f64,
            per_country(self.retained.local_bytes),
            per_country(self.retained.local_spans),
            per_country(self.retained.static_bytes),
            per_country(self.retained.static_spans),
            self.scratch.observations.allocated_bytes(),
            self.memo.allocated_bytes(),
            reads.copy_calls as f64 / n as f64,
            reads.scattered_calls as f64 / n as f64,
            reads.record_calls as f64 / n as f64,
            reads.spans as f64 / n as f64,
            reads.bytes as f64 / n as f64,
        );
    }
}

fn run_fixture(n: usize, timed: bool) {
    let fixture = Fixture::new(n);
    let live = CountedMemory::new(unsafe { LiveMemory::new() });
    let mut cache = Cache::with_capacity(Cache::cells_for_countries(n + 1, 0));
    {
        let seeded = cache.prepare(&live, fixture.arena.base(), fixture.game_state);
        assert_eq!(seeded.seed_status(), SeedStatus::Ready);
    }
    let memory = cache.roster_view(&live);
    let countries =
        ccountry::Reader::new(&memory, fixture.arena.base(), fixture.game_state).unwrap();
    let context = Context::read(
        &memory,
        fixture.arena.base(),
        fixture.game_state,
        fixture.keys,
    )
    .unwrap();
    for &country in &fixture.countries {
        assert_eq!(countries.at(country.canonical_slot).unwrap(), Some(country));
    }
    let mut runs = [
        Run::new(Mode::HourShared, n),
        Run::new(Mode::FreshPerCountry, n),
    ];
    for run in &mut runs {
        run.hour(&fixture, &memory, &countries, &context, false);
        run.validate();
    }
    assert_eq!(runs[0].inputs, runs[1].inputs);
    assert_eq!(
        runs[0].lines[..runs[0].lines_len],
        runs[1].lines[..runs[1].lines_len]
    );
    assert_eq!(
        runs[0].bytes[..runs[0].bytes_len],
        runs[1].bytes[..runs[1].bytes_len]
    );
    if !timed {
        return;
    }

    let mut times = [Vec::with_capacity(RUNS), Vec::with_capacity(RUNS)];
    for round in 0..RUNS {
        for index in [round % 2, 1 - round % 2] {
            ReadCounts::take_current();
            let started = Instant::now();
            runs[index].hour(&fixture, &memory, &countries, &context, false);
            times[index].push(started.elapsed().as_nanos());
        }
    }
    for (index, run) in runs.iter_mut().enumerate() {
        ReadCounts::take_current();
        run.hour(&fixture, &memory, &countries, &context, true);
        let reads = ReadCounts::take_current();
        run.validate();
        run.report(n, &mut times[index], reads);
    }
    println!(
        "FIXTURE | {n} countries | {} arena bytes | {CHILDREN} heap-text children/country | \
         {IDEAS} ideas/country | 1 leader+ideology/country | {ADVISORS} advisors/country | \
         {} shared Context statics | {DYNAMICS} enabled dynamics/country | \
         {LINES} lines/country | {RUNS} measured hours/mode | \
         country census and Context acquisition excluded",
        fixture.arena.0.len(),
        STATIC_SLOTS.len(),
    );
}

#[test]
#[ignore = "synthetic capture bench; run with --release -- --ignored --nocapture"]
fn bench_country_stability_memos() {
    if cfg!(debug_assertions) {
        panic!("this benchmark requires --release");
    }
    for n in [1_500, 5_000] {
        run_fixture(n, true);
    }
}

#[test]
fn stability_bench_fixture_walks_in_both_modes() {
    run_fixture(3, false);
}
