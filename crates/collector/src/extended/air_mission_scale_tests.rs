//! AirRegionalMissions, AirReferenceWitnesses and NavalMineLevels at 5,000
//! countries, with thousands of registered wings.
//!
//! Slot `s` holds country id `s` (links resolve every id to its own slot),
//! except one empty slot with no country, air or navy object. Every live
//! country has a CStrategicAir object. Every third country registers missions
//! in one to four regions; one heavy country has 70 active regions and a region
//! of 130 missions, so the walk crosses both of its 64-entry batch boundaries.
//! Each region's vector lists its wings in the reverse of census order, and
//! wing handles are a permutation of census order, so neither the mission
//! pointer nor the handle order can stand in for the vector order.
//!
//! Every wing with a non-null region is registered. Unregistered wings are
//! training wings (some with no pool base, some sharing a registered wing's base, some with
//! a base of their own) and idle wings, whose resolvable pool bases must not
//! be probed. Pool bases repeat across wings; a tenth of the wings name a
//! transfer base, some shared with pool bases, some of their own and some
//! that never resolve, and a few pool bases never resolve either. A quarter
//! of the registered wings lay mines; some of them fly for another country
//! (a positive override), so (country, region) pairs repeat.
//!
//! A stateless base's host is a carrier with a task force, a carrier
//! without one, or a handle the registry does not hold. A state base without
//! a cached province takes the first province of its state's list for its
//! kind; a fourth of those lists are empty.
//!
//! Memory is two windows: a heap holding every object, and the reference
//! registry's low-type root cells in the module. Anything else, including the
//! wings themselves (the joins never read a wing), is unreadable.

use super::*;
use engine_map_air::{map_air_base_reference::BaseReference, map_air_base_reference::Host};
use engine_native::{
    cgame_state as gs, creference_registry as registry, cship, cstrategic_air, cstrategic_navy,
    ctask_force, cunit, memory::RecordLayout, memory::RecordSources, reference_bucket as bucket,
};
use replay_data::{rows::AirReferenceWitnessRow, rows::TableSchema};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
};

const BASE: usize = 0x1_4000_0000;
const HEAP: usize = 0x10_0000;
/// Unmapped: the joins follow a wing only by its mission address.
const WINGS: usize = 0x7000_0000_0000;
const COUNTRIES: u32 = 5_000;
/// An hour outside the daily slots, so only the hourly air tables walk.
const HOUR: i32 = 1;
/// Every navy's regional mine vector covers region ids below this.
const REGIONS: u32 = 400;
/// Pool bases are ids `1..=POOL_BASES`, transfer bases `1..=TRANSFER_BASES`.
const POOL_BASES: u64 = 2_500;
const TRANSFER_BASES: u64 = 3_000;
const STATES: u64 = 300;
const PROVINCES: u64 = 1_000;
const COUNTRY_OBJECT: usize = 0x20;

fn handle(id: u64) -> u64 {
    id << 32 | u64::from(cair_base::TYPE)
}

fn province_id(index: u64) -> u32 {
    100 + 3 * (index % PROVINCES) as u32
}

/// The first province of a state's list for a base kind, if it has one.
fn first_province(state: u32, kind: i32) -> Option<u32> {
    let (state, kind) = (u64::from(state), kind as u64);
    (!(state + kind).is_multiple_of(4)).then(|| province_id(state * 7 + kind * 101))
}

/// The fields a registered base's object holds, as the reference reads
/// them (without the object's address).
fn reference(id: u64) -> BaseReference {
    let kind = (id % 3) as i32;
    let state = 1 + (id % STATES) as u32;
    let (state, province) = match id % 10 {
        // Stateless: a hosted base, whose cached province is ignored.
        3 => (None, None),
        // No cached province: the state's first one for the kind.
        7 => (Some(state), first_province(state, kind)),
        _ => (Some(state), Some(province_id(id))),
    };
    BaseReference {
        state,
        province,
        host: id << 32 | u64::from(cship::HANDLE_TYPE),
        kind,
        ..Default::default()
    }
}

/// A stateless base's host: a carrier in a task force, a carrier without
/// one, or a handle that does not resolve.
fn host(id: u64) -> Host {
    match id / 10 % 3 {
        0 => Host::Owner(1 + (id % u64::from(COUNTRIES)) as u32),
        1 => Host::NoTaskForce,
        _ => Host::Unresolved,
    }
}

/// A probed handle's witness: the base, its host's owner and whether the
/// host has no task force.
type Witness = (u64, Option<BaseReference>, Option<u32>, bool);

/// A country's raw regional mine level; zero for a fifth of the pairs.
fn mines(slot: u32, region: u32) -> i64 {
    if (slot + region).is_multiple_of(5) {
        0
    } else {
        i64::from(slot) * 1_000_000 + i64::from(region) * 10 + 7
    }
}

/// The two memory windows.
struct World {
    heap: Vec<u8>,
    roots: Vec<u8>,
    game_state: usize,
    /// Region IDs, region vectors and mission pointers: only the
    /// registration walk's regional phase reads them.
    missions: std::ops::Range<usize>,
    /// Each live country's navy, by slot.
    navies: BTreeMap<u32, usize>,
    /// Each state's object, by index.
    states: Vec<usize>,
    /// Each registered base's object, by handle.
    bases: BTreeMap<u64, usize>,
    /// Each registered carrier host, by handle.
    ships: BTreeMap<u64, usize>,
}

impl World {
    fn alloc(&mut self, len: usize) -> usize {
        let at = (HEAP + self.heap.len() + 0xf) & !0xf;
        self.heap.resize(at + len - HEAP, 0);
        at
    }

