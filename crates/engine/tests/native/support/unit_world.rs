use engine_deployment::carmy_upgrades_requests as upgrades;
use engine_native_air::{
    cair_base,
    cair_wing,
    cstrategic_air,
    cstrategic_air_manager as air_manager,
};
use engine_native_parts::{cair_wing_pool, csub_unit_definition as stats};
use engine_native::{
    carmy,
    ccountry,
    cequipment_variant::{self as variant, pool as equipment},
    cfleet,
    cgame_state,
    cprovince,
    creferenced_division_template as template,
    cship,
    ctask_force,
    cunit,
};
use engine_native_memory::memory::{CountedMemory, LiveMemory};

#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub countries: usize,
    pub divisions: usize,
    pub wings: usize,
    pub planes: usize,
    pub ships: usize,
}

impl Shape {
    pub const DESIGN: Self = Self {
        countries: 1_500,
        divisions: 50_000,
        wings: 50_000,
        planes: 500_000,
        ships: 20_000,
    };
    pub const STRESS: Self = Self {
        countries: 1_500,
        divisions: 100_000,
        wings: 100_000,
        planes: 1_000_000,
        ships: 50_000,
    };
}

pub struct World {
    pub bytes: Vec<u8>,
    pub base: usize,
    pub state: usize,
    pub shape: Shape,
    pub armies: Vec<usize>,
}

struct Heap {
    bytes: Vec<u8>,
    pointers: Vec<(usize, usize)>,
}

