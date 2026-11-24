use std::collections::HashMap;
use super::*;
use engine_native::{
    carmy,
    ccountry,
    cgame_state,
    corders_group,
    cprovince::{self, definition as province_definition},
    creferenced_division_template as template,
    cstate,
    cunit,
    cunit_leader,
    cweather_manager::{self, region_weather as sweather_per_region},
};
use crate::{division_combat_inputs as inputs, land_reinforcement_inputs as reinforcement};
use engine_native_parts::{
    cbuilding_database,
    cstrategic_region,
    csub_unit_definition,
    modifier_definition,
};
use engine_native_memory::{cpdx_array::Array, memory::{RecordLayout, RecordSources}};
use std::cell::{Cell, RefCell};

const PAGE: usize = 4096;
const GAME: usize = 0x4000_0000;
const UNMAPPED: usize = 0x1_0000;

#[derive(Default)]
struct World {
    pages: HashMap<usize, Box<[u8; PAGE]>>,
    next: usize,
    copies: Cell<usize>,
    roster: Vec<Option<engine_native_memory::country_identity::Country>>,
    arrays: Option<[Array; 2]>,
    authenticated: bool,
    poisoned: RefCell<Vec<(usize, usize)>>,
    log: RefCell<Option<Vec<(usize, usize)>>>,
}

impl World {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        for (index, byte) in bytes.iter().enumerate() {
            let address = at + index;
            self.pages
                .entry(address / PAGE)
                .or_insert_with(|| Box::new([0; PAGE]))[address % PAGE] = *byte;
        }
    }
    fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn alloc(&mut self, size: usize) -> usize {
        if self.next == 0 {
            self.next = 0x1_0000_0000;
        }
        let at = self.next;
        self.next += size.next_multiple_of(64);
        at
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.u64(at, data as u64);
        self.u32(at + 8, count);
        self.u32(at + 12, count);
    }
    fn object(&mut self, size: usize, vtable: usize) -> usize {
        let at = self.alloc(size);
        self.u64(at, vtable as u64);
        at
    }
    fn accumulator(&mut self, at: usize, entries: &[(u32, i64)]) {
        let data = self.alloc(entries.len().max(1) * 16);
        for (index, (key, value)) in entries.iter().enumerate() {
            self.u32(data + index * 16, *key);
            self.u64(data + index * 16 + 8, *value as u64);
        }
        self.array(at, data, entries.len() as u32);
    }
    fn get_u64(&self, at: usize) -> u64 {
        let mut bytes = [0; 8];
        assert!(self.fill(at, &mut bytes));
        u64::from_le_bytes(bytes)
    }
    fn fill(&self, address: usize, output: &mut [u8]) -> bool {
        if address < UNMAPPED
            || address.checked_add(output.len()).is_none()
            || self
                .poisoned
                .borrow()
                .iter()
                .any(|&(at, len)| address < at + len && at < address + output.len())
        {
            return false;
        }
        for (index, byte) in output.iter_mut().enumerate() {
            let at = address + index;
            *byte = self
                .pages
                .get(&(at / PAGE))
                .map_or(0, |page| page[at % PAGE]);
        }
        true
    }
    fn span(&self, address: usize, output: &mut [u8]) -> bool {
        if let Some(log) = self.log.borrow_mut().as_mut() {
            log.push((address, output.len()));
        }
        self.fill(address, output)
    }
}

impl Memory for World {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.copies.set(self.copies.get() + 1);
        self.span(address, output)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.copies.set(self.copies.get() + 1);
        for (ordinal, (address, destination)) in addresses.into_iter().zip(destinations).enumerate()
        {
            if !destination.is_empty() && !self.span(address, destination) {
                return ordinal;
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
        self.copies.set(self.copies.get() + 1);
        let mut completed = 0;
        for row in 0..sources.len() {
            for field in layout.fields() {
                if field.len != 0 {
                    let Some(address) = sources
                        .pointer(row)
                        .filter(|&pointer| pointer != 0)
                        .and_then(|pointer| pointer.checked_add(field.source_offset))
                    else {
                        return completed;
                    };
                    let start = row * layout.stride() + field.destination_offset;
                    if !self.span(address, &mut destination[start..start + field.len]) {
                        return completed;
                    }
                }
                completed += 1;
            }
        }
        completed
    }
    fn authenticated_countries(
        &self,
        _base: usize,
        game: usize,
    ) -> Option<engine_native_memory::country_identity::AuthenticatedRoster<'_>> {
        (self.authenticated && game == GAME)
            .then(|| {
                self.arrays
                    .map(|arrays| engine_native_memory::country_identity::AuthenticatedRoster::new(&self.roster, arrays))
            })
            .flatten()
    }
    fn core_supply_access(
        &self,
        _base: usize,
        _game: usize,
        owner_raw: u32,
        area: usize,
    ) -> Option<Result<bool, &'static str>> {
        self.copies.set(self.copies.get() + 1);
        Some(Ok(!(owner_raw as usize + area / 64).is_multiple_of(3)))
    }
}

