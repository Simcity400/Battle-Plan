use super::*;
use engine_map_air::map_air_base_reference::BaseReference;
use engine_native_air::{air_mission, cair_base, ccountry_air_container};
use engine_native_parts::{cair_wing_pool, cstrategic_region, csub_unit_definition};
use engine_native::{
    cgame_state,
    cweather_manager::{self, region_weather as sweather_per_region},
};
use engine_native_memory::memory::SliceMemory;
use std::cell::Cell;

const MODULE: usize = 0x4000_0000;
const START: usize = 0x10_0000;
const COUNTRY_SIZE: usize = 0x1500;
const COUNTRY_REGION_MAP: usize = 0x4d8;
const REGION_COVERAGE: usize = 0x50;
const REASSIGNMENT_PENALTY: usize = 0xc0;
const REACHED: u32 = 16;
const EFFICIENCY_OVERRIDES_RVA: usize = 0x033d_da48;

#[test]
fn held_patrol_night_reads_each_demanded_region_once_and_reuses_manager() {
    const APPLIED_NIGHT_FACTOR: usize = 0x268;
    let mut world = World { bytes: Vec::new() };
    let game = world.alloc(0x800);
    let manager = world.alloc(0x500);
    let region = world.alloc(0x180);
    let list = world.alloc(8);
    let province = world.alloc(0x200);
    let clocks = world.alloc(2 * engine_native::cweather_manager::province_weather::CLOCK_STRIDE);
    world.d(game + cgame_state::CURRENT_HOUR, 43_800_000);
    world.put(manager + cweather_manager::SUSPEND_FLAGS, &[1, 0]);
    world.array(manager + cweather_manager::PROVINCE_CLOCKS, clocks, 2);
    world.vtable(region, cstrategic_region::VTABLE_RVA);
    world.d(region + cstrategic_region::ID, 7);
    world.array(region + cstrategic_region::PROVINCES, list, 1);
    world.q(list, province as u64);
    world.vtable(province, engine_native::cprovince::VTABLE_RVA);
    world.d(province + engine_native::cprovince::ID, 1);
    let marker = clocks + engine_native::cweather_manager::province_weather::CLOCK_STRIDE + APPLIED_NIGHT_FACTOR;
    world.q(marker, 50_001);
    struct Trace<'a> {
        memory: SliceMemory<'a>,
        marker: usize,
        reads: Cell<usize>,
    }
    impl Memory for Trace<'_> {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            if at == self.marker {
                self.reads.set(self.reads.get() + 1);
            }
            self.memory.copy(at, out)
        }
    }
    let memory = Trace {
        memory: SliceMemory {
            base: START,
            bytes: &world.bytes,
        },
        marker,
        reads: Cell::new(0),
    };
    let mut sorties = Sorties::with_capacities(&Capacities {
        missions: 3,
        ..Default::default()
    });
    for (wing, count) in [(11, 7), (22, 9), (33, 0)] {
        sorties
            .missions
            .push(Mission {
                wing,
                region: Some(7),
                operands: native::Mission {
                    region_pointer: region,
                    current_type: Some(8),
                    effective_aircraft: Some(count),
                    normalization: Some(native::Normalization {
                        patrol: native::patrol::Inputs {
                            disruption: Some(1),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
    }
    let mut cached = Cached {
        weather_manager: Some(Ok(manager)),
        ..Default::default()
    };
    sorties.patrol_night(&memory, MODULE, game, &mut cached);
    assert_eq!(memory.reads.get(), 1);
    assert_eq!(
        sorties.missions.rows[..3]
            .iter()
            .map(|mission| mission.operands.normalization.unwrap().patrol.night_applied)
            .collect::<Vec<_>>(),
        [Some(50_001), Some(50_001), None]
    );
    assert_eq!(cached.weather_manager, Some(Ok(manager)));
}

struct World {
    bytes: Vec<u8>,
}

impl World {
    fn alloc(&mut self, size: usize) -> usize {
        let at = (START + self.bytes.len() + 0xf) & !0xf;
        self.bytes.resize(at + size - START, 0);
        at
    }
    fn put(&mut self, at: usize, value: &[u8]) {
        self.bytes[at - START..at - START + value.len()].copy_from_slice(value);
    }
    fn q(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn d(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn vtable(&mut self, at: usize, rva: engine_sites::Rva) {
        self.q(at, (MODULE + rva.0) as u64);
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.q(at, data as u64);
        self.d(at + 8, count);
        self.d(at + 12, count);
    }
    fn accumulator(&mut self, at: usize, raw: i64) {
        let entries = self.alloc(16);
        self.d(entries, native::SORTIE_MODIFIER_KEY);
        self.q(entries + 8, raw as u64);
        self.array(at, entries, 1);
    }
}

struct Route<'a> {
    world: SliceMemory<'a>,
}

const NO_OVERRIDES: [u8; 16] = [0; 16];

impl Memory for Route<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address >= MODULE {
            SliceMemory {
                base: MODULE + EFFICIENCY_OVERRIDES_RVA,
                bytes: &NO_OVERRIDES,
            }
            .copy(address, destination)
        } else {
            self.world.copy(address, destination)
        }
    }
}

struct Counting<'a> {
    inner: Route<'a>,
    reads: Cell<usize>,
}

impl<'a> Counting<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            inner: Route {
                world: SliceMemory { base: START, bytes },
            },
            reads: Cell::new(0),
        }
    }
    fn count(&self) {
        self.reads.set(self.reads.get() + 1);
    }
}

