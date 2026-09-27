//! A realistic synthetic world for the ETA input walk: several countries at
//! war, a province grid with rivers, states, weather, leaders, templates and
//! multi-leg routes (land, rail and naval transfer). Every pointer is absolute,
//! so the same bytes serve `SliceMemory` and, at the buffer's own address,
//! `LiveMemory`.

use super::*;
use crate::{
    ccountry, cdiplomacy_status, cgame_state, corders_group, cprovince, crelation,
    crelation_status, cstate, cstrategic_region, csub_unit_definition as stats, cunit,
    cunit_leader, cwar_relation, cweather_manager, memory::SliceMemory, modifier_accumulator,
    modifier_definition, province_definition as definition, province_neighbour as neighbour,
    sweather_per_province, sweather_per_region,
};
use std::cell::RefCell;

/// The module image: every RVA the walk reads lies below this offset.
const IMAGE: usize = 0x0340_0000;
const TERRAINS: usize = 8;
const TERRAIN_VTABLE: usize = 0x029a_a2b0;
const ADJUSTER_VTABLE: usize = 0x0277_3a70;

#[derive(Clone, Copy, Debug)]
pub(super) struct Shape {
    pub countries: u32,
    pub divisions: usize,
    pub group_size: usize,
    pub width: u32,
    pub height: u32,
    pub max_legs: u32,
    pub seed: u64,
}

impl Shape {
    /// The live bench's per-hour shape: ~694 moving divisions, ~7.5k legs.
    pub(super) fn bench(scale: f64) -> Self {
        Self {
            countries: 8,
            divisions: (694.0 * scale) as usize,
            group_size: 12,
            width: 48,
            height: 40,
            max_legs: 21,
            seed: 0x5eed,
        }
    }

