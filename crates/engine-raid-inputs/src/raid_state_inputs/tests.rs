use super::*;
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
use engine_native_air::raid_launch_inputs::test_support::Counted;
use crate::cstrategic_resource_database;
use engine_native_parts::modifier_definition;
use std::collections::BTreeMap;

#[derive(Default)]
struct Mem(BTreeMap<usize, u8>);

impl Memory for Mem {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        for (index, byte) in destination.iter_mut().enumerate() {
            match self.0.get(&(address + index)) {
                Some(value) => *byte = *value,
                None => return false,
            }
        }
        true
    }
}

trait Store {
    fn put(&mut self, address: usize, bytes: &[u8]);
    fn base(&self) -> usize;
    fn rva(&self, rva: engine_sites::Rva) -> u64 {
        (self.base() + rva.0) as u64
    }
    fn u8(&mut self, address: usize, value: u8) {
        self.put(address, &[value]);
    }
    fn u32(&mut self, address: usize, value: u32) {
        self.put(address, &value.to_le_bytes());
    }
    fn u64(&mut self, address: usize, value: u64) {
        self.put(address, &value.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.u64(address, data as u64);
        self.u32(address + 8, count);
        self.u32(address + 12, count);
    }
    fn pointers(&mut self, address: usize, data: usize, pointers: &[usize]) {
        self.array(address, data, pointers.len() as u32);
        for (index, pointer) in pointers.iter().enumerate() {
            self.u64(data + index * 8, *pointer as u64);
        }
    }
}

impl Store for Mem {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        for (index, byte) in bytes.iter().enumerate() {
            self.0.insert(address + index, *byte);
        }
    }
    fn base(&self) -> usize {
        0
    }
}

const GS: usize = 0x4000_0000;
const STATE_A: usize = 0x4600_1000;
const STATE_B: usize = 0x4600_3000;
const COUNTRY: usize = 0x4200_1000;
const RESOURCES: usize = 0x4300_0000;
const COMPANION: usize = 0x4400_3000 + supply_companion::STRIDE;

fn state(m: &mut impl Store, pointer: usize, id: u32, controller: u32, scratch: usize) {
    m.u64(pointer, m.rva(cstate::VTABLE_RVA));
    m.u32(pointer + cstate::ID, id);
    m.u32(pointer + cstate::CONTROLLER, controller);
    m.u64(pointer + cstate::TEMPLATE, 0);
    m.array(pointer + cstate::PROVINCES, scratch, 0);
    m.array(pointer + cstate::CLAIMS, scratch, 0);
    m.array(pointer + cstate::STRATEGIC_LOCATIONS, scratch, 0);
    let status = pointer + cstate::BUILDING_STATUS;
    m.array(status + cbuilding_status::BUILDING_INDICES, scratch, 0);
    m.array(status + cbuilding_status::BUILDINGS, scratch, 0);
    m.u8(pointer + cstate::RESOURCE_MODIFIER_GATE, 0);
    let pool = pointer + cstate::RESOURCE_POOL;
    m.u64(pool, m.rva(cstrategic_resource_pool::VTABLE_RVA));
    m.u64(pool + 8, (scratch + 0x100) as u64);
    m.u32(pool + 16, 3);
    m.u32(pool + 20, 3);
    for (slot, token) in [(0, 0), (1, 101), (2, 102)] {
        m.put(scratch + 0x100 + slot * 16, &[0; 16]);
        m.u32(scratch + 0x100 + slot * 16 + 8, token);
    }
    m.array(pointer + cstate::BASE_MODIFIERS, scratch + 0x200, 0);
    let container = pointer + cstate::DYNAMIC_MODIFIERS;
    m.put(container, &[0; 0x38]);
    m.u64(container, m.rva(cdynamic_modifier_container::VTABLE_RVA));
    m.u64(container + cdynamic_modifier_container::COUNTRY, 0);
    m.u32(container + cdynamic_modifier_container::STATE, id);
    m.array(container + cdynamic_modifier_container::ENTRIES, scratch, 0);
    m.put(pointer + cstate::COUNTRY_MODIFIERS, &[0; 17]);
    m.u64(
        pointer + cstate::COUNTRY_MODIFIERS,
        (scratch + 0x300) as u64,
    );
    m.u32(pointer + cstate::COUNTRY_MODIFIERS + 12, 0);
    m.u8(pointer + cstate::COUNTRY_MODIFIERS + 16, 0);
    m.u8(scratch + 0x300 + 4, 0);
}

#[test]
fn state_cores_read_the_state_s_and_its_provinces_strategic_locations() {
    let mut m = fixture();
    m.array(STATE_A + cstate::CORES, 0, 0);
    m.array(STATE_B + cstate::CORES, 0, 0);
    m.array(STATE_A + cstate::STRATEGIC_LOCATIONS, 0x4642_0000, 2);
    for (index, (token, province)) in [(701_u32, 31_u32), (702, 32)].into_iter().enumerate() {
        m.u32(0x4642_0000 + index * 8, token);
        m.u32(0x4642_0004 + index * 8, province);
    }
    let provinces = 0x4643_0000;
    m.array(STATE_A + cstate::PROVINCES, provinces, 2);
    let province = |m: &mut Mem, slot: usize, at: usize, id: u32, state: usize, tokens: &[u32]| {
        m.u64(provinces + slot * 8, at as u64);
        m.put(at, &[0; 0x150]);
        m.u64(at, m.rva(cprovince::VTABLE_RVA));
        m.u64(at + cprovince::STATE, state as u64);
        m.u32(at + cprovince::ID, id);
        let list = at + 0x1000;
        m.array(
            at + cprovince::STRATEGIC_LOCATIONS,
            list,
            tokens.len() as u32,
        );
        for (index, &token) in tokens.iter().enumerate() {
            m.u32(list + index * 4, token);
        }
    };
    province(&mut m, 0, 0x4644_0000, 31, STATE_A, &[801]);
    province(&mut m, 1, 0x4645_0000, 32, STATE_A, &[802, 803]);
    let run = |m: &Mem| {
        let mut rows = vec![CoreRow::default(); 2];
        let mut links = vec![0; 16];
        let mut required = 0;
        let outcome = state_cores(
            m,
            0,
            GS,
            &[1, 2],
            &|_| true,
            &mut rows,
            &mut links,
            &mut required,
        );
        (outcome, rows, links, required)
    };
    let (outcome, rows, links, required) = run(&m);
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        rows[0],
        CoreRow {
            state: 1,
            locations_len: 2,
            province_locations_start: 4,
            province_locations_len: 3,
            ..CoreRow::default()
        }
    );
    assert_eq!(required, 10);
    assert_eq!(&links[..10], &[701, 31, 702, 32, 31, 801, 32, 802, 32, 803]);
    province(&mut m, 1, 0x4645_0000, 32, STATE_B, &[802]);
    assert!(matches!(run(&m).0, Outcome::Unavailable { .. }));
}

