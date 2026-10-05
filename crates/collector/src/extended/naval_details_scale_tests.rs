use super::*;
use engine::{Memory, cgame_state, memory::SliceMemory};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

const IMAGE: usize = 0x1400_0000;
const STATE: usize = 0x1000;
const REGISTRY: usize = 0x4000;
const BODIES: usize = 0x10_0000;
const STRIDE: usize = 0x2_0000;
const COUNTRY_SLOTS: usize = 0x5000;
const COUNTRY_LINKS: usize = 0x6000;
const COUNTRY_BASE: usize = 0x2_0000;
const COUNTRY_STRIDE: usize = 0x2000;
const COUNTRY_COUNT: usize = 10;
const TASK_BASE: usize = 0x7_0000;
const TASK_STRIDE: usize = 0x1000;
const WEATHER: usize = 0xc000;
const WEATHER_REGIONS: usize = 0xd000;
const TERRAIN: usize = 0xe000;
const REFERENCE_TABLE: usize = 0xf000;
const REFERENCE_BUCKETS: usize = 0xf100;
const CONVOY: usize = 0xf500;
const VARIANT: usize = 0xf800;
const CONVOY_HANDLE: u64 = (1_u64 << 32) | 1;
const TREE_CHILDREN: usize = 200;
const CHILD_ENTRIES: usize = 8;
const ROOT_ENTRIES: usize = 300;
const SHIPS_PER_COMBAT: usize = 30;
const SHIP_TABLE: usize = 0x8_0000;
const SHIP_BUCKETS: usize = 0x8_1000;
const SHIP_MASK: usize = 16383;
const WING: usize = 0xf_0000;
const WING_HANDLE: u64 = (1_u64 << 32) | 52;
// CStrategicRegionTemplate vtable for the shared main/branch fixture.
// 1415938B0 reads its custom-modifier vector.
const TEMPLATE_VTABLE_RVA: usize = 0x029c_5260;

#[derive(Clone, Copy)]
enum Hour {
    Quiet,
    Typical,
    AllChanged,
}
impl Hour {
    fn name(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Typical => "typical",
            Self::AllChanged => "all_changed",
        }
    }
}

