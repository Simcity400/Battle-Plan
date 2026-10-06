//! Populated country-scale coverage for the character inputs
//! (CharacterRoleTraits, CharacterOperatives) and the retired records
//! (CountryRetiredCharacters): at 1,500 and 5,000 countries, every country
//! holds an active character with a country-leader, advisor and scientist
//! role (one trait each) and an operative reference the registry does not
//! hold, and a retired character. The live demand sizes a complete first
//! capture; the default capacities truncate and grow to the whole table.

use super::{Capacities, Captured, Demand};
use crate::{country_scale, extended::hired_advisors};
use engine::{
    Memory, cadvisor, ccharacter, ccharacter_manager, ccountry, ccountry_characters,
    ccountry_leader, cgame_state as gs, character_advisor_node, character_leader_node,
    creference_registry, cscientist, cunit_leader, msvc_tree_node as node, walk::Outcome,
};
use replay::frame::TableView;
use std::collections::BTreeMap;

const BASE: usize = 0x1_0000_0000;
const STATE: usize = 0x1000;
const MANAGER: usize = 0x2000;
const TRAITS: usize = 0x3000;
const TABLE: usize = 0x4000;
/// Per country: its object, character object, records, two characters, their
/// maps, roles, scientist and trait pointers.
const BLOCK: usize = 0x1800;

struct Populated {
    data: Vec<u8>,
    globals: BTreeMap<usize, u8>,
}

impl Memory for Populated {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address >= BASE {
            return output.iter_mut().enumerate().all(|(index, byte)| {
                self.globals
                    .get(&(address + index))
                    .map(|value| *byte = *value)
                    .is_some()
            });
        }
        match self.data.get(address..address + output.len()) {
            Some(bytes) => {
                output.copy_from_slice(bytes);
                true
            }
            None => false,
        }
    }
}