impl Memory for Counting<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.count();
        self.inner.copy(address, destination)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.count();
        self.inner.copy_scattered(addresses, destinations)
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.count();
        self.inner
            .copy_scattered_prefetched(addresses, destinations)
    }
    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.count();
        self.inner.copy_records(sources, layout, destination)
    }
}

struct Fixture {
    world: World,
    game_state: usize,
    wings: Vec<WingIdentity>,
    witnesses: Vec<BaseWitness>,
}

fn fixture(missions: usize, countries: u32) -> Fixture {
    const FLYING: u32 = 300;
    const REGIONS: u32 = 170;
    let mut world = World { bytes: Vec::new() };
    let game_state = world.alloc(0x1000);
    let slots = countries + 1;
    let table = world.alloc(slots as usize * 8);
    let links = world.alloc(slots as usize * 4);
    world.array(game_state + cgame_state::COUNTRIES, table, slots);
    world.array(game_state + cgame_state::COUNTRY_LINKS, links, slots);
    let containers = world.alloc(slots as usize * 8);
    for slot in 1..slots {
        let country = world.alloc(COUNTRY_SIZE);
        world.q(table + slot as usize * 8, country as u64);
        world.d(links + slot as usize * 4, slot);
        world.vtable(country, ccountry::VTABLE_RVA);
        world.d(country + ccountry::ID, slot);
        world.accumulator(country + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, 2_000);
        let map = country + COUNTRY_REGION_MAP;
        let sentinel = world.alloc(16);
        let buckets = world.alloc(16);
        world.q(sentinel, sentinel as u64);
        world.q(sentinel + 8, sentinel as u64);
        world.q(buckets, sentinel as u64);
        world.q(buckets + 8, sentinel as u64);
        world.q(map + 0x08, sentinel as u64);
        world.q(map + 0x18, buckets as u64);
        world.q(map + 0x38, 1);
        if slot <= FLYING {
            let hosts = world.alloc(8);
            world.d(hosts, (slot % countries) + 1);
            world.d(hosts + 4, ((slot + 7) % countries) + 1);
            world.array(country + ccountry::VOLUNTEER_HOSTS, hosts, 2);
            let container = world.alloc(ccountry_air_container::SIZE);
            world.vtable(container, ccountry_air_container::VTABLE_RVA);
            world.d(container + ccountry_air_container::COUNTRY, slot);
            for (offset, raw) in [
                (ccountry_air_container::SUPPLY_RATIO_RAW, 90_000),
                (ccountry_air_container::SORTIE_CAPACITY_FACTOR_RAW, 80_000),
                (ccountry_air_container::FUEL_DEMAND_RAW, 1_000),
                (ccountry_air_container::FUEL_ALLOCATED_RAW, 700),
            ] {
                world.q(container + offset, raw);
            }
            world.q(containers + slot as usize * 8, container as u64);
        }
    }
    let manager = world.alloc(0x100);
    world.q(game_state + cgame_state::WEATHER_MANAGER, manager as u64);
    world.vtable(manager, cweather_manager::VTABLE_RVA);
    let records = world.alloc((REGIONS as usize + 1) * sweather_per_region::STRIDE);
    world.array(manager + cweather_manager::REGIONS, records, REGIONS + 1);
    let mut regions = vec![0];
    for id in 1..=REGIONS {
        let record = records + id as usize * sweather_per_region::STRIDE;
        world.vtable(record, sweather_per_region::VTABLE_RVA);
        world.d(record + sweather_per_region::ID, id);
        world.accumulator(record + sweather_per_region::MODIFIERS, -5_000);
        let region = world.alloc(0x100);
        world.vtable(region, cstrategic_region::VTABLE_RVA);
        world.vtable(
            region + cstrategic_region::SECONDARY,
            cstrategic_region::SECONDARY_VTABLE_RVA,
        );
        regions.push(region);
    }
    let definitions: Vec<usize> = (0..20)
        .map(|_| {
            let definition = world.alloc(csub_unit_definition::AIR_FLAGS + 8);
            world.vtable(definition, csub_unit_definition::VTABLE_RVA);
            definition
        })
        .collect();
    let pools: Vec<usize> = (0..missions.div_ceil(8))
        .map(|index| {
            let pool = world.alloc(cair_wing_pool::SIZE);
            let definition = definitions[index % definitions.len()];
            world.q(pool + cair_wing_pool::DEFINITION, definition as u64);
            pool
        })
        .collect();
    let mut witnesses: Vec<BaseWitness> = (0..missions.div_ceil(4))
        .map(|index| {
            let object = world.alloc(0x80);
            world.array(object + cair_base::COUNTRY_CONTAINERS, containers, slots);
            BaseWitness {
                handle: ((index as u64 + 1) << 32) | u64::from(engine_native_parts::cair_base_layout::TYPE),
                base: Some(BaseReference {
                    object,
                    state: Some(index as u32 + 1),
                    province: Some(index as u32 + 1),
                    host: 0,
                    kind: 0,
                }),
                host_owner: None,
                host_without_task_force: false,
            }
        })
        .collect();
    witnesses.sort_unstable_by_key(|witness| witness.handle);
    let wings = (0..missions)
        .map(|index| {
            let mission = world.alloc(0xd8);
            let region = index as u32 % REGIONS + 1;
            world.vtable(mission, air_mission::VTABLE_RVA);
            world.q(
                mission + air_mission::REGION,
                regions[region as usize] as u64,
            );
            let reached = world.alloc(REACHED as usize * 16);
            for entry in 0..REACHED {
                world.d(reached + entry as usize * 16, 1_000 + (REACHED - entry) * 3);
            }
            world.array(
                mission + air_mission::REACHED_PROVINCES_DATA,
                reached,
                REACHED,
            );
            world.q(mission + REGION_COVERAGE, 90_000);
            world.q(mission + REASSIGNMENT_PENALTY, 10_000);
            world.q(mission + 0xd0, index as u64 * 25_000);
            world.d(mission + air_mission::LAST_TARGET_PROVINCE, 1_000);
            WingIdentity {
                mission_pointer: mission,
                pool: pools[index / 8],
                wing: ((index as u64 + 1) << 32) | 54,
                owner: index as u32 % FLYING + 1,
                region: Some(region),
                mission_mask: 1 << 4,
                air_base: witnesses[index / 4].handle,
                base_override: 0,
                country_override: 0,
                ace: 0,
                planes: 0,
                ..Default::default()
            }
        })
        .collect();
    Fixture {
        world,
        game_state,
        wings,
        witnesses,
    }
}