struct Fixture(Vec<u8>);
impl Fixture {
    fn new(combats: usize) -> Self {
        let tree_children = BODIES + combats * STRIDE + 0x2000;
        let tree_nodes = tree_children + COUNTRY_COUNT * TREE_CHILDREN * 8;
        let child_entries = tree_nodes + COUNTRY_COUNT * TREE_CHILDREN * 0xc0;
        let root_entries = child_entries + COUNTRY_COUNT * TREE_CHILDREN * CHILD_ENTRIES * 16;
        let mut this = Self(vec![
            0;
            root_entries
                + COUNTRY_COUNT * ROOT_ENTRIES * 16
                + 0x1000
                - STATE
        ]);
        this.array(STATE + cgame_state::COMBATS + 8, REGISTRY, combats);
        this.array(
            STATE + cgame_state::COUNTRIES,
            COUNTRY_SLOTS,
            COUNTRY_COUNT + 1,
        );
        this.array(
            STATE + cgame_state::COUNTRY_LINKS,
            COUNTRY_LINKS,
            COUNTRY_COUNT + 1,
        );
        for slot in 1..=COUNTRY_COUNT {
            this.u32(COUNTRY_LINKS + slot * 4, slot as u32);
        }
        this.u64(STATE + cgame_state::WEATHER_MANAGER, WEATHER as u64);
        this.u64(
            WEATHER,
            (IMAGE + engine::cweather_manager::VTABLE_RVA) as u64,
        );
        this.array(WEATHER + 0x40, WEATHER_REGIONS, 1);
        this.u64(
            WEATHER_REGIONS,
            (IMAGE + engine::sweather_per_region::VTABLE_RVA) as u64,
        );
        this.u32(WEATHER_REGIONS + engine::sweather_per_region::ID, 0);
        this.u64(TERRAIN, (IMAGE + engine::cterrain_type::VTABLE_RVA) as u64);
        this.put(TERRAIN + 0x10, &[1]);
        this.inline(TERRAIN + 0xb8 + 0x48, b"terrain");
        this.inline(WEATHER_REGIONS + 0x58 + 0x48, b"weather");
        this.u64(
            REFERENCE_TABLE + engine::creference_registry::ENTRIES,
            REFERENCE_BUCKETS as u64,
        );
        this.u32(REFERENCE_TABLE + engine::creference_registry::COUNT, 1);
        this.u32(REFERENCE_TABLE + engine::creference_registry::MASK, 7);
        this.put(
            REFERENCE_TABLE + engine::creference_registry::OVERFLOW_DISTANCE,
            &[5],
        );
        let hash = CONVOY_HANDLE
            .rotate_left(32)
            .wrapping_mul(engine::creference_registry::HASH_MULTIPLIER);
        let slot = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        let bucket = REFERENCE_BUCKETS + slot * engine::reference_bucket::STRIDE;
        this.put(bucket + engine::reference_bucket::DISTANCE, &[1]);
        this.u64(bucket + engine::reference_bucket::TYPE, CONVOY_HANDLE);
        this.u64(bucket + engine::reference_bucket::OBJECT, CONVOY as u64);
        this.put(
            REFERENCE_BUCKETS
                + 13 * engine::reference_bucket::STRIDE
                + engine::reference_bucket::DISTANCE,
            &[255],
        );
        this.u64(CONVOY, (IMAGE + engine::cconvoy_ship::VTABLE_RVA) as u64);
        this.u64(CONVOY + 8, CONVOY_HANDLE);
        this.u64(CONVOY + 0x18, VARIANT as u64);
        this.u64(CONVOY + 0x20, 50_000);
        this.u64(CONVOY + 0x28, 70_000);
        this.u64(
            VARIANT,
            (IMAGE + engine::cequipment_variant::VTABLE_RVA) as u64,
        );
        this.u64(VARIANT + 8, (1_u64 << 32) | 42);
        this.u64(VARIANT + 0x208, 24_000);
        this.u64(VARIANT + 0x2e0, 100_000);
        this.u64(VARIANT + 0x2e8, 100_000);
        this.u64(
            SHIP_TABLE + engine::creference_registry::ENTRIES,
            SHIP_BUCKETS as u64,
        );
        this.u32(
            SHIP_TABLE + engine::creference_registry::MASK,
            SHIP_MASK as u32,
        );
        this.put(
            SHIP_TABLE + engine::creference_registry::OVERFLOW_DISTANCE,
            &[16],
        );
        this.put(
            SHIP_BUCKETS
                + (SHIP_MASK + 1 + 16) * engine::reference_bucket::STRIDE
                + engine::reference_bucket::DISTANCE,
            &[255],
        );
        this.u64(WING, (IMAGE + engine::cair_wing::VTABLE_RVA) as u64);
        this.u64(
            WING + engine::cair_wing::SECONDARY,
            (IMAGE + engine::cair_wing::SECONDARY_VTABLE_RVA) as u64,
        );
        this.u64(WING + engine::cair_wing::REFERENCE_TYPE, WING_HANDLE);
        this.u64(WING + engine::cair_wing::POOL, (WING + 0x100) as u64);
        this.u64(
            WING + 0x100,
            (IMAGE + engine::cair_wing_pool::VTABLE_RVA) as u64,
        );
        this.u64(
            WING + 0x100 + engine::cair_wing_pool::DEFINITION,
            (WING + 0x200) as u64,
        );
        this.u64(
            WING + 0x200,
            (IMAGE + engine::csub_unit_definition::VTABLE_RVA) as u64,
        );
        this.u32(
            WING + 0x200 + engine::csub_unit_definition::DATABASE_INDEX,
            1,
        );
        let mut occupied = vec![false; SHIP_MASK + 1];
        this.register(
            SHIP_BUCKETS,
            WING_HANDLE,
            WING + engine::cair_wing::SECONDARY,
            &mut occupied,
        );
        let mut next_ship_id = 1_u32;
        for slot in 0..COUNTRY_COUNT {
            let country = COUNTRY_BASE + slot * COUNTRY_STRIDE;
            let task = TASK_BASE + slot * TASK_STRIDE;
            this.u64(COUNTRY_SLOTS + (slot + 1) * 8, country as u64);
            this.u64(country, (IMAGE + engine::ccountry::VTABLE_RVA) as u64);
            this.u32(country + engine::ccountry::ID, slot as u32 + 1);
            let root = country + engine::ccountry::MODIFIER_ACCUMULATOR;
            this.inline(root + 0x48, b"country");
            let root_data = root_entries + slot * ROOT_ENTRIES * 16;
            this.array(root, root_data, ROOT_ENTRIES);
            for entry in 0..ROOT_ENTRIES {
                this.u32(root_data + entry * 16, entry as u32 + 1);
                this.u64(root_data + entry * 16 + 8, 1000);
            }
            let pointers = tree_children + slot * TREE_CHILDREN * 8;
            this.array(root + 0x18, pointers, TREE_CHILDREN);
            for child in 0..TREE_CHILDREN {
                let node = tree_nodes + (slot * TREE_CHILDREN + child) * 0xc0;
                this.u64(pointers + child * 8, (node - 0x10) as u64);
                this.inline(node + 0x48, b"doctrine");
                let entry = child_entries + (slot * TREE_CHILDREN + child) * CHILD_ENTRIES * 16;
                this.array(node, entry, CHILD_ENTRIES);
                for (index, key) in [0x24, 0x59, 0x5a, 0x5b, 0x5c, 0x5d, 0x1b2, 0x1b3]
                    .into_iter()
                    .enumerate()
                {
                    this.u32(entry + index * 16, key);
                    this.u64(entry + index * 16 + 8, 1000);
                }
            }
            this.u64(task, (IMAGE + engine::ctask_force::VTABLE_RVA) as u64);
            this.u64(
                task + engine::ctask_force::SECONDARY_BASE,
                (IMAGE + engine::ctask_force::SECONDARY_VTABLE_RVA) as u64,
            );
            this.u64(task + engine::cunit::HANDLE, ((slot as u64 + 1) << 32) | 61);
            this.u32(task + engine::cunit::ROSTER_OWNER, slot as u32 + 1);
            let head = task + 0x900;
            this.u64(task + 0x640, head as u64);
            for offset in [0, 8, 0x10] {
                this.u64(head + offset, head as u64);
            }
            this.put(head + 0x19, &[1]);
        }
        for index in 0..combats {
            let body = BODIES + index * STRIDE;
            let left = body + 0x200;
            let right = body + 0x600;
            let province = body + 0xb00;
            let region = body + 0xc00;
            let terrain_link = body + 0xd00;
            this.u64(REGISTRY + index * 8, body as u64);
            this.u64(body, (IMAGE + engine::cnaval_combat::VTABLE_RVA) as u64);
            this.u64(
                body + engine::cnaval_combat::HANDLE,
                ((index as u64 + 1) << 32) | engine::cnaval_combat::OBSERVED_REFERENCE_TYPE as u64,
            );
            this.u64(body + engine::cnaval_combat::ATTACKER, left as u64);
            this.u64(body + engine::cnaval_combat::DEFENDER, right as u64);
            this.u64(body + engine::cnaval_combat::PROVINCE, province as u64);
            this.u64(province, (IMAGE + engine::cprovince::VTABLE_RVA) as u64);
            this.u32(province + engine::cprovince::ID, index as u32 + 1);
            this.u64(province + 0xc8, region as u64);
            this.u64(
                region,
                (IMAGE + engine::cstrategic_region::VTABLE_RVA) as u64,
            );
            this.u64(region + 0x30, terrain_link as u64);
            this.u32(region + 0x58, 0);
            this.u64(terrain_link, (IMAGE + TEMPLATE_VTABLE_RVA) as u64);
            this.u64(terrain_link + 0x130, TERRAIN as u64);
            // Side damage matrix, baseline and one +0x288 map node per side
            // are read by the actual CNavalCombatant details path.
            for (number, side) in [left, right].into_iter().enumerate() {
                this.u64(side, (IMAGE + engine::cnaval_combatant::VTABLE_RVA) as u64);
                this.u64(side + engine::cnaval_combatant::COMBAT, body as u64);
                let country_node = side + 0x390;
                this.u64(side + 0x38, country_node as u64);
                let country_id = (index * 2 + number) % COUNTRY_COUNT + 1;
                let task = TASK_BASE + (country_id - 1) * TASK_STRIDE;
                this.u32(side + 0x48, 1);
                this.u32(country_node, country_id as u32);
                this.put(side + 0xb8, &[1]);
                let damage_sentinel = side + 0x3b0;
                let damage_node = side + 0x3d0;
                this.u64(side + 0x290, damage_sentinel as u64);
                this.u64(side + 0x298, 1);
                this.u64(damage_sentinel, damage_node as u64);
                this.u64(damage_node, damage_sentinel as u64);
                this.u32(damage_node + 0x10, 17);
                this.u64(damage_node + 0x18, 3_000);
                this.u64(side + 0x158, 100_000);
                this.u64(side + 0x168, 500);
                let group = body + 0x1000 + number * 0x400;
                let group_slot = body + 0x1f00 + number * 0x20;
                let member_slot = body + 0x2000 + number * 0x100;
                let plane_slot = body + 0xb000 + number * 0x40;
                let unit_slot = body + 0x2800 + number * 0x20;
                this.array(side + engine::cnaval_combatant::UNITS, unit_slot, 1);
                this.u64(unit_slot, task as u64);
                this.array(side + engine::cnaval_combatant::GROUPS, group_slot, 1);
                this.u64(group_slot, group as u64);
                this.u64(group, (IMAGE + engine::cfex_group::VTABLE_RVA) as u64);
                this.u64(group + engine::cfex_group::SIDE, side as u64);
                this.u64(
                    group + engine::cfex_group::OPPONENT_GROUP,
                    (body + 0x1000 + (1 - number) * 0x400) as u64,
                );
                this.array(
                    group + engine::cfex_group::MEMBERS,
                    member_slot,
                    SHIPS_PER_COMBAT / 2,
                );
                for local in 0..SHIPS_PER_COMBAT / 2 {
                    let ordinal = number * SHIPS_PER_COMBAT / 2 + local;
                    let member = body + 0x3000 + ordinal * 0x300;
                    let ship = body + 0xc000 + ordinal * 0xa00;
                    let handle = loop {
                        let handle = (u64::from(next_ship_id) << 32) | 51;
                        next_ship_id += 1;
                        if !occupied[Self::bucket(handle)] {
                            break handle;
                        }
                    };
                    this.register(SHIP_BUCKETS, handle, ship, &mut occupied);
                    this.u64(member_slot + local * 8, member as u64);
                    this.u64(member, (IMAGE + engine::cfex_member::VTABLE_RVA) as u64);
                    this.u64(member + engine::cfex_member::GROUP, group as u64);
                    this.u64(member + engine::cfex_member::SHIP_HANDLE, handle);
                    this.u32(member + engine::cfex_member::OWNER, country_id as u32);
                    this.u32(member + engine::cfex_member::BATTLE_SLOT, ordinal as u32);
                    this.u32(
                        member + engine::cfex_member::STATE,
                        if (number == 0 && (2..5).contains(&local))
                            || (number == 1 && (2..4).contains(&local))
                        {
                            8
                        } else {
                            2
                        },
                    );
                    for offset in [0x48, 0x68, 0x88, 0xa8] {
                        this.inline(member + offset, b"");
                    }
                    this.u64(ship, (IMAGE + engine::cship::VTABLE_RVA) as u64);
                    this.u64(ship + engine::cship::HANDLE, handle);
                    this.u64(ship + engine::cship::TASK_FORCE, task as u64);
                    this.u64(ship + 0x8f8, (task + 0xb00) as u64);
                    this.u64(
                        ship + engine::cship::EQUIPMENT_POOL,
                        (IMAGE + engine::cequipment_variant_pool::VTABLE_RVA) as u64,
                    );
                    this.u64(ship + engine::cship::STRENGTH, 100_000);
                    this.u64(ship + engine::cship::ORGANISATION, 100_000);
                    this.u64(ship + 0x300, 100_000);
                    this.u64(ship + 0x308, 100_000);
                    this.u64(ship + 0x228, 24_000);
                    this.u32(
                        ship + engine::cship::CATEGORY,
                        if local < 2 { 10 } else { 3 },
                    );
                }
                this.array(group + 0x58, plane_slot, 2);
                for carrier in 0..2 {
                    let plane = body + 0xa000 + (number * 2 + carrier) * 0x300;
                    let member = body + 0x3000 + (number * SHIPS_PER_COMBAT / 2 + carrier) * 0x300;
                    let wing_slot = body + 0xb100 + (number * 2 + carrier) * 0x20;
                    this.u64(plane_slot + carrier * 8, plane as u64);
                    this.u64(plane, (IMAGE + engine::cfex_air::VTABLE_RVA) as u64);
                    this.u64(plane + 8, group as u64);
                    this.u64(plane + 0x10, member as u64);
                    this.u32(
                        plane + 0x18,
                        (number * SHIPS_PER_COMBAT / 2 + carrier) as u32,
                    );
                    this.u64(plane + 0xf0, VARIANT as u64);
                    this.u32(plane + 0x64, country_id as u32);
                    this.array(plane + 0x40, wing_slot, 1);
                    this.u64(wing_slot, WING_HANDLE);
                }
            }
        }
        this.u32(
            SHIP_TABLE + engine::creference_registry::COUNT,
            (combats * SHIPS_PER_COMBAT + 1) as u32,
        );
        this
    }
    /// Mutations are outside the measured callback. These write values in
    /// place: neither counts nor pointers certify that the inputs are quiet.
    fn advance(&mut self, combats: usize, hour: Hour, pass: u64) {
        let countries = match hour {
            Hour::Quiet => return,
            Hour::Typical => 2,
            Hour::AllChanged => COUNTRY_COUNT,
        };
        if combats == 0 {
            return;
        }
        for slot in 0..countries {
            let root =
                COUNTRY_BASE + slot * COUNTRY_STRIDE + engine::ccountry::MODIFIER_ACCUMULATOR;
            let data = self.pointer(root);
            let entries = if matches!(hour, Hour::AllChanged) {
                ROOT_ENTRIES
            } else {
                1
            };
            for entry in 0..entries {
                self.u64(data + entry * 16 + 8, 1000 + pass);
            }
            if matches!(hour, Hour::AllChanged) {
                let children = self.pointer(root + 0x18);
                for child in 0..TREE_CHILDREN {
                    let accumulator = self.pointer(children + child * 8) + 0x10;
                    let entries = self.pointer(accumulator);
                    for entry in 0..CHILD_ENTRIES {
                        self.u64(entries + entry * 16 + 8, 1000 + pass);
                    }
                }
                let task = TASK_BASE + slot * TASK_STRIDE;
                self.u64(task + 0x4f8, 1000 + pass);
                self.u64(task + 0x500, 2000 + pass);
            }
        }
        for index in 0..combats {
            let body = BODIES + index * STRIDE;
            for offset in [0x200, 0x600] {
                self.u64(body + offset + 0x168, 500 + pass);
            }
            if matches!(hour, Hour::AllChanged) {
                for ordinal in 0..SHIPS_PER_COMBAT {
                    let ship = body + 0xc000 + ordinal * 0xa00;
                    for offset in [0x1b8, 0x1e0, 0x1e8, 0x1f8, 0x208, 0x210, 0x270, 0x318] {
                        self.u64(ship + offset, 1000 + pass);
                    }
                    self.u64(ship + 0x228, 24_000 + pass);
                }
                for plane in 0..4 {
                    self.u32(body + 0xa000 + plane * 0x300 + 0x5c, 100 + pass as u32);
                }
            }
        }
    }
    fn pointer(&self, at: usize) -> usize {
        u64::from_le_bytes(self.0[at - STATE..at - STATE + 8].try_into().unwrap()) as usize
    }
    fn bucket(handle: u64) -> usize {
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(engine::creference_registry::HASH_MULTIPLIER);
        (((hash >> 32) as u32 ^ hash as u32) as usize) & SHIP_MASK
    }
    fn register(&mut self, buckets: usize, handle: u64, pointer: usize, occupied: &mut [bool]) {
        let slot = Self::bucket(handle);
        assert!(!occupied[slot], "synthetic reference bucket collision");
        occupied[slot] = true;
        let at = buckets + slot * engine::reference_bucket::STRIDE;
        self.put(at + engine::reference_bucket::DISTANCE, &[1]);
        self.u64(at + engine::reference_bucket::TYPE, handle);
        self.u64(at + engine::reference_bucket::OBJECT, pointer as u64);
    }
    fn inline(&mut self, at: usize, value: &[u8]) {
        self.put(at, value);
        self.u64(at + 16, value.len() as u64);
        self.u64(at + 24, 15);
    }
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0[at - STATE..at - STATE + bytes.len()].copy_from_slice(bytes);
    }
    fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: usize) {
        self.u64(at, data as u64);
        self.u32(at + 8, count as u32);
        self.u32(at + 12, count as u32);
    }
}
impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address == IMAGE + engine::creference_registry::LOW_ROOTS_RVA + 8
            && destination.len() == 8
        {
            destination.copy_from_slice(&(REFERENCE_TABLE as u64).to_le_bytes());
            return true;
        }
        if matches!(address,
            x if x == IMAGE + engine::creference_registry::LOW_ROOTS_RVA + 51 * 8
                || x == IMAGE + engine::creference_registry::LOW_ROOTS_RVA + 52 * 8)
            && destination.len() == 8
        {
            destination.copy_from_slice(&(SHIP_TABLE as u64).to_le_bytes());
            return true;
        }
        if address == IMAGE + engine::modifier_definition::TABLE_RVA && destination.len() >= 8 {
            destination.fill(0);
            destination[..8].copy_from_slice(&0x4000_0000_u64.to_le_bytes());
            if destination.len() >= 16 {
                destination[8..12].copy_from_slice(&1024_u32.to_le_bytes());
                destination[12..16].copy_from_slice(&1024_u32.to_le_bytes());
            }
            return true;
        }
        if matches!(address,
            x if x == IMAGE + engine::modifier_definition::ELIGIBILITY_CALLBACK_RVA
                || x == IMAGE + engine::modifier_definition::TRADE_FEATURE_FLAGS_RVA)
        {
            destination.fill(0);
            return true;
        }
        if (0x4000_0000..0x4004_0000).contains(&address) {
            destination.fill(0);
            return true;
        }
        if (IMAGE..IMAGE + 0x0400_0000).contains(&address) {
            destination.fill(0);
            return true;
        }
        let ok = SliceMemory {
            base: STATE,
            bytes: &self.0,
        }
        .copy(address, destination);
        if !ok {
            eprintln!(
                "synthetic native read failed at {address:#x} len {}",
                destination.len()
            );
        }
        ok
    }
}