struct Shape {
    countries: u32,
    divisions: usize,
    owners: usize,
    templates: usize,
    states: usize,
    provinces: usize,
    relations: usize,
    transports: usize,
    leaders: usize,
}

struct Built {
    world: World,
    divisions: Vec<(usize, u64)>,
    intel: Vec<usize>,
    regional: Vec<usize>,
    enemies: Vec<Vec<usize>>,
    subunits: Vec<usize>,
    table: usize,
}

const TOKEN: u32 = 0x1a0;

fn world(shape: &Shape) -> Built {
    let mut w = World::default();
    let slots = shape.countries as usize + 1;
    let table = w.alloc(0x78 * 0x300);
    w.u64(modifier_definition::TABLE_RVA.0, table as u64);
    for key in [0x2b_usize, 0x5e, 0x17f, 0x1e9, 0xfb, 0x10f, TOKEN as usize] {
        w.u32(
            table + key * modifier_definition::STRIDE + modifier_definition::FLAGS,
            3,
        );
    }
    let database = w.alloc(0x400);
    w.u64(cbuilding_database::SINGLETON_RVA.0, database as u64);
    let definition = w.alloc(0x400);
    w.u64(database + 0x3b0, definition as u64);
    let countries = w.alloc(slots * 8);
    let links = w.alloc(slots * 4);
    w.array(GAME + cgame_state::COUNTRIES, countries, slots as u32);
    w.array(GAME + cgame_state::COUNTRY_LINKS, links, slots as u32);
    let relation = w.alloc(0x40);
    w.u32(relation + 8, 0x3801);
    let air_manager = w.alloc(0x100);
    w.u64(
        GAME + cgame_state::STRATEGIC_AIR_MANAGER,
        air_manager as u64,
    );
    let air_countries = w.alloc(slots * 8);
    w.array(air_manager + 0x30, air_countries, slots as u32);
    w.roster = vec![None; slots];
    for slot in 1..slots {
        w.u32(links + slot * 4, slot as u32);
    }
    let mut owners = Vec::new();
    let mut diplomacies = Vec::new();
    let mut objects = Vec::new();
    let mut regional_data = Vec::new();
    w.u32(modifier_definition::TRADE_FEATURE_FLAGS_RVA.0, 1 << 4);
    let definitions = w.alloc(0x100);
    w.u64(0x033d_e590, definitions as u64);
    let static_modifier = w.object(0x100, engine_native_parts::cstatic_modifier::VTABLE_RVA.0);
    w.accumulator(
        static_modifier + engine_native_parts::cstatic_modifier::ACCUMULATOR,
        &[(0x29, 6)],
    );
    w.u64(definitions + 0xf8, static_modifier as u64);
    w.u32(
        table + 0x29 * modifier_definition::STRIDE + modifier_definition::CATEGORY_MASK,
        5,
    );
    let navy_manager = w.object(0x100, engine_native_naval::cstrategic_navy_manager::VTABLE_RVA.0);
    w.u64(
        GAME + cgame_state::STRATEGIC_NAVY_MANAGER,
        navy_manager as u64,
    );
    let navies = w.alloc(slots * 8);
    w.array(
        navy_manager + engine_native_naval::cstrategic_navy_manager::NAVIES,
        navies,
        slots as u32,
    );
    for owner in 0..shape.owners {
        let slot = 1 + owner * (slots - 1) / shape.owners.max(1);
        let country = w.object(0x1000, ccountry::VTABLE_RVA.0);
        w.u32(country + ccountry::ID, slot as u32);
        w.u64(countries + slot * 8, country as u64);
        w.accumulator(
            country + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
            &[
                (0x2b, 5_000),
                (0x5e, -2_000),
                (0xfb, 7),
                (0x17f, 11),
                (TOKEN, 17),
                (0x1e9, 13),
            ],
        );
        let diplomacy = w.object(0x300, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0);
        w.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
        w.u64(
            diplomacy + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
            country as u64,
        );
        diplomacies.push(diplomacy);
        objects.push(country);
        w.u32(
            country + ccountry::FOG_NETWORK_TABLE_COUNT,
            u32::from(owner % 4 != 3),
        );
        let navy = w.object(0x100, engine_native_naval::cstrategic_navy::VTABLE_RVA.0);
        w.u64(
            navy + 8,
            engine_native_naval::cstrategic_navy::SECONDARY_VTABLE_RVA.0 as u64,
        );
        w.u32(navy + engine_native_naval::cstrategic_navy::OWNER, slot as u32);
        let regional = w.alloc(16);
        w.u64(regional, 700 + owner as u64);
        w.u64(regional + 8, 800 + owner as u64);
        w.array(
            navy + engine_native_naval::cstrategic_navy::REGIONAL_OPERANDS,
            regional,
            2,
        );
        regional_data.push(regional);
        w.u64(navies + slot * 8, navy as u64);
        let list = w.alloc(shape.relations.max(1) * 8);
        for index in 0..shape.relations {
            w.u64(list + index * 8, relation as u64);
        }
        w.array(
            diplomacy + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS,
            list,
            shape.relations as u32,
        );
        let air = w.alloc(0x100);
        w.u64(air_countries + slot * 8, air as u64);
        let regions = w.alloc(0xa0 * 4);
        w.array(air + 0xe0, regions, 4);
        for index in 0..4 {
            w.u64(regions + index * 0xa0, 100 + index as u64);
            w.u64(regions + index * 0xa0 + 8, index as u64);
            w.u64(regions + index * 0xa0 + 0x18, 900 + index as u64);
        }
        w.roster[slot] = Some(engine_native_memory::country_identity::Country {
            pointer: country,
            canonical_slot: slot as u32,
            id: slot as u32,
        });
        owners.push(slot as u32);
    }
    let mut intel_tables = Vec::new();
    let mut enemy_lists = Vec::new();
    for (index, (&country, &diplomacy)) in objects.iter().zip(&diplomacies).enumerate() {
        let intel = w.object(0x100, engine_native_intelligence::ccountry_intel::VTABLE_RVA.0);
        w.u32(intel + engine_native_intelligence::ccountry_intel::OWNER, owners[index]);
        let rows = w.alloc(slots * engine_native_intelligence::country_intel_row::STRIDE);
        for (other, &slot) in owners.iter().enumerate() {
            w.u64(
                rows + slot as usize * engine_native_intelligence::country_intel_row::STRIDE
                    + engine_native_intelligence::country_intel_row::NAVY,
                (index * 100 + other) as u64,
            );
        }
        w.array(intel + engine_native_intelligence::ccountry_intel::DATA, rows, slots as u32);
        w.u64(country + ccountry::INTEL_TABLE, intel as u64);
        intel_tables.push(intel);
        let enemies: Vec<usize> = if index % 3 == 0 {
            (1..=3).map(|step| (index + step) % owners.len()).collect()
        } else {
            Vec::new()
        };
        let list = w.alloc(enemies.len().max(1) * 4);
        for (at, enemy) in enemies.iter().enumerate() {
            w.u32(list + at * 4, owners[*enemy]);
        }
        w.array(
            diplomacy + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES,
            list,
            enemies.len() as u32,
        );
        enemy_lists.push(enemies);
        w.put(
            diplomacy + engine_native_politics::cdiplomacy_status::CAPITULATED,
            &[u8::from(index % 5 == 4)],
        );
    }
    for (from, &diplomacy) in diplomacies.iter().enumerate() {
        let row = w.alloc(slots * 8);
        w.array(
            diplomacy + engine_native_parts::cdiplomacy_status_layout::RELATIONS,
            row,
            slots as u32,
        );
        for (to, &target) in owners.iter().enumerate() {
            if to == from {
                continue;
            }
            let tuple = w.object(0x300, engine_native::crelation::status::VTABLE_RVA.0);
            w.u64(
                tuple + engine_native::crelation::status::DIPLOMACY_STATUS,
                diplomacy as u64,
            );
            w.u32(tuple + engine_native::crelation::status::TARGET_COUNTRY, target);
            w.u64(tuple + 0x2a0, u64::from((from + to) % 2 == 0));
            w.u64(row + target as usize * 8, tuple as u64);
        }
    }
    w.arrays = Some([
        Array::read(&w, GAME + cgame_state::COUNTRIES, 8).unwrap(),
        Array::read(&w, GAME + cgame_state::COUNTRY_LINKS, 4).unwrap(),
    ]);
    let weather = w.object(0x100, cweather_manager::VTABLE_RVA.0);
    w.u64(GAME + cgame_state::WEATHER_MANAGER, weather as u64);
    let records = w.alloc(sweather_per_region::STRIDE * 2);
    w.array(weather + cweather_manager::REGIONS, records, 2);
    let mut regions = Vec::new();
    for id in 0..2_u32 {
        let record = records + id as usize * sweather_per_region::STRIDE;
        w.u64(record, sweather_per_region::VTABLE_RVA.0 as u64);
        w.u32(record + sweather_per_region::ID, id);
        w.accumulator(record + sweather_per_region::MODIFIERS, &[(0x29, 3)]);
        let region = w.object(0x100, cstrategic_region::VTABLE_RVA.0);
        w.u32(region + cstrategic_region::ID, id);
        w.u32(region + 0x60, id + 1);
        regions.push(region);
    }
    let subunits: Vec<usize> = (0..6)
        .map(|index| {
            let unit = w.object(0x600, csub_unit_definition::VTABLE_RVA.0);
            w.u32(unit + 8, 40 + index);
            w.u64(unit + 0x58, 200_000 + u64::from(index) * 50_000);
            if index % 2 == 0 {
                w.u32(unit + 0x5d8, TOKEN);
                let categories = w.alloc(16);
                for (at, token) in [0x17f_u32, 0].into_iter().enumerate() {
                    let category = w.alloc(0x80);
                    w.u32(category + 0x60, token);
                    w.u64(categories + at * 8, category as u64);
                }
                w.array(unit + 0x508, categories, 1 + (index / 2) % 2);
            }
            unit
        })
        .collect();
    let templates: Vec<usize> = (0..shape.templates)
        .map(|index| {
            let t = w.object(0x600, template::VTABLE_RVA.0);
            w.u64(
                t + 8,
                ((index as u64 + 1) << 32) | u64::from(template::REFERENCE_TYPE),
            );
            let terms = w.alloc(16 * 4);
            let count = 2 + index % 3;
            for term in 0..count {
                w.u64(terms + term * 16, subunits[(index + term) % 6] as u64);
                w.u32(terms + term * 16 + 8, 1 + term as u32);
            }
            w.array(t + 0xc0, terms, count as u32);
            w.u64(t + template::PRIORITY_UNIT, subunits[index % 6] as u64);
            t
        })
        .collect();
    let states: Vec<usize> = (0..shape.states)
        .map(|index| {
            let state = w.object(0x1000, cstate::VTABLE_RVA.0);
            w.u32(state + cstate::ID, 500 + index as u32);
            w.u32(state + cstate::CONTROLLER, owners[index % owners.len()]);
            let buckets = w.alloc(0x40);
            w.u64(state + cstate::COUNTRY_MODIFIERS, buckets as u64);
            w.accumulator(
                state + cstate::RESOURCE_MODIFIER + 0x10,
                &[(0x2b, 4), (0x1e7, 9)],
            );
            state
        })
        .collect();
    let first_province = 1000;
    let registered = w.alloc((first_province + shape.provinces) * 8);
    w.array(
        GAME + cgame_state::PROVINCES,
        registered,
        (first_province + shape.provinces) as u32,
    );
    let provinces: Vec<usize> = (0..shape.provinces)
        .map(|index| {
            let id = first_province + index;
            let province = w.object(0x200, cprovince::VTABLE_RVA.0);
            let definition = w.alloc(0x100);
            w.u32(province + cprovince::ID, id as u32);
            w.u32(definition + province_definition::ID, id as u32);
            w.u64(province + cprovince::DEFINITION, definition as u64);
            w.u64(
                province + cprovince::STATE,
                states[index % states.len()] as u64,
            );
            w.u32(
                province + cprovince::CONTROLLER,
                owners[index % owners.len()],
            );
            w.u64(
                province + cprovince::CONTROLLER_AREA,
                (0x9000 + (index % 5) * 64) as u64,
            );
            if index % 4 != 3 {
                w.u64(
                    province + cprovince::STRATEGIC_REGION,
                    regions[index % 2] as u64,
                );
            }
            w.u64(registered + id * 8, province as u64);
            province
        })
        .collect();
    let groups: Vec<usize> = (0..shape.leaders)
        .map(|index| {
            let leader = w.object(0x1000, engine_native::carmy_leader::VTABLE_RVA.0);
            w.u64(
                leader + cunit_leader::TYPE,
                ((index as u64 + 1) << 32) | u64::from(cunit_leader::REFERENCE_TYPE),
            );
            w.accumulator(
                leader + engine_native::carmy_leader::MOVEMENT_MODIFIER + 0x10,
                &[(0x2b, 21), (0x5e, -22), (0x10f, 23), (0x17f, 24)],
            );
            let group = w.object(0x200, corders_group::VTABLE_RVA.0);
            w.u64(
                group + corders_group::HANDLE,
                ((index as u64 + 1) << 32) | u64::from(corders_group::REFERENCE_TYPE),
            );
            w.u64(group + corders_group::LEADER, leader as u64);
            group
        })
        .collect();
    let climates: Vec<usize> = (0..2)
        .map(|index| {
            let climate = w.alloc(0x40);
            w.u64(climate + 0x30, 300 + index as u64);
            climate
        })
        .collect();
    let mut divisions = Vec::new();
    for index in 0..shape.divisions {
        let army = w.object(0x800, carmy::VTABLE_RVA.0);
        let handle = ((index as u64 + 1) << 32) | 3;
        w.u64(army + cunit::HANDLE, handle);
        let stats = w.object(0x600, csub_unit_definition::VTABLE_RVA.0);
        w.u64(army + cunit::STAT_BLOCK, stats as u64);
        let province = provinces[(index * 7) % provinces.len()];
        w.u64(army + cunit::LOCATION, province as u64);
        w.u64(
            army + carmy::STANDARD_TEMPLATE,
            templates[(index * 5) % templates.len()] as u64,
        );
        let owner = owners[index % owners.len()];
        w.u32(army + cunit::ROSTER_OWNER, owner);
        w.u32(army + cunit::LOGICAL_OWNER, owner);
        let name = w.alloc(0x100);
        w.u64(army + engine_native::carmy_combat_inputs::NAME_GROUP_MEMBER, name as u64);
        w.u64(name, engine_native::cname_group_member::VTABLE_RVA.0 as u64);
        w.u32(name + engine_native::cname_group_member::TYPE, (index % 2) as u32);
        w.u32(name + engine_native::cname_group_member::ORDER, index as u32);
        w.put(name + engine_native::cname_group_member::FLAG_B, &[1]);
        w.put(army + 0x2e, &[u8::from(index % 2 == 0)]);
        if !groups.is_empty() && index % 4 == 1 {
            w.u64(
                army + carmy::ORDERS_GROUP,
                groups[(index / 4) % groups.len()] as u64,
            );
        }
        if index % 3 == 0 {
            let object = army + carmy::ACCLIMATIZATION;
            w.u64(object + 0x20, climates[index % 2] as u64);
            let entries = w.alloc(32);
            for (at, &climate) in climates.iter().enumerate() {
                w.u64(entries + at * 16, climate as u64);
                w.u64(entries + at * 16 + 8, (index + at) as u64);
            }
            w.array(object + 8, entries, 2);
            w.u64(object + 0x38, 40);
        }
        if index % 5 == 2 {
            let route = w.alloc(8);
            w.u32(
                route,
                (first_province + (index + 1) % shape.provinces) as u32,
            );
            w.u32(
                route + 4,
                (first_province + (index + 3) % shape.provinces) as u32,
            );
            w.array(army + cunit::ORDINARY_ROUTE_DATA, route, 2);
        }
        if index < shape.transports {
            w.u64(army + carmy::NAVAL_TRANSFER_MEMBERSHIP, 1);
            w.u64(army + cunit::LOCATION, provinces[0] as u64);
        }
        divisions.push((army, handle));
    }
    Built {
        world: w,
        divisions,
        intel: intel_tables,
        regional: regional_data,
        enemies: enemy_lists,
        subunits,
        table,
    }
}