fn capacities() -> Capacities {
    Capacities {
        missions: 8192,
        reached: 1 << 17,
        rows: 16_384,
        keys: 65_536,
        hosts: 2,
    }
}

#[test]
fn volunteer_hosts_past_capacity_grow() {
    let capacity = Capacities {
        hosts: 1,
        ..capacities()
    };
    let mut sorties = Sorties::with_capacities(&capacity);
    let fixture = fixture(8, 1_500);
    let memory = Counting::new(&fixture.world.bytes);
    let capture = |sorties: &mut Sorties| {
        sorties.capture(
            &memory,
            MODULE,
            fixture.game_state,
            false,
            fixture.wings.iter().map(Ok),
            &fixture.witnesses,
        )
    };
    capture(&mut sorties);
    assert_eq!(sorties.outcome, Outcome::Rows { len: 0 });
    assert_eq!(sorties.containers.len, 18);
    let rows = &sorties.containers.rows[..18];
    assert_eq!(
        rows.iter()
            .filter(|row| row.fuel_demand_raw.is_some())
            .count(),
        8
    );
}

#[test]
fn reached_provinces_keep_the_native_order() {
    let mut sorties = Sorties::with_capacities(&capacities());
    let fixture = fixture(2, 16);
    let memory = Counting::new(&fixture.world.bytes);
    sorties.capture(
        &memory,
        MODULE,
        fixture.game_state,
        false,
        fixture.wings.iter().map(Ok),
        &fixture.witnesses,
    );
    let mission = sorties.missions.rows[0];
    let reached =
        &sorties.reached.rows[mission.reached_start..mission.reached_start + mission.reached_len];
    let native: Vec<u32> = (0..REACHED)
        .map(|entry| 1_000 + (REACHED - entry) * 3)
        .collect();
    assert_eq!(reached, native);
}