    pub(super) fn small() -> Self {
        Self {
            countries: 4,
            divisions: 12,
            group_size: 3,
            width: 8,
            height: 6,
            max_legs: 6,
            seed: 7,
        }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

pub(super) struct World {
    pub bytes: Vec<u8>,
    pub base: usize,
    next: usize,
    /// Every written field, for fault injection.
    pub fields: Vec<(usize, usize)>,
    labels: Vec<(usize, &'static str)>,
    pub state: usize,
    pub seeds: Vec<DivisionSeed>,
}

impl World {
    fn offset(&self, address: usize) -> usize {
        address - self.base
    }
    fn alloc(&mut self, size: usize, label: &'static str) -> usize {
        let at = self.next;
        self.next = (self.next + size + 15) & !15;
        assert!(self.next <= self.bytes.len(), "fixture buffer exhausted");
        self.labels.push((self.base + at, label));
        self.base + at
    }

    /// The allocation an address falls in, and its offset there.
    pub(super) fn locate(&self, address: usize) -> (&'static str, usize) {
        if address >= self.base && address < self.base + IMAGE {
            return ("global", address - self.base);
        }
        match self.labels.partition_point(|&(start, _)| start <= address) {
            0 => ("outside", address),
            index => {
                let (start, label) = self.labels[index - 1];
                (label, address - start)
            }
        }
    }
    fn put(&mut self, at: usize, value: &[u8]) {
        let offset = self.offset(at);
        self.bytes[offset..offset + value.len()].copy_from_slice(value);
        self.fields.push((at, value.len()));
    }
    fn u8(&mut self, at: usize, value: u8) {
        self.put(at, &[value]);
    }
    fn u16(&mut self, at: usize, value: u16) {
        self.put(at, &value.to_le_bytes());
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn ptr(&mut self, at: usize, value: usize) {
        self.u64(at, value as u64);
    }
    fn vtable(&mut self, at: usize, rva: usize) {
        self.u64(at, (self.base + rva) as u64);
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.ptr(at, data);
        self.u32(at + 8, count);
        self.u32(at + 12, count);
    }
    fn global(&mut self, rva: usize, value: i64) {
        self.u64(self.base + rva, value as u64);
    }
    /// A sorted accumulator with the given (key, value) entries.
    fn accumulator(&mut self, at: usize, entries: &[(u32, i64)]) {
        let mut sorted = entries.to_vec();
        sorted.sort_unstable_by_key(|(key, _)| *key);
        sorted.dedup_by_key(|(key, _)| *key);
        let data = self.alloc(sorted.len().max(1) * 16, "accumulator");
        self.array(at, data, sorted.len() as u32);
        for (index, (key, value)) in sorted.into_iter().enumerate() {
            self.u32(data + index * 16, key);
            self.u64(data + index * 16 + 8, value as u64);
        }
    }

    pub(super) fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: self.base,
            bytes: &self.bytes,
        }
    }

    /// Build at `base`, which must be the address `bytes[0]` is read at.
    pub(super) fn build(shape: Shape, bytes: Vec<u8>, base: usize) -> Self {
        let mut w = World {
            bytes,
            base,
            next: IMAGE,
            fields: Vec::new(),
            labels: Vec::new(),
            state: 0,
            seeds: Vec::new(),
        };
        let mut rng = Rng(shape.seed);
        let k = shape.countries;
        let gs = w.alloc(0x800, "gs");
        w.state = gs;

        // Globals the graph reads: speed defines and modifier metadata.
        for (rva, value) in [
            (TRANSPORT_SCALE_RVA, 33_333_i64),
            (INVASION_SPEED_RVA, 50_000),
            (OUTER_SCALE_RVA, 100_000),
            (0x0331_cec0, 50_000),
            (0x0331_aa18, -40_000),
            (0x0331_97d0, 100_000),
            (0x0331_9f90, 25_000),
            (0x0331_94e0, 30_000),
            (0x0331_91c0, 24),
            (0x0331_9394, 2),
            (0x0331_9618, -20_000),
            (0x0331_9568, -10_000),
            (0x0331_9758, 25_003),
            (0x0331_9808, -5_000),
            (0x0331_d150, 90_000),
            (RIVER_CROSSING_TWO_RVA, -30_000),
            (RIVER_CROSSING_THREE_RVA, -50_000),
            (0x0331_ddd8, 70_000),
            (stats::AIR_DEFENSE_DENOMINATOR_RVA, 100_000),
            (stats::AIR_DEFENSE_REDUCTION_RVA, 50_000),
            (0x0331_aac0, -2_000),
            (0x0331_b5b4, 5),
            (0x0331_cf40, 500_000),
            (0x0331_cfd0, 20_000),
            (0x0331_d068, 100_000),
            (0x0331_d0e8, 10_000),
            (crate::cnaval_unit_transfer::BASE_SPEED_RVA, 200_000),
            (cunit::EMBARK_MOVEMENT_DISTANCE_RVA, 250_000),
            (cgame_state::PLAY_SESSION_STARTED_AT_RVA, 7),
        ] {
            w.global(rva, value);
        }
        w.u32(base + 0x0331_bfe0, 1);
        // DLC bits 4 (transport composition) and 17 (separate HQ) stay off.
        w.u32(base + modifier_definition::TRADE_FEATURE_FLAGS_RVA, 0);
        let definitions = w.alloc(0x400 * modifier_definition::STRIDE, "definitions");
        w.global(modifier_definition::TABLE_RVA, definitions as i64);
        let keys = [
            0x2b_u32, 0x2c, 0x5e, 0x16c, 0x16d, 0x17f, 0x1e7, 0x1e8, 0x1e9, 0x1ea, 0x1eb, 0x286,
            0x29, 0x77, 0x91, 0xa2, 0x130, 0x131,
        ];
        for key in keys {
            let row = definitions + key as usize * modifier_definition::STRIDE;
            w.u32(row + modifier_definition::FEATURE_SELECTOR, 0);
            w.u32(
                row + modifier_definition::FLAGS,
                if key == 0x2c {
                    modifier_definition::CLAMP_MAXIMUM
                } else {
                    0
                },
            );
        }

        // Countries: 1..=k, two sides at war. Slot zero is the reserved sentinel.
        let countries = w.alloc((k as usize + 1) * 8, "countries");
        let links = w.alloc((k as usize + 1) * 4, "links");
        w.array(gs + cgame_state::COUNTRIES, countries, k + 1);
        w.array(gs + cgame_state::COUNTRY_LINKS, links, k + 1);
        let sentinel = w.alloc(0x40, "sentinel");
        w.ptr(countries, sentinel);
        w.u32(links, 0);
        let side = |id: u32| u32::from(id > k / 2);
        let factions = [w.alloc(0x40, "faction"), w.alloc(0x40, "faction")];
        let mut country = vec![0_usize; k as usize + 1];
        let mut diplomacy = vec![0_usize; k as usize + 1];
        for id in 1..=k {
            let c = w.alloc(0x1100, "country");
            let d = w.alloc(0x300, "diplomacy");
            country[id as usize] = c;
            diplomacy[id as usize] = d;
            w.ptr(countries + id as usize * 8, c);
            w.u32(links + id as usize * 4, id);
            w.vtable(c, ccountry::VTABLE_RVA);
            w.u32(c + ccountry::ID, id);
            w.ptr(c + ccountry::DIPLOMACY_STATUS, d);
            w.vtable(d, cdiplomacy_status::VTABLE_RVA);
            w.ptr(d + cdiplomacy_status::COUNTRY, c);
            w.ptr(d + cdiplomacy_status::FACTION, factions[side(id) as usize]);
            // Air superiority is contested: the war flag makes air active.
            w.u32(d + 0xa4, 1);
            let mut modifiers = vec![
                (0x2b, 5_000 + id as i64 * 100),
                (0x2c, 20_000),
                (0x5e, -10_000),
                (0x17f, 15_000),
                (0x1e9, 10_000),
                (0x286, -5_000),
            ];
            // Realistic accumulators hold dozens of unrelated modifiers.
            for filler in 0..40_u32 {
                modifiers.push((0x300 + filler * 3, i64::from(filler)));
            }
            w.accumulator(c + ccountry::MODIFIER_ACCUMULATOR, &modifiers);
        }
        // Relations: every ordered pair has a tuple; opposite sides are at war.
        for owner in 1..=k {
            let d = diplomacy[owner as usize];
            let tuples = w.alloc((k as usize + 1) * 8, "tuples");
            w.array(d + cdiplomacy_status::RELATIONS, tuples, k + 1);
            for target in 1..=k {
                let tuple = w.alloc(0x300, "tuple");
                w.ptr(tuples + target as usize * 8, tuple);
                w.vtable(tuple, crelation_status::VTABLE_RVA);
                w.ptr(tuple + crelation_status::DIPLOMACY_STATUS, d);
                w.u32(tuple + crelation_status::TARGET_COUNTRY, target);
                w.accumulator(
                    tuple + 0x140,
                    &[(0x16d, -(i64::from(owner) * 1_000 + i64::from(target)))],
                );
                if side(owner) != side(target) {
                    let war = w.alloc(0x80, "war");
                    w.ptr(tuple + crelation_status::WAR_RELATION, war);
                    w.vtable(war, cwar_relation::VTABLE_RVA);
                    w.u32(war + crelation::TYPE_ID, cwar_relation::TYPE);
                    w.u8(war + crelation::RETIRED, 0);
                }
            }
        }

        // Terrains and their one-based database indices.
        let mut terrains = Vec::new();
        for index in 1..=TERRAINS as u32 {
            let t = w.alloc(0x200, "terrain");
            w.vtable(t, TERRAIN_VTABLE);
            w.u32(t + 8, 100 + index);
            w.u32(t + crate::cterrain_type::DATABASE_INDEX, index);
            w.vtable(t + 0x168, ADJUSTER_VTABLE);
            w.u64(t + 0x188, (-(i64::from(index) * 1_111)) as u64);
            terrains.push(t);
        }

        // Weather manager with per-province and per-region records.
        let provinces = (shape.width * shape.height) as usize;
        let regions_per_row = shape.width.div_ceil(6);
        let region_count = (regions_per_row * shape.height.div_ceil(5)) as usize;
        let weather = w.alloc(0x400, "weather");
        w.ptr(gs + cgame_state::WEATHER_MANAGER, weather);
        w.vtable(weather, cweather_manager::VTABLE_RVA);
        w.u64(weather + cweather_manager::MAX_SNOW, 400_000);
        let province_weather = w.alloc(
            (provinces + 1) * sweather_per_province::STRIDE,
            "province_weather",
        );
        w.array(
            weather + cweather_manager::PROVINCES,
            province_weather,
            provinces as u32 + 1,
        );
        let region_weather = w.alloc(
            (region_count + 1) * sweather_per_region::STRIDE,
            "region_weather",
        );
        w.array(
            weather + cweather_manager::REGIONS,
            region_weather,
            region_count as u32 + 1,
        );
        for id in 0..=provinces {
            let record = province_weather + id * sweather_per_province::STRIDE;
            w.vtable(record, sweather_per_province::VTABLE_RVA);
            w.u32(record + sweather_per_province::ID, id as u32);
            let snowy = rng.chance(20);
            w.u64(record + 0x30, if snowy { 10_000 } else { 0 });
            w.u64(
                record + sweather_per_province::SNOW,
                if snowy { 50_000 } else { 0 },
            );
            if rng.chance(30) {
                w.accumulator(
                    record + sweather_per_province::MODIFIERS,
                    &[(0x2b, -15_000), (0x1e9, -5_000)],
                );
            }
        }
        // Strategic regions, their weather, and the air manager's region cache.
        let region_roster = w.alloc((region_count + 1) * 8, "region_roster");
        w.array(
            gs + cgame_state::STRATEGIC_REGIONS,
            region_roster,
            region_count as u32 + 1,
        );
        let air = w.alloc(0x80, "air");
        w.ptr(gs + cgame_state::STRATEGIC_AIR_MANAGER, air);
        let air_countries = w.alloc((k as usize + 1) * 8, "air_countries");
        w.array(air + 0x30, air_countries, k + 1);
        let mut air_entries = vec![0_usize; k as usize + 1];
        for id in 1..=k {
            let entry = w.alloc(0x100, "entry");
            let data = w.alloc((region_count + 1) * 0xa0, "air_regions");
            w.ptr(air_countries + id as usize * 8, entry);
            w.array(entry + 0xe0, data, region_count as u32 + 1);
            air_entries[id as usize] = data;
        }
        let mut regions = vec![0_usize];
        for id in 1..=region_count {
            let r = w.alloc(0x100, "region");
            regions.push(r);
            w.vtable(r, cstrategic_region::VTABLE_RVA);
            w.u32(r + cstrategic_region::ID, id as u32);
            w.u32(r + cstrategic_region::RUNTIME_SLOT, id as u32);
            w.ptr(region_roster + id * 8, r);
            let record = region_weather + id * sweather_per_region::STRIDE;
            w.vtable(record, sweather_per_region::VTABLE_RVA);
            w.u32(record + sweather_per_region::ID, id as u32);
            w.accumulator(
                record + sweather_per_region::MODIFIERS,
                &[(0x29, -10_000), (0x2b, -3_000)],
            );
            for owner in 1..=k {
                let entry = air_entries[owner as usize] + id * 0xa0;
                w.u64(entry, rng.below(200_000));
                w.u64(entry + 8, rng.below(3) * 100_000);
                w.u64(entry + 0x18, (-(rng.below(40_000) as i64)) as u64);
            }
        }

        // Rail: a manager whose province slots are null, so rail speed is the
        // road term from each destination state's infrastructure.
        let rail = w.alloc(0x40, "rail");
        w.ptr(gs + cgame_state::RAIL_MANAGER, rail);
        w.vtable(rail, crate::crailway_manager::VTABLE_RVA);
        let rail_slots = w.alloc((provinces + 1) * 8, "rail_slots");
        w.array(rail + 8, rail_slots, provinces as u32 + 1);

        // Infrastructure: one database definition with the building lookup on.
        let database = w.alloc(0x400, "database");
        w.global(crate::cbuilding_database::SINGLETON_RVA, database as i64);
        let infrastructure = w.alloc(0x400, "infrastructure");
        w.ptr(database + 0x3b0, infrastructure);
        w.u32(infrastructure + crate::cbuilding_template::SCOPE, 5);
        w.u8(
            infrastructure + crate::cbuilding_template::LOOKUP_ENABLED_CANDIDATE,
            1,
        );
        w.u32(
            infrastructure + crate::cbuilding_template::DATABASE_ORDINAL,
            0,
        );
        w.u32(infrastructure + crate::cbuilding_template::TYPE_ID, 7);
        let building_templates = w.alloc(8, "building_templates");
        w.array(
            database + crate::cbuilding_database::TEMPLATES,
            building_templates,
            1,
        );
        w.ptr(building_templates, infrastructure);
        let fort_kind = w.alloc(0x400, "fort_kind");
        w.u32(
            fort_kind + crate::cbuilding_template::MOVEMENT_FORT_WEIGHT,
            1,
        );

        // Province grid: bands of columns per country, states of 3x2 blocks.
        let band = |x: u32| 1 + (x * k / shape.width).min(k - 1);
        let roster = w.alloc((provinces + 1) * 8, "roster");
        w.array(gs + cgame_state::PROVINCES, roster, provinces as u32 + 1);
        let id_of = |x: u32, y: u32| y * shape.width + x + 1;
        let mut state_of = std::collections::HashMap::new();
        let mut states = Vec::new();
        let mut province_ptr = vec![0_usize; provinces + 1];
        for y in 0..shape.height {
            for x in 0..shape.width {
                let id = id_of(x, y);
                let p = w.alloc(0x200, "province");
                let def = w.alloc(0xe0, "definition");
                province_ptr[id as usize] = p;
                w.ptr(roster + id as usize * 8, p);
                w.vtable(p, cprovince::VTABLE_RVA);
                w.u32(p + cprovince::ID, id);
                w.ptr(p + cprovince::DEFINITION, def);
                w.u32(def + definition::ID, id);
                w.u8(def + definition::TERRAIN_FLAGS, 1);
                let terrain = terrains[rng.below(TERRAINS as u64) as usize];
                w.ptr(def + definition::TERRAIN, terrain);
                let controller = band(x);
                w.u32(p + cprovince::CONTROLLER, controller);
                let region = regions[(1 + (y / 5) * regions_per_row + x / 6) as usize];
                w.ptr(p + cprovince::STRATEGIC_REGION, region);
                let key = (x / 3, y / 2);
                let s = match state_of.get(&key) {
                    Some(&s) => s,
                    None => {
                        let s = w.alloc(0x900, "state");
                        states.push(s);
                        w.vtable(s, cstate::VTABLE_RVA);
                        w.u32(s + cstate::ID, states.len() as u32);
                        w.u32(s + cstate::CONTROLLER, controller);
                        // An empty country hash: every owner takes the base modifier.
                        let buckets = w.alloc(cstate::COUNTRY_MODIFIER_STRIDE * 2, "buckets");
                        w.ptr(s + cstate::COUNTRY_MODIFIERS, buckets);
                        w.u32(s + cstate::COUNTRY_MODIFIERS + 12, 0);
                        w.u8(s + cstate::COUNTRY_MODIFIERS + 16, 0);
                        w.u8(buckets + 4, 0);
                        w.accumulator(
                            s + cstate::RESOURCE_MODIFIER + 0x10,
                            &[(0x2b, 2_000), (0x1e7, 7_000), (0x1e8, -7_000)],
                        );
                        let status = s + cstate::BUILDING_STATUS;
                        w.vtable(status, crate::cbuilding_status::VTABLE_RVA);
                        let indices = w.alloc(16, "indices");
                        w.array(
                            status + crate::cbuilding_status::BUILDING_INDICES,
                            indices,
                            1,
                        );
                        w.u32(indices, 0);
                        let buildings = w.alloc(16, "buildings");
                        w.array(status + crate::cbuilding_status::BUILDINGS, buildings, 1);
                        let building = w.alloc(0x200, "building");
                        w.ptr(buildings, building);
                        w.vtable(building, crate::cbuilding::VTABLE_RVA);
                        w.ptr(building + crate::cbuilding::BUILDING_STATUS, status);
                        w.u32(building + crate::cbuilding::TYPE_ID, 7);
                        w.ptr(building + crate::cbuilding::TEMPLATE, infrastructure);
                        w.u16(
                            building + crate::cbuilding::HEALTHY_LEVEL,
                            1 + rng.below(5) as u16,
                        );
                        state_of.insert(key, s);
                        s
                    }
                };
                w.ptr(p + cprovince::STATE, s);
                // Forts on a few provinces.
                if rng.chance(8) {
                    let forts = w.alloc(16, "forts");
                    w.array(p + cprovince::MOVEMENT_BUILDINGS, forts, 1);
                    let fort = w.alloc(0x200, "fort");
                    w.ptr(forts, fort);
                    w.ptr(fort + crate::cbuilding::TEMPLATE, fort_kind);
                    w.u16(fort + crate::cbuilding::HEALTHY_LEVEL, 2);
                }
            }
        }
        let state_roster = w.alloc((states.len() + 1) * 8, "state_roster");
        w.array(
            gs + cgame_state::STATES,
            state_roster,
            states.len() as u32 + 1,
        );
        for (index, &s) in states.iter().enumerate() {
            w.ptr(state_roster + (index + 1) * 8, s);
        }
        // Six-neighbour adjacency in a shuffled native order; rivers on some.
        let mut river = std::collections::HashMap::new();
        for y in 0..shape.height {
            for x in 0..shape.width {
                let id = id_of(x, y);
                let mut near = Vec::new();
                for (dx, dy) in [(1_i32, 0_i32), (-1, 0), (0, 1), (0, -1), (1, -1), (-1, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx >= 0 && ny >= 0 && (nx as u32) < shape.width && (ny as u32) < shape.height
                    {
                        near.push(id_of(nx as u32, ny as u32));
                    }
                }
                for index in (1..near.len()).rev() {
                    near.swap(index, rng.below(index as u64 + 1) as usize);
                }
                let data = w.alloc(near.len() * neighbour::STRIDE, "neighbours");
                let def = w.offset(province_ptr[id as usize] + cprovince::DEFINITION);
                let def = u64::from_le_bytes(w.bytes[def..def + 8].try_into().unwrap()) as usize;
                w.array(def + definition::NEIGHBOURS, data, near.len() as u32);
                for (slot, &to) in near.iter().enumerate() {
                    let entry = data + slot * neighbour::STRIDE;
                    let pair = (id.min(to), id.max(to));
                    let crossing = *river.entry(pair).or_insert_with(|| {
                        if rng.chance(15) {
                            1 + rng.below(3) as u8
                        } else {
                            0
                        }
                    });
                    w.u32(entry + neighbour::PROVINCE_ID, to);
                    w.u8(entry + neighbour::CROSSING_KIND, crossing);
                    w.u64(
                        entry + neighbour::MOVEMENT_DISTANCE,
                        50_000 + rng.below(100_000),
                    );
                }
            }
        }

        // Templates, the selected priority unit and its categories.
        let mut templates = vec![Vec::new(); k as usize + 1];
        for id in 1..=k {
            for _ in 0..3 {
                let t = w.alloc(0x200, "template");
                w.vtable(t, creferenced_division_template::VTABLE_RVA);
                let selected = w.alloc(0x600, "selected");
                w.ptr(t + creferenced_division_template::PRIORITY_UNIT, selected);
                w.vtable(selected, stats::VTABLE_RVA);
                w.u8(selected + 0x5a8, 4);
                w.u32(selected + 0x5d8, 0x77);
                let categories = w.alloc(3 * 8, "categories");
                w.array(selected + 0x508, categories, 3);
                for (index, token) in [0x91_u32, 0, 0xa2].into_iter().enumerate() {
                    let category = w.alloc(0x80, "category");
                    w.ptr(categories + index * 8, category);
                    w.u32(category + 0x60, token);
                }
                let units = w.alloc(2 * 0x10, "units");
                w.array(t + 0xc0, units, 2);
                for (index, (count, special)) in [(9_i32, 0_u8), (1, 1)].into_iter().enumerate() {
                    let unit = w.alloc(0x600, "unit");
                    w.ptr(units + index * 0x10, unit);
                    w.u32(units + index * 0x10 + 8, count as u32);
                    w.u8(unit + 0x5b7, special);
                }
                templates[id as usize].push(t);
            }
        }

        // Divisions in orders groups, most of them led.
        let mut group = 0;
        let mut group_country = 0;
        for index in 0..shape.divisions {
            let owner = 1 + (index as u32 * k / shape.divisions.max(1) as u32).min(k - 1);
            if index % shape.group_size == 0 || owner != group_country {
                group_country = owner;
                group = w.alloc(0x240, "group");
                w.vtable(group, corders_group::VTABLE_RVA);
                w.u32(group + corders_group::HANDLE, corders_group::REFERENCE_TYPE);
                let mut leader = 0;
                if rng.chance(80) {
                    leader = w.alloc(0x500, "leader");
                    w.vtable(leader, crate::carmy_leader::VTABLE_RVA);
                    w.u32(leader + cunit_leader::TYPE, cunit_leader::REFERENCE_TYPE);
                    w.accumulator(
                        leader + crate::carmy_leader::MOVEMENT_MODIFIER + 0x10,
                        &[
                            (0x2b, 10_000),
                            (0x2c, 5_000),
                            (0x16c, 20_000),
                            (0x5e, -5_000),
                        ],
                    );
                    let terrain = w.alloc((TERRAINS + 1) * 0x28, "leader_terrain");
                    w.array(
                        leader + crate::carmy_leader::MOVEMENT_TERRAIN_DATA,
                        terrain,
                        TERRAINS as u32 + 1,
                    );
                    for slot in 0..=TERRAINS {
                        w.u64(terrain + slot * 0x28 + 0x20, (slot as u64) * 1_500);
                    }
                }
                w.ptr(group + corders_group::LEADER, leader);
            }
            // A location in the owner's band, and a random walk from it.
            let (lo, hi) = (
                (owner - 1) * shape.width / k,
                (owner * shape.width / k).max((owner - 1) * shape.width / k + 1),
            );
            let (mut x, mut y) = (
                lo + rng.below(u64::from(hi - lo)) as u32,
                rng.below(u64::from(shape.height)) as u32,
            );
            let location = province_ptr[id_of(x, y) as usize];
            let legs = 1 + rng.below(u64::from(shape.max_legs)) as u32;
            let mut route = Vec::new();
            for _ in 0..legs {
                loop {
                    let (dx, dy) = [(1_i32, 0_i32), (-1, 0), (0, 1), (0, -1), (1, -1), (-1, 1)]
                        [rng.below(6) as usize];
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx >= 0 && ny >= 0 && (nx as u32) < shape.width && (ny as u32) < shape.height
                    {
                        (x, y) = (nx as u32, ny as u32);
                        break;
                    }
                }
                route.push(id_of(x, y));
            }
            let a = w.alloc(0x700, "army");
            let s = w.alloc(0x500, "stats");
            let terrain = w.alloc(TERRAINS * 0x28, "adjusters");
            let division = ((index as u64 + 1) << 32) | 9000;
            w.vtable(a, VTABLE_RVA);
            w.vtable(a + SECONDARY_BASE, SECONDARY_VTABLE_RVA);
            w.u64(a + cunit::HANDLE, division);
            w.u32(a + cunit::CLASS, 0);
            w.u32(a + cunit::ROSTER_OWNER, owner);
            w.u32(a + cunit::LOGICAL_OWNER, owner);
            w.u32(a + cunit::EXPEDITIONARY_OWNER, 0);
            w.ptr(a + cunit::LOCATION, location);
            let template = templates[owner as usize][index % 3];
            w.ptr(a + STANDARD_TEMPLATE, template);
            w.u64(a + STRENGTH, 900_000);
            w.u64(a + ORGANISATION, 2_000_000 + rng.below(3_000_000));
            w.ptr(a + ORDERS_GROUP, group);
            w.ptr(a + cunit::STAT_BLOCK, s);
            let data = w.alloc(route.len() * 4, "route");
            w.array(a + cunit::ORDINARY_ROUTE_DATA, data, route.len() as u32);
            for (slot, &id) in route.iter().enumerate() {
                w.u32(data + slot * 4, id);
            }
            let transfer = rng.chance(4);
            if transfer {
                let membership = w.alloc(0x40, "membership");
                w.ptr(a + NAVAL_TRANSFER_MEMBERSHIP, membership);
            }
            w.u8(
                a + STRATEGIC_REDEPLOYMENT,
                u8::from(!transfer && rng.chance(6)),
            );
            w.u8(a + RETREAT, 0);
            w.u8(a + 0x24d, 0);
            w.u8(a + 0x2ae, 0);
            // The core-supply predicate's early bypass.
            w.u8(a + 0x2e, 1);
            w.u64(a + 0x600, 1);
            w.u64(a + 0x610, 12 * 100_000);
            w.u64(a + 0x40, 30_000);
            w.u32(a + 0x498, 1);
            w.u64(a + FUEL_CANDIDATE, 400_000 + rng.below(600_000));
            w.vtable(s, stats::VTABLE_RVA);
            w.u64(s + stats::MAXIMUM_ORGANISATION, 5_000_000);
            w.u64(s + stats::MAXIMUM_STRENGTH, 1_000_000);
            w.u64(s + 0x290, 400_000 + rng.below(200_000));
            w.u64(s + 0x120, 10 * 100_000);
            w.u64(s + 0x378, -30_000_i64 as u64);
            w.u64(s + 0x3f0, -20_000_i64 as u64);
            w.u64(s + 0x3a0, -10_000_i64 as u64);
            w.u64(s + stats::MOVEMENT_AIR_DEFENSE, 20_000 + rng.below(20_000));
            w.array(
                s + stats::MOVEMENT_TERRAIN_ADJUSTERS,
                terrain,
                TERRAINS as u32,
            );
            for slot in 0..TERRAINS {
                let entry = terrain + slot * 0x28;
                w.vtable(entry, ADJUSTER_VTABLE);
                w.u32(entry + 8, 101 + slot as u32);
                w.u64(entry + 0x20, (slot as i64 * 2_000 - 5_000) as u64);
            }
            w.seeds.push(DivisionSeed {
                pointer: a,
                division,
                ..Default::default()
            });
        }
        let mut seeds = std::mem::take(&mut w.seeds);
        capture_division_seeds(&w.memory(), &mut seeds);
        w.seeds = seeds;
        w
    }

    pub(super) fn synthetic(shape: Shape) -> Self {
        let size = Self::size(shape);
        Self::build(shape, vec![0; size], 0x7f00_0000_0000)
    }

    pub(super) fn size(shape: Shape) -> usize {
        let provinces = (shape.width * shape.height) as usize;
        IMAGE
            + 0x0100_0000
            + shape.divisions * 0x1800
            + provinces * 0xb00
            + (shape.countries as usize).pow(2) * 0x800
    }
}

/// The collector's per-range adapter: one raw-array snapshot per accumulator
/// per walk, the callback's authenticated country census, and the pure air
/// defense memo. Every source read goes to `inner`.
pub(super) struct Production<'a, M> {
    pub inner: &'a M,
    pub arena: RefCell<modifier_accumulator::SnapshotArena>,
    pub roster: Option<(usize, usize, ccountry::AuthenticatedRoster<'a>)>,
    pub air: RefCell<std::collections::HashMap<[i64; 3], i64>>,
    pub definitions: modifier_definition::PassDefinitions,
}

impl<M: Memory> Memory for Production<'_, M> {
    fn read_scalar<T: crate::memory::Raw>(&self, address: usize) -> Option<T> {
        self.inner.read_scalar::<T>(address)
    }
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.inner.copy(address, destination)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.inner.copy_scattered(addresses, destinations)
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.inner
            .copy_scattered_prefetched(addresses, destinations)
    }
    fn copy_records(
        &self,
        sources: crate::memory::RecordSources<'_>,
        layout: &crate::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.inner.copy_records(sources, layout, destination)
    }
    fn authenticated_countries(
        &self,
        base: usize,
        state: usize,
    ) -> Option<ccountry::AuthenticatedRoster<'_>> {
        self.roster
            .filter(|(module, game, _)| (*module, *game) == (base, state))
            .map(|(_, _, roster)| roster)
    }
    fn modifier_definitions(&self) -> Option<&modifier_definition::PassDefinitions> {
        Some(&self.definitions)
    }
    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<modifier_accumulator::Value, &'static str>> {
        let mut arena = self.arena.borrow_mut();
        match arena.capture_at(self.inner, address, key) {
            Ok(observation) => Some(arena.value(observation)),
            Err(
                modifier_accumulator::SNAPSHOT_NODE_CAPACITY
                | modifier_accumulator::SNAPSHOT_ENTRY_CAPACITY,
            ) => None,
            Err(reason) => Some(Err(reason)),
        }
    }
    fn movement_air_defense_value(
        &self,
        defense: i64,
        denominator: i64,
        reduction: i64,
    ) -> Option<Result<i64, &'static str>> {
        let key = [defense, denominator, reduction];
        if let Some(&value) = self.air.borrow().get(&key) {
            return Some(Ok(value));
        }
        let value = stats::movement_air_defense_value(defense, denominator, reduction);
        if let Ok(value) = value {
            self.air.borrow_mut().insert(key, value);
        }
        Some(value)
    }
}