#[test]
fn state_cores_read_each_selected_states_list_in_stored_order() {
    let mut m = fixture();
    m.array(STATE_A + cstate::CORES, 0x4640_0000, 2);
    m.u32(0x4640_0000, 42);
    m.u32(0x4640_0004, 5);
    m.array(STATE_B + cstate::CORES, 0, 0);
    m.array(STATE_B + cstate::CLAIMS, 0x4641_0000, 2);
    m.u32(0x4641_0000, 9);
    m.u32(0x4641_0004, 42);
    let census = [1, 2];
    let run = |m: &Mem, selected: &dyn Fn(u32) -> bool, links: usize| {
        let mut rows = vec![CoreRow::default(); 2];
        let mut buffer = vec![0; links];
        let mut required = 0;
        let outcome = state_cores(
            m,
            0,
            GS,
            &census,
            selected,
            &mut rows,
            &mut buffer,
            &mut required,
        );
        (outcome, rows, buffer, required)
    };
    let (outcome, rows, links, required) = run(&m, &|_| true, 4);
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        rows,
        [
            CoreRow {
                state: 1,
                start: 0,
                len: 2,
                claims_start: 2,
                claims_len: 0,
                locations_start: 2,
                province_locations_start: 2,
                ..CoreRow::default()
            },
            CoreRow {
                state: 2,
                start: 2,
                len: 0,
                claims_start: 2,
                claims_len: 2,
                locations_start: 4,
                province_locations_start: 4,
                ..CoreRow::default()
            }
        ]
    );
    assert_eq!((&links[..4], required), (&[42, 5, 9, 42][..], 4));
    let (outcome, rows, _, required) = run(&m, &|state| state == 2, 4);
    assert_eq!(
        (outcome, rows[0].state, required),
        (Outcome::Rows { len: 1 }, 2, 2)
    );
    let (outcome, rows, _, required) = run(&m, &|_| true, 2);
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    assert_eq!((rows[0].state, required), (1, 4));
    m.array(STATE_B + cstate::CLAIMS, 0x4642_0000, 1);
    assert_eq!(
        run(&m, &|_| true, 4).0,
        Outcome::Unavailable {
            reason: "state claim list unreadable"
        }
    );
    m.array(STATE_B + cstate::CLAIMS, 0x4641_0000, 2);
    m.u32(STATE_B + cstate::ID, 7);
    assert!(matches!(
        run(&m, &|_| true, 4).0,
        Outcome::Unavailable { .. }
    ));
}