    fn put(&mut self, at: usize, value: &[u8]) {
        let roots = BASE + registry::LOW_ROOTS_RVA;
        let (window, offset) = if at >= roots {
            (&mut self.roots, at - roots)
        } else {
            (&mut self.heap, at - HEAP)
        };
        window[offset..offset + value.len()].copy_from_slice(value);
    }

    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }

    fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }

    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.u64(at, data as u64);
        self.u32(at + 8, count);
        self.u32(at + 12, count);
    }

    fn object(&mut self, len: usize, vtable_rva: usize) -> usize {
        let object = self.alloc(len);
        self.u64(object, (BASE + vtable_rva) as u64);
        object
    }

    /// A reference registry table of `kind` holding `objects` by handle,
    /// filled by Robin Hood insertion.
    fn registry(&mut self, kind: u32, objects: &BTreeMap<u64, usize>) {
        let exponent = (objects.len() * 2)
            .next_power_of_two()
            .trailing_zeros()
            .max(registry::MINIMUM_EXPONENT);
        let mask = (1_u32 << exponent) - 1;
        let overflow = exponent + 2;
        let terminal = (mask + 1 + overflow) as usize;
        let mut cells = vec![(0_u32, 0_u64, 0_usize); terminal + 1];
        for (&handle, &object) in objects {
            let hash = handle
                .rotate_left(32)
                .wrapping_mul(registry::HASH_MULTIPLIER);
            let mut index = (((hash >> 32) as u32 ^ hash as u32) & mask) as usize;
            let mut entry = (1, handle, object);
            while cells[index].0 != 0 {
                if cells[index].0 < entry.0 {
                    std::mem::swap(&mut cells[index], &mut entry);
                }
                index += 1;
                entry.0 += 1;
                assert!(index < terminal);
            }
            cells[index] = entry;
        }
        assert!(cells.iter().all(|cell| cell.0 <= overflow + 1));
        cells[terminal].0 = 255;
        let entries = self.alloc(cells.len() * bucket::STRIDE);
        for (index, &(distance, handle, object)) in cells.iter().enumerate() {
            let cell = entries + index * bucket::STRIDE;
            self.put(cell + bucket::DISTANCE, &[distance as u8]);
            self.u64(cell + bucket::TYPE, handle);
            self.u64(cell + bucket::OBJECT, object as u64);
        }
        let root = self.alloc(registry::SIZE);
        self.u64(root + registry::ENTRIES, entries as u64);
        self.u32(root + registry::COUNT, objects.len() as u32);
        self.u32(root + registry::MASK, mask);
        self.put(root + registry::OVERFLOW_DISTANCE, &[overflow as u8]);
        self.u64(
            BASE + registry::LOW_ROOTS_RVA + kind as usize * 8,
            root as u64,
        );
    }
}

fn window(bytes: &[u8], start: usize, address: usize, len: usize) -> Option<&[u8]> {
    let offset = address.checked_sub(start)?;
    bytes.get(offset..offset.checked_add(len)?)
}

impl Memory for World {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let len = destination.len();
        match window(&self.heap, HEAP, address, len)
            .or_else(|| window(&self.roots, BASE + registry::LOW_ROOTS_RVA, address, len))
        {
            Some(source) => {
                destination.copy_from_slice(source);
                true
            }
            None => false,
        }
    }
}

/// One registered country's regions in ascending order, each with its
/// wings' census indices in vector order.
struct Registered {
    slot: u32,
    regions: Vec<(u32, Vec<usize>)>,
}

struct Scale {
    world: World,
    /// The AirWings census in visit order.
    census: Vec<cair_wing::Source>,
    registered: Vec<Registered>,
    empty: u32,
}

