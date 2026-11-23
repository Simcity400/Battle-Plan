use super::*;
use engine_map_air::{map_air_base_reference::BaseReference, map_air_base_reference::Host};
use engine_native::{cgame_state as gs, cship, ctask_force, cunit};
use engine_native_parts::{creference_registry as registry, reference_bucket as bucket};
use engine_native_memory::memory::{RecordLayout, RecordSources};
use engine_native_air::cstrategic_air;
use engine_native_naval::{cstrategic_navy, cstrategic_navy_manager};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
};

const BASE: usize = 0x1_4000_0000;
const HEAP: usize = 0x10_0000;
const WINGS: usize = 0x7000_0000_0000;
const COUNTRIES: u32 = 5_000;
const HOUR: i32 = 1;
const REGIONS: u32 = 400;
const POOL_BASES: u64 = 2_500;
const TRANSFER_BASES: u64 = 3_000;
const STATES: u64 = 300;
const PROVINCES: u64 = 1_000;
const COUNTRY_OBJECT: usize = 0x20;

fn handle(id: u64) -> u64 {
    id << 32 | u64::from(engine_native_parts::cair_base_layout::TYPE)
}

fn province_id(index: u64) -> u32 {
    100 + 3 * (index % PROVINCES) as u32
}

fn first_province(state: u32, kind: i32) -> Option<u32> {
    let (state, kind) = (u64::from(state), kind as u64);
    (!(state + kind).is_multiple_of(4)).then(|| province_id(state * 7 + kind * 101))
}