#[derive(Debug, PartialEq)]
struct Captured {
    stats: inputs::Row,
    context: inputs::ContextRow,
    widths: Vec<inputs::WidthTerm>,
    reinforcement: reinforcement::ContextRow,
    modifiers: Vec<reinforcement::ModifierTerm>,
    supply: Vec<reinforcement::SupplyTerm>,
    acclimatization: Vec<reinforcement::AcclimatizationTerm>,
    transport: Vec<reinforcement::TransportTerm>,
}

type Walked = Vec<Result<Captured, &'static str>>;

const IMAGE_BATCH: usize = 20;

struct Scratch {
    widths: Vec<inputs::WidthTerm>,
    modifiers: Vec<reinforcement::ModifierTerm>,
    supply: Vec<reinforcement::SupplyTerm>,
    acclimatization: Vec<reinforcement::AcclimatizationTerm>,
    transport: Vec<reinforcement::TransportTerm>,
    groups: engine_native::cname_group_member::GroupKeys,
    names: Vec<u8>,
}

impl Scratch {
    fn new() -> Self {
        Self {
            widths: vec![Default::default(); main_combat::WIDTH_TERM_LIMIT],
            modifiers: vec![Default::default(); main_reinforcement::MODIFIER_LIMIT],
            supply: vec![Default::default(); main_reinforcement::SUPPLY_TERM_LIMIT],
            acclimatization: vec![Default::default(); main_reinforcement::ACCLIMATIZATION_LIMIT],
            transport: Vec::new(),
            groups: engine_native::cname_group_member::GroupKeys::with_capacity(64, 64),
            names: vec![0; 4096],
        }
    }
    fn captured(
        &self,
        stats: inputs::Row,
        context: inputs::ContextRow,
        row: reinforcement::ContextRow,
    ) -> Captured {
        Captured {
            stats,
            widths: self.widths[..context.width_term_count].to_vec(),
            context,
            modifiers: self.modifiers[..row.modifier_count].to_vec(),
            supply: self.supply[..row.supply_term_count].to_vec(),
            acclimatization: self.acclimatization[..row.acclimatization_count].to_vec(),
            transport: self.transport[..row.transport_term_count].to_vec(),
            reinforcement: row,
        }
    }
}