impl Scale {
    fn new() -> Self {
        let countries = COUNTRIES;
        let empty = countries * 4 / 5;
        let heavy = 3 * (countries / 6);
        let slots = countries + 1;
        let mut world = World {
            heap: Vec::new(),
            roots: vec![0; registry::LOW_ROOT_COUNT as usize * 8],
            game_state: 0,
            missions: 0..0,
            navies: BTreeMap::new(),
            states: Vec::new(),
            bases: BTreeMap::new(),
            ships: BTreeMap::new(),
        };
        let state = world.alloc(0x800);
        world.game_state = state;
        let live = |slot: u32| slot != empty;

        // The country registry.
        let table = world.alloc(slots as usize * 8);
        let links = world.alloc(slots as usize * 4);
        world.array(state + gs::COUNTRIES, table, slots);
        world.array(state + gs::COUNTRY_LINKS, links, slots);
        for slot in (1..slots).filter(|&slot| live(slot)) {
            let country = world.object(COUNTRY_OBJECT, ccountry::VTABLE_RVA);
            world.u32(country + ccountry::ID, slot);
            world.u64(table + slot as usize * 8, country as u64);
        }
        for slot in 1..slots {
            world.u32(links + slot as usize * 4, slot);
        }

        // The wing census, country by country.
        let mut census = Vec::new();
        let mut registered = Vec::new();
        let mut unique_bases = BTreeSet::new();
        let wing = |census: &mut Vec<cair_wing::Source>, row: cair_wing::Row| {
            let index = census.len();
            census.push(cair_wing::Source {
                pointer: WINGS + index * 0x200,
                pool: 0,
                row: cair_wing::Row {
                    // A permutation of census order.
                    id: ((index as u64 * 7_919) % 1_000_003 + 1) << 32 | 0x47,
                    base_override: match index % 10 {
                        0 => handle(1 + (index as u64 * 13) % TRANSFER_BASES),
                        5 => handle(30_000 + index as u64),
                        _ => 0,
                    },
                    ..row
                },
            });
            index
        };
        for slot in (1..slots).filter(|&slot| live(slot)) {
            if slot.is_multiple_of(3) {
                let ids: Vec<u32> = if slot == heavy {
                    (0..70).map(|j| 3 + j * 5).collect()
                } else {
                    let start = 1 + (slot * 7) % 300;
                    (0..1 + (slot / 3) % 4)
                        .map(|j| start + j * (1 + slot % 5))
                        .collect()
                };
                let mut regions = Vec::new();
                for (j, &region) in ids.iter().enumerate() {
                    let missions = if slot == heavy && j == 10 {
                        130
                    } else {
                        1 + (slot as usize + j) % 3
                    };
                    let mut wings: Vec<usize> = (0..missions)
                        .map(|_| {
                            let index = census.len();
                            let lays = index.is_multiple_of(4);
                            wing(
                                &mut census,
                                cair_wing::Row {
                                    owner: slot,
                                    mission_region: Some(region),
                                    mission_mask: if lays {
                                        rows::AIR_MISSION_LAY_MINES
                                    } else {
                                        1 << (index % 3)
                                    },
                                    air_base: handle(1 + index as u64 % POOL_BASES),
                                    country_override: match index % 12 {
                                        0 => 3 * (1 + (index / 12 % 7) as i32),
                                        4 => -1,
                                        _ => 0,
                                    },
                                    ..Default::default()
                                },
                            )
                        })
                        .collect();
                    wings.reverse();
                    regions.push((region, wings));
                }
                registered.push(Registered { slot, regions });
            }
            match slot % 12 {
                // Training without a region: no base, a shared base, its own.
                1 | 5 | 9 => {
                    let air_base = match slot % 12 {
                        1 => 0,
                        5 => handle(1 + u64::from(slot) % POOL_BASES),
                        _ => {
                            unique_bases.insert(10_000 + u64::from(slot));
                            handle(10_000 + u64::from(slot))
                        }
                    };
                    wing(
                        &mut census,
                        cair_wing::Row {
                            owner: slot,
                            mission_mask: rows::AIR_MISSION_TRAINING,
                            air_base,
                            ..Default::default()
                        },
                    );
                }
                // Idle, and a mine layer, but without a region: its pool
                // base resolves and is never probed.
                2 | 6 | 10 => {
                    unique_bases.insert(20_000 + u64::from(slot));
                    wing(
                        &mut census,
                        cair_wing::Row {
                            owner: slot,
                            mission_mask: rows::AIR_MISSION_LAY_MINES,
                            air_base: handle(20_000 + u64::from(slot)),
                            ..Default::default()
                        },
                    );
                }
                _ => {}
            }
        }

        // Country air objects and, in one window, their registrations.
        let manager = world.object(0x100, cstrategic_air_manager::VTABLE_RVA);
        world.u64(state + gs::STRATEGIC_AIR_MANAGER, manager as u64);
        let roster = world.alloc(slots as usize * 8);
        world.array(manager + cstrategic_air_manager::COUNTRY_AIR, roster, slots);
        let mut air = BTreeMap::new();
        for slot in (1..slots).filter(|&slot| live(slot)) {
            let object = world.object(cstrategic_air::SIZE, cstrategic_air::VTABLE_RVA);
            world.u32(object + cstrategic_air::COUNTRY, slot);
            world.u64(roster + slot as usize * 8, object as u64);
            air.insert(slot, object);
        }
        let start = world.alloc(0);
        for country in &registered {
            let object = air[&country.slot];
            let last = country.regions.last().unwrap().0;
            let outer = world.alloc((last as usize + 1) * engine_native::cpdx_array::STRIDE);
            let ids = world.alloc(country.regions.len() * 4);
            let mut all = 0;
            for (index, (region, wings)) in country.regions.iter().enumerate() {
                world.u32(ids + index * 4, *region);
                let vector = world.alloc(wings.len() * 8);
                for (ordinal, &wing) in wings.iter().enumerate() {
                    let mission = census[wing].pointer + cair_wing::MISSION;
                    world.u64(vector + ordinal * 8, mission as u64);
                }
                world.array(
                    outer + *region as usize * engine_native::cpdx_array::STRIDE,
                    vector,
                    wings.len() as u32,
                );
                all += wings.len() as u32;
            }
            world.array(object + cstrategic_air::REGIONAL_MISSIONS, outer, last + 1);
            world.array(
                object + cstrategic_air::ACTIVE_MISSION_REGIONS,
                ids,
                country.regions.len() as u32,
            );
            // Only the count is compared; the data is never read.
            world.array(object + cstrategic_air::ALL_ACTIVE_MISSIONS, ids, all);
        }
        world.missions = start..world.alloc(0);

        // The reference registry's air-base table and its bases.
        world.states = (0..STATES)
            .map(|index| {
                let object = world.object(0x60, cstate::VTABLE_RVA);
                world.u32(object + cstate::ID, 1 + index as u32);
                object
            })
            .collect();
        let provinces: Vec<usize> = (0..PROVINCES)
            .map(|index| {
                let object = world.object(0xa8, cprovince::VTABLE_RVA);
                world.u32(object + cprovince::ID, province_id(index));
                object
            })
            .collect();
        // The province roster by id, and each kind's state province lists.
        let ids = province_id(PROVINCES - 1) as usize + 1;
        let roster = world.alloc(ids * 8);
        world.array(state + gs::PROVINCES, roster, ids as u32);
        for (index, &object) in provinces.iter().enumerate() {
            world.u64(
                roster + province_id(index as u64) as usize * 8,
                object as u64,
            );
        }
        for (kind, lists) in [
            cstrategic_air_manager::DEFAULT_STATE_PROVINCE_LISTS,
            cstrategic_air_manager::KIND1_STATE_PROVINCE_LISTS,
            cstrategic_air_manager::KIND2_STATE_PROVINCE_LISTS,
        ]
        .into_iter()
        .enumerate()
        {
            let stride = engine_native::cpdx_array::STRIDE;
            let states = world.alloc((STATES as usize + 1) * stride);
            world.array(manager + lists, states, STATES as u32 + 1);
            for id in 1..=STATES as u32 {
                if let Some(first) = first_province(id, kind as i32) {
                    let list = world.alloc(8);
                    world.u32(list, first);
                    world.u32(list + 4, province_id(u64::from(first)));
                    world.array(states + id as usize * stride, list, 2);
                }
            }
        }
        let ids: Vec<u64> = (1..=TRANSFER_BASES)
            .filter(|id| !id.is_multiple_of(97))
            .chain(unique_bases)
            .collect();
        for &id in &ids {
            let object = world.object(cair_base::SIZE, cair_base::VTABLE_RVA);
            let fields = reference(id);
            world.u64(object + cair_base::HANDLE, handle(id));
            world.u64(object + cair_base::HOST_REFERENCE, fields.host);
            if fields.state.is_some() {
                let state = world.states[(id % STATES) as usize];
                world.u64(object + cair_base::STATE, state as u64);
            }
            if id % 10 != 7 {
                let province = provinces[(id % PROVINCES) as usize];
                world.u64(object + cair_base::PROVINCE, province as u64);
            }
            world.u32(object + cair_base::KIND, fields.kind as u32);
            world.bases.insert(handle(id), object);
            if fields.state.is_some() {
                continue;
            }
            let task_force = match host(id) {
                Host::Unresolved => continue,
                Host::NoTaskForce => 0,
                Host::Owner(owner) => {
                    let task_force = world.object(0x1e0, ctask_force::VTABLE_RVA);
                    world.u64(
                        task_force + ctask_force::SECONDARY_BASE,
                        (BASE + ctask_force::SECONDARY_VTABLE_RVA) as u64,
                    );
                    world.u32(task_force + cunit::ROSTER_OWNER, owner);
                    task_force
                }
            };
            let ship = world.object(cship::TASK_FORCE + 8, cship::VTABLE_RVA);
            world.u64(ship + cship::HANDLE, fields.host);
            world.u64(ship + cship::TASK_FORCE, task_force as u64);
            world.ships.insert(fields.host, ship);
        }
        let bases = world.bases.clone();
        world.registry(cair_base::TYPE, &bases);
        let ships = world.ships.clone();
        world.registry(cship::HANDLE_TYPE, &ships);
        // The navies of the registered countries and their mine levels.
        let navies = world.object(0x20, cstrategic_navy_manager::VTABLE_RVA);
        world.u64(state + gs::STRATEGIC_NAVY_MANAGER, navies as u64);
        let roster = world.alloc(slots as usize * 8);
        world.array(navies + cstrategic_navy_manager::NAVIES, roster, slots);
        for country in &registered {
            let slot = country.slot;
            let navy = world.object(0xd8, cstrategic_navy::VTABLE_RVA);
            world.u64(
                navy + cstrategic_navy::SECONDARY,
                (BASE + cstrategic_navy::SECONDARY_VTABLE_RVA) as u64,
            );
            world.u32(navy + cstrategic_navy::OWNER, slot);
            let levels = world.alloc(REGIONS as usize * 8);
            for region in 0..REGIONS {
                world.u64(levels + region as usize * 8, mines(slot, region) as u64);
            }
            world.array(navy + cstrategic_navy::REGIONAL_OPERANDS, levels, REGIONS);
            world.u64(roster + slot as usize * 8, navy as u64);
            world.navies.insert(slot, navy);
        }
        Self {
            world,
            census,
            registered,
            empty,
        }
    }

