use super::*;
use crate::rows::{CountryLinkRow, WarRelationRow};
use engine_native::{
    Memory,
    ccountry,
    cgame_state,
    crelation,
    crelation_status,
    cwar_relation,
    memory::SliceMemory,
    read,
    walk::Outcome,
};
use engine_native_politics::cdiplomacy_status;
use std::collections::BTreeMap;

const BASE: usize = 0;
const GAME_STATE: usize = 0x1000;
const COUNTRY_TABLE: usize = 0x10000;
const LINK_TABLE: usize = 0x11000;

fn country(slot: usize) -> usize {
    0x20000 + slot * 0x100
}
fn diplomacy(slot: usize) -> usize {
    0x40000 + slot * 0x400
}
fn owned_data(slot: usize) -> usize {
    0x60000 + slot * 0x400
}
fn matrix(slot: usize) -> usize {
    0x80000 + slot * 0x100
}
fn status(owner: usize, target: usize) -> usize {
    0x100000 + (owner * 64 + target) * 0x800
}
fn war(index: usize) -> usize {
    0x400000 + index * 0x200
}

#[derive(Clone, Copy)]
struct War {
    first: u32,
    second: u32,
    retired: bool,
}

struct World {
    links: Vec<u32>,
    countries: usize,
    wars: Vec<War>,
    cells: BTreeMap<(usize, usize), usize>,
    subrelations: BTreeMap<(usize, usize), Vec<usize>>,
    serializable: BTreeMap<(usize, usize), Vec<usize>>,
    owned: Vec<Vec<usize>>,
    enemies: Vec<Vec<u32>>,
}

impl World {
    fn new(countries: usize, aliases: &[usize]) -> Self {
        let mut links: Vec<u32> = (0..=countries as u32).collect();
        links.extend(aliases.iter().map(|&slot| slot as u32));
        Self {
            links,
            countries,
            wars: Vec::new(),
            cells: BTreeMap::new(),
            subrelations: BTreeMap::new(),
            serializable: BTreeMap::new(),
            owned: vec![Vec::new(); countries + 1],
            enemies: vec![Vec::new(); countries + 1],
        }
    }

    fn slot(&self, raw: u32) -> usize {
        self.links[raw as usize] as usize
    }

    fn replace(&mut self, owner: usize, target: usize, new: Option<usize>) {
        let key = (owner, target);
        if let Some(old) = self.cells.remove(&key) {
            self.subrelations
                .entry(key)
                .or_default()
                .retain(|&w| w != old);
            self.owned[owner].retain(|&w| w != old);
            if self.slot(self.wars[old].first) == owner {
                self.serializable
                    .entry(key)
                    .or_default()
                    .retain(|&w| w != old);
            }
        }
        if let Some(new) = new {
            self.cells.insert(key, new);
            self.subrelations.entry(key).or_default().push(new);
            self.owned[owner].push(new);
            if self.slot(self.wars[new].first) == owner {
                self.serializable.entry(key).or_default().push(new);
            }
        }
    }

    fn declare(&mut self, first: u32, second: u32) -> usize {
        self.declare_war(War {
            first,
            second,
            retired: false,
        })
    }

    fn declare_war(&mut self, war: War) -> usize {
        let index = self.wars.len();
        self.wars.push(war);
        let (first, second) = (self.slot(war.first), self.slot(war.second));
        self.replace(second, first, Some(index));
        self.replace(first, second, Some(index));
        index
    }

    fn end(&mut self, index: usize) {
        let war = self.wars[index];
        let (first, second) = (self.slot(war.first), self.slot(war.second));
        self.replace(second, first, None);
        self.replace(first, second, None);
    }

    fn capitulate(&mut self, slot: usize) {
        for index in self.owned[slot].clone() {
            if !self.wars[index].retired {
                self.end(index);
            }
        }
    }

    fn rebuild(&mut self, slot: usize) {
        let mut enemies: Vec<u32> = self.owned[slot]
            .iter()
            .filter(|&&index| !self.wars[index].retired)
            .map(|&index| {
                let war = self.wars[index];
                if self.slot(war.first) == slot {
                    war.second
                } else {
                    war.first
                }
            })
            .collect();
        enemies.sort_unstable();
        self.enemies[slot] = enemies;
    }