fn walk_main(memory: &World, divisions: &[(usize, u64)]) -> Walked {
    use super::{main_combat as combat, main_reinforcement as reinforcement};
    let mut memo = combat::ContextModifiers::default();
    let mut s = Scratch::new();
    let countries = engine_native::ccountry::Reader::new(memory, 0, GAME);
    divisions
        .iter()
        .map(|&(army, handle)| {
            let stats = combat::capture_stats(
                memory,
                0,
                army,
                handle,
                &mut s.groups,
                &mut engine_native::cname_group_member::Names {
                    bytes: engine_native_memory::walk::Buffer::Growing(&mut s.names),
                    len: 0,
                    required: 0,
                },
            )?;
            let countries = countries.as_ref().map_err(|reason| *reason)?;
            let context = combat::capture_context(
                memory,
                0,
                countries,
                army,
                handle,
                &mut s.widths,
                &mut memo,
            )?;
            let row = reinforcement::capture_context(
                memory,
                0,
                GAME,
                army,
                handle,
                &mut s.modifiers,
                &mut s.supply,
                &mut s.acclimatization,
                &mut s.transport,
                0,
            )?;
            Ok(s.captured(stats, context, row))
        })
        .collect()
}

fn walk_shared(memory: &World, sources: &mut Sources, divisions: &[(usize, u64)]) -> Walked {
    sources.clear();
    let mut s = Scratch::new();
    let countries = engine_native::ccountry::Reader::new(memory, 0, GAME);
    let mut images = Images::default();
    divisions
        .iter()
        .enumerate()
        .map(|(index, &(army, handle))| {
            let row = index % IMAGE_BATCH;
            if row == 0 {
                let armies: Vec<usize> = divisions[index..]
                    .iter()
                    .take(IMAGE_BATCH)
                    .map(|division| division.0)
                    .collect();
                images.capture(memory, &armies).unwrap();
            }
            let image = images.division(memory, row, army);
            let memory = &image;
            let stats = inputs::capture_stats(
                memory,
                0,
                sources,
                army,
                handle,
                &mut s.groups,
                &mut engine_native::cname_group_member::Names {
                    bytes: engine_native_memory::walk::Buffer::Growing(&mut s.names),
                    len: 0,
                    required: 0,
                },
            )?;
            let countries = countries.as_ref().map_err(|reason| *reason)?;
            let context = inputs::capture_context(
                memory,
                0,
                countries,
                sources,
                army,
                handle,
                &mut s.widths,
            )?;
            let row = reinforcement::capture_context(
                memory,
                0,
                GAME,
                sources,
                army,
                handle,
                &mut s.modifiers,
                &mut s.supply,
                &mut s.acclimatization,
                &mut s.transport,
                0,
            )?;
            Ok(s.captured(stats, context, row))
        })
        .collect()
}