    /// Every registration in walk order: countries by slot, regions
    /// ascending, wings in vector order.
    fn registrations(&self) -> Vec<cair_regional_mission::Row> {
        self.registered
            .iter()
            .flat_map(|country| &country.regions)
            .flat_map(|(_, wings)| {
                wings
                    .iter()
                    .enumerate()
                    .map(|(ordinal, &wing)| cair_regional_mission::Row {
                        wing: self.census[wing].row.id,
                        ordinal: ordinal as u32,
                    })
            })
            .collect()
    }

    fn registered_wings(&self) -> impl Iterator<Item = &cair_wing::Row> {
        self.registered
            .iter()
            .flat_map(|country| &country.regions)
            .flat_map(|(_, wings)| wings.iter().map(|&wing| &self.census[wing].row))
    }

    /// Every registered pool base, every unregistered training wing's pool
    /// base and every transfer base, each once in handle order, with the
    /// fields of those the registry resolves.
    fn witnesses(&self) -> Vec<Witness> {
        let probes: BTreeSet<u64> = self
            .registered_wings()
            .map(|wing| wing.air_base)
            .chain(
                self.census
                    .iter()
                    .map(|source| &source.row)
                    .filter(|wing| {
                        wing.mission_region.is_none()
                            && wing.mission_mask == rows::AIR_MISSION_TRAINING
                            && wing.air_base != 0
                    })
                    .map(|wing| wing.air_base),
            )
            .chain(
                self.census
                    .iter()
                    .map(|source| source.row.base_override)
                    .filter(|&handle| handle != 0),
            )
            .collect();
        probes
            .into_iter()
            .map(|handle| {
                let Some(&object) = self.world.bases.get(&handle) else {
                    return (handle, None, None, false);
                };
                let base = BaseReference {
                    object,
                    ..reference(handle >> 32)
                };
                let host = match base.state {
                    Some(_) => Host::Unresolved,
                    None => host(handle >> 32),
                };
                let owner = match host {
                    Host::Owner(owner) => Some(owner),
                    _ => None,
                };
                (handle, Some(base), owner, host == Host::NoTaskForce)
            })
            .collect()
    }

    /// One row per (effective country, region) of the registered mine
    /// layers, in key order.
    fn mines(&self) -> Vec<rows::NavalMineLevelRow> {
        let pairs: BTreeSet<(u32, u32)> = self
            .registered_wings()
            .filter(|wing| wing.mission_mask == rows::AIR_MISSION_LAY_MINES)
            .map(|wing| {
                let country = if wing.country_override > 0 {
                    wing.country_override as u32
                } else {
                    wing.owner
                };
                (country, wing.mission_region.unwrap())
            })
            .collect();
        pairs
            .into_iter()
            .map(|(country, region)| rows::NavalMineLevelRow {
                country,
                region,
                raw_mines: mines(country, region),
            })
            .collect()
    }

    /// One hour of the owned path: the wing job's census installed, the
    /// registered missions captured without history parts (as the Late job
    /// does when no AirMissions job ran), then the pack's capture.
    fn hour(&self, census: &[cair_wing::Source], air: &mut Captured) -> [Outcome; N] {
        let mut job = AirWingJob {
            sources: census.to_vec(),
            source_outcome: Outcome::Rows { len: census.len() },
            identities: census
                .iter()
                .map(|source| WingIdentity::new(source.pointer, source.pool, &source.row).unwrap())
                .collect(),
            identity_outcome: Outcome::Rows { len: census.len() },
            ..Default::default()
        };
        let parent = Outcome::Rows { len: census.len() };
        air.reset();
        air.prepare_hour(HOUR, false);
        air.install_owned_mission_identities(&mut job);
        air.finish_wings(parent);
        air.capture_missions(
            &self.world,
            BASE,
            self.world.game_state,
            MissionSources {
                parent,
                histories: &mut [],
                changes: &crate::history_changes::Unobserved,
                carry: None,
            },
        );
        air.capture(
            &self.world,
            BASE,
            self.world.game_state,
            HOUR,
            false,
            Some(&mut Default::default()),
        );
        air.outcomes()
    }
}

/// The census as the joins receive it after a complete hour: sorted by
/// handle, with the registrations in walk order.
fn joined(air: &Captured) -> (Vec<WingIdentity>, Vec<cair_regional_mission::Row>) {
    let (Outcome::Rows { len: wings }, Outcome::Rows { len }) =
        (air.identity_outcome, air.registrations.outcome)
    else {
        panic!("incomplete joins");
    };
    (
        air.identities.first[..wings].to_vec(),
        air.registrations.rows[..len].to_vec(),
    )
}

/// Guarded calls by kind: gathers, copies inside the registration window,
/// reads of the air-base registry root and of the air manager pointer, all
/// copies, and each base object's field copy.
struct Counted<'a> {
    world: &'a World,
    gathers: Cell<usize>,
    regional: Cell<usize>,
    roots: Cell<usize>,
    managers: Cell<usize>,
    copies: Cell<usize>,
    fields: std::cell::RefCell<BTreeMap<usize, usize>>,
}

impl<'a> Counted<'a> {
    fn new(world: &'a World) -> Self {
        Self {
            world,
            gathers: Cell::new(0),
            regional: Cell::new(0),
            roots: Cell::new(0),
            managers: Cell::new(0),
            copies: Cell::new(0),
            fields: Default::default(),
        }
    }
}

impl Memory for Counted<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.copies.set(self.copies.get() + 1);
        if self.world.missions.contains(&address) {
            self.regional.set(self.regional.get() + 1);
        }
        if address == BASE + registry::LOW_ROOTS_RVA + cair_base::TYPE as usize * 8 {
            self.roots.set(self.roots.get() + 1);
        }
        if address == self.world.game_state + gs::STRATEGIC_AIR_MANAGER {
            self.managers.set(self.managers.get() + 1);
        }
        self.world.copy(address, destination)
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.gathers.set(self.gathers.get() + 1);
        let completed = self.world.copy_records(sources, layout, destination);
        // A base's six fields, gathered (`Pass::read_bases`): each base read
        // counts as its per-object scatter does.
        if layout.field_count() == 6
            && let RecordSources::Pointers(pointers) = sources
        {
            let mut fields = self.fields.borrow_mut();
            for &pointer in &pointers[..completed / 6] {
                *fields.entry(pointer).or_default() += 1;
            }
        }
        completed
    }

    fn copy_scattered<const K: usize>(
        &self,
        addresses: [usize; K],
        destinations: &mut [&mut [u8]; K],
    ) -> usize {
        *self.fields.borrow_mut().entry(addresses[0]).or_default() += 1;
        self.world.copy_scattered(addresses, destinations)
    }
}

/// Room for every row of the scale world.
fn ample() -> Capacities {
    let mut capacity = Capacities {
        wings: 1 << 14,
        ..Default::default()
    };
    capacity.rows[5..].fill(1 << 14);
    capacity
}