fn reference(id: u64) -> BaseReference {
    let kind = (id % 3) as i32;
    let state = 1 + (id % STATES) as u32;
    let (state, province) = match id % 10 {
        3 => (None, None),
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

fn host(id: u64) -> Host {
    match id / 10 % 3 {
        0 => Host::Owner(1 + (id % u64::from(COUNTRIES)) as u32),
        1 => Host::NoTaskForce,
        _ => Host::Unresolved,
    }
}

type Witness = (u64, Option<BaseReference>, Option<u32>, bool);

fn mines(slot: u32, region: u32) -> i64 {
    if (slot + region).is_multiple_of(5) {
        0
    } else {
        i64::from(slot) * 1_000_000 + i64::from(region) * 10 + 7
    }
}

struct World {
    heap: Vec<u8>,
    roots: Vec<u8>,
    game_state: usize,
    missions: std::ops::Range<usize>,
    navies: BTreeMap<u32, usize>,
    states: Vec<usize>,
    bases: BTreeMap<u64, usize>,
    ships: BTreeMap<u64, usize>,
}

impl World {
    fn alloc(&mut self, len: usize) -> usize {
        let at = (HEAP + self.heap.len() + 0xf) & !0xf;
        self.heap.resize(at + len - HEAP, 0);
        at
    }

    fn put(&mut self, at: usize, value: &[u8]) {
        let roots = BASE + registry::LOW_ROOTS_RVA.0;
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

    fn object(&mut self, len: usize, vtable_rva: engine_sites::Rva) -> usize {
        let object = self.alloc(len);
        self.u64(object, (BASE + vtable_rva.0) as u64);
        object
    }

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
            BASE + registry::LOW_ROOTS_RVA.0 + kind as usize * 8,
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
            .or_else(|| window(&self.roots, BASE + registry::LOW_ROOTS_RVA.0, address, len))
        {
            Some(source) => {
                destination.copy_from_slice(source);
                true
            }
            None => false,
        }
    }
}

struct Registered {
    slot: u32,
    regions: Vec<(u32, Vec<usize>)>,
}

struct Scale {
    world: World,
    census: Vec<cair_wing::Source>,
    registered: Vec<Registered>,
    _empty: u32,
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

        let mut census = Vec::new();
        let mut registered = Vec::new();
        let mut unique_bases = BTreeSet::new();
        let wing = |census: &mut Vec<cair_wing::Source>, row: cair_wing::Row| {
            let index = census.len();
            census.push(cair_wing::Source {
                pointer: WINGS + index * 0x200,
                pool: 0,
                row: cair_wing::Row {
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
            let outer = world.alloc((last as usize + 1) * engine_native_memory::cpdx_array::STRIDE);
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
                    outer + *region as usize * engine_native_memory::cpdx_array::STRIDE,
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
            world.array(object + cstrategic_air::ALL_ACTIVE_MISSIONS, ids, all);
        }
        world.missions = start..world.alloc(0);

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
            let stride = engine_native_memory::cpdx_array::STRIDE;
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
            let object = world.object(cair_base::SIZE, engine_native_parts::cair_base_layout::VTABLE_RVA);
            let fields = reference(id);
            world.u64(object + engine_native_parts::cair_base_layout::HANDLE, handle(id));
            world.u64(object + engine_native_parts::cair_base_layout::HOST_REFERENCE, fields.host);
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
                        (BASE + ctask_force::SECONDARY_VTABLE_RVA.0) as u64,
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
        world.registry(engine_native_parts::cair_base_layout::TYPE, &bases);
        let ships = world.ships.clone();
        world.registry(cship::HANDLE_TYPE, &ships);
        let navies = world.object(0x20, cstrategic_navy_manager::VTABLE_RVA);
        world.u64(state + gs::STRATEGIC_NAVY_MANAGER, navies as u64);
        let roster = world.alloc(slots as usize * 8);
        world.array(navies + cstrategic_navy_manager::NAVIES, roster, slots);
        for country in &registered {
            let slot = country.slot;
            let navy = world.object(0xd8, cstrategic_navy::VTABLE_RVA);
            world.u64(
                navy + cstrategic_navy::SECONDARY,
                (BASE + cstrategic_navy::SECONDARY_VTABLE_RVA.0) as u64,
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
            _empty: empty,
        }
    }

    fn registered_wings(&self) -> impl Iterator<Item = &cair_wing::Row> {
        self.registered
            .iter()
            .flat_map(|country| &country.regions)
            .flat_map(|(_, wings)| wings.iter().map(|&wing| &self.census[wing].row))
    }

    fn witnesses(&self) -> Vec<Witness> {
        let probes: BTreeSet<u64> = self
            .census
            .iter()
            .map(|source| source.row.air_base)
            .filter(|&handle| handle != 0)
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
        air.bases_raw_mode = true;
        air.stats_raw_mode = true;
        air.install_owned_mission_identities(&mut job);
        air.finish_wings(parent);
        // SAFETY: Unit test runs an isolated fixture without carried session watches.
        unsafe {
            air.capture_missions(
                &self.world,
                BASE,
                self.world.game_state,
                MissionSources {
                    parent,
                    histories: &mut [],
                    changes: &collector_air_histories::history_changes::Unobserved,
                    carry: None,
                },
            );
        }
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
        if address == BASE + registry::LOW_ROOTS_RVA.0 + engine_native_parts::cair_base_layout::TYPE as usize * 8 {
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

fn ample() -> Capacities {
    let mut capacity = Capacities {
        wings: 1 << 14,
        ..Default::default()
    };
    capacity.rows[5..].fill(1 << 14);
    capacity
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
    air.bases_raw_mode = true;
    air.stats_raw_mode = true;
    air.install_owned_mission_identities(&mut job);
    air.finish_wings(parent);
    // SAFETY: Unit test runs an isolated fixture without carried session watches.
    unsafe {
        air.capture_missions(
            memory,
            BASE,
            scale.world.game_state,
            MissionSources {
                parent,
                histories: &mut [],
                changes: &collector_air_histories::history_changes::Unobserved,
                carry: None,
            },
        );
    }
    air
}

#[test]
fn the_mission_joins_take_country_bytes_from_the_hours_snapshot() {
    let scale = Scale::new();
    let game_state = scale.world.game_state;
    let mut storage = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(1 << 18).unwrap(),
    ).unwrap();
    let snapshot = engine_native_intelligence::country_memory::Cache::prepare_shared(
        &mut storage,
        &scale.world,
        BASE,
        game_state,
    )
    .unwrap();
    let mut native = mission_pack(&scale, &scale.world);
    let counted = Counted::new(&scale.world);
    native.join_missions(&counted, BASE, game_state, None);
    let native_copies = counted.copies.get();
    let mut loaned = mission_pack(&scale, &scale.world);
    let counted = Counted::new(&scale.world);
    loaned.join_missions(&snapshot.bytes_view(&counted), BASE, game_state, None);
    assert!(
        counted.copies.get() < native_copies,
        "{} {native_copies}",
        counted.copies.get()
    );
    assert_eq!(loaned.outcomes(), native.outcomes());
    assert_eq!(loaned.tables()[6..12], native.tables()[6..12]);
}

#[test]
fn the_mission_job_leaves_the_joins_complete_for_the_late_capture() {
    let scale = Scale::new();
    let mut reference = Captured::with_capacities(&ample());
    let expected = scale.hour(&scale.census, &mut reference);
    let missions = |join: bool| {
        let mut air = mission_pack(&scale, &scale.world);
        if join {
            air.join_missions(&scale.world, BASE, scale.world.game_state, None);
        }
        air
    };
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
    for table in [6, 7] {
        assert_eq!(joined.outcomes()[table], expected[table], "table {table}");
    }
    let mut unjoined = missions(false);
    assert_eq!(late(&mut unjoined), late(&mut joined) + 1);
    assert_eq!(joined.outcomes()[..15], expected[..15]);
    assert_eq!(
        joined.outcomes()[15],
        Outcome::Unavailable {
            reason: "Air transfer writer store unavailable"
        }
    );
    assert_eq!(joined.tables()[6..12], reference.tables()[6..12]);
    assert_eq!(unjoined.tables()[6..12], reference.tables()[6..12]);
}

#[test]
fn base_witnesses_at_scale_read_each_distinct_handle_once() {
    let mut scale = Scale::new();
    let expected = scale.witnesses();
    let resolved = expected
        .iter()
        .filter(|(_, base, ..)| base.is_some())
        .count();
    assert!(resolved > 2_500 && expected.len() - resolved > 100);
    let registered = scale.registered_wings().count();
    assert!(expected.len() < registered);
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
    let outcome = capture_references_buffer(
        &memory,
        BASE,
        scale.world.game_state,
        &wings,
        Some(&registrations),
        engine_native_memory::walk::Buffer::Growing(&mut handles),
        engine_native_memory::walk::Buffer::Growing(&mut output),
        None,
    );
    assert_eq!(
        outcome,
        Ok(Outcome::Rows {
            len: expected.len()
        })
    );
    assert_eq!(witnessed(&output), expected);
    assert_eq!((memory.roots.get(), memory.managers.get()), (1, 1));
    let fields: BTreeMap<usize, usize> = expected
        .iter()
        .filter(|(_, base, ..)| base.is_some())
        .map(|(handle, ..)| (scale.world.bases[handle], 1))
        .collect();
    assert_eq!(*memory.fields.borrow(), fields);

    let mut null = wings.clone();
    let at = null
        .binary_search_by_key(&registrations[0].wing, |wing| wing.wing)
        .unwrap();
    null[at].air_base = 0;
    assert_eq!(
        capture_references_buffer(&scale.world, BASE, scale.world.game_state, &null, Some(&registrations), engine_native_memory::walk::Buffer::Growing(&mut handles), engine_native_memory::walk::Buffer::Growing(&mut output), None)
        .err(),
        Some("registered wing has a null pool base")
    );

    let (_, base, ..) = expected
        .iter()
        .find(|(_, _, owner, _)| owner.is_some())
        .unwrap();
    let ship = scale.world.ships[&base.unwrap().host];
    scale.world.u64(ship + cship::HANDLE, 0);
    assert_eq!(
        capture_references_buffer(&scale.world, BASE, scale.world.game_state, &wings, Some(&registrations), engine_native_memory::walk::Buffer::Growing(&mut handles), engine_native_memory::walk::Buffer::Growing(&mut output), None)
        .err(),
        Some("carrier host reference handle mismatch")
    );
    scale.world.u64(ship + cship::HANDLE, base.unwrap().host);

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
            &mut output,
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
    assert_eq!(
        outcomes[5],
        Outcome::Rows {
            len: registrations.len()
        }
    );
}

#[test]
fn retired_air_mine_table_has_no_producer_metadata_or_output() {
    let scale = Scale::new();
    let mut air = Captured::with_capacities(&ample());
    let outcomes = scale.hour(&scale.census, &mut air);
    assert_eq!(outcomes.len(), TABLES.len());
    assert!(!TABLES.contains(&TableId::NavalMineLevels));
    assert_eq!(TABLES[7], TableId::AirMissionInputs);
    for view in air.tables() {
        if let TableView::Complete(rows) = view {
            assert_ne!(rows.table(), TableId::NavalMineLevels);
        }
    }
}