fn fixture() -> Mem {
    let mut m = Mem::default();
    let database = 0x4100_0000;
    m.u64(
        cstrategic_resource_database::SINGLETON_RVA.0,
        database as u64,
    );
    m.pointers(
        database + cstrategic_resource_database::ITEMS,
        0x4100_1000,
        &[0x4100_2000, 0x4100_3000, 0x4100_4000],
    );
    m.array(
        database + cstrategic_resource_database::KEYS,
        0x4100_1100,
        2,
    );
    for (index, (descriptor, token, first_key)) in
        [(0x4100_3000, 101, 0x10), (0x4100_4000, 102, 0x20)]
            .into_iter()
            .enumerate()
    {
        m.u32(0x4100_1100 + index * 4, token);
        m.u64(descriptor, cstrategic_resource::VTABLE_RVA.0 as u64);
        m.u32(descriptor + cstrategic_resource::TOKEN_ID, token);
        m.u8(descriptor + cstrategic_resource::ACTIVE, 1);
        m.u32(descriptor + cstrategic_resource::INDEX, index as u32 + 1);
        m.u64(descriptor + cstrategic_resource::FACTORY_COST, 0);
        for offset in [0, 4, 8, 12] {
            m.u32(
                descriptor + cstrategic_resource::STATE_ADDITIVE_KEY + offset,
                first_key + offset as u32 / 4,
            );
        }
    }
    let table = 0x4a00_0000;
    m.u64(modifier_definition::TABLE_RVA.0, table as u64);
    for key in [0x10_usize, 0x13, 0x223, 0x292] {
        let row = table + key * modifier_definition::STRIDE;
        m.u32(row + modifier_definition::FEATURE_SELECTOR, 0);
        m.u32(row + modifier_definition::FLAGS, 0);
    }
    m.pointers(GS + cgame_state::COUNTRIES, 0x4200_0000, &[0, COUNTRY]);
    m.array(GS + cgame_state::COUNTRY_LINKS, 0x4200_0100, 2);
    m.u32(0x4200_0100, 0);
    m.u32(0x4200_0104, 1);
    m.u64(COUNTRY, ccountry::VTABLE_RVA.0 as u64);
    m.u32(COUNTRY + ccountry::ID, 1);
    m.u64(COUNTRY + ccountry::RESOURCES, RESOURCES as u64);
    m.u64(RESOURCES, ccountry_resources::VTABLE_RVA.0 as u64);
    m.u32(RESOURCES + ccountry_resources::OWNER, 1);
    m.pointers(
        RESOURCES + ccountry_resources::RESOURCE_ORIGINS,
        0x4300_1000,
        &[0x4300_2000],
    );
    m.put(0x4300_2000, &[0; 0x88]);
    m.u64(0x4300_2000, cresource_origin::VTABLE_RVA.0 as u64);
    m.u64(0x4300_2000 + cresource_origin::STATE, STATE_A as u64);
    m.array(
        RESOURCES + ccountry_resources::BUILDING_RESOURCE_OVERRIDES,
        0x4300_3000,
        2,
    );
    m.put(0x4300_3000, &[0; 0x40]);
    m.u32(0x4300_3000, 555);
    m.array(0x4300_3008, 0x4300_4000, 1);
    m.put(0x4300_4000, &[0; 16]);
    m.u32(0x4300_4000, 2);
    m.u64(0x4300_4008, 50_000);
    m.u32(0x4300_3020, 556);
    m.array(0x4300_3028, 0, 0);
    let root = 0x4400_0000;
    m.u64(GS + cgame_state::SUPPLY_SYSTEM, root as u64);
    m.u64(root, csupply_system::VTABLE_RVA.0 as u64);
    m.u64(root + 8, csupply_system::SECONDARY_VTABLE_RVA.0 as u64);
    m.pointers(
        root + csupply_system::COUNTRY_SYSTEM_DATA,
        0x4400_1000,
        &[0, 0x4400_2000],
    );
    m.array(
        root + csupply_system::COUNTRY_COMPANION_DATA,
        0x4400_3000,
        2,
    );
    let system = 0x4400_2000;
    m.u64(system, ccountry_supply_system::VTABLE_RVA.0 as u64);
    m.u64(
        system + 8,
        ccountry_supply_system::SECONDARY_VTABLE_RVA.0 as u64,
    );
    m.u64(system + ccountry_supply_system::SUPPLY_SYSTEM, root as u64);
    m.u64(system + ccountry_supply_system::COUNTRY, COUNTRY as u64);
    m.u64(system + ccountry_supply_system::COMPANION, COMPANION as u64);
    m.array(
        COMPANION + supply_companion::ROUTES_SUBTYPE_ZERO,
        0x4400_5000,
        2,
    );
    for (index, province) in [7_u32, 9].into_iter().enumerate() {
        m.u32(0x4400_5000 + index * 8, province);
        m.u32(0x4400_5004 + index * 8, index as u32);
        let node = 0x4400_6000 + index * engine_native_parts::supply_node_layout::STRIDE;
        m.put(node, &[0; engine_native_parts::supply_node_layout::STRIDE]);
        m.u32(node, index as u32);
        m.u32(node + 4, province);
        m.u64(node + supply_node::ROUTE_VALUE, index as u64 * 5);
    }
    m.array(COMPANION + supply_companion::NODES, 0x4400_6000, 2);
    m.u64(cbuilding_database::SINGLETON_RVA.0, 0x4500_0000);
    m.u64(0x4500_0000 + INFRASTRUCTURE_TEMPLATE, 0x4500_1000);
    m.u8(
        0x4500_1000 + cbuilding_template::LOOKUP_ENABLED_CANDIDATE,
        1,
    );
    m.u32(0x4500_1000 + cbuilding_template::DATABASE_ORDINAL, 0);
    m.pointers(
        GS + cgame_state::STATES,
        0x4600_0000,
        &[0, STATE_A, STATE_B],
    );
    state(&mut m, STATE_A, 1, 1, 0x4610_0000);
    state(&mut m, STATE_B, 2, 0, 0x4620_0000);
    m.pointers(
        STATE_A + cstate::PROVINCES,
        0x4630_0000,
        &[0x4700_0000, 0x4700_1000],
    );
    for (pointer, id) in [(0x4700_0000, 7), (0x4700_1000, 9)] {
        m.put(pointer, &[0; 0x150]);
        m.u64(pointer, cprovince::VTABLE_RVA.0 as u64);
        m.u32(pointer + cprovince::ID, id);
        m.u64(pointer + cprovince::STATE, STATE_A as u64);
        m.array(pointer + cprovince::STRATEGIC_LOCATIONS, 0, 0);
    }
    let status = STATE_A + cstate::BUILDING_STATUS;
    m.array(status + cbuilding_status::BUILDING_INDICES, 0x4630_1000, 1);
    m.u32(0x4630_1000, 0);
    m.pointers(
        status + cbuilding_status::BUILDINGS,
        0x4630_2000,
        &[0x4800_0000],
    );
    m.u64(0x4800_0000 + cbuilding::TEMPLATE, 0x4500_1000);
    m.put(0x4800_0000 + cbuilding::LEVEL, &3_i16.to_le_bytes());
    m.u8(STATE_A + cstate::RESOURCE_MODIFIER_GATE, 1);
    m.u64(0x4610_0000 + 0x110, 800_000);
    m.array(STATE_A + cstate::BASE_MODIFIERS, 0x4630_3000, 3);
    for (index, (key, value)) in [(0x10_u32, 1000_i64), (0x13, 200), (0x223, -300)]
        .into_iter()
        .enumerate()
    {
        m.put(0x4630_3000 + index * 16, &[0; 16]);
        m.u32(0x4630_3000 + index * 16, key);
        m.u64(0x4630_3008 + index * 16, value as u64);
    }
    let container = STATE_A + cstate::DYNAMIC_MODIFIERS;
    m.array(
        container + cdynamic_modifier_container::ENTRIES,
        0x4630_4000,
        1,
    );
    let entry = 0x4630_4000;
    m.put(entry, &[0; dynamic_modifier_entry::STRIDE]);
    m.u64(entry, dynamic_modifier_entry::VTABLE_RVA.0 as u64);
    m.u32(entry + dynamic_modifier_entry::SCOPE_COUNTRY, 0);
    m.u32(entry + dynamic_modifier_entry::SCOPE_STATE, 0);
    m.u32(entry + dynamic_modifier_entry::DAYS_REMAINING, u32::MAX);
    m.u64(entry + dynamic_modifier_entry::DEFINITION, 0x4900_0000);
    m.u8(entry + dynamic_modifier_entry::ENABLED, 1);
    m.array(entry + dynamic_modifier_entry::VALUES, 0, 0);
    m.put(0x4900_0000, &[0; 0x48]);
    m.u64(0x4900_0000, cdynamic_modifier::VTABLE_RVA.0 as u64);
    m.put(0x4900_0000 + cdynamic_modifier::ID, b"refinery\0");
    m.u64(0x4900_0000 + cdynamic_modifier::ID + 16, 8);
    m.u64(0x4900_0000 + cdynamic_modifier::ID + 24, 15);
    m
}