fn percentiles(mut values: Vec<Duration>) -> (Duration, Duration, Duration) {
    values.sort_unstable();
    let p50 = values[values.len() / 2];
    let p95 = values[values.len() * 95 / 100];
    (p50, p95, *values.last().unwrap())
}

#[test]
fn naval_empty_hour_keeps_all_tables_complete_without_tree_storage() {
    let memory = Fixture::new(0);
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.capture(&memory, IMAGE, STATE);
    assert_eq!(captured.outcomes(), [Outcome::Rows { len: 0 }; N]);
    assert_eq!(captured.breakdown_requests.len, 0);
    assert_eq!(captured.breakdown_nodes.len, 0);
    assert_eq!(
        captured.breakdown_requests.allocated_bytes()
            + captured.breakdown_roots.allocated_bytes()
            + captured.breakdown_nodes.allocated_bytes()
            + captured.breakdown_text.allocated_bytes()
            + captured.breakdown_depth.allocated_bytes(),
        0
    );
    assert!(
        captured
            .tables()
            .iter()
            .all(|view| matches!(view, TableView::Complete(_)))
    );
}

#[test]
fn naval_shared_country_roots_walk_once_across_combat_side_and_ship_uses() {
    struct Counted {
        memory: Fixture,
        visits: std::cell::Cell<[u32; COUNTRY_COUNT]>,
    }
    impl Memory for Counted {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for slot in 0..COUNTRY_COUNT {
                let root =
                    COUNTRY_BASE + slot * COUNTRY_STRIDE + engine::ccountry::MODIFIER_ACCUMULATOR;
                if address == root + 0x18 && destination.len() == 16 {
                    let mut visits = self.visits.get();
                    visits[slot] += 1;
                    self.visits.set(visits);
                }
            }
            self.memory.copy(address, destination)
        }
    }
    let memory = Counted {
        memory: Fixture::new(20),
        visits: std::cell::Cell::new([0; COUNTRY_COUNT]),
    };
    let mut captured = Captured::with_capacities(&Capacities {
        breakdown_requests: 20 * 128,
        breakdown_roots: 20 * 80 + 2100,
        breakdown_nodes: 20 * 80 + 2100,
        breakdown_text: 32_000,
        breakdown_depth: 128,
        ..Capacities::default()
    });
    captured.capture(&memory, IMAGE, STATE);
    assert!(
        matches!(captured.breakdown_outcome, Outcome::Rows { .. }),
        "{:?}",
        captured.breakdown_outcome
    );
    assert_eq!(memory.visits.get(), [1; COUNTRY_COUNT]);
    let rows = captured.project_breakdowns().unwrap();
    let country = COUNTRY_BASE + engine::ccountry::MODIFIER_ACCUMULATOR;
    let shared = rows
        .iter()
        .find(|row| row.accumulator == country as u64 && row.key == 0x1b2)
        .unwrap();
    assert_eq!(shared.uses.len(), 4);
    assert_eq!(shared.nodes.len(), TREE_CHILDREN + 1);
}

