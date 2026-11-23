use super::*;
use engine_decisions::cdecision;
use engine_native_politics::{ccountry_leader, cpolitical_status};
use engine_native::{
    cdynamic_modifier,
    cgame_state,
    cidea,
    cideology_group,
    memory::SliceMemory,
    walk::Outcome,
};
use engine_stability::dynamic_modifier_entry;

const BASE: usize = 0x1_4000_0000;
const STATE: usize = 0x1000;
const COUNTRY_COUNT: usize = 5;
const PARTIES: usize = 9;
const MODIFIERS: usize = 11;
const TOKEN: &[u8] = b"mod_population";

struct World {
    bytes: Vec<u8>,
    countries: Vec<usize>,
}

impl World {
    fn alloc(&mut self, len: usize) -> usize {
        let address = self.bytes.len();
        self.bytes.resize(address + len, 0);
        address
    }
    fn put(&mut self, address: usize, bytes: &[u8]) {
        self.bytes[address..address + bytes.len()].copy_from_slice(bytes);
    }
    fn u64(&mut self, address: usize, word: u64) {
        self.put(address, &word.to_le_bytes());
    }
    fn u32(&mut self, address: usize, word: u32) {
        self.put(address, &word.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: usize) {
        self.u64(address, data as u64);
        self.u32(address + 8, count as u32);
        self.u32(address + 12, count as u32);
    }
    fn text(&mut self, address: usize) {
        self.put(address, TOKEN);
        self.u64(address + 16, TOKEN.len() as u64);
        self.u64(address + 24, 15);
    }
    fn new(count: usize) -> Self {
        let mut world = Self {
            bytes: vec![0; 0x2000],
            countries: Vec::new(),
        };
        let table = world.alloc((count + 1) * 8);
        let links = world.alloc((count + 1) * 4);
        world.array(STATE + cgame_state::COUNTRIES, table, count + 1);
        world.array(STATE + cgame_state::COUNTRY_LINKS, links, count + 1);
        let ideologies: Vec<_> = (0..PARTIES)
            .map(|ordinal| {
                let address = world.alloc(0x40);
                world.u64(address, (BASE + cideology_group::VTABLE_RVA.0) as u64);
                world.u32(address + cideology_group::ID, ordinal as u32);
                world.text(address + cideology_group::TOKEN);
                address
            })
            .collect();
        let idea = world.alloc(cidea::TOKEN + 32);
        world.u64(idea, (BASE + cidea::VTABLE_RVA.0) as u64);
        world.text(idea + cidea::TOKEN);
        let active_ideas = world.alloc(2 * 8);
        world.u64(active_ideas, idea as u64);
        world.u64(active_ideas + 8, idea as u64);
        let timed_ideas = world.alloc(2 * cpolitical_status::TIMED_IDEA_STRIDE);
        for ordinal in 0..2 {
            let entry = timed_ideas + ordinal * cpolitical_status::TIMED_IDEA_STRIDE;
            world.u64(
                entry,
                (BASE + cpolitical_status::TIMED_IDEA_VTABLE_RVA.0) as u64,
            );
            world.u64(entry + 8, idea as u64);
            world.u32(entry + 16, [i32::MIN as u32, 17][ordinal]);
        }
        let leader = world.alloc(ccountry_leader::LEADER_ID + 4);
        world.u64(leader, (BASE + ccountry_leader::VTABLE_RVA.0) as u64);
        world.u32(leader + ccountry_leader::LEADER_ID, (-17_i32) as u32);
        let leaders = world.alloc(2 * 8);
        world.u64(leaders, leader as u64);
        world.u64(leaders + 8, leader as u64);
        let definition = world.alloc(0x200);
        world.u64(definition, (BASE + cdynamic_modifier::VTABLE_RVA.0) as u64);
        world.text(definition + cdynamic_modifier::ID);
        world.u32(definition + cdynamic_modifier::VALUE_COUNT, 7);
        let entries = world.alloc(MODIFIERS * dynamic_modifier_entry::STRIDE);
        let values = world.alloc(7 * 8);
        for ordinal in 0..MODIFIERS {
            let entry = entries + ordinal * dynamic_modifier_entry::STRIDE;
            world.u64(entry, (BASE + dynamic_modifier_entry::VTABLE_RVA.0) as u64);
            world.u64(
                entry + dynamic_modifier_entry::DEFINITION,
                definition as u64,
            );
            world.array(entry + dynamic_modifier_entry::VALUES, values, 7);
        }
        for id in 1..=count {
            let country = world.alloc(0x1100);
            world.countries.push(country);
            world.u64(table + id * 8, country as u64);
            world.u32(links + id * 4, id as u32);
            world.u64(country, (BASE + ccountry::VTABLE_RVA.0) as u64);
            world.u32(country + ccountry::ID, id as u32);
            let status = world.alloc(0x110);
            world.u64(country + ccountry::POLITICAL_STATUS, status as u64);
            world.u64(status, (BASE + cpolitical_status::VTABLE_RVA.0) as u64);
            world.u64(status + cpolitical_status::COUNTRY, country as u64);
            world.array(status + cpolitical_status::IDEAS, active_ideas, 2);
            world.array(status + cpolitical_status::TIMED_IDEAS, timed_ideas, 2);
            let parties = world.alloc(PARTIES * 8);
            world.array(status + cpolitical_status::PARTIES, parties, PARTIES);
            for (ordinal, &ideology) in ideologies.iter().enumerate() {
                let party = world.alloc(0xa0);
                world.u64(parties + ordinal * 8, party as u64);
                world.u64(party, (BASE + cpolitical_party::VTABLE_RVA.0) as u64);
                world.u32(party + cpolitical_party::ID, ordinal as u32);
                world.u64(party + cpolitical_party::IDEOLOGY_GROUP, ideology as u64);
                world.u32(party + cpolitical_party::TAG, id as u32);
                world.text(party + cpolitical_party::NAME);
                world.text(party + cpolitical_party::LONG_NAME);
                world.array(party + cpolitical_party::COUNTRY_LEADERS, leaders, 2);
            }
            let decisions = world.alloc(cdecision_status::RANDOM_ITEMS + 16);
            let random = world.alloc(cdecision_status::RANDOM_ITEM_STRIDE * 2);
            let definition = world.alloc(cdecision::TOKEN + 32);
            world.u64(country + ccountry::DECISION_STATUS, decisions as u64);
            world.u64(decisions, (BASE + cdecision_status::VTABLE_RVA.0) as u64);
            world.u64(decisions + cdecision_status::COUNTRY, country as u64);
            world.array(decisions + cdecision_status::RANDOM_ITEMS, random, 2);
            world.u64(definition, (BASE + cdecision::VTABLE_RVA.0) as u64);
            world.text(definition + cdecision::TOKEN);
            for ordinal in 0..2 {
                let entry = random + ordinal * cdecision_status::RANDOM_ITEM_STRIDE;
                world.u64(
                    entry,
                    (BASE + cdecision_status::RANDOM_ITEM_VTABLE_RVA.0) as u64,
                );
                world.u64(
                    entry + cdecision_status::RANDOM_ITEM_DEFINITION,
                    definition as u64,
                );
                world.u32(entry + cdecision_status::RANDOM_ITEM_SEED, ordinal as u32);
                world.u32(entry + cdecision_status::RANDOM_ITEM_TARGET, u32::MAX);
            }
            let container = country + ccountry::DYNAMIC_MODIFIERS;
            world.u64(
                container,
                (BASE + cdynamic_modifier_container::VTABLE_RVA.0) as u64,
            );
            world.u64(
                container + cdynamic_modifier_container::COUNTRY,
                country as u64,
            );
            world.array(
                container + cdynamic_modifier_container::ENTRIES,
                entries,
                MODIFIERS,
            );
        }
        world
    }
    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
    }
}