type Row = Result<reinforcement::ContextRow, &'static str>;

fn reinforcement_alone(memory: &World, divisions: &[(usize, u64)]) -> [Vec<Row>; 2] {
    let mut s = Scratch::new();
    let main = divisions
        .iter()
        .map(|&(army, handle)| {
            super::main_reinforcement::capture_context(
                memory,
                0,
                GAME,
                army,
                handle,
                &mut s.modifiers,
                &mut s.supply,
                &mut s.acclimatization,
                &mut s.transport,
                0,
            )
        })
        .collect();
    let mut sources = Sources::new();
    let shared = divisions
        .iter()
        .map(|&(army, handle)| {
            reinforcement::capture_context(
                memory,
                0,
                GAME,
                &mut sources,
                army,
                handle,
                &mut s.modifiers,
                &mut s.supply,
                &mut s.acclimatization,
                &mut s.transport,
                0,
            )
        })
        .collect();
    [main, shared]
}

fn both(built: &Built) -> ((Walked, usize), (Walked, usize)) {
    let w = &built.world;
    w.copies.set(0);
    let main = walk_main(w, &built.divisions);
    let main_calls = w.copies.replace(0);
    let shared = walk_shared(w, &mut Sources::new(), &built.divisions);
    ((main, main_calls), (shared, w.copies.replace(0)))
}