#[test]
fn naval_tree_arena_shortage_withholds_only_trees_and_grows_after_capture() {
    let memory = Fixture::new(1);
    let mut capacities = Capacities::default();
    let mut captured = Captured::with_capacities(&capacities);
    captured.capture(&memory, IMAGE, STATE);
    assert_eq!(captured.details.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        captured.ships.outcome,
        Outcome::Rows {
            len: SHIPS_PER_COMBAT
        }
    );
    assert_eq!(
        captured.breakdown_outcome,
        Outcome::Unavailable {
            reason: engine::naval_ship_modifier_breakdown::REQUEST_CAPACITY,
        }
    );
    assert!(matches!(
        captured.breakdown_table(),
        TableView::Unavailable { .. }
    ));
    assert_eq!(captured.breakdown_requests.first.len(), 0);
    assert_eq!(captured.breakdown_requests.required, 1);
    captured.grow(&mut capacities);
    assert!(capacities.breakdown_requests >= 1);
    assert_eq!(
        captured.breakdown_requests.first.len(),
        capacities.breakdown_requests
    );
    for _ in 0..64 {
        captured.reset();
        captured.capture(&memory, IMAGE, STATE);
        assert_eq!(captured.details.outcome, Outcome::Rows { len: 1 });
        if matches!(captured.breakdown_outcome, Outcome::Rows { .. }) {
            break;
        }
        captured.grow(&mut capacities);
    }
    assert!(
        matches!(captured.breakdown_table(), TableView::Complete(_)),
        "tree arenas did not grow to the live fixture demand: {:?}",
        captured.breakdown_outcome
    );
}