fn complete<S: TableSchema>(table: &TableView) -> &[S::Row] {
    let TableView::Complete(rows) = table else {
        panic!("expected a complete table, got {table:?}");
    };
    rows.rows::<S>().unwrap()
}

fn witness_rows(witnesses: &[Witness]) -> Vec<AirReferenceWitnessRow> {
    witnesses
        .iter()
        .filter_map(|&(handle, base, host_owner, host_without_task_force)| {
            let base = base?;
            Some(AirReferenceWitnessRow {
                handle,
                base_kind: Some(base.kind),
                state: base.state,
                province: base.province,
                host: Some(base.host),
                host_owner,
                host_without_task_force,
            })
        })
        .collect()
}

fn witnessed(output: &[BaseWitness]) -> Vec<Witness> {
    output
        .iter()
        .map(|witness| {
            (
                witness.handle,
                witness.base,
                witness.host_owner,
                witness.host_without_task_force,
            )
        })
        .collect()
}

/// The walk over 5,000 country air objects returns every registration with
/// its vector ordinal, gathers the roster, identities and mission headers of
/// 64 objects per guarded call, and reads each registered region's header
/// once, its mission pointers and each country's region IDs 64 at a time.
#[test]
fn registrations_at_5000_countries_are_complete_and_gathered_64_air_objects_at_a_time() {
    let scale = Scale::new();
    let expected = scale.registrations();
    let heavy = scale
        .registered
        .iter()
        .find(|country| country.regions.len() > 64)
        .unwrap();
    assert!(heavy.regions.iter().any(|(_, wings)| wings.len() > 128));
    assert!(expected.len() > 7_000, "{}", expected.len());
    let regions: usize = scale.registered.iter().map(|c| c.regions.len()).sum();
    assert!(regions > 3_000, "{regions}");

    let mut air = Captured::with_capacities(&ample());
    let mut job = AirWingJob {
        sources: scale.census.clone(),
        source_outcome: Outcome::Rows {
            len: scale.census.len(),
        },
        ..Default::default()
    };
    air.install_owned_mission_identities(&mut job);
    let mut wings = air.identities.first[..scale.census.len()].to_vec();
    wings.sort_unstable_by_key(|wing| wing.mission_pointer);

    let memory = Counted::new(&scale.world);
    let mut rows = Vec::new();
    let walk = cair_regional_mission::walk_with_diagnostic_growing(
        &memory,
        BASE,
        scale.world.game_state,
        &wings,
        &mut rows,
        &mut Default::default(),
    );
    assert_eq!(
        walk,
        Outcome::Rows {
            len: expected.len()
        }
    );
    assert_eq!(&rows[..expected.len()], expected);
    // The roster's slot pointers and identities per 64 slots, then the
    // headers of 64 country air objects per gather: one for each 64 objects.
    let slots = COUNTRIES as usize + 1;
    let objects = COUNTRIES as usize - 1;
    assert_eq!(
        memory.gathers.get(),
        2 * slots.div_ceil(64) + objects.div_ceil(64)
    );
    let regional: usize = scale
        .registered
        .iter()
        .map(|country| {
            country.regions.len().div_ceil(64)
                + country
                    .regions
                    .iter()
                    .map(|(_, wings)| 1 + wings.len().div_ceil(64))
                    .sum::<usize>()
        })
        .sum();
    assert_eq!(memory.regional.get(), regional);

    // A short output keeps the walk-order prefix and counts the rest.
    let short = expected.len() - 77;
    let walk = cair_regional_mission::walk(
        &scale.world,
        BASE,
        scale.world.game_state,
        &wings,
        &mut rows[..short],
    );
    assert_eq!(
        walk,
        Outcome::Truncated {
            len: short,
            unread_rows: 77
        }
    );
    assert_eq!(rows[..short], expected[..short]);

    // The heavy country's last registered wing missing from the census, or
    // assigned another region there, fails the whole walk.
    let last = *heavy.regions.last().unwrap().1.last().unwrap();
    let pointer = scale.census[last].pointer + cair_wing::MISSION;
    let at = wings
        .binary_search_by_key(&pointer, |wing| wing.mission_pointer)
        .unwrap();
    let mut other = wings.clone();
    other[at].region = Some(1);
    let mut missing = wings.clone();
    missing.remove(at);
    for (wings, reason) in [
        (
            &other,
            "registered mission owner or region differs from AirWings",
        ),
        (&missing, "registered mission missing from AirWings"),
    ] {
        assert_eq!(
            cair_regional_mission::walk(
                &scale.world,
                BASE,
                scale.world.game_state,
                wings,
                &mut rows
            ),
            Outcome::Unavailable { reason }
        );
    }
}

/// Cold worker storage captures every mission, witness and mine pair on its
/// first 5,000-country hour. Larger later content needs no unpublished retry.
#[test]
fn mission_tables_grow_before_the_first_5000_country_publication() {
    let scale = Scale::new();
    let registrations = scale.registrations();
    let witnesses = scale.witnesses();
    let mines = scale.mines();
    let mut capacity = Capacities::default();
    capacity.rows[5..].fill(0);
    capacity.wings = 0;
    let mut air = Captured::with_capacities(&capacity);
    let joins = Outcome::Unavailable {
        reason: "air mission joins lack the complete registration census",
    };
    let outcomes = scale.hour(&scale.census, &mut air);
    // AirBases is complete and empty, so every witness keeps its fields.
    assert_eq!(outcomes[1], Outcome::Rows { len: 0 });
    assert_eq!(
        outcomes[5..8],
        [
            Outcome::Rows {
                len: registrations.len()
            },
            Outcome::Rows {
                len: witnesses.len()
            },
            Outcome::Rows { len: mines.len() }
        ]
    );
    assert_eq!(air.registrations.rows[..registrations.len()], registrations);
    let tables = air.tables();
    let mut by_wing: Vec<_> = registrations
        .iter()
        .map(|row| rows::AirRegionalMissionRow {
            wing: row.wing,
            ordinal: row.ordinal,
        })
        .collect();
    by_wing.sort_unstable_by_key(|row| row.wing);
    assert_eq!(complete::<tables::AirRegionalMissions>(&tables[5]), by_wing);
    assert_eq!(
        complete::<tables::AirReferenceWitnesses>(&tables[6]),
        witness_rows(&witnesses)
    );
    assert_eq!(complete::<tables::NavalMineLevels>(&tables[7]), mines);

    // Content can grow without a population change. Discard only output
    // preparation and prove that a later hour publishes the complete demand.
    air.registrations.rows.clear();
    air.references.rows.clear();
    air.mines.rows.clear();
    air.handles.clear();
    let later = scale.hour(&scale.census, &mut air);
    assert_eq!(later[5..8], outcomes[5..8]);

    // A registered wing missing from the census fails the registrations,
    // and the joins with them.
    let mut census = scale.census.clone();
    census.remove(scale.registered[0].regions[0].1[0]);
    let outcomes = scale.hour(&census, &mut air);
    assert_eq!(
        outcomes[5..8],
        [
            Outcome::Unavailable {
                reason: "registered mission missing from AirWings"
            },
            joins,
            joins
        ]
    );
}