#[test]
fn cas_ratio_projection_preserves_zero_signed_values_and_field_local_faults() {
    let mut fixture = fixture(3, 16);
    let values = [0_i64, -7, i64::MIN];
    for (wing, value) in fixture.wings.iter().zip(values) {
        fixture.world.q(wing.mission_pointer + 0xd0, value as u64);
    }
    struct Fault<'a> {
        memory: &'a Counting<'a>,
        at: Option<usize>,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if self
                .at
                .is_some_and(|at| address <= at && at < address.saturating_add(destination.len()))
            {
                return false;
            }
            self.memory.copy(address, destination)
        }
    }
    let memory = Counting::new(&fixture.world.bytes);
    let mut sorties = Sorties::with_capacities(&capacities());
    for fault in [None, Some(fixture.wings[1].mission_pointer + 0xd0)] {
        sorties.capture(
            &Fault {
                memory: &memory,
                at: fault,
            },
            MODULE,
            fixture.game_state,
            false,
            fixture.wings.iter().map(Ok),
            &fixture.witnesses,
        );
        let tables = sorties.tables();
        let TableView::Complete(replay_data::rows::TableRows::AirMissionInputs(rows)) = &tables[0]
        else {
            panic!("complete mission inputs expected");
        };
        assert_eq!(rows.len(), 3);
        for (index, row) in rows.iter().enumerate() {
            let expected = if fault.is_some() && index == 1 {
                None
            } else {
                Some(values[index])
            };
            assert_eq!(
                row.cas_ratio_raw,
                replay_data::rows::Recorded::Value(expected)
            );
            assert_eq!(row.coverage_raw, 90_000);
        }
    }
}

#[test]
fn sortie_cost_scales_with_missions_not_countries() {
    let mut sorties = Sorties::with_capacities(&capacities());
    for (missions, countries) in [
        (1_000, 1_500),
        (2_000, 1_500),
        (5_000, 1_500),
        (5_000, 5_000),
    ] {
        let fixture = fixture(missions, countries);
        let memory = Counting::new(&fixture.world.bytes);
        let mut best = std::time::Duration::MAX;
        for _ in 0..5 {
            memory.reads.set(0);
            let begin = std::time::Instant::now();
            sorties.capture(
                &memory,
                MODULE,
                fixture.game_state,
                false,
                fixture.wings.iter().map(Ok),
                &fixture.witnesses,
            );
            best = best.min(begin.elapsed());
        }
        assert_eq!(sorties.outcome, Outcome::Rows { len: 0 });
        assert_eq!(sorties.missions.len, missions);
        let calls = memory.reads.get();
        println!(
            "{missions} missions, {countries} countries: {calls} guarded calls ({:.1}/mission), {best:?}",
            calls as f64 / missions as f64,
        );
        assert!(calls <= 30 * missions + 3_000, "{calls} guarded calls");
    }
}

#[test]
fn detection_override_failure_drops_partial_values_and_preserves_other_modifiers() {
    struct Observer {
        fail: bool,
    }
    impl RegionObserver for Observer {
        fn observed(
            &mut self,
            _: usize,
            _: usize,
            _: Option<map_air_country_region_modifier::Region>,
        ) {
        }
        fn detection_overrides(
            &mut self,
            emit: &mut dyn FnMut(u32, i64) -> Result<(), &'static str>,
        ) -> Result<(), &'static str> {
            emit(7, 0)?;
            if self.fail {
                Err("synthetic changed certificate")
            } else {
                emit(8, -123_456)
            }
        }
    }
    let mut capture = Sorties::with_capacities(&capacities());
    for fail in [true, false] {
        capture.modifiers.reset();
        capture
            .modifiers
            .push(AirSortieModifierRow {
                scope: AirSortieModifierRow::WEATHER,
                region: 1,
                raw: -7,
                ..Default::default()
            })
            .unwrap();
        capture.detection_overrides(&mut Observer { fail }).unwrap();
        let rows = &capture.modifiers.rows[..capture.modifiers.len];
        assert_eq!(rows[0].raw, -7);
        let status = rows
            .iter()
            .find(|row| row.scope == AirSortieModifierRow::DETECTION_OVERRIDE_STATUS)
            .unwrap();
        assert_eq!(status.raw, i64::from(!fail));
        assert_eq!(
            rows.iter()
                .filter(|row| row.scope == AirSortieModifierRow::DETECTION_OVERRIDE)
                .count(),
            if fail { 0 } else { 2 }
        );
    }
}
