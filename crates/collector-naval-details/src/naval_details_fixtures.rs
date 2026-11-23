
use engine_native::{Memory, cgame_state, memory::SliceMemory};

pub const IMAGE: usize = 0x1400_0000;
pub const STATE: usize = 0x1000;
pub const REGISTRY: usize = 0x4000;
pub const BODIES: usize = 0x10_0000;
pub const STRIDE: usize = 0x2_0000;
pub const COUNTRY_SLOTS: usize = 0x5000;
pub const COUNTRY_LINKS: usize = 0x6000;
pub const COUNTRY_BASE: usize = 0x2_0000;
pub const COUNTRY_STRIDE: usize = 0x2000;
pub const COUNTRY_COUNT: usize = 10;
pub const TASK_BASE: usize = 0x7_0000;
pub const TASK_STRIDE: usize = 0x1000;
pub const WEATHER: usize = 0xc000;
pub const WEATHER_REGIONS: usize = 0xd000;
pub const TERRAIN: usize = 0xe000;
pub const REFERENCE_TABLE: usize = 0xf000;
pub const REFERENCE_BUCKETS: usize = 0xf100;
pub const CONVOY: usize = 0xf500;
pub const VARIANT: usize = 0xf800;
pub const CONVOY_HANDLE: u64 = (1_u64 << 32) | 1;
pub const TREE_CHILDREN: usize = 200;
pub const CHILD_ENTRIES: usize = 8;
pub const ROOT_ENTRIES: usize = 300;
pub const SHIPS_PER_COMBAT: usize = 30;
const LEXER: usize = 0xe_8000;
const LEXER_MARK: u32 = 0x1e8e;
const DOCTRINE_ENTRIES: usize = 4;
pub const LIVE_ENTRIES: [usize; 3] = [30, 8, 10];
pub const SHIP_TABLE: usize = 0x8_0000;
pub const SHIP_BUCKETS: usize = 0x8_1000;
pub const SHIP_MASK: usize = 16383;
pub const WING: usize = 0xf_0000;
pub const WING_HANDLE: u64 = (1_u64 << 32) | 52;
pub const TEMPLATE_VTABLE_RVA: usize = 0x02a5_5348;

#[derive(Clone, Copy)]
pub enum Hour {
    Quiet,
    Typical,
    AllChanged,
}
impl Hour {
    pub fn name(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Typical => "typical",
            Self::AllChanged => "all_changed",
        }
    }
}