fn small() -> Shape {
    Shape {
        countries: 60,
        divisions: 240,
        owners: 9,
        templates: 17,
        states: 11,
        provinces: 23,
        relations: 40,
        transports: 5,
        leaders: 4,
    }
}

#[test]
fn shared_sources_change_no_output() {
    for authenticated in [true, false] {
        let mut built = world(&small());
        built.world.authenticated = authenticated;
        let ((main, main_calls), (shared, shared_calls)) = both(&built);
        assert!(
            main.iter().all(Result::is_ok),
            "main's walk failed: {:?}",
            main.iter().find(|r| r.is_err())
        );
        assert_eq!(main, shared);
        let rows: Vec<_> = main.iter().map(|r| r.as_ref().unwrap()).collect();
        let r = |row: &&Captured| row.reinforcement;
        assert!(
            rows.iter()
                .map(r)
                .any(|r| r.flags[7] == 1 && r.scalar_values[15] != 0)
        );
        assert!(rows.iter().map(r).any(|r| r.flags[11] == 2));
        assert!(rows.iter().any(|row| !row.transport.is_empty()));
        assert!(rows.iter().map(r).any(|r| r.flags[8] == 1));
        assert!(
            rows.iter()
                .map(r)
                .any(|r| r.flags[3] == 0 && r.flags[8] == 0)
        );
        assert!(rows.iter().map(r).any(|r| r.flags[6] == 1 && r.ids[2] != 0));
        assert!(rows.iter().map(r).any(|r| r.ids[1] != 0));
        assert!(rows.iter().any(|row| row.context.leader_modifier_kind == 1));
        assert!(rows.iter().any(|row| {
            row.modifiers
                .iter()
                .any(|term| term.scope == 5 && term.present == 1)
        }));
        assert!(rows.iter().any(|row| {
            row.modifiers
                .iter()
                .any(|term| term.scope == 6 && term.present == 1)
        }));
        assert!(rows.iter().any(|row| !row.acclimatization.is_empty()));
        assert!(rows.iter().any(|row| row.stats.naming.unwrap().kind == 1));
        assert!(rows.iter().all(|row| !row.widths.is_empty()));
        assert!(rows.iter().any(|row| !row.supply.is_empty()));
        assert!(
            shared_calls * 3 < main_calls,
            "{shared_calls} of {main_calls}"
        );
    }
}

