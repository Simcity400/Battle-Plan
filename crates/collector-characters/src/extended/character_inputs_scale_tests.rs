use super::{Capacities, Captured, Demand};
use engine_characters::cscientist;
use engine_native::{
    Memory, cadvisor, ccharacter, ccharacter_manager, ccountry, ccountry_characters,
    ccountry_leader, cgame_state as gs, character_advisor_node, character_leader_node,
    creference_registry, cunit_leader, msvc_tree_node as node,
};
use replay_frame::frame::TableView;
use std::collections::BTreeMap;

pub(super) const BASE: usize = 0x1_0000_0000;
pub(super) const STATE: usize = 0x1000;
pub(super) const MANAGER: usize = 0x2000;
pub(super) const TRAITS: usize = 0x3000;
const TABLE: usize = 0x4000;
const BLOCK: usize = 0x1800;

pub(super) struct Populated {
    pub(super) data: Vec<u8>,
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
    pub(super) fn u64(&mut self, at: usize, value: u64) {
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
    fn vtable(&mut self, at: usize, rva: engine_sites::Rva) {
        self.u64(at, (BASE + rva.0) as u64);
    }
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

    pub(super) fn new(countries: usize) -> Self {
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
                    .map(|(index, byte)| (BASE + rva.0 + index, byte)),
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
            m.map(retired, ccharacter::LEADER_MAP_HEAD, b + 0x13e0, None);
            m.map(retired, ccharacter::ADVISOR_MAP_HEAD, b + 0x1410, None);
        }
        m
    }
}

pub(super) fn resolved_operatives(countries: usize) -> (Populated, Vec<usize>) {
    use engine_native::{cname_group_member as naming, reference_bucket as bucket};
    use engine_native_intelligence::coperative_leader;
    let mut world = Populated::new(countries);
    let first = world.data.len();
    let registry = first + countries * 0x1100;
    let entries = registry + 0x40;
    let capacity = (countries * 4).next_power_of_two();
    let overflow = capacity.trailing_zeros() as usize + 2;
    world
        .data
        .resize(entries + (capacity + overflow + 1) * bucket::STRIDE, 0);
    for (index, byte) in (registry as u64).to_le_bytes().into_iter().enumerate() {
        world
            .globals
            .insert(BASE + creference_registry::HIGH_ROOT_RVA.0 + index, byte);
    }
    world.u64(registry + creference_registry::ENTRIES, entries as u64);
    world.u32(registry + creference_registry::COUNT, countries as u32);
    world.u32(registry + creference_registry::MASK, (capacity - 1) as u32);
    world.data[registry + creference_registry::OVERFLOW_DISTANCE] = overflow as u8;
    world.data[entries + (capacity + overflow) * bucket::STRIDE + bucket::DISTANCE] = 255;
    let roster = u64::from_le_bytes(
        world.data[MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS..][..8]
            .try_into()
            .unwrap(),
    ) as usize;
    let mut used = vec![false; capacity];
    let mut id = 0_u32;
    for index in 0..countries {
        let (reference, cell) = loop {
            id += 1;
            let reference = (u64::from(id) << 32) | u64::from(cunit_leader::REFERENCE_TYPE);
            let hash = reference
                .rotate_left(32)
                .wrapping_mul(creference_registry::HASH_MULTIPLIER);
            let cell = ((hash >> 32) as usize ^ hash as usize) & (capacity - 1);
            if !used[cell] {
                used[cell] = true;
                break (reference, cell);
            }
        };
        let character =
            u64::from_le_bytes(world.data[roster + index * 16..][..8].try_into().unwrap()) as usize;
        let leader = first + index * 0x1100;
        world.u64(character + ccharacter::OPERATIVE, reference);
        world.u64(character + ccharacter::UNIT_LEADER, leader as u64);
        world.data[entries + cell * bucket::STRIDE + bucket::DISTANCE] = 1;
        world.u64(entries + cell * bucket::STRIDE + bucket::TYPE, reference);
        world.u64(
            entries + cell * bucket::STRIDE + bucket::OBJECT,
            leader as u64,
        );
        world.vtable(leader, coperative_leader::VTABLE_RVA);
        world.u64(leader + cunit_leader::TYPE, reference);
        world.u32(leader + cunit_leader::CATEGORY, 8);
        world.u32(leader + cunit_leader::OWNER, (index + 1) as u32);
        for (offset, value) in [
            (cunit_leader::NAME, b"A\xfflias".as_slice()),
            (cunit_leader::TOKEN, b"".as_slice()),
            (
                coperative_leader::CODENAME + naming::FIRST_NAME_TEXT,
                b"GROUP".as_slice(),
            ),
            (
                coperative_leader::CODENAME + naming::OVERRIDE_NAME,
                b"Code".as_slice(),
            ),
        ] {
            let at = leader + offset;
            world.data[at..at + value.len()].copy_from_slice(value);
            world.u64(at + 16, value.len() as u64);
            world.u64(at + 24, 15);
        }
        let member = leader + coperative_leader::CODENAME;
        world.vtable(member, naming::VTABLE_RVA);
        world.u32(member + naming::TYPE, 2);
        world.u32(member + naming::ORDER, index as u32);
        world.data[member + naming::FLAG_A] = 1;
    }
    let leaders = (0..countries).map(|index| first + index * 0x1100).collect();
    (world, leaders)
}

#[test]
fn resolved_operatives_carry_their_names_and_codename_operands() {
    use replay_data::rows::TableRows;
    const COUNTRIES: usize = 3;
    let (world, _) = resolved_operatives(COUNTRIES);
    let demand = Demand::observe(&world, BASE, STATE).unwrap();
    assert_eq!(demand.bytes, COUNTRIES * 15);
    let mut capacity = Capacities::default();
    capacity.raise_for_demand(demand);
    let mut captured = Captured::with_capacities(&capacity);
    captured.capture(&world, BASE, STATE, 0, false);
    let [_, TableView::Complete(TableRows::CharacterOperatives(rows))] = captured.tables() else {
        panic!("complete operative inputs");
    };
    assert_eq!(rows.len(), COUNTRIES);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            row.name
                .require("CharacterOperatives.name")
                .unwrap()
                .as_deref(),
            Some(b"A\xfflias".as_slice())
        );
        let name = row
            .codename
            .require("CharacterOperatives.codename")
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(
            (name.group.as_slice(), name.custom.as_slice()),
            (b"GROUP".as_slice(), b"Code".as_slice())
        );
        assert_eq!(name.order, index as i32);
    }
}

#[test]
fn demand_observation_keeps_invalid_roster_errors_named() {
    let mut world = Populated::new(1);
    world.u64(MANAGER, 0);
    assert_eq!(
        Demand::observe(&world, BASE, STATE),
        Err("character manager vtable mismatch")
    );
}