#[test]
fn reads_every_stored_operand_and_the_state_container() {
    let m = fixture();
    let mut buffers = Buffers::new(&fixture_capacity()).unwrap();
    let walk = walk(&m, 0, GS, &mut buffers, Selection::ALL);
    assert_eq!(
        walk.outcomes,
        [
            Outcome::Rows { len: 2 },
            Outcome::Rows { len: 1 },
            Outcome::Rows { len: 1 },
            Outcome::Rows { len: 2 }
        ]
    );
    assert_eq!(
        buffers.inputs[..2],
        [
            InputRow {
                state: 1,
                gate: 1,
                global_factor: -300,
                recruitment: Some([0; 4]),
                origin: 1,
                infrastructure_level: Some(3),
                infrastructure_modifier: 0,
                supply_route: 1,
            },
            InputRow {
                state: 2,
                ..InputRow::default()
            }
        ]
    );
    assert_eq!(
        buffers.resources[0],
        ResourceRow {
            state: 1,
            resource: 101,
            pool: 800_000,
            additive: 1000,
            factor: 0,
            cost: 200,
        }
    );
    let modifier = buffers.modifiers[0];
    assert_eq!(
        (modifier.state, modifier.scope_country, modifier.scope_state),
        (1, 0, 0)
    );
    assert_eq!(
        &buffers.tokens[modifier.token_start..modifier.token_start + modifier.token_len],
        b"refinery"
    );
    assert_eq!(
        buffers.overrides[..2],
        [
            OverrideRow {
                country: 1,
                ordinal: 0,
                entry: 0,
                building: 555,
                count: 1,
                resource: 102,
                amount: 50_000,
            },
            OverrideRow {
                country: 1,
                ordinal: 1,
                entry: 1,
                building: 556,
                ..OverrideRow::default()
            }
        ]
    );
}

#[test]
fn no_route_value_and_no_origin_leave_the_factors_unread() {
    let mut m = fixture();
    m.u64(
        0x4400_6000 + engine_native_parts::supply_node_layout::STRIDE + supply_node::ROUTE_VALUE,
        0,
    );
    let mut buffers = Buffers::new(&fixture_capacity()).unwrap();
    walk(&m, 0, GS, &mut buffers, Selection::ALL);
    assert_eq!(buffers.inputs[0].supply_route, 0);
    m.u64(0x4300_2000 + cresource_origin::STATE, STATE_B as u64);
    walk(&m, 0, GS, &mut buffers, Selection::ALL);
    assert_eq!(
        (
            buffers.inputs[0].origin,
            buffers.inputs[0].infrastructure_level
        ),
        (0, None)
    );
}

#[test]
fn short_buffers_grow_and_report_live_sizes() {
    let m = fixture();
    let small = Capacity {
        token_bytes: 4,
        override_rows: 1,
        ..fixture_capacity()
    };
    let mut buffers = Buffers::new(&small).unwrap();
    let first = walk(&m, 0, GS, &mut buffers, Selection::ALL);
    assert!(
        first
            .outcomes
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Rows { .. }))
    );
    assert_eq!(
        (first.required.token_bytes, first.required.override_rows),
        (8, 2)
    );
}

#[test]
fn growing_walk_fills_the_first_pass_and_keeps_the_first_native_fault() {
    use std::cell::RefCell;
    struct Traced<'a> {
        memory: &'a Mem,
        copies: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.copies.borrow_mut().push((address, destination.len()));
            self.memory.copy(address, destination)
        }
    }
    let mut memory = fixture();
    let traced = Traced {
        memory: &memory,
        copies: RefCell::new(Vec::new()),
    };
    let mut buffers = Buffers::empty();
    let captured = walk(&traced, 0, GS, &mut buffers, Selection::ALL);
    assert_eq!(
        captured.outcomes,
        [
            Outcome::Rows { len: 2 },
            Outcome::Rows { len: 1 },
            Outcome::Rows { len: 1 },
            Outcome::Rows { len: 2 }
        ]
    );
    for header in [
        GS + cgame_state::STATES,
        GS + cgame_state::COUNTRIES,
        GS + cgame_state::COUNTRY_LINKS,
        0x4100_0000 + cstrategic_resource_database::ITEMS,
        0x4100_0000 + cstrategic_resource_database::KEYS,
    ] {
        assert_eq!(
            traced
                .copies
                .borrow()
                .iter()
                .filter(|&&(address, len)| address == header && len == 16)
                .count(),
            1,
            "header {header:x} reread"
        );
    }
    assert_eq!(&buffers.tokens[..captured.token_bytes], b"refinery");
    assert_eq!(buffers.read_countries, [1]);
    memory.u64(STATE_A, 7);
    memory.0.remove(&(GS + cgame_state::COUNTRIES));
    assert_eq!(
        walk(&memory, 0, GS, &mut Buffers::empty(), Selection::ALL).outcomes,
        [Outcome::Unavailable {
            reason: "state vtable mismatch"
        }; 4]
    );
}