/// The AirMissions job's pack once its missions are captured, after the
/// wing census was installed, before its joins.
fn mission_pack(scale: &Scale, memory: &impl Memory) -> Captured {
    let mut air = Captured::with_capacities(&ample());
    let len = scale.census.len();
    let mut job = AirWingJob {
        identities: scale
            .census
            .iter()
            .map(|source| WingIdentity::new(source.pointer, source.pool, &source.row).unwrap())
            .collect(),
        identity_outcome: Outcome::Rows { len },
        ..Default::default()
    };
    let parent = Outcome::Rows { len };
    air.reset();
    air.prepare_hour(HOUR, false);
    air.install_owned_mission_identities(&mut job);
    air.finish_wings(parent);
    air.capture_missions(
        memory,
        BASE,
        scale.world.game_state,
        MissionSources {
            parent,
            histories: &mut [],
            changes: &crate::history_changes::Unobserved,
            carry: None,
        },
    );
    air
}

/// Every read's address in order; `fault` fails the first read of one
/// address once, as a one-shot guard page does.
struct Ordered<'a> {
    world: &'a World,
    reads: std::cell::RefCell<Vec<usize>>,
    fault: Option<usize>,
    faulted: Cell<bool>,
}

impl<'a> Ordered<'a> {
    fn new(world: &'a World, fault: Option<usize>) -> Self {
        Self {
            world,
            reads: Default::default(),
            fault,
            faulted: Cell::new(false),
        }
    }
}

impl Memory for Ordered<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(address);
        if self.fault == Some(address) && !self.faulted.replace(true) {
            return false;
        }
        self.world.copy(address, destination)
    }
}

/// Without a mission job the Late job captures the missions itself and
/// meets every shared read where it did: the joins run after the AirBases,
/// AirAces and AirStatistics walks, so a one-shot fault on the air manager
/// fails AirBases, as before, and not the joins.
#[test]
fn without_the_mission_job_late_meets_shared_reads_in_their_old_order() {
    let scale = Scale::new();
    let game_state = scale.world.game_state;
    // The read order of the whole fallback: the missions, then the capture.
    let ordered = Ordered::new(&scale.world, None);
    let mut clean = mission_pack(&scale, &ordered);
    clean.capture(
        &ordered,
        BASE,
        game_state,
        HOUR,
        false,
        Some(&mut Default::default()),
    );
    let reads = ordered.reads.into_inner();
    let manager = engine_native::read::<u64>(&scale.world, game_state + gs::STRATEGIC_AIR_MANAGER)
        .unwrap() as usize;
    let bases = reads
        .iter()
        .position(|&at| at == manager + cstrategic_air_manager::STATE_AIR_BASES)
        .expect("the AirBases walk");
    let witnesses = reads
        .iter()
        .position(|&at| at == BASE + registry::LOW_ROOTS_RVA + cair_base::TYPE as usize * 8)
        .expect("the base witnesses");
    assert!(bases < witnesses, "{bases} {witnesses}");
    // The capture's first read of the manager pointer faults once.
    let mut faulted = mission_pack(&scale, &scale.world);
    let memory = Ordered::new(&scale.world, Some(game_state + gs::STRATEGIC_AIR_MANAGER));
    faulted.capture(
        &memory,
        BASE,
        game_state,
        HOUR,
        false,
        Some(&mut Default::default()),
    );
    assert!(memory.faulted.get());
    let (outcomes, expected) = (faulted.outcomes(), clean.outcomes());
    assert!(matches!(outcomes[1], Outcome::Unavailable { .. }));
    assert_ne!(outcomes[1], expected[1]);
    assert_eq!(outcomes[6..13], expected[6..13]);
    assert_eq!(faulted.tables()[6..13], clean.tables()[6..13]);
}

/// The joins resolve their countries from the hour's snapshot bytes: the
/// same tables with fewer reads of the game.
#[test]
fn the_mission_joins_take_country_bytes_from_the_hours_snapshot() {
    let scale = Scale::new();
    let game_state = scale.world.game_state;
    let mut storage = std::sync::Arc::new(
        engine_native::country_memory::Cache::try_with_capacity(1 << 18).unwrap(),
    );
    let snapshot = engine_native::country_memory::Cache::prepare_shared(
        &mut storage,
        &scale.world,
        BASE,
        game_state,
    )
    .unwrap();
    let mut native = mission_pack(&scale, &scale.world);
    let counted = Counted::new(&scale.world);
    native.join_missions(&counted, BASE, game_state);
    let native_copies = counted.copies.get();
    let mut loaned = mission_pack(&scale, &scale.world);
    let counted = Counted::new(&scale.world);
    loaned.join_missions(&snapshot.bytes_view(&counted), BASE, game_state);
    assert!(
        counted.copies.get() < native_copies,
        "{} {native_copies}",
        counted.copies.get()
    );
    assert_eq!(loaned.outcomes(), native.outcomes());
    assert_eq!(loaned.tables()[6..13], native.tables()[6..13]);
}

/// The AirMissions job joins the registered missions itself: once it
/// returns, the base witnesses, sortie inputs and mine levels are complete,
/// and the Late job's capture keeps them without probing a base again.
#[test]
fn the_mission_job_leaves_the_joins_complete_for_the_late_capture() {
    let scale = Scale::new();
    let mut reference = Captured::with_capacities(&ample());
    let expected = scale.hour(&scale.census, &mut reference);
    let missions = |join: bool| {
        let mut air = mission_pack(&scale, &scale.world);
        if join {
            air.join_missions(&scale.world, BASE, scale.world.game_state);
        }
        air
    };
    // The Late capture's probes of the base registry.
    let late = |air: &mut Captured| {
        let memory = Counted::new(&scale.world);
        air.capture(
            &memory,
            BASE,
            scale.world.game_state,
            HOUR,
            false,
            Some(&mut Default::default()),
        );
        memory.roots.get()
    };
    let mut joined = missions(true);
    for table in [6, 7, 8] {
        assert_eq!(joined.outcomes()[table], expected[table], "table {table}");
    }
    // Without the mission job's joins the Late capture reads the base
    // registry's header itself.
    let mut unjoined = missions(false);
    assert_eq!(late(&mut unjoined), late(&mut joined) + 1);
    assert_eq!(joined.outcomes(), expected);
    assert_eq!(joined.tables()[6..13], reference.tables()[6..13]);
    assert_eq!(unjoined.tables()[6..13], reference.tables()[6..13]);
}