    fn bytes(&self) -> Vec<u8> {
        fn p32(bytes: &mut [u8], at: usize, value: u32) {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        fn p64(bytes: &mut [u8], at: usize, value: u64) {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        fn vector(bytes: &mut [u8], header: usize, data: usize, values: &[u64]) {
            p64(bytes, header, data as u64);
            p32(bytes, header + 8, values.len() as u32);
            p32(bytes, header + 12, values.len() as u32);
            for (index, &value) in values.iter().enumerate() {
                p64(bytes, data + index * 8, value);
            }
        }
        let slots = self.links.len();
        let mut bytes = vec![0; war(self.wars.len() + 1)];
        let table: Vec<u64> = (0..slots)
            .map(|raw| match raw {
                0 => 0,
                raw => country(self.links[raw] as usize) as u64,
            })
            .collect();
        vector(
            &mut bytes,
            GAME_STATE + cgame_state::COUNTRIES,
            COUNTRY_TABLE,
            &table,
        );
        let links = GAME_STATE + cgame_state::COUNTRY_LINKS;
        p64(&mut bytes, links, LINK_TABLE as u64);
        p32(&mut bytes, links + 8, slots as u32);
        p32(&mut bytes, links + 12, slots as u32);
        for (raw, &slot) in self.links.iter().enumerate() {
            p32(&mut bytes, LINK_TABLE + raw * 4, slot);
        }
        for owner in 1..=self.countries {
            let (c, d) = (country(owner), diplomacy(owner));
            p64(&mut bytes, c, ccountry::VTABLE_RVA.0 as u64);
            p32(&mut bytes, c + ccountry::ID, owner as u32);
            p64(&mut bytes, c + ccountry::DIPLOMACY_STATUS, d as u64);
            p64(&mut bytes, d, cdiplomacy_status::VTABLE_RVA.0 as u64);
            p64(&mut bytes, d + cdiplomacy_status::COUNTRY, c as u64);
            let cells: Vec<u64> = (0..slots).map(|t| status(owner, t) as u64).collect();
            vector(
                &mut bytes,
                d + cdiplomacy_status::RELATIONS,
                matrix(owner),
                &cells,
            );
            let owned: Vec<u64> = self.owned[owner].iter().map(|&w| war(w) as u64).collect();
            vector(
                &mut bytes,
                d + cdiplomacy_status::OWNED_RELATIONS,
                owned_data(owner),
                &owned,
            );
            for target in 0..slots {
                let s = status(owner, target);
                p64(&mut bytes, s, crelation_status::VTABLE_RVA.0 as u64);
                p64(&mut bytes, s + crelation_status::DIPLOMACY_STATUS, d as u64);
                p32(
                    &mut bytes,
                    s + crelation_status::TARGET_COUNTRY,
                    target as u32,
                );
                if let Some(&index) = self.cells.get(&(owner, target)) {
                    p64(
                        &mut bytes,
                        s + crelation_status::WAR_RELATION,
                        war(index) as u64,
                    );
                }
                let list = |map: &BTreeMap<(usize, usize), Vec<usize>>| -> Vec<u64> {
                    map.get(&(owner, target))
                        .map(|wars| wars.iter().map(|&w| war(w) as u64).collect())
                        .unwrap_or_default()
                };
                vector(
                    &mut bytes,
                    s + crelation_status::SUBRELATIONS,
                    s + 0x600,
                    &list(&self.subrelations),
                );
                vector(
                    &mut bytes,
                    s + crelation_status::SERIALIZABLE_RELATIONS,
                    s + 0x700,
                    &list(&self.serializable),
                );
            }
        }
        for (index, w) in self.wars.iter().enumerate() {
            let at = war(index);
            p64(&mut bytes, at, cwar_relation::VTABLE_RVA.0 as u64);
            p32(&mut bytes, at + crelation::TYPE_ID, cwar_relation::TYPE);
            p32(&mut bytes, at + crelation::FIRST_COUNTRY, w.first);
            p32(&mut bytes, at + crelation::SECOND_COUNTRY, w.second);
            bytes[at + crelation::RETIRED] = u8::from(w.retired);
        }
        bytes
    }

    fn recorded(&self, memory: &impl Memory) -> Fact<(Vec<WarRelationRow>, Vec<CountryLinkRow>)> {
        let mut rows = vec![cwar_relation::Row::default(); self.wars.len()];
        let len = match cwar_relation::walk(memory, BASE, GAME_STATE, &mut rows) {
            Outcome::Rows { len } => len,
            outcome => return Err(format!("WarRelations walk incomplete: {outcome:?}")),
        };
        let wars = rows[..len]
            .iter()
            .map(|row| WarRelationRow {
                first_country: row.first_country,
                second_country: row.second_country,
                start_hour: row.start_hour,
                end_hour: row.end_hour,
                cancel: row.cancel,
                first_casualties: row.first_casualties,
                second_casualties: row.second_casualties,
                first_unknown_casualties: row.first_unknown_casualties,
                second_unknown_casualties: row.second_unknown_casualties,
                first_was_instigator: row.first_was_instigator,
            })
            .collect();
        let links = (1..self.links.len() as u32)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: self.links[id as usize],
            })
            .collect();
        Ok((wars, links))
    }
}

fn native(memory: &impl Memory, receiver: usize, target: usize) -> bool {
    let cells = read::<u64>(memory, diplomacy(receiver) + cdiplomacy_status::RELATIONS).unwrap();
    let status = read::<u64>(memory, cells as usize + target * 8).unwrap();
    if status == 0 {
        return false;
    }
    let war = read::<u64>(memory, status as usize + crelation_status::WAR_RELATION).unwrap();
    war != 0 && read::<u8>(memory, war as usize + crelation::RETIRED).unwrap() == 0
}