#[test]
fn growing_resource_database_and_name_dictionary_follow_mod_demand() {
    let mut memory = fixture();
    let database = 0x4100_0000;
    let count = 257;
    let items = 0x4b00_0000;
    let tokens = 0x4b01_0000;
    memory.array(
        database + cstrategic_resource_database::ITEMS,
        items,
        count + 1,
    );
    memory.array(database + cstrategic_resource_database::KEYS, tokens, count);
    memory.u64(items, 0x4100_2000);
    for index in 0..count as usize {
        let descriptor = 0x4b02_0000 + index * 0x200;
        memory.u64(items + (index + 1) * 8, descriptor as u64);
        memory.u32(tokens + index * 4, index as u32 + 100);
        memory.u64(descriptor, cstrategic_resource::VTABLE_RVA.0 as u64);
        memory.u32(descriptor + cstrategic_resource::INDEX, index as u32 + 1);
        memory.u32(
            descriptor + cstrategic_resource::TOKEN_ID,
            index as u32 + 100,
        );
        for offset in 0..4 {
            memory.u32(
                descriptor + cstrategic_resource::STATE_ADDITIVE_KEY + offset * 4,
                index as u32 * 4 + offset as u32 + 1,
            );
        }
    }
    let (mut descriptors, mut keys, mut bytes) = (Vec::new(), Vec::new(), Vec::new());
    assert_eq!(
        resource_database(&memory, 0, &mut descriptors, &mut keys, &mut bytes),
        Ok(Ok(count as usize))
    );
    assert_eq!(descriptors[count as usize - 1].token, count + 99);
    let (mut table, mut name_bytes) = (Vec::new(), Vec::new());
    let mut names = Names {
        table: &mut table,
        bytes: &mut name_bytes,
        used: 0,
        entries: 0,
        limit: 0,
    };
    let mut required = Capacity::zero();
    for index in 0..2_201 {
        let definition = 0x4c00_0000 + index * 0x100;
        let text = format!("name_{index:04}");
        memory.put(definition, &[0; 0x48]);
        memory.u64(definition, cdynamic_modifier::VTABLE_RVA.0 as u64);
        memory.put(definition + cdynamic_modifier::ID, text.as_bytes());
        memory.u64(definition + cdynamic_modifier::ID + 16, text.len() as u64);
        memory.u64(definition + cdynamic_modifier::ID + 24, 15);
        let Name::Kept(start, len) = names.get(&memory, 0, definition, &mut required).unwrap()
        else {
            panic!("growing names must retain the first occurrence");
        };
        assert_eq!(&names.bytes[start..start + len], text.as_bytes());
    }
    for index in 0..2_201 {
        let definition = 0x4c00_0000 + index * 0x100;
        memory.0.remove(&definition);
        let Name::Kept(start, len) = names.get(&memory, 0, definition, &mut required).unwrap()
        else {
            panic!("known names must not reread native storage");
        };
        assert_eq!(
            &names.bytes[start..start + len],
            format!("name_{index:04}").as_bytes()
        );
    }
    assert_eq!(required.definitions, 2_201);
    assert_eq!(required.name_bytes, 2_201 * 9);
}

#[test]
fn live_demand_sizes_every_raid_state_table_without_acquiring_payloads() {
    let mut memory = fixture();
    memory.array(STATE_A + cstate::CORES, 0x4d00_0000, 5_000);
    memory.array(STATE_B + cstate::CORES, 0, 0);
    memory.array(STATE_B + cstate::CLAIMS, 0x4e00_0000, 3_000);
    let (capacity, cores) = demand(&memory, 0, GS).unwrap();
    assert_eq!(
        (capacity.states, capacity.countries, capacity.resources),
        (3, 2, 2)
    );
    assert_eq!(cores, 8_000);
    assert_eq!(
        (
            capacity.modifier_rows,
            capacity.token_bytes,
            capacity.override_rows
        ),
        (1, 8, 2)
    );
    assert_eq!(
        (
            capacity.origins,
            capacity.routes,
            capacity.provinces,
            capacity.entries
        ),
        (1, 2, 2, 2)
    );
    assert_eq!(
        capacity.batch_bytes,
        3 * cstrategic_resource_pool::VALUE_STRIDE
            + 3 * modifier_entry::STRIDE
            + dynamic_modifier_entry::STRIDE
    );
    let mut buffers = Buffers::new(&capacity).unwrap();
    assert!(
        walk(&memory, 0, GS, &mut buffers, Selection::ALL)
            .outcomes
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Rows { .. }))
    );
    memory.array(STATE_B + cstate::CORES, 0x4d10_0000, 6_000);
    assert_eq!(demand(&memory, 0, GS).unwrap().1, 14_000);
}