impl Heap {
    fn alloc(&mut self, len: usize) -> usize {
        let at = self.bytes.len().next_multiple_of(16);
        self.bytes.resize(at + len, 0);
        at
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn u64(&mut self, at: usize, value: u64) {
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn ptr(&mut self, at: usize, value: usize) {
        self.pointers.push((at, value));
    }
    fn array(&mut self, at: usize, values: &[usize]) {
        if values.is_empty() {
            return;
        }
        let data = self.alloc(values.len() * 8);
        self.ptr(at, data);
        self.u32(at + 8, values.len() as u32);
        self.u32(at + 12, values.len() as u32);
        for (i, &value) in values.iter().enumerate() {
            if value != 0 {
                self.ptr(data + i * 8, value);
            }
        }
    }
}

pub fn build(shape: Shape) -> World {
    build_with_activity(shape, false)
}

pub fn build_with_activity(shape: Shape, active: bool) -> World {
    assert!(shape.countries > 0 && shape.countries < u16::MAX as usize);
    assert!(shape.wings > 0 && shape.planes.is_multiple_of(shape.wings));
    let mut h = Heap {
        bytes: vec![0; 0x0350_0000],
        pointers: Vec::new(),
    };
    let state = h.alloc(0x1000);
    h.u64(carmy::COUNTER_ATTRITION_THRESHOLD_RVA.0, 500);
    let variants: Vec<_> = (1..=3)
        .map(|id| {
            let p = h.alloc(16);
            h.ptr(p, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0);
            h.u64(
                p + variant::TYPE,
                (id << 32) | variant::REFERENCE_TYPE as u64,
            );
            p
        })
        .collect();
    let provinces: Vec<_> = (0..shape.countries + 1)
        .map(|id| {
            let p = h.alloc(0x200);
            h.ptr(p, cprovince::VTABLE_RVA.0);
            h.u32(p + cprovince::ID, id as u32);
            p
        })
        .collect();
    h.array(state + cgame_state::PROVINCES, &provinces);
    let manager = h.alloc(0x200);
    h.ptr(manager, air_manager::VTABLE_RVA.0);
    h.ptr(state + cgame_state::STRATEGIC_AIR_MANAGER, manager);
    let mut countries = vec![0];
    let mut air_countries = vec![0];
    let mut bases = Vec::new();
    let mut armies = Vec::new();
    let mut wing_id = 0;
    let mut ship_id = 0;
    for slot in 1..=shape.countries {
        let country =
            h.alloc(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR.max(ccountry::DIVISION_TEMPLATES) + 0x100);
        h.ptr(country, ccountry::VTABLE_RVA.0);
        h.u32(country + ccountry::ID, slot as u32);
        countries.push(country);
        let tpl = h.alloc(0x500);
        h.ptr(tpl, template::VTABLE_RVA.0);
        h.u64(
            tpl + template::TYPE,
            ((slot as u64) << 32) | template::REFERENCE_TYPE as u64,
        );
        h.array(country + ccountry::DIVISION_TEMPLATES, &[tpl]);
        let mut roster = Vec::new();
        for _ in share(shape.divisions, shape.countries, slot) {
            let army = h.alloc(0x800);
            h.ptr(army, carmy::VTABLE_RVA.0);
            h.ptr(army + carmy::SECONDARY_BASE, carmy::SECONDARY_VTABLE_RVA.0);
            h.u64(army + cunit::HANDLE, ((armies.len() as u64 + 1) << 32) | 51);
            h.u32(army + cunit::ROSTER_OWNER, slot as u32);
            h.u32(army + cunit::LOGICAL_OWNER, slot as u32);
            h.ptr(army + cunit::LOCATION, provinces[slot]);
            if active && armies.len().is_multiple_of(4) {
                let destination = slot % shape.countries + 1;
                let route = h.alloc(8);
                h.u32(route, destination as u32);
                h.u32(route + 4, slot as u32);
                h.ptr(army + cunit::ORDINARY_ROUTE_DATA, route);
                h.u32(army + cunit::ORDINARY_ROUTE_CAPACITY, 2);
                h.u32(army + cunit::ORDINARY_ROUTE_COUNT, 2);
                h.ptr(army + carmy::MOVEMENT_TARGET, provinces[destination]);
                h.ptr(army + carmy::MOVEMENT_SOURCE, provinces[slot]);
            }
            h.ptr(army + carmy::STANDARD_TEMPLATE, tpl);
            h.ptr(army + carmy::EQUIPMENT_POOL, equipment::VTABLE_RVA.0);
            let entries = h.alloc(3 * 16);
            h.ptr(army + carmy::EQUIPMENT_POOL + equipment::ITEMS, entries);
            h.u32(army + carmy::EQUIPMENT_POOL + equipment::CAPACITY, 3);
            h.u32(army + carmy::EQUIPMENT_POOL + equipment::COUNT, 3);
            for (i, &variant) in variants.iter().enumerate() {
                h.ptr(entries + i * 16, variant);
                h.u64(entries + i * 16 + 8, 100_000);
            }
            let stat = h.alloc(0x300);
            h.ptr(stat, stats::VTABLE_RVA.0);
            h.u64(stat + stats::MAXIMUM_STRENGTH, 100_000);
            h.u64(stat + stats::MAXIMUM_ORGANISATION, 100_000);
            h.ptr(army + cunit::STAT_BLOCK, stat);
            h.ptr(
                army + carmy::ACCLIMATIZATION,
                engine_native_parts::acclimatization_data::VTABLE_RVA.0,
            );
            let viewers = h.alloc(3 * engine_visibility::unit_viewer_entry::STRIDE);
            h.ptr(army + cunit::VIEWER_DATA, viewers);
            h.u32(army + cunit::VIEWER_DATA + 8, 3);
            h.u32(army + cunit::VIEWER_COUNT, 3);
            for i in 0..3 {
                h.u32(
                    viewers + i * engine_visibility::unit_viewer_entry::STRIDE,
                    ((slot + i) % shape.countries + 1) as u32,
                );
                h.bytes[viewers
                    + i * engine_visibility::unit_viewer_entry::STRIDE
                    + engine_visibility::unit_viewer_entry::LEVEL] = 2;
            }
            let requests = h.alloc(0x100);
            h.ptr(army + carmy::REQUESTS, requests);
            let group = requests + upgrades::IN_ARMY_REQUESTS;
            h.ptr(group, upgrades::VTABLE_RVA.0);
            h.ptr(
                group + upgrades::PERSISTENT_SUBOBJECT,
                upgrades::PERSISTENT_VTABLE_RVA.0,
            );
            h.ptr(group + upgrades::ARMY, army);
            h.u64(army + carmy::STRENGTH, 100_000);
            h.u64(army + carmy::ORGANISATION, 100_000);
            roster.push(army);
            armies.push(army);
        }
        h.array(country + ccountry::DIVISIONS, &roster);
        let air = h.alloc(cstrategic_air::SIZE);
        h.ptr(air, cstrategic_air::VTABLE_RVA.0);
        h.u32(air + cstrategic_air::COUNTRY, slot as u32);
        air_countries.push(air);
        let pool = h.alloc(cair_wing_pool::SIZE);
        h.ptr(pool, cair_wing_pool::VTABLE_RVA.0);
        h.ptr(pool + cair_wing_pool::OWNER, air);
        h.array(air + cstrategic_air::POOLS, &[pool]);
        let mut wings = Vec::new();
        for _ in share(shape.wings, shape.countries, slot) {
            let wing = h.alloc(cair_wing::COUNTRY_OVERRIDE + 16);
            wing_id += 1;
            h.ptr(wing, engine_native_parts::cair_wing_layout::VTABLE_RVA.0);
            h.ptr(wing + engine_native_parts::cair_wing_layout::SECONDARY, engine_native_parts::cair_wing_layout::SECONDARY_VTABLE_RVA.0);
            h.u64(
                wing + engine_native_parts::cair_wing_layout::REFERENCE_TYPE,
                (wing_id << 32) | engine_native_parts::cair_wing_layout::TYPE as u64,
            );
            h.bytes[wing + cair_wing::NAME..wing + cair_wing::NAME + 4].copy_from_slice(b"Wing");
            h.u64(
                wing + cair_wing::NAME + engine_native_memory::msvc_string::LENGTH,
                4,
            );
            h.u64(
                wing + cair_wing::NAME + engine_native_memory::msvc_string::CAPACITY,
                engine_native_memory::msvc_string::INLINE_CAPACITY,
            );
            h.ptr(wing + engine_native_parts::cair_wing_layout::POOL, pool);
            h.ptr(wing + cair_wing::EQUIPMENT_POOL, equipment::VTABLE_RVA.0);
            let bank = h.alloc(16);
            h.ptr(bank, variants[0]);
            h.u64(bank + 8, (shape.planes / shape.wings) as u64 * 100_000);
            h.ptr(wing + cair_wing::EQUIPMENT_BANK, bank);
            h.u32(wing + cair_wing::EQUIPMENT_BANK_CAPACITY, 1);
            h.u32(wing + cair_wing::EQUIPMENT_BANK_COUNT, 1);
            h.u32(
                wing + cair_wing::PLANE_COUNT,
                (shape.planes / shape.wings) as u32,
            );
            wings.push(wing);
        }
        h.array(pool + cair_wing_pool::WINGS, &wings);
        let base = h.alloc(cair_base::SIZE);
        h.ptr(base, engine_native_parts::cair_base_layout::VTABLE_RVA.0);
        h.u64(
            base + engine_native_parts::cair_base_layout::HANDLE,
            ((slot as u64) << 32) | engine_native_parts::cair_base_layout::TYPE as u64,
        );
        h.u64(
            base + engine_native_parts::cair_base_layout::HOST_REFERENCE,
            (slot as u64) << 32 | cship::HANDLE_TYPE as u64,
        );
        h.u32(base + cair_base::CAPACITY, wings.len() as u32 * 10);
        let container = h.alloc(engine_native_air::ccountry_air_container::SIZE);
        h.ptr(container, engine_native_air::ccountry_air_container::VTABLE_RVA.0);
        h.u32(
            container + engine_native_air::ccountry_air_container::COUNTRY,
            slot as u32,
        );
        h.array(
            container + engine_native_air::ccountry_air_container::POOLS,
            &[pool],
        );
        let mut containers = vec![0; shape.countries + 1];
        containers[slot] = container;
        h.array(base + cair_base::COUNTRY_CONTAINERS, &containers);
        h.array(base + cair_base::REGISTERED_CONTAINERS, &[container]);
        bases.push(base);
        let fleet = h.alloc(0x140);
        h.ptr(fleet, cfleet::VTABLE_RVA.0);
        h.u64(
            fleet + cfleet::HANDLE,
            ((slot as u64) << 32) | cfleet::HANDLE_TYPE as u64,
        );
        h.ptr(fleet + cfleet::OWNER, country);
        h.array(country + ccountry::FLEETS, &[fleet]);
        let task = h.alloc(0x520);
        h.ptr(task, ctask_force::VTABLE_RVA.0);
        h.ptr(
            task + ctask_force::SECONDARY_BASE,
            ctask_force::SECONDARY_VTABLE_RVA.0,
        );
        h.u64(
            task + cunit::HANDLE,
            ((slot as u64) << 32) | ctask_force::DEFAULT_HANDLE_TYPE as u64,
        );
        h.u32(task + cunit::ROSTER_OWNER, slot as u32);
        h.ptr(task + ctask_force::FLEET, fleet);
        h.array(fleet + cfleet::TASK_FORCES, &[task]);
        let mut ships = Vec::new();
        for _ in share(shape.ships, shape.countries, slot) {
            let ship = h.alloc(cship::SIZE);
            ship_id += 1;
            h.ptr(ship, cship::VTABLE_RVA.0);
            h.u64(
                ship + cship::HANDLE,
                (ship_id << 32) | cship::HANDLE_TYPE as u64,
            );
            h.ptr(ship + cship::TASK_FORCE, task);
            ships.push(ship);
        }
        h.array(task + ctask_force::SHIPS, &ships);
    }
    if active {
        use engine_native::{cland_combat as combat, cland_combatant as side};
        let mut combats = Vec::new();
        for pair in armies[..(shape.divisions / 20) * 2].chunks_exact(2) {
            let battle = h.alloc(0x100);
            h.ptr(battle, combat::VTABLE_RVA.0);
            h.ptr(battle + combat::BODY, combat::BODY_VTABLE_RVA.0);
            h.u64(
                battle + combat::HANDLE,
                (((combats.len() + 1) as u64) << 32) | combat::TYPE_WORD as u64,
            );
            h.ptr(
                battle + combat::PROVINCE,
                provinces[combats.len() % shape.countries + 1],
            );
            for (index, unit) in pair.iter().enumerate() {
                let participant = h.alloc(side::IS_ATTACKER + 16);
                h.ptr(participant, side::VTABLE_RVA.0);
                h.ptr(participant + side::COMBAT, battle);
                h.bytes[participant + side::IS_ATTACKER] = u8::from(index == 0);
                h.array(participant + side::UNITS, &[*unit]);
                h.ptr(
                    battle
                        + if index == 0 {
                            combat::ATTACKER
                        } else {
                            combat::DEFENDER
                        },
                    participant,
                );
            }
            combats.push(battle);
        }
        h.array(state + cgame_state::COMBATS + 8, &combats);
    }
    h.array(state + cgame_state::COUNTRIES, &countries);
    let links = h.alloc(countries.len() * 4);
    h.ptr(state + cgame_state::COUNTRY_LINKS, links);
    h.u32(
        state + cgame_state::COUNTRY_LINKS + 8,
        countries.len() as u32,
    );
    h.u32(
        state + cgame_state::COUNTRY_LINKS + 12,
        countries.len() as u32,
    );
    for i in 0..countries.len() {
        h.u32(links + i * 4, i as u32);
    }
    h.array(manager + air_manager::COUNTRY_AIR, &air_countries);
    h.array(manager + air_manager::STATELESS_AIR_BASES, &bases);
    let base = h.bytes.as_ptr() as usize;
    for (at, target) in std::mem::take(&mut h.pointers) {
        h.u64(at, (base + target) as u64);
    }
    World {
        base,
        state: base + state,
        bytes: h.bytes,
        shape,
        armies: armies.into_iter().map(|p| base + p).collect(),
    }
}

fn share(total: usize, countries: usize, slot: usize) -> std::ops::Range<usize> {
    (slot - 1) * total / countries..slot * total / countries
}

impl World {
    pub fn live(&self) -> CountedMemory<LiveMemory> {
        CountedMemory::new(unsafe { LiveMemory::new() })
    }
}

pub fn time(mut work: impl FnMut()) -> (f64, f64) {
    if std::env::var_os("BATTLEPLAN_UNIT_VALIDATE").or_else(|| std::env::var_os("HOI4_UNIT_VALIDATE")).is_some() {
        work();
        return (0.0, 0.0);
    }
    for _ in 0..3 {
        work();
    }
    let mut samples = Vec::new();
    for _ in 0..21 {
        let start = std::time::Instant::now();
        work();
        samples.push(start.elapsed().as_secs_f64() * 1e3);
    }
    samples.sort_by(f64::total_cmp);
    (samples[10], samples[19])
}

pub fn shapes() -> Vec<Shape> {
    match std::env::var("BATTLEPLAN_UNIT_SHAPE").or_else(|_| std::env::var("HOI4_UNIT_SHAPE")).as_deref() {
        Ok("design") => vec![Shape::DESIGN],
        Ok("stress") => vec![Shape::STRESS],
        Err(_) => vec![Shape::DESIGN, Shape::STRESS],
        Ok(value) => panic!("unknown BATTLEPLAN_UNIT_SHAPE: {value}"),
    }
}