pub struct Fixture(pub Vec<u8>);
impl Fixture {
    pub fn new(combats: usize) -> Self {
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
            (IMAGE + engine_native::cweather_manager::VTABLE_RVA.0) as u64,
        );
        this.array(WEATHER + 0x40, WEATHER_REGIONS, 1);
        this.u64(
            WEATHER_REGIONS,
            (IMAGE + engine_native::sweather_per_region::VTABLE_RVA.0) as u64,
        );
        this.u32(WEATHER_REGIONS + engine_native::sweather_per_region::ID, 0);
        this.u64(
            TERRAIN,
            (IMAGE + engine_native::cterrain_type::VTABLE_RVA.0) as u64,
        );
        this.put(TERRAIN + 0x10, &[1]);
        this.inline(TERRAIN + 0xb8 + 0x48, b"terrain");
        this.inline(WEATHER_REGIONS + 0x58 + 0x48, b"weather");
        this.u64(
            REFERENCE_TABLE + engine_native::creference_registry::ENTRIES,
            REFERENCE_BUCKETS as u64,
        );
        this.u32(
            REFERENCE_TABLE + engine_native::creference_registry::COUNT,
            1,
        );
        this.u32(
            REFERENCE_TABLE + engine_native::creference_registry::MASK,
            7,
        );
        this.put(
            REFERENCE_TABLE + engine_native::creference_registry::OVERFLOW_DISTANCE,
            &[5],
        );
        let hash = CONVOY_HANDLE
            .rotate_left(32)
            .wrapping_mul(engine_native::creference_registry::HASH_MULTIPLIER);
        let slot = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        let bucket = REFERENCE_BUCKETS + slot * engine_native::reference_bucket::STRIDE;
        this.put(bucket + engine_native::reference_bucket::DISTANCE, &[1]);
        this.u64(
            bucket + engine_native::reference_bucket::TYPE,
            CONVOY_HANDLE,
        );
        this.u64(
            bucket + engine_native::reference_bucket::OBJECT,
            CONVOY as u64,
        );
        this.put(
            REFERENCE_BUCKETS
                + 13 * engine_native::reference_bucket::STRIDE
                + engine_native::reference_bucket::DISTANCE,
            &[255],
        );
        this.u64(
            CONVOY,
            (IMAGE + engine_native::cconvoy_ship::VTABLE_RVA.0) as u64,
        );
        this.u64(CONVOY + 8, CONVOY_HANDLE);
        this.u64(CONVOY + 0x18, VARIANT as u64);
        this.u64(CONVOY + 0x20, 50_000);
        this.u64(CONVOY + 0x28, 70_000);
        this.u64(
            VARIANT,
            (IMAGE + engine_native::cequipment_variant::VTABLE_RVA.0) as u64,
        );
        this.u64(VARIANT + 8, (1_u64 << 32) | 42);
        this.u64(VARIANT + 0x208, 24_000);
        this.u64(VARIANT + 0x2e0, 100_000);
        this.u64(VARIANT + 0x2e8, 100_000);
        this.u64(
            SHIP_TABLE + engine_native::creference_registry::ENTRIES,
            SHIP_BUCKETS as u64,
        );
        this.u32(
            SHIP_TABLE + engine_native::creference_registry::MASK,
            SHIP_MASK as u32,
        );
        this.put(
            SHIP_TABLE + engine_native::creference_registry::OVERFLOW_DISTANCE,
            &[16],
        );
        this.put(
            SHIP_BUCKETS
                + (SHIP_MASK + 1 + 16) * engine_native::reference_bucket::STRIDE
                + engine_native::reference_bucket::DISTANCE,
            &[255],
        );
        this.u64(WING, (IMAGE + engine_native_air::cair_wing::VTABLE_RVA.0) as u64);
        this.u64(
            WING + engine_native_air::cair_wing::SECONDARY,
            (IMAGE + engine_native_air::cair_wing::SECONDARY_VTABLE_RVA.0) as u64,
        );
        this.u64(WING + engine_native_air::cair_wing::REFERENCE_TYPE, WING_HANDLE);
        this.u64(WING + engine_native_air::cair_wing::POOL, (WING + 0x100) as u64);
        this.u64(
            WING + 0x100,
            (IMAGE + engine_native::cair_wing_pool::VTABLE_RVA.0) as u64,
        );
        this.u64(
            WING + 0x100 + engine_native::cair_wing_pool::DEFINITION,
            (WING + 0x200) as u64,
        );
        this.u64(
            WING + 0x200,
            (IMAGE + engine_native::csub_unit_definition::VTABLE_RVA.0) as u64,
        );
        this.u32(
            WING + 0x200 + engine_native::csub_unit_definition::DATABASE_INDEX,
            1,
        );
        let mut occupied = vec![false; SHIP_MASK + 1];
        this.register(
            SHIP_BUCKETS,
            WING_HANDLE,
            WING + engine_native_air::cair_wing::SECONDARY,
            &mut occupied,
        );
        let mut next_ship_id = 1_u32;
        for slot in 0..COUNTRY_COUNT {
            let country = COUNTRY_BASE + slot * COUNTRY_STRIDE;
            let task = TASK_BASE + slot * TASK_STRIDE;
            this.u64(COUNTRY_SLOTS + (slot + 1) * 8, country as u64);
            this.u64(
                country,
                (IMAGE + engine_native::ccountry::VTABLE_RVA.0) as u64,
            );
            this.u32(country + engine_native::ccountry::ID, slot as u32 + 1);
            let root = country + engine_native::ccountry::MODIFIER_ACCUMULATOR;
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
            this.u64(
                task,
                (IMAGE + engine_native::ctask_force::VTABLE_RVA.0) as u64,
            );
            this.u64(
                task + engine_native::ctask_force::SECONDARY_BASE,
                (IMAGE + engine_native::ctask_force::SECONDARY_VTABLE_RVA.0) as u64,
            );
            this.u64(
                task + engine_native::cunit::HANDLE,
                ((slot as u64 + 1) << 32) | 61,
            );
            this.u32(task + engine_native::cunit::ROSTER_OWNER, slot as u32 + 1);
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
            this.u64(
                body,
                (IMAGE + engine_native::cnaval_combat::VTABLE_RVA.0) as u64,
            );
            this.u64(
                body + engine_native::cnaval_combat::HANDLE,
                ((index as u64 + 1) << 32)
                    | engine_native::cnaval_combat::OBSERVED_REFERENCE_TYPE as u64,
            );
            this.u64(body + engine_native::cnaval_combat::ATTACKER, left as u64);
            this.u64(body + engine_native::cnaval_combat::DEFENDER, right as u64);
            this.u64(
                body + engine_native::cnaval_combat::PROVINCE,
                province as u64,
            );
            this.u64(
                province,
                (IMAGE + engine_native::cprovince::VTABLE_RVA.0) as u64,
            );
            this.u32(province + engine_native::cprovince::ID, index as u32 + 1);
            this.u64(province + 0xc8, region as u64);
            this.u64(
                region,
                (IMAGE + engine_native::cstrategic_region::VTABLE_RVA.0) as u64,
            );
            this.u64(region + 0x30, terrain_link as u64);
            this.u32(region + 0x58, 0);
            this.u64(terrain_link, (IMAGE + TEMPLATE_VTABLE_RVA) as u64);
            this.u64(terrain_link + 0x130, TERRAIN as u64);
            for (number, side) in [left, right].into_iter().enumerate() {
                this.u64(
                    side,
                    (IMAGE + engine_native::cnaval_combatant::VTABLE_RVA.0) as u64,
                );
                this.u64(side + engine_native::cnaval_combatant::COMBAT, body as u64);
                let country_node = side + 0x390;
                this.u64(side + 0x38, country_node as u64);
                let country_id = (index * 2 + number) % COUNTRY_COUNT + 1;
                let task = TASK_BASE + (country_id - 1) * TASK_STRIDE;
                this.u32(side + 0x48, 1);
                this.u32(country_node, country_id as u32);
                this.put(side + 0xb8, &[1]);
                let damage_sentinel = side + 0x3b0;
                let damage_node = side + 0x3d0;
                this.u64(side + 0x2b0, damage_sentinel as u64);
                this.u64(side + 0x2b8, 1);
                this.u64(damage_sentinel, damage_node as u64);
                this.u64(damage_node, damage_sentinel as u64);
                this.u64(damage_node + 8, damage_sentinel as u64);
                this.u32(damage_node + 0x10, 17);
                this.u64(damage_node + 0x18, 3_000);
                this.u64(side + 0x158, 100_000);
                this.u64(side + 0x168, 500);
                let group = body + 0x1000 + number * 0x400;
                let group_slot = body + 0x1f00 + number * 0x20;
                let member_slot = body + 0x2000 + number * 0x100;
                let plane_slot = body + 0xb000 + number * 0x40;
                let unit_slot = body + 0x2800 + number * 0x20;
                this.array(side + engine_native::cnaval_combatant::UNITS, unit_slot, 1);
                this.u64(unit_slot, task as u64);
                this.array(
                    side + engine_native::cnaval_combatant::GROUPS,
                    group_slot,
                    1,
                );
                this.u64(group_slot, group as u64);
                this.u64(
                    group,
                    (IMAGE + engine_native::cfex_group::VTABLE_RVA.0) as u64,
                );
                this.u64(group + engine_native::cfex_group::SIDE, side as u64);
                this.u64(
                    group + engine_native::cfex_group::OPPONENT_GROUP,
                    (body + 0x1000 + (1 - number) * 0x400) as u64,
                );
                this.array(
                    group + engine_native::cfex_group::MEMBERS,
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
                    this.u64(
                        member,
                        (IMAGE + engine_native::cfex_member::VTABLE_RVA.0) as u64,
                    );
                    this.u64(member + engine_native::cfex_member::GROUP, group as u64);
                    this.u64(member + engine_native::cfex_member::SHIP_HANDLE, handle);
                    this.u32(
                        member + engine_native::cfex_member::OWNER,
                        country_id as u32,
                    );
                    this.u32(
                        member + engine_native::cfex_member::BATTLE_SLOT,
                        ordinal as u32,
                    );
                    this.u32(
                        member + engine_native::cfex_member::STATE,
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
                    this.u64(ship, (IMAGE + engine_native::cship::VTABLE_RVA.0) as u64);
                    this.u64(ship + engine_native::cship::HANDLE, handle);
                    this.u64(ship + engine_native::cship::TASK_FORCE, task as u64);
                    this.u64(ship + 0x8f8, (task + 0xb00) as u64);
                    this.u64(
                        ship + engine_native::cship::EQUIPMENT_POOL,
                        (IMAGE + engine_native::cequipment_variant_pool::VTABLE_RVA.0) as u64,
                    );
                    this.u64(ship + engine_native::cship::STRENGTH, 100_000);
                    this.u64(ship + engine_native::cship::ORGANISATION, 100_000);
                    this.u64(ship + 0x300, 100_000);
                    this.u64(ship + 0x308, 100_000);
                    this.u64(ship + 0x228, 24_000);
                    this.u32(
                        ship + engine_native::cship::CATEGORY,
                        if local < 2 { 10 } else { 3 },
                    );
                }
                this.array(group + 0x58, plane_slot, 2);
                for carrier in 0..2 {
                    let plane = body + 0xa000 + (number * 2 + carrier) * 0x300;
                    let member = body + 0x3000 + (number * SHIPS_PER_COMBAT / 2 + carrier) * 0x300;
                    let wing_slot = body + 0xb100 + (number * 2 + carrier) * 0x20;
                    this.u64(plane_slot + carrier * 8, plane as u64);
                    this.u64(plane, (IMAGE + engine_native::cfex_air::VTABLE_RVA.0) as u64);
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
            SHIP_TABLE + engine_native::creference_registry::COUNT,
            (combats * SHIPS_PER_COMBAT + 1) as u32,
        );
        this
    }
    pub fn advance(&mut self, combats: usize, hour: Hour, pass: u64) {
        let countries = match hour {
            Hour::Quiet => return,
            Hour::Typical => 2,
            Hour::AllChanged => COUNTRY_COUNT,
        };
        if combats == 0 {
            return;
        }
        for slot in 0..countries {
            let root = COUNTRY_BASE
                + slot * COUNTRY_STRIDE
                + engine_native::ccountry::MODIFIER_ACCUMULATOR;
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
    pub fn live_sized(combats: usize) -> Self {
        let mut this = Self::new(combats);
        for slot in 0..COUNTRY_COUNT {
            let task = TASK_BASE + slot * TASK_STRIDE;
            this.entries(task + 0xb00 + 0x1b0, LIVE_ENTRIES[1], 400);
            this.entries(task + 0x668, LIVE_ENTRIES[2], 500);
        }
        for index in 0..combats {
            for ordinal in 0..SHIPS_PER_COMBAT {
                let ship = BODIES + index * STRIDE + 0xc000 + ordinal * 0xa00;
                this.entries(ship + 0x750, LIVE_ENTRIES[0], 300);
            }
        }
        this
    }

    pub fn with_doctrines(mut self, nodes: usize) -> Self {
        assert!(nodes > 0);
        let tokens = self.allocate((nodes + 1) * 32);
        for index in 0..=nodes {
            self.u64(tokens + index * 32 + 24, 15);
        }
        let generator = self.allocate(0x60);
        for offset in [0x28, 0x48] {
            self.u64(generator + offset, 15);
        }
        let dictionary = self.allocate(16);
        self.u64(dictionary, dictionary as u64);
        self.u64(dictionary + 8, dictionary as u64);
        let mut witness = [0_u8; 64];
        witness[0] = 1;
        for offset in [4, 12] {
            witness[offset..offset + 4].copy_from_slice(&(nodes as u32).to_le_bytes());
        }
        witness[48..56].copy_from_slice(&(tokens as u64).to_le_bytes());
        for offset in [56, 60] {
            witness[offset..offset + 4].copy_from_slice(&(nodes as u32 + 1).to_le_bytes());
        }
        self.put(LEXER, &witness);
        self.u64(LEXER + 0x40, generator as u64);
        self.u64(LEXER + 0x48, dictionary as u64);
        self.u32(LEXER + 0x50, LEXER_MARK);
        let modifiers = self.allocate(nodes * 0x20);
        for node in 0..nodes {
            let modifier = modifiers + node * 0x20;
            self.entries(modifier + 0x10, DOCTRINE_ENTRIES, 700);
        }
        for slot in 0..COUNTRY_COUNT {
            let task = TASK_BASE + slot * TASK_STRIDE;
            let head = self.pointer(task + 0x640);
            let first = self.allocate(nodes * 0x38);
            let last = first + (nodes - 1) * 0x38;
            self.u64(task + 0x648, nodes as u64);
            self.u64(head, first as u64);
            self.u64(head + 8, first as u64);
            self.u64(head + 0x10, last as u64);
            for index in 0..nodes {
                let node = first + index * 0x38;
                self.u64(node, head as u64);
                let previous = if index == 0 { head } else { node - 0x38 };
                let next = if node == last { head } else { node + 0x38 };
                self.u64(node + 8, previous as u64);
                self.u64(node + 0x10, next as u64);
                self.u32(node + 0x20, index as u32 + 1);
                self.u32(node + 0x28, 1);
                self.u32(node + 0x2c, 2);
                self.u64(node + 0x30, (modifiers + index * 0x20) as u64);
            }
        }
        self
    }

    pub fn with_locations(mut self) -> Self {
        let province = BODIES + 0xb00;
        let provinces = self.allocate(2 * 0x330);
        self.array(WEATHER + 0x370, provinces, 2);
        self.inline(TERRAIN + 0x18, b"ocean");
        let buckets = self.allocate(12);
        self.u64(TERRAIN + 0x1b8, buckets as u64);
        self.u32(TERRAIN + 0x1b8 + 12, 0);
        for slot in 0..COUNTRY_COUNT {
            let country = COUNTRY_BASE + slot * COUNTRY_STRIDE;
            let diplomacy = self.allocate(0x40);
            let relations = self.allocate((COUNTRY_COUNT + 1) * 8);
            self.u64(
                diplomacy,
                (IMAGE + engine_native_politics::cdiplomacy_status::VTABLE_RVA.0) as u64,
            );
            self.array(
                diplomacy + engine_native_politics::cdiplomacy_status::RELATIONS,
                relations,
                COUNTRY_COUNT + 1,
            );
            self.u64(
                country + engine_native::ccountry::DIPLOMACY_STATUS,
                diplomacy as u64,
            );
            let task = TASK_BASE + slot * TASK_STRIDE;
            self.u64(task + engine_native::cunit::LOCATION, province as u64);
        }
        self
    }

    pub fn without_live_ships(mut self, combats: usize) -> Self {
        for index in 0..combats {
            for ordinal in 0..SHIPS_PER_COMBAT {
                let member = BODIES + index * STRIDE + 0x3000 + ordinal * 0x300;
                let handle = self.pointer(member + engine_native::cfex_member::SHIP_HANDLE) as u64;
                let at =
                    SHIP_BUCKETS + Self::bucket(handle) * engine_native::reference_bucket::STRIDE;
                self.put(at, &[0; engine_native::reference_bucket::STRIDE]);
            }
        }
        self
    }

    fn entries(&mut self, at: usize, count: usize, key: u32) {
        let data = self.allocate(count * 16);
        self.array(at, data, count);
        for entry in 0..count {
            self.u32(data + entry * 16, key + entry as u32);
            self.u64(data + entry * 16 + 8, 1000 + entry as u64);
        }
    }

    pub fn pointer(&self, at: usize) -> usize {
        u64::from_le_bytes(self.0[at - STATE..at - STATE + 8].try_into().unwrap()) as usize
    }
    pub fn allocate(&mut self, bytes: usize) -> usize {
        let at = STATE + self.0.len();
        self.0.resize(self.0.len() + bytes, 0);
        at
    }
    pub fn bucket(handle: u64) -> usize {
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(engine_native::creference_registry::HASH_MULTIPLIER);
        (((hash >> 32) as u32 ^ hash as u32) as usize) & SHIP_MASK
    }
    pub fn register(&mut self, buckets: usize, handle: u64, pointer: usize, occupied: &mut [bool]) {
        let slot = Self::bucket(handle);
        assert!(!occupied[slot], "synthetic reference bucket collision");
        occupied[slot] = true;
        let at = buckets + slot * engine_native::reference_bucket::STRIDE;
        self.put(at + engine_native::reference_bucket::DISTANCE, &[1]);
        self.u64(at + engine_native::reference_bucket::TYPE, handle);
        self.u64(at + engine_native::reference_bucket::OBJECT, pointer as u64);
    }
    pub fn inline(&mut self, at: usize, value: &[u8]) {
        self.put(at, value);
        self.u64(at + 16, value.len() as u64);
        self.u64(at + 24, 15);
    }
    pub fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0[at - STATE..at - STATE + bytes.len()].copy_from_slice(bytes);
    }
    pub fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    pub fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    pub fn array(&mut self, at: usize, data: usize, count: usize) {
        self.u64(at, data as u64);
        self.u32(at + 8, count as u32);
        self.u32(at + 12, count as u32);
    }
}
impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if u32::from_le_bytes(
            self.0[LEXER + 0x50 - STATE..LEXER + 0x54 - STATE]
                .try_into()
                .unwrap(),
        ) == LEXER_MARK
        {
            let witness = IMAGE + engine_native::cpdx_lexer::INITIALIZED_RVA.0;
            let served = if (witness..witness + 64).contains(&address) {
                Some(LEXER + (address - witness))
            } else if address == IMAGE + 0x0343_9158 || address == IMAGE + 0x0343_9150 {
                Some(LEXER + 0x40)
            } else if address == IMAGE + 0x030f_eba8 {
                Some(LEXER + 0x48)
            } else {
                None
            };
            if let Some(at) = served {
                return SliceMemory {
                    base: STATE,
                    bytes: &self.0,
                }
                .copy(at, destination);
            }
        }
        if address == IMAGE + cgame_state::SINGLETON_RVA.0 && destination.len() == 8 {
            destination.copy_from_slice(&(STATE as u64).to_le_bytes());
            return true;
        }
        if address == IMAGE + engine_native::creference_registry::LOW_ROOTS_RVA.0 + 8
            && destination.len() == 8
        {
            destination.copy_from_slice(&(REFERENCE_TABLE as u64).to_le_bytes());
            return true;
        }
        if matches!(address,
            x if x == IMAGE + engine_native::creference_registry::LOW_ROOTS_RVA.0 + 51 * 8
                || x == IMAGE + engine_native::creference_registry::LOW_ROOTS_RVA.0 + 52 * 8)
            && destination.len() == 8
        {
            destination.copy_from_slice(&(SHIP_TABLE as u64).to_le_bytes());
            return true;
        }
        if address == IMAGE + engine_native::modifier_definition::TABLE_RVA.0
            && destination.len() >= 8
        {
            destination.fill(0);
            destination[..8].copy_from_slice(&0x4000_0000_u64.to_le_bytes());
            if destination.len() >= 16 {
                destination[8..12].copy_from_slice(&1024_u32.to_le_bytes());
                destination[12..16].copy_from_slice(&1024_u32.to_le_bytes());
            }
            return true;
        }
        if matches!(address,
            x if x == IMAGE + engine_native::modifier_definition::ELIGIBILITY_CALLBACK_RVA.0
                || x == IMAGE + engine_native::modifier_definition::TRADE_FEATURE_FLAGS_RVA.0)
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

pub fn live_shape() -> LiveShape {
    let mut memory = Fixture::new(3);
    let keys = TREE_KEYS;
    memory.u64(
        WING + 0x200 + engine_native::csub_unit_definition::AIR_FLAGS,
        0x400,
    );
    let weather_provinces = memory.allocate(4 * 0x330);
    memory.array(WEATHER + 0x370, weather_provinces, 4);
    let mut occupied: Vec<_> = (0..=SHIP_MASK)
        .map(|slot| {
            memory.0[SHIP_BUCKETS
                + slot * engine_native::reference_bucket::STRIDE
                + engine_native::reference_bucket::DISTANCE
                - STATE]
                != 0
        })
        .collect();
    let mut next_id = 1_u64;
    for combat in 0..3 {
        for carrier in 0..4 {
            let body = BODIES + combat * STRIDE;
            let member_slot = (carrier / 2) * (SHIPS_PER_COMBAT / 2) + carrier % 2;
            let ship = body + 0xc000 + member_slot * 0xa00;
            let ship_handle = memory.pointer(ship + engine_native::cship::HANDLE) as u64;
            let country = (combat * 2 + carrier / 2) % COUNTRY_COUNT + 1;
            let block = memory.allocate(0x1000);
            let air_base = block;
            let container = block + 0x200;
            let pool = block + 0x400;
            let wing = block + 0x500;
            let mut handle = |kind: u32, pointer: usize, memory: &mut Fixture| {
                let reference = loop {
                    let reference = (next_id << 32) | u64::from(kind);
                    next_id += 1;
                    if !occupied[Fixture::bucket(reference)] {
                        break reference;
                    }
                };
                memory.register(SHIP_BUCKETS, reference, pointer, &mut occupied);
                reference
            };
            let base_handle = handle(engine_native_air::cair_base::TYPE, air_base, &mut memory);
            let wing_handle = handle(
                engine_native_air::cair_wing::TYPE,
                wing + engine_native_air::cair_wing::SECONDARY,
                &mut memory,
            );
            memory.u64(
                air_base,
                (IMAGE + engine_native_air::cair_base::VTABLE_RVA.0) as u64,
            );
            memory.u64(air_base + engine_native_air::cair_base::HANDLE, base_handle);
            memory.u64(
                air_base + engine_native_air::cair_base::HOST_REFERENCE,
                ship_handle,
            );
            memory.array(
                air_base + engine_native_air::cair_base::COUNTRY_CONTAINERS,
                block + 0x100,
                COUNTRY_COUNT + 1,
            );
            memory.u64(block + 0x100 + country * 8, container as u64);
            memory.array(
                air_base + engine_native_air::cair_base::REGISTERED_CONTAINERS,
                block + 0x300,
                1,
            );
            memory.u64(block + 0x300, container as u64);
            memory.u64(
                container,
                (IMAGE + engine_native_air::ccountry_air_container::VTABLE_RVA.0) as u64,
            );
            memory.u32(
                container + engine_native_air::ccountry_air_container::COUNTRY,
                country as u32,
            );
            memory.array(
                container + engine_native_air::ccountry_air_container::POOLS,
                block + 0x320,
                1,
            );
            memory.u64(block + 0x320, pool as u64);
            memory.u64(
                pool,
                (IMAGE + engine_native::cair_wing_pool::VTABLE_RVA.0) as u64,
            );
            memory.u64(
                pool + engine_native::cair_wing_pool::DEFINITION,
                (WING + 0x200) as u64,
            );
            memory.u64(
                pool + engine_native::cair_wing_pool::AIR_BASE_REFERENCE,
                base_handle,
            );
            memory.array(
                pool + engine_native::cair_wing_pool::WINGS,
                block + 0x480,
                1,
            );
            memory.u64(block + 0x480, wing as u64);
            memory.u64(wing, (IMAGE + engine_native_air::cair_wing::VTABLE_RVA.0) as u64);
            memory.u64(
                wing + engine_native_air::cair_wing::SECONDARY,
                (IMAGE + engine_native_air::cair_wing::SECONDARY_VTABLE_RVA.0) as u64,
            );
            memory.u64(wing + engine_native_air::cair_wing::REFERENCE_TYPE, wing_handle);
            memory.u64(wing + engine_native_air::cair_wing::POOL, pool as u64);
            memory.u64(
                ship + engine_native::cship::CARRIER_AIR_BASE,
                air_base as u64,
            );
            let plane = body + 0xa000 + carrier * 0x300;
            memory.u64(plane + 0x1c, base_handle);
            memory.u64(memory.pointer(plane + 0x40), wing_handle);
        }
    }
    memory.u32(
        SHIP_TABLE + engine_native::creference_registry::COUNT,
        (3 * SHIPS_PER_COMBAT + 1 + 24) as u32,
    );
    for combat in 0..3 {
        memory.put(BODIES + combat * STRIDE + 0x200 + 0xb8, &[0]);
    }
    memory.u32(BODIES + STRIDE + engine_native::cland_combat::DURATION, 13);
    memory.u64(
        BODIES + 2 * STRIDE + 0xc000 + 2 * 0xa00 + engine_native::cship::FORMATION_FLAGS,
        1 << 7,
    );
    let convoy_member = BODIES + 2 * STRIDE + 0x3000 + 3 * 0x300;
    memory.u64(convoy_member + engine_native::cfex_member::SHIP_HANDLE, 0);
    memory.u64(
        convoy_member + engine_native::cfex_member::CONVOY_HANDLE,
        CONVOY_HANDLE,
    );
    for country in 0..6 {
        let root =
            COUNTRY_BASE + country * COUNTRY_STRIDE + engine_native::ccountry::MODIFIER_ACCUMULATOR;
        let children = memory.pointer(root + 0x18);
        for child in 0..8 {
            let parent = memory.pointer(children + child * 8) + 0x10;
            let pointers = STATE + memory.0.len();
            memory.0.resize(memory.0.len() + 8 * 8, 0);
            memory.array(parent + 0x18, pointers, 8);
            for grandchild in 0..8 {
                let node = STATE + memory.0.len();
                memory.0.resize(memory.0.len() + 0xc0 + keys.len() * 16, 0);
                memory.u64(pointers + grandchild * 8, (node - 0x10) as u64);
                memory.array(node, node + 0xc0, keys.len());
                memory.inline(node + 0x48, b"deep doctrine");
                for (index, key) in keys.into_iter().enumerate() {
                    memory.u32(node + 0xc0 + index * 16, key);
                    memory.u64(node + 0xc0 + index * 16 + 8, 1000);
                }
                if grandchild < 2 {
                    let pointers = STATE + memory.0.len();
                    memory.0.resize(memory.0.len() + 4 * 8, 0);
                    memory.array(node + 0x18, pointers, 4);
                    for great in 0..4 {
                        let leaf = STATE + memory.0.len();
                        memory.0.resize(memory.0.len() + 0xc0 + keys.len() * 16, 0);
                        memory.u64(pointers + great * 8, (leaf - 0x10) as u64);
                        memory.array(leaf, leaf + 0xc0, keys.len());
                        memory.inline(leaf + 0x48, b"leaf doctrine");
                        for (index, key) in keys.into_iter().enumerate() {
                            memory.u32(leaf + 0xc0 + index * 16, key);
                            memory.u64(leaf + 0xc0 + index * 16 + 8, 2000);
                        }
                    }
                }
            }
        }
    }
    LiveShape(memory)
}

const TREE_KEYS: [u32; 9] = [0x24, 0x59, 0x5a, 0x5b, 0x5c, 0x5d, 0x1b2, 0x1b3, 0x1b6];

pub struct LiveShape(pub Fixture);
impl Memory for LiveShape {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if matches!(address,
                x if x == IMAGE + engine_native::creference_registry::LOW_ROOTS_RVA.0
                    + engine_native_air::cair_base::TYPE as usize * 8
                    || x == IMAGE + engine_native::creference_registry::LOW_ROOTS_RVA.0
                        + engine_native_air::cair_wing::TYPE as usize * 8)
            && output.len() == 8
        {
            output.copy_from_slice(&(SHIP_TABLE as u64).to_le_bytes());
            return true;
        }
        self.0.copy(address, output)
    }
}