/// AirReferenceWitnesses over thousands of wings: each distinct probed
/// handle is looked up once and its base's fields copied once, a short
/// output is truncated with the unread count, and a read failure fails the
/// table even past the output's end.
#[test]
fn base_witnesses_at_scale_read_each_distinct_handle_once() {
    let mut scale = Scale::new();
    let expected = scale.witnesses();
    let resolved = expected
        .iter()
        .filter(|(_, base, ..)| base.is_some())
        .count();
    assert!(resolved > 2_500 && expected.len() - resolved > 100);
    // Shared pool bases, shared and unregistered transfer bases.
    let registered = scale.registered_wings().count();
    assert!(expected.len() < registered);
    // Stateless bases hosted by a task-force carrier, a carrier without a
    // task force and an unresolved host; state bases without a cached
    // province, with and without a first province for their kind.
    let stateless = |host_owner: bool, bare: bool| {
        expected.iter().any(|&(_, base, owner, without)| {
            base.is_some_and(|base| base.state.is_none())
                && owner.is_some() == host_owner
                && without == bare
        })
    };
    assert!(stateless(true, false) && stateless(false, true) && stateless(false, false));
    let uncached = |province: bool| {
        expected.iter().any(|&(handle, base, ..)| {
            (handle >> 32) % 10 == 7 && base.is_some_and(|base| base.province.is_some() == province)
        })
    };
    assert!(uncached(true) && uncached(false));
    let mut air = Captured::with_capacities(&ample());
    let outcomes = scale.hour(&scale.census, &mut air);
    assert_eq!(
        outcomes[6],
        Outcome::Rows {
            len: expected.len()
        }
    );
    let (wings, registrations) = joined(&air);

    let memory = Counted::new(&scale.world);
    let mut handles = vec![0; wings.len() * 2];
    let mut output = vec![BaseWitness::default(); expected.len()];
    let outcome = capture_references(
        &memory,
        BASE,
        scale.world.game_state,
        &wings,
        Some(&registrations),
        &mut handles,
        &mut output,
    );
    assert_eq!(
        outcome,
        Ok(Outcome::Rows {
            len: expected.len()
        })
    );
    assert_eq!(witnessed(&output), expected);
    // One pass reads the base registry's header and the air manager once,
    // however many bases it probes or finds without a cached province.
    assert_eq!((memory.roots.get(), memory.managers.get()), (1, 1));
    let fields: BTreeMap<usize, usize> = expected
        .iter()
        .filter(|(_, base, ..)| base.is_some())
        .map(|(handle, ..)| (scale.world.bases[handle], 1))
        .collect();
    assert_eq!(*memory.fields.borrow(), fields);

    // A short output keeps the first probes; every other handle is still
    // read and counted as unread.
    let memory = Counted::new(&scale.world);
    let short = expected.len() / 3;
    let outcome = capture_references(
        &memory,
        BASE,
        scale.world.game_state,
        &wings,
        Some(&registrations),
        &mut handles,
        &mut output[..short],
    );
    assert_eq!(
        outcome,
        Ok(Outcome::Truncated {
            len: short,
            unread_rows: (expected.len() - short) as u32
        })
    );
    assert_eq!(witnessed(&output[..short]), expected[..short]);
    assert_eq!(memory.roots.get(), 1);

    // Scratch too small for every probe, duplicates included, reads nothing.
    let probes = registered
        + scale
            .census
            .iter()
            .filter(|source| {
                source.row.mission_region.is_none()
                    && source.row.mission_mask == rows::AIR_MISSION_TRAINING
                    && source.row.air_base != 0
            })
            .count()
        + scale
            .census
            .iter()
            .filter(|source| source.row.base_override != 0)
            .count();
    let memory = Counted::new(&scale.world);
    assert_eq!(
        capture_references(
            &memory,
            BASE,
            scale.world.game_state,
            &wings,
            Some(&registrations),
            &mut handles[..probes - 1],
            &mut output,
        ),
        Ok(Outcome::Truncated {
            len: 0,
            unread_rows: probes as u32
        })
    );
    assert_eq!(memory.roots.get(), 0);

    // A registered wing without a pool base is an error.
    let mut null = wings.clone();
    let at = null
        .binary_search_by_key(&registrations[0].wing, |wing| wing.wing)
        .unwrap();
    null[at].air_base = 0;
    assert_eq!(
        capture_references(
            &scale.world,
            BASE,
            scale.world.game_state,
            &null,
            Some(&registrations),
            &mut handles,
            &mut output,
        )
        .err(),
        Some("registered wing has a null pool base")
    );

    // A task-force carrier host whose stored handle differs is an error.
    let (_, base, ..) = expected
        .iter()
        .find(|(_, _, owner, _)| owner.is_some())
        .unwrap();
    let ship = scale.world.ships[&base.unwrap().host];
    scale.world.u64(ship + cship::HANDLE, 0);
    assert_eq!(
        capture_references(
            &scale.world,
            BASE,
            scale.world.game_state,
            &wings,
            Some(&registrations),
            &mut handles,
            &mut output,
        )
        .err(),
        Some("carrier host reference handle mismatch")
    );
    scale.world.u64(ship + cship::HANDLE, base.unwrap().host);

    // The last probed state base's state fails authentication: the table
    // fails, even when the output ends long before it.
    let id = expected
        .iter()
        .rev()
        .find_map(|(_, base, ..)| base.and_then(|base| base.state))
        .unwrap();
    let first = expected
        .iter()
        .position(|(_, base, ..)| base.is_some_and(|base| base.state == Some(id)))
        .unwrap();
    assert!(first > 0);
    let state = scale.world.states[id as usize - 1];
    scale.world.u64(state, 0);
    let reason = "air reference object identity mismatch";
    assert_eq!(
        capture_references(
            &scale.world,
            BASE,
            scale.world.game_state,
            &wings,
            Some(&registrations),
            &mut handles,
            &mut output[..1],
        )
        .err(),
        Some(reason)
    );
    let outcomes = scale.hour(&scale.census, &mut air);
    assert_eq!(outcomes[6], Outcome::Unavailable { reason });
    assert_eq!(
        air.tables()[6],
        TableView::Unavailable {
            reason: reason.into()
        }
    );
    // The registrations and mines stand.
    assert_eq!(
        outcomes[5],
        Outcome::Rows {
            len: registrations.len()
        }
    );
    assert_eq!(
        outcomes[7],
        Outcome::Rows {
            len: scale.mines().len()
        }
    );
}