#[track_caller]
fn assert_derived_matches(world: &World) {
    let bytes = world.bytes();
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let (wars, links) = world.recorded(&memory).unwrap();
    for receiver in 1..=world.countries {
        let derived = ReceiverWars::from_rows(&wars, &links, receiver as u32).unwrap();
        for target in 0..=world.countries {
            assert_eq!(
                derived.pointer_active(target as u32),
                native(&memory, receiver, target),
                "receiver {receiver} target {target}"
            );
        }
    }
}

fn active_targets(world: &World, receiver: usize) -> Vec<u32> {
    let bytes = world.bytes();
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    (0..=world.countries as u32)
        .filter(|&target| native(&memory, receiver, target as usize))
        .collect()
}

#[test]
fn derived_cells_follow_declarations_and_peace() {
    let mut world = World::new(5, &[]);
    assert_derived_matches(&world);
    let first = world.declare(1, 2);
    world.declare(3, 1);
    world.declare(4, 5);
    assert_derived_matches(&world);
    assert_eq!(active_targets(&world, 1), [2, 3]);
    world.end(first);
    assert_derived_matches(&world);
    assert_eq!(active_targets(&world, 1), [3]);
    assert!(active_targets(&world, 2).is_empty());
}

#[test]
fn derived_cells_follow_capitulation() {
    let mut world = World::new(4, &[]);
    world.declare(3, 1);
    world.declare(2, 3);
    world.declare(1, 4);
    assert_derived_matches(&world);
    world.capitulate(3);
    assert_derived_matches(&world);
    assert!(active_targets(&world, 3).is_empty());
    assert_eq!(active_targets(&world, 1), [4]);
}

#[test]
fn derived_cells_resolve_alias_endpoints_to_canonical_slots() {
    let mut world = World::new(4, &[2, 4]);
    world.declare(1, 5);
    world.declare(6, 3);
    assert_derived_matches(&world);
    assert_eq!(active_targets(&world, 2), [1]);
    assert_eq!(active_targets(&world, 3), [4]);
}

#[test]
fn derived_cells_follow_a_civil_war_whose_enemy_vector_lags() {
    let mut world = World::new(4, &[]);
    let wars = [world.declare(2, 1), world.declare(1, 3)];
    for slot in 1..=4 {
        world.rebuild(slot);
    }
    for (index, enemy) in [(wars[0], 2), (wars[1], 3)] {
        world.declare(4, enemy);
        world.end(index);
        world.rebuild(1);
        world.rebuild(enemy as usize);
    }
    assert_derived_matches(&world);
    assert_eq!(active_targets(&world, 4), [2, 3]);
    assert!(world.enemies[4].is_empty());
    world.declare(1, 4);
    world.rebuild(1);
    world.rebuild(4);
    assert_derived_matches(&world);
    assert_eq!(world.enemies[4], [1, 2, 3]);
}

#[test]
fn a_retired_war_reads_inactive_on_both_sides() {
    let mut world = World::new(3, &[]);
    world.declare_war(War {
        first: 1,
        second: 2,
        retired: true,
    });
    world.declare(2, 3);
    assert_derived_matches(&world);
    assert!(active_targets(&world, 1).is_empty());
}

#[test]
fn a_cell_outside_its_owned_vector_refuses_instead_of_answering() {
    let mut world = World::new(3, &[]);
    let index = world.declare(1, 2);
    world.owned[2].retain(|&w| w != index);
    let bytes = world.bytes();
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert!(world.recorded(&memory).is_err());
}

#[test]
fn derivation_refuses_unlinked_or_reserved_partners_and_self_wars() {
    let link = |id, canonical_slot| CountryLinkRow { id, canonical_slot };
    let links = [link(1, 1), link(2, 2), link(3, 0), link(4, 1)];
    let war = |first, second| WarRelationRow {
        first_country: first,
        second_country: second,
        ..Default::default()
    };
    let derived = ReceiverWars::from_rows(&[war(1, 2), war(2, 9)], &links, 1).unwrap();
    assert_eq!(derived.targets(), [2]);
    assert!(!derived.pointer_active(0));
    assert!(!derived.pointer_active(1));
    assert!(ReceiverWars::from_rows(&[war(9, 1)], &links, 1).is_err());
    assert!(ReceiverWars::from_rows(&[war(1, 3)], &links, 1).is_err());
    assert!(ReceiverWars::from_rows(&[war(4, 1)], &links, 1).is_err());
    assert!(ReceiverWars::from_rows(&[war(1, 2)], &links, 0).is_err());
    let mut unordered = links;
    unordered.swap(0, 1);
    assert!(ReceiverWars::from_rows(&[], &unordered, 1).is_err());
}