/// One walk's owned result, excluding timing counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Captured {
    pub walk: EtaInputWalk,
    pub rows: Vec<EtaInputRow>,
    pub terrain: Vec<(u8, i64)>,
    pub river: Vec<i64>,
    pub transport: Vec<i64>,
    pub rail: Vec<i64>,
    pub required_geometry: (usize, usize),
}

pub(super) struct Walker {
    pub geometry: cprovince::GeometryPacket,
    pub statics: cprovince::StaticGeometry,
    pub packet: EtaInputPacket,
    pub capacities: (usize, usize, usize),
}

impl Walker {
    pub(super) fn new(rows: usize, terrain: usize, legs: usize) -> Self {
        Self {
            geometry: cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
                provinces: 1 << 14,
                neighbour_entries: 1 << 16,
            }),
            statics: cprovince::StaticGeometry::default(),
            packet: EtaInputPacket::new(rows, terrain, legs),
            capacities: (rows, terrain, legs),
        }
    }

    /// One production walk: geometry begins, then the range is acquired.
    pub(super) fn walk(
        &mut self,
        memory: &impl Memory,
        world: &World,
        seeds: &[DivisionSeed],
        span: std::ops::Range<usize>,
        source: Outcome,
    ) -> Captured {
        let _ = self.geometry.begin(memory, world.base, world.state);
        let (rows, terrain, legs) = self.capacities;
        let walk = capture_eta_inputs(
            memory,
            world.base,
            world.state,
            seeds,
            span,
            source,
            rows,
            terrain,
            legs,
            &mut self.geometry,
            &mut self.statics,
            &mut self.packet,
        );
        let arenas = self.packet.legs();
        let required = self.geometry.required_capacities();
        Captured {
            walk,
            rows: self.packet.rows().to_vec(),
            terrain: self.packet.terrain().to_vec(),
            river: arenas.river.to_vec(),
            transport: arenas.transport.to_vec(),
            rail: arenas.rail.to_vec(),
            required_geometry: (required.provinces, required.neighbour_entries),
        }
    }
}