#[test]
fn shared_sources_fail_as_main_does() {
    let shape = Shape {
        divisions: 20,
        ..small()
    };
    for authenticated in [true, false] {
        let mut built = world(&shape);
        built.world.authenticated = authenticated;
        *built.world.log.borrow_mut() = Some(Vec::new());
        walk_main(&built.world, &built.divisions);
        let _ = reinforcement_alone(&built.world, &built.divisions);
        let log = built.world.log.borrow_mut().take().unwrap();
        let mut seen = std::collections::HashSet::new();
        let spans: Vec<_> = log.into_iter().filter(|span| seen.insert(*span)).collect();
        let step = (spans.len() / 200).max(1);
        let mut failures = 0;
        for &span in spans.iter().step_by(step) {
            *built.world.poisoned.borrow_mut() = vec![span];
            let ((main, _), (shared, _)) = both(&built);
            assert_eq!(main, shared, "{span:x?}, roster {authenticated}");
            let [main, shared] = reinforcement_alone(&built.world, &built.divisions);
            assert_eq!(main, shared, "{span:x?} alone, roster {authenticated}");
            failures += usize::from(main.iter().any(Result::is_err));
        }
        assert!(failures > 0);
    }
}

#[test]
fn staged_sources_keep_main_failure_order() {
    type Faults = fn(&Built) -> Vec<(usize, usize)>;
    let cases: [(&str, Faults); 3] = [
        ("regional before later intel", |built| {
            let owner = (0..built.enemies.len())
                .find(|&owner| built.enemies[owner].len() >= 2)
                .unwrap();
            let [first, second] = [built.enemies[owner][0], built.enemies[owner][1]];
            vec![
                (built.regional[first], 16),
                (built.intel[second] + engine_native_intelligence::ccountry_intel::OWNER, 4),
            ]
        }),
        ("regional before own intel", |built| {
            let owner = (0..built.enemies.len())
                .find(|&owner| !built.enemies[owner].is_empty())
                .unwrap();
            let enemy = built.enemies[owner][0];
            vec![
                (built.regional[enemy], 16),
                (built.intel[enemy] + engine_native_intelligence::ccountry_intel::OWNER, 4),
            ]
        }),
        ("token before categories", |built| {
            vec![
                (
                    built.table
                        + TOKEN as usize * modifier_definition::STRIDE
                        + modifier_definition::FLAGS,
                    12,
                ),
                (built.subunits[0] + 0x508, 16),
            ]
        }),
    ];
    for (label, faults) in cases {
        let built = world(&small());
        *built.world.poisoned.borrow_mut() = faults(&built);
        let ((main, _), (shared, _)) = both(&built);
        assert!(main.iter().any(Result::is_err), "{label}");
        assert_eq!(main, shared, "{label}");
        let [main, shared] = reinforcement_alone(&built.world, &built.divisions);
        assert_eq!(main, shared, "{label} alone");
    }
    let mut built = world(&small());
    let unit = built.subunits[0];
    let data = built.world.get_u64(unit + 0x508);
    built.world.u64(unit + 0x508, data);
    built.world.u32(unit + 0x508 + 8, 1);
    built.world.u32(unit + 0x508 + 12, 2);
    let metadata =
        built.table + TOKEN as usize * modifier_definition::STRIDE + modifier_definition::FLAGS;
    let first = |walked: &Walked| walked.iter().find_map(|r| r.as_ref().err().copied());
    let ((header, _), (shared, _)) = both(&built);
    assert_eq!(header, shared);
    assert_eq!(first(&header), Some("reinforcement array invalid"));
    *built.world.poisoned.borrow_mut() = vec![(metadata, 12)];
    let ((both_fail, _), (shared, _)) = both(&built);
    assert_eq!(both_fail, shared);
    assert_eq!(first(&both_fail), Some("reinforcement input unreadable"));
}