/// NavalMineLevels over thousands of mine layers: one row per distinct
/// (effective country, region) in key order with the raw level, zero
/// included; truncation reads no pair past the output; a region outside the
/// navy's vector or an override naming an empty slot fails the table.
#[test]
fn mine_levels_at_scale_are_one_row_per_pair_in_key_order() {
    let mut scale = Scale::new();
    let expected = scale.mines();
    let layers = scale
        .registered_wings()
        .filter(|wing| wing.mission_mask == rows::AIR_MISSION_LAY_MINES)
        .count();
    assert!(layers > 1_500 && expected.len() < layers, "{layers}");
    assert!(expected.iter().any(|row| row.raw_mines == 0));
    // Overrides fly for country 3 outside its own regions.
    let own: BTreeSet<u32> = scale.registered[0]
        .regions
        .iter()
        .map(|&(r, _)| r)
        .collect();
    assert_eq!(scale.registered[0].slot, 3);
    assert!(
        expected
            .iter()
            .any(|row| row.country == 3 && !own.contains(&row.region))
    );
    let mut air = Captured::with_capacities(&ample());
    let outcomes = scale.hour(&scale.census, &mut air);
    assert_eq!(
        outcomes[7],
        Outcome::Rows {
            len: expected.len()
        }
    );
    assert_eq!(
        complete::<tables::NavalMineLevels>(&air.tables()[7]),
        expected
    );
    let (wings, registrations) = joined(&air);

    let mut pairs = vec![0; registrations.len()];
    let mut output = vec![rows::NavalMineLevelRow::default(); expected.len()];
    let short = expected.len() / 2;
    let capture = |memory: &World, pairs: &mut [u64], output: &mut [rows::NavalMineLevelRow]| {
        capture_mines(
            memory,
            BASE,
            memory.game_state,
            &wings,
            Some(&registrations),
            pairs,
            output,
        )
    };
    assert_eq!(
        capture(&scale.world, &mut pairs, &mut output),
        Ok(Outcome::Rows {
            len: expected.len()
        })
    );
    assert_eq!(output, expected);
    assert_eq!(
        capture(&scale.world, &mut pairs, &mut output[..short]),
        Ok(Outcome::Truncated {
            len: short,
            unread_rows: (expected.len() - short) as u32
        })
    );
    assert_eq!(output[..short], expected[..short]);
    assert_eq!(
        capture(&scale.world, &mut pairs[..layers - 1], &mut output).err(),
        Some("mine pair scratch smaller than the registrations")
    );

    // The last pair's region outside its navy's vector.
    let last = expected.last().unwrap();
    let navy = scale.world.navies[&last.country];
    let count = navy + cstrategic_navy::REGIONAL_OPERANDS + 12;
    scale.world.u32(count, last.region);
    let reason = "navy regional operand outside array";
    assert_eq!(
        capture(&scale.world, &mut pairs, &mut output).err(),
        Some(reason)
    );
    // Past a short output's end the pair is not read.
    assert_eq!(
        capture(&scale.world, &mut pairs, &mut output[..short]),
        Ok(Outcome::Truncated {
            len: short,
            unread_rows: (expected.len() - short) as u32
        })
    );
    let outcomes = scale.hour(&scale.census, &mut air);
    assert_eq!(outcomes[7], Outcome::Unavailable { reason });
    assert_eq!(
        air.tables()[7],
        TableView::Unavailable {
            reason: reason.into()
        }
    );
    scale.world.u32(count, REGIONS);

    // A mine layer flying for the empty slot.
    let mut census = scale.census.clone();
    let layer = census
        .iter_mut()
        .find(|source| {
            source.row.mission_region.is_some()
                && source.row.mission_mask == rows::AIR_MISSION_LAY_MINES
                && source.row.country_override == 0
        })
        .unwrap();
    layer.row.country_override = scale.empty as i32;
    let outcomes = scale.hour(&census, &mut air);
    assert_eq!(
        outcomes[5..8],
        [
            Outcome::Rows {
                len: registrations.len()
            },
            Outcome::Rows {
                len: scale.witnesses().len()
            },
            Outcome::Unavailable {
                reason: "country link resolves to null country"
            }
        ]
    );
}

/// One AirMissions hour with held registrations: the owned census
/// installed, then the registrations captured through `store`. Returns the
/// outcome and the reads of active-region ids, regional vectors and mission
/// pointers.
fn carried_hour(
    scale: &Scale,
    air: &mut Captured,
    store: &mut MissionStore,
    watches: &crate::mission_changes::Watches,
) -> (Outcome, usize) {
    let census = &scale.census;
    let mut job = AirWingJob {
        sources: census.to_vec(),
        source_outcome: Outcome::Rows { len: census.len() },
        identities: census
            .iter()
            .map(|source| WingIdentity::new(source.pointer, source.pool, &source.row).unwrap())
            .collect(),
        identity_outcome: Outcome::Rows { len: census.len() },
        ..Default::default()
    };
    let parent = Outcome::Rows { len: census.len() };
    air.reset();
    air.prepare_hour(HOUR, false);
    air.install_owned_mission_identities(&mut job);
    air.finish_wings(parent);
    let memory = Counted::new(&scale.world);
    air.capture_missions(
        &memory,
        BASE,
        scale.world.game_state,
        MissionSources {
            parent,
            histories: &mut [],
            changes: &crate::history_changes::Unobserved,
            carry: Some(MissionCarry {
                store,
                changes: watches,
                check: crate::sampled_verify::Check::Nothing,
            }),
        },
    );
    (air.registrations.outcome, memory.regional.get())
}

/// The AirMissions job's held registrations at 5,000 countries: a quiet
/// hour publishes the fresh walk's rows without reading one active-region
/// id, regional vector or mission pointer (the old walk read them all every
/// hour), and a registration on the heavy country re-reads only its own.
#[test]
fn carried_registrations_at_5000_countries_reread_only_notified_countries() {
    let scale = Scale::new();
    let expected = scale.registrations();
    let reads = |country: &Registered| {
        country.regions.len().div_ceil(64)
            + country
                .regions
                .iter()
                .map(|(_, wings)| 1 + wings.len().div_ceil(64))
                .sum::<usize>()
    };
    let every: usize = scale.registered.iter().map(reads).sum();
    let watches = crate::mission_changes::Watches::enabled(COUNTRIES as usize);
    let mut store = MissionStore::new();
    let mut air = Captured::with_capacities(&ample());
    let complete = Outcome::Rows {
        len: expected.len(),
    };
    let first = carried_hour(&scale, &mut air, &mut store, &watches);
    assert_eq!(first, (complete, every));
    assert_eq!(air.registrations.rows[..expected.len()], expected);
    watches.boundary(false);
    let quiet = carried_hour(&scale, &mut air, &mut store, &watches);
    assert_eq!(quiet, (complete, 0));
    assert_eq!(air.registrations.rows[..expected.len()], expected);
    assert_eq!(store.counts().reused, COUNTRIES as u64 - 1);
    assert!(store.counts().quiet);

    let heavy = scale
        .registered
        .iter()
        .find(|country| country.regions.len() > 64)
        .unwrap();
    let mut address = None;
    engine_native::cstrategic_air::visit(&scale.world, BASE, scale.world.game_state, |root| {
        if root.canonical_slot == heavy.slot {
            address = Some(root.address);
        }
        Ok(())
    })
    .unwrap();
    drop(watches.notify(address.unwrap(), 0));
    watches.boundary(false);
    let notified = carried_hour(&scale, &mut air, &mut store, &watches);
    assert_eq!(notified, (complete, reads(heavy)));
    assert_eq!(air.registrations.rows[..expected.len()], expected);
    assert_eq!(
        (store.counts().read, store.counts().reused),
        (1, COUNTRIES as u64 - 2)
    );
}