/// A production adapter over `memory` with the census's country roster.
pub(super) fn production<'a, M: Memory>(
    memory: &'a M,
    world: &World,
    roster: Option<ccountry::AuthenticatedRoster<'a>>,
) -> Production<'a, M> {
    Production {
        inner: memory,
        arena: RefCell::new(modifier_accumulator::SnapshotArena::new(
            modifier_accumulator::SnapshotCapacity {
                nodes: 1 << 16,
                entries: 1 << 20,
            },
        )),
        roster: roster.map(|roster| (world.base, world.state, roster)),
        air: RefCell::new(std::collections::HashMap::new()),
        definitions: Default::default(),
    }
}

/// FNV-1a over a captured walk's Debug form: stable across builds.
pub(super) fn digest(captured: &Captured) -> u64 {
    let text = format!("{captured:?}");
    text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn realistic_world_walks_every_division() {
    for shape in [Shape::small(), Shape::bench(1.0)] {
        let world = World::synthetic(shape);
        let source = world.memory();
        let mut cache = crate::country_memory::Cache::with_capacity(1024);
        let view = cache.prepare(&source, world.base, world.state);
        let memory = production(
            &source,
            &world,
            view.authenticated_countries(world.base, world.state),
        );
        let mut walker = Walker::new(1 << 13, 1 << 16, 1 << 17);
        let seeds = world.seeds.clone();
        let captured = walker.walk(
            &memory,
            &world,
            &seeds,
            0..seeds.len(),
            Outcome::Rows { len: seeds.len() },
        );
        assert_eq!(
            captured.walk.outcome,
            Outcome::Rows {
                len: shape.divisions
            },
            "{shape:?}"
        );
        let legs = captured.river.len();
        println!(
            "{shape:?}: {} rows, {legs} legs, rivers {}, transport {}, rail {}",
            captured.rows.len(),
            captured.rows.iter().filter(|row| row.any_river).count(),
            captured.rows.iter().filter(|row| row.any_transport).count(),
            captured.rows.iter().filter(|row| row.any_rail).count(),
        );
    }
}

/// Records the first source address and length of every guarded call.
struct Traced<'a> {
    source: SliceMemory<'a>,
    calls: RefCell<Vec<(&'static str, usize, usize)>>,
}

impl Memory for Traced<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls
            .borrow_mut()
            .push(("copy", address, destination.len()));
        self.source.copy(address, destination)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls
            .borrow_mut()
            .push(("scatter", addresses[0], destinations[0].len()));
        self.source.copy_scattered(addresses, destinations)
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls
            .borrow_mut()
            .push(("scatter", addresses[0], destinations[0].len()));
        self.source.copy_scattered(addresses, destinations)
    }
    fn copy_records(
        &self,
        sources: crate::memory::RecordSources<'_>,
        layout: &crate::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.calls.borrow_mut().push((
            "records",
            sources.pointer(0).unwrap_or(0) + layout.fields()[0].source_offset,
            layout.fields()[0].len,
        ));
        self.source.copy_records(sources, layout, destination)
    }
}