#[test]
fn image_lookup_keeps_linear_boundaries_and_army_before_aliased_stats() {
    let army: Vec<u8> = (0..ARMY_STRIDE).map(|index| index as u8).collect();
    let stats: Vec<u8> = (0..STAT_STRIDE)
        .map(|index| (index as u8).wrapping_add(71))
        .collect();
    let source = engine_native_memory::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    for stat in [0x2000, 0x2080, 0x4000] {
        let image = DivisionImage {
            inner: &source,
            army: 0x2000,
            army_row: Some(&army),
            stat,
            stat_row: Some(&stats),
        };
        let linear = |address: usize, len: usize| {
            [
                (image.army, &army[..], &ARMY_FIELDS[..]),
                (stat, &stats[..], &STAT_FIELDS[..]),
            ]
            .into_iter()
            .find_map(|(base, row, fields)| {
                let offset = address.checked_sub(base)?;
                let end = offset.checked_add(len)?;
                fields
                    .iter()
                    .find(|field| {
                        field.len != 0
                            && field.source_offset <= offset
                            && end <= field.source_offset + field.len
                    })
                    .map(|field| {
                        let start = field.destination_offset + offset - field.source_offset;
                        &row[start..start + len]
                    })
            })
        };
        for base in [image.army, stat] {
            for offset in 0..0x640 {
                for len in 0..=40 {
                    assert_eq!(image.served(base + offset, len), linear(base + offset, len));
                }
            }
        }
        assert_eq!(image.served(0, 1), None);
        assert_eq!(image.served(usize::MAX, usize::MAX), None);
    }
}

#[test]
fn batched_relation_walk_matches_the_sequential_walk() {
    #[derive(Clone, Copy, Debug)]
    enum Fault {
        Null,
        Pointer,
        Kind,
    }
    let sequential = |w: &World, relations: Array| -> Result<bool, &'static str> {
        for i in 0..relations.count {
            let at = relations.element(i, 8).unwrap();
            let relation = read::<u64>(w, at).ok_or("reinforcement input unreadable")? as usize;
            let kind = relation
                .checked_add(8)
                .and_then(|at| read::<u32>(w, at))
                .ok_or("reinforcement input unreadable")?;
            if kind == 0x380a {
                return Ok(true);
            }
        }
        Ok(false)
    };
    let mut cases = vec![
        (0, None, None),
        (1, Some(0), None),
        (63, None, None),
        (64, Some(63), None),
        (65, Some(64), None),
        (200, Some(130), None),
    ];
    for fault in [Fault::Null, Fault::Pointer, Fault::Kind] {
        for (war, at) in [(None, 77), (Some(90), 150), (Some(150), 90), (Some(64), 64)] {
            cases.push((200, war, Some((fault, at))));
        }
        cases.push((64, None, Some((fault, 63))));
        cases.push((65, Some(64), Some((fault, 0))));
    }
    for (count, war, fault) in cases {
        let mut w = World::default();
        let peace = w.alloc(0x20);
        w.u32(peace + 8, 0x3801);
        let at_war = w.alloc(0x20);
        w.u32(at_war + 8, 0x380a);
        let list = w.alloc(count.max(1) * 8);
        let mut relations = Vec::new();
        for index in 0..count {
            let relation = match (war, fault) {
                (_, Some((Fault::Null, n))) if n == index => 0,
                (Some(n), _) if n == index => at_war,
                _ => {
                    let own = w.alloc(0x20);
                    w.u32(own + 8, 0x3801);
                    own
                }
            };
            relations.push(relation);
            w.u64(list + index * 8, relation as u64);
        }
        match fault {
            Some((Fault::Pointer, n)) => w.poisoned.borrow_mut().push((list + n * 8, 8)),
            Some((Fault::Kind, n)) => w.poisoned.borrow_mut().push((relations[n] + 8, 4)),
            _ => {}
        }
        let header = w.alloc(16);
        w.array(header, list, count as u32);
        let relations = Array::read(&w, header, 8).unwrap();
        let expected = sequential(&w, relations);
        *w.log.borrow_mut() = Some(Vec::new());
        let batched = reinforcement::owned_war_relation(&w, relations);
        assert_eq!(
            batched, expected,
            "{count} relations, war {war:?}, fault {fault:?}"
        );
        if fault.is_some() && war.is_none() {
            assert!(expected.is_err());
        }
        let log = w.log.borrow_mut().take().unwrap();
        let mut seen = std::collections::HashSet::new();
        assert!(log.iter().all(|span| seen.insert(*span)), "a span reread");
    }
}