#[test]
fn naval_bench_in_place_changes_keep_headers_and_reach_capture() {
    let mut memory = Fixture::new(1);
    let root = COUNTRY_BASE + engine::ccountry::MODIFIER_ACCUMULATOR;
    let root_header = memory.0[root - STATE..root - STATE + 16].to_vec();
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.capture(&memory, IMAGE, STATE);
    let terms = captured.weapon_terms.first[..captured.weapon_terms.len].to_vec();
    let speed: Vec<_> = captured.ships.rows[..SHIPS_PER_COMBAT]
        .iter()
        .filter(|row| row.member.state == 8)
        .map(|row| row.escape_speed_base)
        .collect();
    assert_eq!(speed.len(), 5);
    memory.advance(1, Hour::Quiet, 7);
    captured.reset();
    captured.capture(&memory, IMAGE, STATE);
    assert_eq!(
        &captured.weapon_terms.first[..captured.weapon_terms.len],
        terms
    );
    memory.advance(1, Hour::AllChanged, 7);
    assert_eq!(&memory.0[root - STATE..root - STATE + 16], root_header);
    captured.reset();
    captured.capture(&memory, IMAGE, STATE);
    assert_eq!(captured.details.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        captured.ships.outcome,
        Outcome::Rows {
            len: SHIPS_PER_COMBAT
        }
    );
    assert_ne!(
        &captured.weapon_terms.first[..captured.weapon_terms.len],
        terms
    );
    for (row, before) in captured.ships.rows[..SHIPS_PER_COMBAT]
        .iter()
        .filter(|row| row.member.state == 8)
        .zip(speed)
    {
        assert_eq!(row.escape_speed_base, before + 7);
    }
    assert!(
        captured.planes.first[..4]
            .iter()
            .all(|plane| plane.current_planes == 107)
    );
}