#[test]
fn a_foreign_container_or_torn_pool_fails_the_whole_walk() {
    let mut m = fixture();
    m.u32(
        STATE_A + cstate::DYNAMIC_MODIFIERS + cdynamic_modifier_container::STATE,
        9,
    );
    let mut buffers = Buffers::new(&fixture_capacity()).unwrap();
    assert!(
        walk(&m, 0, GS, &mut buffers, Selection::ALL)
            .outcomes
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
    );
    let mut m = fixture();
    m.u32(0x4610_0000 + 0x100 + 16 + 8, 7);
    assert!(matches!(
        walk(&m, 0, GS, &mut buffers, Selection::ALL).outcomes[1],
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn a_walk_reads_rows_only_for_selected_states_and_countries() {
    let mut m = fixture();
    m.u32(
        STATE_A + cstate::DYNAMIC_MODIFIERS + cdynamic_modifier_container::STATE,
        9,
    );
    m.0.remove(&0x4300_3000);
    let mut buffers = Buffers::new(&fixture_capacity()).unwrap();
    let only_two = Selection {
        states: &|state| state == 2,
        countries: &|_| false,
        any_country: false,
    };
    let read = walk(&m, 0, GS, &mut buffers, only_two);
    assert_eq!(
        read.outcomes,
        [
            Outcome::Rows { len: 1 },
            Outcome::Rows { len: 0 },
            Outcome::Rows { len: 0 },
            Outcome::Rows { len: 0 }
        ]
    );
    assert_eq!(buffers.inputs[0].state, 2);
    let m = fixture();
    let country = Selection {
        states: &|_| false,
        countries: &|country| country == 1,
        any_country: true,
    };
    let read = walk(&m, 0, GS, &mut buffers, country);
    assert_eq!(read.outcomes.map(|outcome| outcome.len()), [0, 0, 0, 2]);
    assert_eq!(buffers.overrides[0].country, 1);
    let one = Selection {
        states: &|state| state == 1,
        countries: &|_| false,
        any_country: false,
    };
    let read = walk(&m, 0, GS, &mut buffers, one);
    assert_eq!(read.outcomes.map(|outcome| outcome.len()), [1, 1, 1, 2]);
    assert_eq!(
        (buffers.inputs[0].origin, buffers.inputs[0].supply_route),
        (1, 1)
    );
    assert_eq!(buffers.read_countries, [1]);
    let alone = (
        buffers.inputs[0],
        buffers.resources[0],
        buffers.modifiers[0],
        buffers.overrides[..2].to_vec(),
    );
    walk(&m, 0, GS, &mut buffers, Selection::ALL);
    assert_eq!(
        (
            *buffers.inputs[..2]
                .iter()
                .find(|row| row.state == 1)
                .unwrap(),
            buffers.resources[0],
            buffers.modifiers[0],
            buffers.overrides[..2].to_vec(),
        ),
        alone
    );
    let read = walk(&m, 0, GS, &mut buffers, only_two);
    assert_eq!(read.outcomes.map(|outcome| outcome.len()), [1, 0, 0, 0]);
    assert!(buffers.read_countries.is_empty());
    let mut m = fixture();
    m.0.remove(&(STATE_A + cstate::RESOURCE_POOL));
    let read = walk(&m, 0, GS, &mut buffers, only_two);
    assert_eq!(read.outcomes.map(|outcome| outcome.len()), [1, 0, 0, 0]);
    assert_eq!(
        walk(&m, 0, GS, &mut buffers, one).outcomes[0],
        Outcome::Unavailable {
            reason: "state resource fields unreadable"
        }
    );
    let mut m = fixture();
    m.u64(STATE_B, 7);
    let read = walk(&m, 0, GS, &mut buffers, one);
    assert_eq!(read.outcomes.map(|outcome| outcome.len()), [1, 1, 1, 2]);
    assert_eq!(
        walk(&m, 0, GS, &mut buffers, country).outcomes[3],
        Outcome::Unavailable {
            reason: "state vtable mismatch"
        }
    );
}

#[test]
fn the_batched_resource_database_keeps_each_failure_and_its_order() {
    let reason = |m: &Mem| {
        let mut buffers = Buffers::new(&fixture_capacity()).unwrap();
        match walk(m, 0, GS, &mut buffers, Selection::ALL).outcomes[0] {
            Outcome::Unavailable { reason } => reason,
            other => panic!("{other:?}"),
        }
    };
    let keys = cstrategic_resource::STATE_ADDITIVE_KEY;
    let mut m = fixture();
    m.u32(0x4100_1100 + 4, 7);
    assert_eq!(
        reason(&m),
        "resource descriptor disagrees with database identity"
    );
    let mut m = fixture();
    m.0.remove(&(0x4100_3000 + keys + 4));
    assert_eq!(reason(&m), "resource modifier keys unreadable");
    m.u64(0x4100_4000, 7);
    assert_eq!(reason(&m), "resource descriptor vtable mismatch");
    let mut m = fixture();
    m.u64(0x4100_1000 + 16, 0);
    assert_eq!(reason(&m), "resource descriptor pointer unreadable or null");
}

struct Arena {
    bytes: Vec<u8>,
    base: usize,
    next: usize,
}

impl Arena {
    const MODULE: usize = 0x0340_0000;

    fn new(len: usize) -> Self {
        let bytes = vec![0; Self::MODULE + len];
        let base = bytes.as_ptr() as usize;
        Self {
            bytes,
            base,
            next: base + Self::MODULE,
        }
    }

    fn alloc(&mut self, len: usize) -> usize {
        let address = self.next.next_multiple_of(16);
        self.next = address + len;
        assert!(
            self.next <= self.base + self.bytes.len(),
            "bench arena full"
        );
        address
    }
}

impl Store for Arena {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        let at = address - self.base;
        self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn base(&self) -> usize {
        self.base
    }
}

const RESOURCES_PER_STATE: usize = 8;
const PROVINCES_PER_STATE: usize = 4;
const MODIFIERS_PER_STATE: usize = 5;
const DEFINITIONS: usize = 20;

fn scaled(countries: usize, states: usize) -> (Arena, usize) {
    scaled_with(countries, states, true)
}

fn scaled_with(countries: usize, states: usize, with_origins: bool) -> (Arena, usize) {
    let mut m = Arena::new(states * 0x1e00 + countries * 0x2400 + (1 << 20));
    let game_state = m.alloc(0x1000);
    let database = m.alloc(0x80);
    m.u64(
        m.base + cstrategic_resource_database::SINGLETON_RVA.0,
        database as u64,
    );
    let items = m.alloc(8 * (RESOURCES_PER_STATE + 1));
    let keys = m.alloc(4 * RESOURCES_PER_STATE);
    let null = m.alloc(0x100);
    m.pointers(
        database + cstrategic_resource_database::ITEMS,
        items,
        &[null],
    );
    m.array(
        database + cstrategic_resource_database::ITEMS,
        items,
        RESOURCES_PER_STATE as u32 + 1,
    );
    m.array(
        database + cstrategic_resource_database::KEYS,
        keys,
        RESOURCES_PER_STATE as u32,
    );
    for index in 0..RESOURCES_PER_STATE {
        let descriptor = m.alloc(0x100);
        let token = 1000 + index as u32;
        m.u64(items + (index + 1) * 8, descriptor as u64);
        m.u32(keys + index * 4, token);
        m.u64(descriptor, m.rva(cstrategic_resource::VTABLE_RVA));
        m.u32(descriptor + cstrategic_resource::TOKEN_ID, token);
        m.u8(descriptor + cstrategic_resource::ACTIVE, 1);
        m.u32(descriptor + cstrategic_resource::INDEX, index as u32 + 1);
        for offset in 0..4 {
            m.u32(
                descriptor + cstrategic_resource::STATE_ADDITIVE_KEY + offset * 4,
                0x100 + (index * 4 + offset) as u32,
            );
        }
    }
    let table = m.alloc(0x300 * modifier_definition::STRIDE);
    m.u64(m.base + modifier_definition::TABLE_RVA.0, table as u64);
    let buildings = m.alloc(0x400);
    let template = m.alloc(0x300);
    m.u64(
        m.base + cbuilding_database::SINGLETON_RVA.0,
        buildings as u64,
    );
    m.u64(buildings + INFRASTRUCTURE_TEMPLATE, template as u64);
    m.u8(template + cbuilding_template::LOOKUP_ENABLED_CANDIDATE, 1);
    let definitions: Vec<usize> = (0..DEFINITIONS)
        .map(|index| {
            let definition = m.alloc(0x1d0);
            m.u64(definition, m.rva(cdynamic_modifier::VTABLE_RVA));
            let name = format!("modifier_{index:02}\0");
            m.put(definition + cdynamic_modifier::ID, name.as_bytes());
            m.u64(
                definition + cdynamic_modifier::ID + 16,
                name.len() as u64 - 1,
            );
            m.u64(definition + cdynamic_modifier::ID + 24, 15);
            definition
        })
        .collect();
    let slots = countries + 1;
    let country_array = m.alloc(8 * slots);
    let links = m.alloc(4 * slots);
    m.array(
        game_state + cgame_state::COUNTRIES,
        country_array,
        slots as u32,
    );
    m.array(game_state + cgame_state::COUNTRY_LINKS, links, slots as u32);
    let root = m.alloc(0x200);
    let systems = m.alloc(8 * slots);
    let companions = m.alloc(supply_companion::STRIDE * slots);
    m.u64(game_state + cgame_state::SUPPLY_SYSTEM, root as u64);
    m.u64(root, m.rva(csupply_system::VTABLE_RVA));
    m.u64(root + 8, m.rva(csupply_system::SECONDARY_VTABLE_RVA));
    m.array(
        root + csupply_system::COUNTRY_SYSTEM_DATA,
        systems,
        slots as u32,
    );
    m.array(
        root + csupply_system::COUNTRY_COMPANION_DATA,
        companions,
        slots as u32,
    );
    let mut owned: Vec<Vec<usize>> = vec![Vec::new(); slots];
    for state in 1..=states {
        owned[1 + (state - 1) % countries].push(state);
    }
    let census = m.alloc(8);
    m.array(
        game_state + cgame_state::PROVINCES,
        census,
        ((states + 1) * PROVINCES_PER_STATE) as u32,
    );
    let state_array = m.alloc(8 * (states + 1));
    m.array(
        game_state + cgame_state::STATES,
        state_array,
        states as u32 + 1,
    );
    for (slot, members) in owned.iter().enumerate().skip(1) {
        let country = m.alloc(ccountry::RESOURCES + 8);
        let resources = m.alloc(ccountry_resources::BUILDING_RESOURCE_OVERRIDES + 0x10);
        m.u64(country_array + slot * 8, country as u64);
        m.u32(links + slot * 4, slot as u32);
        m.u64(country, m.rva(ccountry::VTABLE_RVA));
        m.u32(country + ccountry::ID, slot as u32);
        m.u64(country + ccountry::RESOURCES, resources as u64);
        m.u64(resources, m.rva(ccountry_resources::VTABLE_RVA));
        m.u32(resources + ccountry_resources::OWNER, slot as u32);
        if slot % 10 == 0 {
            let entry = m.alloc(ccountry_resources::OVERRIDE_STRIDE);
            let element = m.alloc(16);
            m.array(
                resources + ccountry_resources::BUILDING_RESOURCE_OVERRIDES,
                entry,
                1,
            );
            m.u32(entry, 777);
            m.array(entry + ccountry_resources::OVERRIDE_RESOURCES, element, 1);
            m.u32(element, 1);
            m.u64(element + 8, 50_000);
        }
        let system = m.alloc(0x100);
        let companion = companions + slot * supply_companion::STRIDE;
        m.u64(systems + slot * 8, system as u64);
        m.u64(system, m.rva(ccountry_supply_system::VTABLE_RVA));
        m.u64(
            system + 8,
            m.rva(ccountry_supply_system::SECONDARY_VTABLE_RVA),
        );
        m.u64(system + ccountry_supply_system::SUPPLY_SYSTEM, root as u64);
        m.u64(system + ccountry_supply_system::COUNTRY, country as u64);
        m.u64(system + ccountry_supply_system::COMPANION, companion as u64);
        let origins = m.alloc(8 * members.len());
        m.array(
            resources + ccountry_resources::RESOURCE_ORIGINS,
            origins,
            if with_origins {
                members.len() as u32
            } else {
                0
            },
        );
        let pairs = members.len() * PROVINCES_PER_STATE;
        let routes = m.alloc(8 * pairs);
        let nodes = m.alloc(engine_native_parts::supply_node_layout::STRIDE * pairs);
        m.array(
            companion + supply_companion::ROUTES_SUBTYPE_ZERO,
            routes,
            pairs as u32,
        );
        m.array(companion + supply_companion::NODES, nodes, pairs as u32);
        for (ordinal, &id) in members.iter().enumerate() {
            let pointer = m.alloc(0x900);
            let scratch = m.alloc(0x400);
            m.u64(state_array + id * 8, pointer as u64);
            state(&mut m, pointer, id as u32, slot as u32, scratch);
            let origin = m.alloc(0x90);
            m.u64(origins + ordinal * 8, origin as u64);
            m.u64(origin, m.rva(cresource_origin::VTABLE_RVA));
            m.u64(origin + cresource_origin::STATE, pointer as u64);
            let pool = m.alloc(16 * (RESOURCES_PER_STATE + 1));
            m.array(
                pointer + cstate::RESOURCE_POOL + 8,
                pool,
                RESOURCES_PER_STATE as u32 + 1,
            );
            for index in 0..RESOURCES_PER_STATE {
                let record = pool + (index + 1) * 16;
                m.u32(record + 8, 1000 + index as u32);
                if index < 3 {
                    m.u64(record, 100_000 * (index as u64 + 1));
                }
            }
            let entries = m.alloc(16 * 20);
            m.array(pointer + cstate::BASE_MODIFIERS, entries, 20);
            let keys = (0x50..0x60).chain([0x100, 0x101, 0x103, GLOBAL_FACTOR_KEY]);
            for (index, key) in keys.enumerate() {
                m.u32(entries + index * 16, key);
                m.u64(entries + index * 16 + 8, 1_000);
            }
            let status = pointer + cstate::BUILDING_STATUS;
            let indices = m.alloc(4);
            let list = m.alloc(8);
            let building = m.alloc(cbuilding::TEMPLATE + 8);
            m.array(status + cbuilding_status::BUILDING_INDICES, indices, 1);
            m.array(status + cbuilding_status::BUILDINGS, list, 1);
            m.u64(list, building as u64);
            m.u64(building + cbuilding::TEMPLATE, template as u64);
            m.put(building + cbuilding::LEVEL, &5_i16.to_le_bytes());
            let provinces = m.alloc(8 * PROVINCES_PER_STATE);
            m.array(
                pointer + cstate::PROVINCES,
                provinces,
                PROVINCES_PER_STATE as u32,
            );
            for index in 0..PROVINCES_PER_STATE {
                let id = (id * PROVINCES_PER_STATE + index) as u32;
                let province = m.alloc(cprovince::STRATEGIC_LOCATIONS + 16);
                m.u64(provinces + index * 8, province as u64);
                m.u64(province, m.rva(cprovince::VTABLE_RVA));
                m.u32(province + cprovince::ID, id);
                m.u64(province + cprovince::STATE, pointer as u64);
                m.array(province + cprovince::STRATEGIC_LOCATIONS, 0, 0);
                let pair = ordinal * PROVINCES_PER_STATE + index;
                m.u32(routes + pair * 8, id);
                m.u32(routes + pair * 8 + 4, pair as u32);
                let node = nodes + pair * engine_native_parts::supply_node_layout::STRIDE;
                m.u32(node, pair as u32);
                m.u32(node + 4, id);
                m.u64(node + supply_node::ROUTE_VALUE, (index / 3) as u64);
            }
            let container = m.alloc(dynamic_modifier_entry::STRIDE * MODIFIERS_PER_STATE);
            m.array(
                pointer + cstate::DYNAMIC_MODIFIERS + cdynamic_modifier_container::ENTRIES,
                container,
                MODIFIERS_PER_STATE as u32,
            );
            for index in 0..MODIFIERS_PER_STATE {
                let entry = container + index * dynamic_modifier_entry::STRIDE;
                let definition = definitions[(id + index) % DEFINITIONS];
                m.u64(entry, m.rva(dynamic_modifier_entry::VTABLE_RVA));
                m.u32(entry + dynamic_modifier_entry::DAYS_REMAINING, 30);
                m.u64(
                    entry + dynamic_modifier_entry::DEFINITION,
                    definition as u64,
                );
                m.u8(entry + dynamic_modifier_entry::ENABLED, 1);
            }
        }
    }
    (m, game_state)
}

fn grown_walk(memory: &impl Memory, arena: &Arena, game_state: usize) -> Buffers {
    let mut capacity = fixture_capacity();
    loop {
        let mut buffers = Buffers::new(&capacity).unwrap();
        let walk = walk(memory, arena.base, game_state, &mut buffers, Selection::ALL);
        if walk
            .outcomes
            .iter()
            .all(|o| matches!(o, Outcome::Rows { .. }))
        {
            return buffers;
        }
        assert!(
            !walk
                .outcomes
                .iter()
                .any(|o| matches!(o, Outcome::Unavailable { .. })),
            "{:?}",
            walk.outcomes
        );
        capacity.raise(&walk.required);
    }
}

#[test]
fn capacity_arithmetic_failure_is_allocation_failure() {
    let capacity = Capacity {
        resources: usize::MAX,
        ..Capacity::zero()
    };
    assert_eq!(
        Buffers::new(&capacity).err().unwrap().kind(),
        std::io::ErrorKind::OutOfMemory
    );
    let capacity = Capacity {
        definitions: usize::MAX,
        ..Capacity::zero()
    };
    assert_eq!(
        Buffers::new(&capacity).err().unwrap().kind(),
        std::io::ErrorKind::OutOfMemory
    );
}

fn fixture_capacity() -> Capacity {
    Capacity {
        states: 4096,
        resources: 32,
        resource_rows: 8192,
        modifier_rows: 4096,
        token_bytes: 1 << 17,
        override_rows: 1024,
        origins: 4096,
        routes: 1 << 16,
        provinces: 1024,
        entries: 1024,
        countries: 4096,
        definitions: 2048,
        name_bytes: 1 << 16,
        batch_bytes: 1 << 18,
    }
}

#[test]
fn manpower_without_state_certificate_adds_no_country_hash_probe() {
    let mut memory = fixture();
    memory.u64(0x4300_2000 + cresource_origin::STATE, STATE_B as u64);
    struct NoProbe<'a> {
        memory: &'a Mem,
        forbidden: [usize; 2],
    }
    impl Memory for NoProbe<'_> {
        fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
            assert!(
                !self.forbidden.contains(&address),
                "uncertified recruitment probe"
            );
            self.memory.copy(address, bytes)
        }
    }
    let forbidden = [STATE_A, STATE_B]
        .map(|state| read::<u64>(&memory, state + cstate::COUNTRY_MODIFIERS).unwrap() as usize + 4);
    let guarded = NoProbe {
        memory: &memory,
        forbidden,
    };
    let mut buffers = Buffers::new(&fixture_capacity()).unwrap();
    buffers.recruitment_enabled = false;
    let result = walk(&guarded, 0, GS, &mut buffers, Selection::ALL);
    assert_eq!(result.outcomes[0], Outcome::Rows { len: 2 });
    assert!(
        buffers.inputs[..2]
            .iter()
            .all(|row| row.recruitment.is_none())
    );
}