/// Attribute every guarded call of one warm bench-shape walk to the object
/// and field it starts at. Diagnostic output only.
#[test]
#[ignore = "profiling report"]
fn profile_guarded_calls_by_site() {
    let shape = Shape::bench(1.0);
    let world = World::synthetic(shape);
    let raw = world.memory();
    let traced = Traced {
        source: world.memory(),
        calls: RefCell::new(Vec::new()),
    };
    let mut cache = crate::country_memory::Cache::with_capacity(1024);
    let view = cache.prepare(&raw, world.base, world.state);
    let roster = view.authenticated_countries(world.base, world.state);
    let mut walker = Walker::new(1 << 13, 1 << 16, 1 << 17);
    let seeds = world.seeds.clone();
    for pass in 0..2 {
        let memory = production(&traced, &world, roster);
        traced.calls.borrow_mut().clear();
        let captured = walker.walk(
            &memory,
            &world,
            &seeds,
            0..seeds.len(),
            Outcome::Rows { len: seeds.len() },
        );
        let calls = traced.calls.borrow();
        println!(
            "pass {pass}: {} calls, {} rows, {} legs, {:.1} calls/row; counters {:?}",
            calls.len(),
            captured.rows.len(),
            captured.river.len(),
            calls.len() as f64 / captured.rows.len() as f64,
            walker.packet.counters()
        );
        let mut sites = std::collections::BTreeMap::<(&str, &str, usize, usize), usize>::new();
        let mut labels = std::collections::BTreeMap::<&str, usize>::new();
        for &(kind, address, len) in calls.iter() {
            let (label, offset) = world.locate(address);
            *sites.entry((kind, label, offset, len)).or_default() += 1;
            *labels.entry(label).or_default() += 1;
        }
        let mut labels: Vec<_> = labels.into_iter().collect();
        labels.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        println!("by object: {labels:?}");
        let mut sites: Vec<_> = sites.into_iter().collect();
        sites.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        for ((kind, label, offset, len), count) in sites.iter().take(90) {
            println!("{count:>7} {kind:>8} {label:>20} +{offset:#x} len {len}");
        }
    }
}