impl Populated {
    fn u64(&mut self, at: usize, value: u64) {
        self.data[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: usize) {
        self.u64(at, data as u64);
        self.u32(at + 8, count as u32);
        self.u32(at + 12, count as u32);
    }
    fn vtable(&mut self, at: usize, rva: usize) {
        self.u64(at, (BASE + rva) as u64);
    }
    /// A map at `character + offset`: one node, or none.
    fn map(&mut self, character: usize, offset: usize, head: usize, entry: Option<usize>) {
        self.u64(character + offset, head as u64);
        self.u64(character + offset + 8, u64::from(entry.is_some()));
        let first = entry.unwrap_or(head);
        for link in [node::LEFT, node::PARENT, node::RIGHT] {
            self.u64(head + link, first as u64);
        }
        self.data[head + node::NIL] = 1;
        if let Some(entry) = entry {
            for link in [node::LEFT, node::PARENT, node::RIGHT] {
                self.u64(entry + link, head as u64);
            }
        }
    }
    fn character(&mut self, at: usize, id: u32, country: u32) {
        self.vtable(at, ccharacter::VTABLE_RVA);
        self.u32(at + ccharacter::TYPE, ccharacter::REFERENCE_TYPE);
        self.u32(at + ccharacter::ID, id);
        self.u32(at + ccharacter::COUNTRY, country);
    }

    fn new(countries: usize) -> Self {
        let slots = countries + 1;
        let blocks = TABLE + slots * 12;
        let blocks = blocks.next_multiple_of(0x1000);
        let mut m = Self {
            data: vec![0; blocks + countries * BLOCK + countries * 16],
            globals: BTreeMap::new(),
        };
        for (rva, value) in [
            (gs::SINGLETON_RVA, STATE as u64),
            (creference_registry::HIGH_ROOT_RVA, 0),
        ] {
            m.globals.extend(
                value
                    .to_le_bytes()
                    .into_iter()
                    .enumerate()
                    .map(|(index, byte)| (BASE + rva + index, byte)),
            );
        }
        let links = TABLE + slots * 8;
        m.array(STATE + gs::COUNTRIES, TABLE, slots);
        m.array(STATE + gs::COUNTRY_LINKS, links, slots);
        m.u64(STATE + gs::CHARACTER_MANAGER, MANAGER as u64);
        m.vtable(MANAGER, ccharacter_manager::VTABLE_RVA);
        let roster = blocks + countries * BLOCK;
        m.array(
            MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
            roster,
            countries * 2,
        );
        for (index, (rva, token)) in [
            (ccountry_leader::TRAIT_VTABLE_RVA, 101),
            (ccountry_leader::TRAIT_VTABLE_RVA, 102),
            (cscientist::TRAIT_VTABLE_RVA, 201),
        ]
        .into_iter()
        .enumerate()
        {
            m.vtable(TRAITS + index * 0x10, rva);
            m.u32(TRAITS + index * 0x10 + 8, token);
        }
        for slot in 1..slots {
            let b = blocks + (slot - 1) * BLOCK;
            let (country, object) = (b, b + 0x1000);
            let (records, active, retired) = (b + 0x1040, b + 0x1100, b + 0x1200);
            m.u64(TABLE + slot * 8, country as u64);
            m.u32(links + slot * 4, slot as u32);
            m.vtable(country, ccountry::VTABLE_RVA);
            m.u32(country + ccountry::ID, slot as u32);
            m.u64(country + ccountry::CHARACTERS, object as u64);
            m.vtable(object, ccountry_characters::VTABLE_RVA);
            m.u64(object + ccountry_characters::OWNER, country as u64);
            m.array(object + ccountry_characters::ACTIVE_CHARACTERS, records, 1);
            m.array(
                object + ccountry_characters::RETIRED_CHARACTERS,
                records + 0x10,
                1,
            );
            m.u64(records, active as u64);
            m.u64(records + 0x10, retired as u64);
            let id = slot as u32 * 2;
            m.character(active, id, slot as u32);
            m.character(retired, id + 1, 0);
            m.u64(roster + (slot - 1) * 16, active as u64);
            m.u64(roster + (slot - 1) * 16 + 8, retired as u64);
            // The active character's roles, one trait each.
            let (leader_node, advisor_node) = (b + 0x1330, b + 0x1390);
            let (leader, advisor, scientist) = (b + 0x1440, b + 0x14a0, b + 0x1580);
            m.map(
                active,
                ccharacter::LEADER_MAP_HEAD,
                b + 0x1300,
                Some(leader_node),
            );
            m.map(
                active,
                ccharacter::ADVISOR_MAP_HEAD,
                b + 0x1360,
                Some(advisor_node),
            );
            m.u64(leader_node + character_leader_node::IDEOLOGY, 5);
            m.u64(leader_node + character_leader_node::LEADER, leader as u64);
            m.u64(
                advisor_node + character_advisor_node::ADVISOR,
                advisor as u64,
            );
            m.vtable(leader, ccountry_leader::VTABLE_RVA);
            m.u64(leader + ccountry_leader::CHARACTER, active as u64);
            m.array(leader + ccountry_leader::TRAITS, b + 0x15c0, 1);
            m.u64(b + 0x15c0, TRAITS as u64);
            m.vtable(advisor, cadvisor::VTABLE_RVA);
            m.u64(advisor + cadvisor::CHARACTER, active as u64);
            m.array(advisor + cadvisor::TRAITS, b + 0x15c8, 1);
            m.u64(b + 0x15c8, (TRAITS + 0x10) as u64);
            m.u64(active + ccharacter::SCIENTIST, scientist as u64);
            m.vtable(scientist, cscientist::VTABLE_RVA);
            m.array(scientist + cscientist::TRAIT_DATA, b + 0x15d0, 1);
            m.u64(b + 0x15d0, (TRAITS + 0x20) as u64);
            m.u64(
                active + ccharacter::OPERATIVE,
                (u64::from(id) << 32) | u64::from(cunit_leader::REFERENCE_TYPE),
            );
            // The retired character has empty maps and no other role.
            m.map(retired, ccharacter::LEADER_MAP_HEAD, b + 0x13e0, None);
            m.map(retired, ccharacter::ADVISOR_MAP_HEAD, b + 0x1410, None);
        }
        m
    }
}

fn rows(table: &TableView) -> usize {
    match table {
        TableView::Complete(rows) => rows.len(),
        other => panic!("not complete: {other:?}"),
    }
}

fn check(countries: usize) {
    let world = Populated::new(countries);
    let demand = country_scale::observe_politics(&world, BASE).unwrap();
    assert_eq!(
        (demand.active_characters, demand.retired_characters),
        (countries, countries)
    );
    assert_eq!(
        demand.character_inputs,
        Demand {
            traits: 3 * countries,
            values: 3 * countries,
            operatives: countries,
            definitions: 0,
        }
    );

    // The live demand sizes a complete first capture.
    let mut sized = Capacities::default();
    sized.raise_for_demand(demand.character_inputs);
    let mut captured = Captured::with_capacities(&sized);
    captured.capture(&world, BASE, STATE, 0, false);
    assert_eq!(
        captured.outcomes(),
        [
            Outcome::Rows { len: 3 * countries },
            Outcome::Rows { len: countries }
        ]
    );
    let tables = captured.tables();
    assert_eq!(
        (rows(&tables[0]), rows(&tables[1])),
        (3 * countries, countries)
    );
    let mut records = hired_advisors::Capacities::default();
    records.raise_for_character_records(demand.active_characters, demand.retired_characters);
    let mut hired = hired_advisors::Captured::with_capacities(&records);
    hired.capture(&world, BASE, STATE, 0, false);
    assert_eq!(hired.outcomes()[1], Outcome::Rows { len: countries });
    assert_eq!(hired.outcomes()[3], Outcome::Rows { len: countries });
    assert_eq!(rows(&hired.tables()[3]), countries);

    // The default capacities truncate the larger tables and grow to them.
    let mut capacity = Capacities::default();
    let mut captured = Captured::with_capacities(&capacity);
    captured.capture(&world, BASE, STATE, 0, false);
    assert!(
        matches!(captured.outcomes()[1], Outcome::Truncated { len, unread_rows } if len + unread_rows as usize == countries)
    );
    captured.grow(&mut capacity);
    let mut captured = Captured::with_capacities(&capacity);
    captured.capture(&world, BASE, STATE, 0, false);
    assert!(
        captured
            .outcomes()
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Rows { .. }))
    );
    let mut capacity = hired_advisors::Capacities::default();
    let mut hired = hired_advisors::Captured::with_capacities(&capacity);
    hired.capture(&world, BASE, STATE, 0, false);
    assert!(matches!(hired.outcomes()[3], Outcome::Truncated { .. }));
    hired.grow(&mut capacity);
    let mut hired = hired_advisors::Captured::with_capacities(&capacity);
    hired.capture(&world, BASE, STATE, 0, false);
    assert_eq!(hired.outcomes()[3], Outcome::Rows { len: countries });
}

#[test]
fn fifteen_hundred_countries_are_read_whole_from_their_live_demand() {
    check(1_500);
}

#[test]
fn five_thousand_countries_are_read_whole_from_their_live_demand() {
    check(5_000);
    // At 5,000 countries the role traits outgrow their default too.
    let world = Populated::new(5_000);
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.capture(&world, BASE, STATE, 0, false);
    assert!(matches!(captured.outcomes()[0], Outcome::Truncated { .. }));
}