#[test]
fn first_capture_grows_from_original_headers_and_shares_sizes_with_idle_frames() {
    let world = World::new(COUNTRY_COUNT);
    let mut capacity = Capacities::default();
    let mut captured = super::super::Captured::with_capacities(&capacity);
    captured.capture(&world.memory(), BASE, STATE, 0, true);
    assert_eq!(
        captured.parties.outcome,
        Outcome::Rows {
            len: COUNTRY_COUNT * PARTIES
        }
    );
    assert_eq!(
        captured.modifiers.outcome,
        Outcome::Rows {
            len: COUNTRY_COUNT * MODIFIERS
        }
    );
    assert_eq!(
        captured.party_text.len,
        COUNTRY_COUNT * PARTIES * TOKEN.len() * 3
    );
    assert_eq!(captured.modifier_values.len, COUNTRY_COUNT * MODIFIERS * 7);
    assert_eq!(
        captured.random_items.outcome,
        Outcome::Rows {
            len: COUNTRY_COUNT * 2
        }
    );
    assert_eq!(
        captured.random_item_text.len,
        COUNTRY_COUNT * 2 * TOKEN.len()
    );
    assert!(
        captured.party_countries.first[..captured.party_countries.len]
            .chunks_exact(PARTIES)
            .enumerate()
            .all(|(index, rows)| rows.iter().all(|&id| id as usize == index + 1))
    );
    captured.reset();
    captured.grow(&mut capacity);
    let mut idle = super::super::Captured::with_capacities(&capacity);
    idle.capture(&world.memory(), BASE, STATE, 1, true);
    assert_eq!(
        idle.parties.outcome,
        Outcome::Rows {
            len: COUNTRY_COUNT * PARTIES
        }
    );
    assert_eq!(
        idle.modifiers.outcome,
        Outcome::Rows {
            len: COUNTRY_COUNT * MODIFIERS
        }
    );
}

#[test]
fn failed_party_hints_keep_readable_modifier_and_other_country_demand() {
    let mut world = World::new(COUNTRY_COUNT);
    let country = world.countries[0];
    world.u64(country + ccountry::POLITICAL_STATUS, 0);
    let mut capacity = Capacities::default();
    assert!(capacity.presize(&world.memory(), BASE, STATE).is_err());
    assert!(capacity.rows[6] >= (COUNTRY_COUNT - 1) * PARTIES);
    assert!(capacity.rows[10] >= COUNTRY_COUNT * MODIFIERS);
    assert!(capacity.modifier_values >= COUNTRY_COUNT * MODIFIERS * 7);
}