/// Pack reset and real collector capture callback, then separate spool projection.
/// Synthetic population: ten countries with 200 children each, eight sorted
/// entries per child and 300 root entries; 30 live ships per combat, including
/// four carrier-linked wing planes and five state-8 ships. Each side has one
/// damage-map node. These are fixture dimensions, not a measured live census.
/// Quiet changes nothing; typical changes two roots and one damage cell per
/// side; all-changed also changes every country child value, task-force fuel,
/// ship stat and plane count. All three run the same production capture path.
#[test]
#[ignore]
fn naval_window_operand_scale_bench() {
    for combats in [0, 20, 40] {
        for hour in [Hour::Quiet, Hour::Typical, Hour::AllChanged] {
            let mut memory = Fixture::new(combats);
            let mut captured = if combats == 0 {
                Captured::vacant()
            } else {
                Captured::with_capacities(&Capacities {
                    details: combats,
                    ships: combats * SHIPS_PER_COMBAT,
                    planes: combats * 4,
                    side_units: combats * 2,
                    weapon_sources: combats * SHIPS_PER_COMBAT * 12,
                    weapon_terms: 16_384,
                    breakdown_requests: combats * 128,
                    breakdown_roots: combats * 80 + 2100,
                    breakdown_nodes: combats * 80 + 2100,
                    breakdown_text: 32_000,
                    breakdown_depth: 128,
                    ..Capacities::default()
                })
            };
            let mut block = Vec::with_capacity(256);
            let mut projection = Vec::with_capacity(256);
            let mut allocated = None;
            for pass in 0..320 {
                memory.advance(combats, hour, pass);
                let began = Instant::now();
                captured.reset();
                black_box(captured.capture(&memory, IMAGE, STATE));
                let elapsed = began.elapsed();
                assert_eq!(captured.details.outcome, Outcome::Rows { len: combats });
                assert_eq!(
                    captured.ships.outcome,
                    Outcome::Rows {
                        len: combats * SHIPS_PER_COMBAT
                    }
                );
                assert_eq!(captured.planes.len, combats * 4);
                assert!(
                    matches!(captured.breakdown_outcome, Outcome::Rows { len }
                    if if combats == 0 { len == 0 } else { len > COUNTRY_COUNT }),
                    "tree capture refused the fixture: {:?}",
                    captured.outcomes()
                );
                if pass == 64 {
                    allocated = Some(captured.allocated_bytes());
                }
                if pass >= 64 {
                    assert_eq!(Some(captured.allocated_bytes()), allocated);
                }
                if pass == 0 {
                    assert!(matches!(captured.breakdown_table(), TableView::Complete(_)));
                }
                let began_projection = Instant::now();
                black_box(captured.tables());
                let projected = began_projection.elapsed();
                if pass >= 64 {
                    block.push(elapsed);
                    projection.push(projected);
                }
            }
            if combats == 0 {
                assert_eq!(captured.allocated_bytes(), 0);
            }
            let (b50, b95, bmax) = percentiles(block);
            let (p50, p95, pmax) = percentiles(projection);
            let ms = |d: Duration| d.as_secs_f64() * 1000.0;
            eprintln!(
                "NAVAL_BENCH combats={combats} hour={} block={:.6}/{:.6}/{:.6} projection={:.6}/{:.6}/{:.6} arena_mib={:.3}",
                hour.name(),
                ms(b50),
                ms(b95),
                ms(bmax),
                ms(p50),
                ms(p95),
                ms(pmax),
                captured.allocated_bytes() as f64 / 1048576.0
            );
        }
    }
}