/// Fails every read overlapping one fixture field, after scribbling over the
/// destination as a short read would.
struct Faulted<'a> {
    source: SliceMemory<'a>,
    fault: (usize, usize),
}

impl Memory for Faulted<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let (start, len) = self.fault;
        if address < start + len && start < address + destination.len() {
            destination.fill(0xab);
            return false;
        }
        self.source.copy(address, destination)
    }
}

fn fnv(hash: u64, value: u64) -> u64 {
    value.to_le_bytes().iter().fold(hash, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Cold and warm walks over the plain source and the production adapter.
fn scenario_digests(world: &World, capacities: (usize, usize, usize)) -> [u64; 4] {
    let source = world.memory();
    let mut cache = crate::country_memory::Cache::with_capacity(1024);
    let view = cache.prepare(&source, world.base, world.state);
    let roster = view.authenticated_countries(world.base, world.state);
    let seeds = &world.seeds;
    let source_outcome = Outcome::Rows { len: seeds.len() };
    let mut digests = [0; 4];
    for (index, adapted) in [false, true].into_iter().enumerate() {
        let mut walker = Walker::new(capacities.0, capacities.1, capacities.2);
        for pass in 0..2 {
            let captured = if adapted {
                let memory = production(&source, world, roster);
                walker.walk(&memory, world, seeds, 0..seeds.len(), source_outcome)
            } else {
                walker.walk(&source, world, seeds, 0..seeds.len(), source_outcome)
            };
            digests[index * 2 + pass] = digest(&captured);
        }
    }
    digests
}

/// Golden digests of the packets the pre-batching walk produced: complete,
/// truncated by each capacity, and at thousands of divisions. All three
/// goldens were recorded on main's unbatched walk as of the perf chain
/// (8e686a539e: main e62fe7fe70 with #80, #73 and #77), whose packet no
/// longer holds the distance and crossing arenas.
#[test]
fn eta_walk_matches_its_golden_packets() {
    let small = World::synthetic(Shape::small());
    let large = World::synthetic(Shape::bench(4.0));
    let mut digests = Vec::new();
    for (world, capacities) in [
        (&small, (64, 256, 256)),
        (&small, (5, 256, 256)),
        (&small, (64, 256, 20)),
        (&small, (64, 7, 256)),
        (&small, (0, 0, 0)),
        (&large, (1 << 13, 1 << 16, 1 << 17)),
    ] {
        digests.extend(scenario_digests(world, capacities));
    }
    assert_eq!(digests, GOLDEN_PACKETS);
}

const GOLDEN_PACKETS: [u64; 24] = [
    18189401739612085451,
    18189401739612085451,
    18189401739612085451,
    18189401739612085451,
    383695225760221508,
    383695225760221508,
    383695225760221508,
    383695225760221508,
    16228887519528128836,
    16228887519528128836,
    16228887519528128836,
    16228887519528128836,
    7707527433829944049,
    7707527433829944049,
    7707527433829944049,
    7707527433829944049,
    11288763696107729991,
    11288763696107729991,
    11288763696107729991,
    11288763696107729991,
    1263167948750048186,
    1263167948750048186,
    1263167948750048186,
    1263167948750048186,
];

const GOLDEN_FAULTS: (usize, [u64; 2]) = (4169, [15411669950108721350, 6421612874394399558]);

/// Every fixture field, faulted in turn, reproduces the golden first failure,
/// row set and arenas, cold and after a clean warm walk.
#[test]
fn eta_walk_faults_match_their_golden_packets() {
    let world = World::synthetic(Shape::small());
    let seeds = &world.seeds;
    let source_outcome = Outcome::Rows { len: seeds.len() };
    let mut hashes = [0xcbf2_9ce4_8422_2325_u64; 2];
    let mut changed = 0;
    let clean = {
        let mut walker = Walker::new(64, 256, 256);
        digest(&walker.walk(
            &world.memory(),
            &world,
            seeds,
            0..seeds.len(),
            source_outcome,
        ))
    };
    for (index, &fault) in world.fields.iter().enumerate() {
        let memory = Faulted {
            source: world.memory(),
            fault,
        };
        for (warm, hash) in hashes.iter_mut().enumerate() {
            let mut walker = Walker::new(64, 256, 256);
            if warm == 1 {
                walker.walk(
                    &world.memory(),
                    &world,
                    seeds,
                    0..seeds.len(),
                    source_outcome,
                );
            }
            let result =
                digest(&walker.walk(&memory, &world, seeds, 0..seeds.len(), source_outcome));
            changed += usize::from(result != clean);
            *hash = fnv(fnv(*hash, index as u64), result);
        }
    }
    assert_eq!((changed, hashes), GOLDEN_FAULTS);
}

/// The same faults through the production adapter, whose accumulator snapshot
/// and census roster change which reads a fault can reach.
#[test]
fn eta_walk_production_faults_match_their_golden_packets() {
    let world = World::synthetic(Shape::small());
    let seeds = &world.seeds;
    let source_outcome = Outcome::Rows { len: seeds.len() };
    let clean = world.memory();
    let mut cache = crate::country_memory::Cache::with_capacity(1024);
    let view = cache.prepare(&clean, world.base, world.state);
    let roster = view.authenticated_countries(world.base, world.state);
    let mut hashes = [0xcbf2_9ce4_8422_2325_u64; 2];
    let mut changed = 0;
    let baseline = {
        let mut walker = Walker::new(64, 256, 256);
        let memory = production(&clean, &world, roster);
        digest(&walker.walk(&memory, &world, seeds, 0..seeds.len(), source_outcome))
    };
    for (index, &fault) in world.fields.iter().enumerate() {
        let faulted = Faulted {
            source: world.memory(),
            fault,
        };
        for (warm, hash) in hashes.iter_mut().enumerate() {
            let mut walker = Walker::new(64, 256, 256);
            if warm == 1 {
                let memory = production(&clean, &world, roster);
                walker.walk(&memory, &world, seeds, 0..seeds.len(), source_outcome);
            }
            let memory = production(&faulted, &world, roster);
            let result =
                digest(&walker.walk(&memory, &world, seeds, 0..seeds.len(), source_outcome));
            changed += usize::from(result != baseline);
            *hash = fnv(fnv(*hash, index as u64), result);
        }
    }
    assert_eq!((changed, hashes), GOLDEN_PRODUCTION_FAULTS);
}

const GOLDEN_PRODUCTION_FAULTS: (usize, [u64; 2]) =
    (4139, [16057994072682618205, 14133751343559739669]);

/// Bench, not a check: guarded calls, then the median and minimum wall time
/// and thread cycles of a warm walk, and of a walk after evicting the caches,
/// over the real SEH-guarded copier at the live shape and 3.4x/11.4x its units.
/// Run with `cargo test --release -p engine eta_walk_bench -- --ignored --nocapture`.
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
#[test]
#[ignore = "benchmark"]
fn eta_walk_bench() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> isize;
        fn QueryThreadCycleTime(thread: isize, cycles: *mut u64) -> i32;
    }
    let cycles = || {
        let mut value = 0;
        // SAFETY: the pseudo-handle names this thread; the output is local.
        unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut value) };
        value
    };
    let mut evict = vec![0_u8; 256 << 20];
    for scale in [1.0, 3.4, 11.4] {
        let shape = Shape::bench(scale);
        let bytes = vec![0_u8; World::size(shape)];
        let base = bytes.as_ptr() as usize;
        let world = World::build(shape, bytes, base);
        let raw = world.memory();
        let mut cache = crate::country_memory::Cache::with_capacity(1024);
        let view = cache.prepare(&raw, world.base, world.state);
        let roster = view.authenticated_countries(world.base, world.state);
        // SAFETY: the fixture buffer is immutable and outlives every read.
        let live = crate::memory::CountedMemory::new(unsafe { crate::memory::LiveMemory::new() });
        let seeds = &world.seeds;
        let mut walker = Walker::new(1 << 14, 1 << 18, 1 << 18);
        let mut samples = [[Vec::new(), Vec::new()], [Vec::new(), Vec::new()]];
        let mut calls = 0;
        for run in 0..82 {
            let cold = run % 2;
            if cold == 1 {
                for (index, byte) in evict.iter_mut().enumerate().step_by(64) {
                    *byte = byte.wrapping_add(index as u8);
                }
                std::hint::black_box(&evict);
            }
            let memory = production(&live, &world, roster);
            crate::memory::ReadCounts::take_current();
            let (started, begun) = (std::time::Instant::now(), cycles());
            let captured = walker.walk(
                &memory,
                &world,
                seeds,
                0..seeds.len(),
                Outcome::Rows { len: seeds.len() },
            );
            let (elapsed, used) = (started.elapsed(), cycles() - begun);
            let counts = crate::memory::ReadCounts::take_current();
            assert_eq!(captured.walk.outcome, Outcome::Rows { len: seeds.len() });
            calls = counts.copy_calls
                + counts.scattered_calls
                + counts.record_calls
                + counts.plan_calls;
            // The first two walks fill the session-static map memo.
            if run > 1 {
                samples[cold][0].push(elapsed.as_secs_f64() * 1e3);
                samples[cold][1].push(used as f64 / 1e6);
            }
        }
        for (label, [mut times, mut spent]) in ["warm", "cold"].into_iter().zip(samples) {
            times.sort_by(f64::total_cmp);
            spent.sort_by(f64::total_cmp);
            println!(
                "scale {scale:>4} {label}: {} divisions, {} legs, {calls} calls/walk, \
                 {:.3} ms median, {:.3} min; {:.2} Mcycles median, {:.2} min",
                seeds.len(),
                walker.packet.legs().river.len(),
                times[times.len() / 2],
                times[0],
                spent[spent.len() / 2],
                spent[0],
            );
        }
    }
}
