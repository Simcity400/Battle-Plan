use super::*;
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
use crate::raid_launch_inputs::tests::Counted;
use crate::{cstrategic_resource_database, modifier_definition};
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

/// Fixture writes; `base` is where the module image starts.
trait Store {
    fn put(&mut self, address: usize, bytes: &[u8]);
    fn base(&self) -> usize;
    fn rva(&self, rva: usize) -> u64 {
        (self.base() + rva) as u64
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
    // An empty country-modifier hash: one bucket, nothing stored.
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
fn state_cores_read_each_selected_states_list_in_stored_order() {
    let mut m = fixture();
    m.array(STATE_A + cstate::CORES, 0x4640_0000, 2);
    m.u32(0x4640_0000, 42);
    m.u32(0x4640_0004, 5);
    m.array(STATE_B + cstate::CORES, 0, 0);
    let census = [1, 2];
    let run = |m: &Mem, selected: &dyn Fn(u32) -> bool, links: usize| {
        let mut rows = [CoreRow::default(); 2];
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
                len: 2
            },
            CoreRow {
                state: 2,
                start: 2,
                len: 0
            }
        ]
    );
    assert_eq!((&links[..2], required), (&[42, 5][..], 2));
    // Only the selected states are read.
    let (outcome, rows, _, required) = run(&m, &|state| state == 2, 4);
    assert_eq!(
        (outcome, rows[0].state, required),
        (Outcome::Rows { len: 1 }, 2, 0)
    );
    // A list that does not fit is counted, and the demand still reported.
    let (outcome, rows, _, required) = run(&m, &|_| true, 1);
    assert_eq!(
        outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert_eq!((rows[0].state, required), (2, 2));
    // A record that is not its census slot's state refuses the read.
    m.u32(STATE_B + cstate::ID, 7);
    assert!(matches!(
        run(&m, &|_| true, 4).0,
        Outcome::Unavailable { .. }
    ));
}

/// Base zero; state 1 is controlled by country 1, which has an origin
/// for it; state 2 has no controller.
fn fixture() -> Mem {
    let mut m = Mem::default();
    // Resource database: the null object, then tokens 101 and 102.
    let database = 0x4100_0000;
    m.u64(cstrategic_resource_database::SINGLETON_RVA, database as u64);
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
        m.u64(descriptor, cstrategic_resource::VTABLE_RVA as u64);
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
    // Modifier metadata: every key enabled, none clamped.
    let table = 0x4a00_0000;
    m.u64(modifier_definition::TABLE_RVA, table as u64);
    for key in [0x10_usize, 0x13, 0x223, 0x292] {
        let row = table + key * modifier_definition::STRIDE;
        m.u32(row + modifier_definition::FEATURE_SELECTOR, 0);
        m.u32(row + modifier_definition::FLAGS, 0);
    }
    // Country 1 and its resources.
    m.pointers(GS + cgame_state::COUNTRIES, 0x4200_0000, &[0, COUNTRY]);
    m.array(GS + cgame_state::COUNTRY_LINKS, 0x4200_0100, 2);
    m.u32(0x4200_0100, 0);
    m.u32(0x4200_0104, 1);
    m.u64(COUNTRY, ccountry::VTABLE_RVA as u64);
    m.u32(COUNTRY + ccountry::ID, 1);
    m.u64(COUNTRY + ccountry::RESOURCES, RESOURCES as u64);
    m.u64(RESOURCES, ccountry_resources::VTABLE_RVA as u64);
    m.u32(RESOURCES + ccountry_resources::OWNER, 1);
    m.pointers(
        RESOURCES + ccountry_resources::RESOURCE_ORIGINS,
        0x4300_1000,
        &[0x4300_2000],
    );
    m.put(0x4300_2000, &[0; 0x88]);
    m.u64(0x4300_2000, cresource_origin::VTABLE_RVA as u64);
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
    // Supply: provinces 7 and 9 map to nodes 0 and 1; node 1 has a route.
    let root = 0x4400_0000;
    m.u64(GS + cgame_state::SUPPLY_SYSTEM, root as u64);
    m.u64(root, csupply_system::VTABLE_RVA as u64);
    m.u64(root + 8, csupply_system::SECONDARY_VTABLE_RVA as u64);
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
    m.u64(system, ccountry_supply_system::VTABLE_RVA as u64);
    m.u64(
        system + 8,
        ccountry_supply_system::SECONDARY_VTABLE_RVA as u64,
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
        let node = 0x4400_6000 + index * supply_node::STRIDE;
        m.put(node, &[0; supply_node::STRIDE]);
        m.u32(node, index as u32);
        m.u32(node + 4, province);
        m.u64(node + supply_node::ROUTE_VALUE, index as u64 * 5);
    }
    m.array(COMPANION + supply_companion::NODES, 0x4400_6000, 2);
    // Infrastructure: database ordinal 0.
    m.u64(cbuilding_database::SINGLETON_RVA, 0x4500_0000);
    m.u64(0x4500_0000 + INFRASTRUCTURE_TEMPLATE, 0x4500_1000);
    m.u8(
        0x4500_1000 + cbuilding_template::LOOKUP_ENABLED_CANDIDATE,
        1,
    );
    m.u32(0x4500_1000 + cbuilding_template::DATABASE_ORDINAL, 0);
    // States.
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
        m.put(pointer, &[0; 0xa8]);
        m.u64(pointer, cprovince::VTABLE_RVA as u64);
        m.u32(pointer + cprovince::ID, id);
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
    // One unscoped permanent modifier named "refinery".
    let container = STATE_A + cstate::DYNAMIC_MODIFIERS;
    m.array(
        container + cdynamic_modifier_container::ENTRIES,
        0x4630_4000,
        1,
    );
    let entry = 0x4630_4000;
    m.put(entry, &[0; dynamic_modifier_entry::STRIDE]);
    m.u64(entry, dynamic_modifier_entry::VTABLE_RVA as u64);
    m.u32(entry + dynamic_modifier_entry::SCOPE_COUNTRY, 0);
    m.u32(entry + dynamic_modifier_entry::SCOPE_STATE, 0);
    m.u32(entry + dynamic_modifier_entry::DAYS_REMAINING, u32::MAX);
    m.u64(entry + dynamic_modifier_entry::DEFINITION, 0x4900_0000);
    m.u8(entry + dynamic_modifier_entry::ENABLED, 1);
    m.array(entry + dynamic_modifier_entry::VALUES, 0, 0);
    m.put(0x4900_0000, &[0; 0x48]);
    m.u64(0x4900_0000, cdynamic_modifier::VTABLE_RVA as u64);
    m.put(0x4900_0000 + cdynamic_modifier::ID, b"refinery\0");
    m.u64(0x4900_0000 + cdynamic_modifier::ID + 16, 8);
    m.u64(0x4900_0000 + cdynamic_modifier::ID + 24, 15);
    m
}

#[test]
fn reads_every_stored_operand_and_the_state_container() {
    let m = fixture();
    let mut buffers = Buffers::new(&Capacity::default()).unwrap();
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
        0x4400_6000 + supply_node::STRIDE + supply_node::ROUTE_VALUE,
        0,
    );
    let mut buffers = Buffers::new(&Capacity::default()).unwrap();
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
fn short_buffers_truncate_and_report_live_sizes() {
    let m = fixture();
    let small = Capacity {
        token_bytes: 4,
        override_rows: 1,
        ..Capacity::default()
    };
    let mut buffers = Buffers::new(&small).unwrap();
    let first = walk(&m, 0, GS, &mut buffers, Selection::ALL);
    let truncated = Outcome::Truncated {
        len: 0,
        unread_rows: 1,
    };
    assert_eq!(first.outcomes[2], truncated);
    assert_eq!(first.outcomes[3], truncated);
    assert_eq!(
        (first.required.token_bytes, first.required.override_rows),
        (8, 2)
    );
    let mut grown = small;
    grown.raise(&first.required);
    let mut buffers = Buffers::new(&grown).unwrap();
    let second = walk(&m, 0, GS, &mut buffers, Selection::ALL);
    assert!(
        second
            .outcomes
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Rows { .. }))
    );
}

#[test]
fn a_foreign_container_or_torn_pool_fails_the_whole_walk() {
    let mut m = fixture();
    m.u32(
        STATE_A + cstate::DYNAMIC_MODIFIERS + cdynamic_modifier_container::STATE,
        9,
    );
    let mut buffers = Buffers::new(&Capacity::default()).unwrap();
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
    // State 1's container and country 1's overrides are foreign or torn:
    // only a walk that reads them fails.
    let mut m = fixture();
    m.u32(
        STATE_A + cstate::DYNAMIC_MODIFIERS + cdynamic_modifier_container::STATE,
        9,
    );
    m.0.remove(&0x4300_3000);
    let mut buffers = Buffers::new(&Capacity::default()).unwrap();
    let only_two = Selection {
        states: &|state| state == 2,
        countries: &|_| false,
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
    // Country 1's overrides alone, from an intact image.
    let m = fixture();
    let country = Selection {
        states: &|_| false,
        countries: &|country| country == 1,
    };
    let read = walk(&m, 0, GS, &mut buffers, country);
    assert_eq!(read.outcomes.map(|outcome| outcome.len()), [0, 0, 0, 2]);
    assert_eq!(buffers.overrides[0].country, 1);
    // State 1 alone keeps its controller's origin but not its overrides.
    let one = Selection {
        states: &|state| state == 1,
        countries: &|_| false,
    };
    let read = walk(&m, 0, GS, &mut buffers, one);
    assert_eq!(read.outcomes.map(|outcome| outcome.len()), [1, 1, 1, 0]);
    assert_eq!(
        (buffers.inputs[0].origin, buffers.inputs[0].supply_route),
        (1, 1)
    );
}

#[test]
fn the_batched_resource_database_keeps_each_failure_and_its_order() {
    let reason = |m: &Mem| {
        let mut buffers = Buffers::new(&Capacity::default()).unwrap();
        match walk(m, 0, GS, &mut buffers, Selection::ALL).outcomes[0] {
            Outcome::Unavailable { reason } => reason,
            other => panic!("{other:?}"),
        }
    };
    let keys = cstrategic_resource::STATE_ADDITIVE_KEY;
    // Item 2's token disagrees with its database key.
    let mut m = fixture();
    m.u32(0x4100_1100 + 4, 7);
    assert_eq!(
        reason(&m),
        "resource descriptor disagrees with database identity"
    );
    // Item 1's keys are unreadable: met after every identity.
    let mut m = fixture();
    m.0.remove(&(0x4100_3000 + keys + 4));
    assert_eq!(reason(&m), "resource modifier keys unreadable");
    m.u64(0x4100_4000, 7);
    assert_eq!(reason(&m), "resource descriptor vtable mismatch");
    // A null item.
    let mut m = fixture();
    m.u64(0x4100_1000 + 16, 0);
    assert_eq!(reason(&m), "resource descriptor pointer unreadable or null");
}

/// One flat buffer: the module image, then bump-allocated objects at
/// their real addresses, so `LiveMemory` reads it with guarded copies.
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

/// `countries` countries controlling `states` states round-robin, each
/// state with an origin, infrastructure, a supply route, eight resources,
/// twenty accumulator entries and five dynamic modifiers.
fn scaled(countries: usize, states: usize) -> (Arena, usize) {
    scaled_with(countries, states, true)
}

/// [`scaled`], optionally without any resource origin.
fn scaled_with(countries: usize, states: usize, with_origins: bool) -> (Arena, usize) {
    let mut m = Arena::new(states * 0x1800 + countries * 0x2400 + (1 << 20));
    let game_state = m.alloc(0x1000);
    // Resources: tokens 1000 + i, keys 0x100 + 4 i .. + 3.
    let database = m.alloc(0x80);
    m.u64(
        m.base + cstrategic_resource_database::SINGLETON_RVA,
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
    // Modifier metadata: every key enabled, none clamped.
    let table = m.alloc(0x300 * modifier_definition::STRIDE);
    m.u64(m.base + modifier_definition::TABLE_RVA, table as u64);
    // Infrastructure template, ordinal 0.
    let buildings = m.alloc(0x400);
    let template = m.alloc(0x300);
    m.u64(m.base + cbuilding_database::SINGLETON_RVA, buildings as u64);
    m.u64(buildings + INFRASTRUCTURE_TEMPLATE, template as u64);
    m.u8(template + cbuilding_template::LOOKUP_ENABLED_CANDIDATE, 1);
    // Shared dynamic modifier definitions.
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
    // Countries 1..=countries, raw id = slot; supply systems and companions.
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
        // Every tenth country has one override entry with one element.
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
        let nodes = m.alloc(supply_node::STRIDE * pairs);
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
            // Pool: eight records, three nonzero.
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
            // Twenty sorted accumulator entries, four of them read.
            let entries = m.alloc(16 * 20);
            m.array(pointer + cstate::BASE_MODIFIERS, entries, 20);
            let keys = (0x50..0x60).chain([0x100, 0x101, 0x103, GLOBAL_FACTOR_KEY]);
            for (index, key) in keys.enumerate() {
                m.u32(entries + index * 16, key);
                m.u64(entries + index * 16 + 8, 1_000);
            }
            // Infrastructure level 5 at building index 0.
            let status = pointer + cstate::BUILDING_STATUS;
            let indices = m.alloc(4);
            let list = m.alloc(8);
            let building = m.alloc(cbuilding::TEMPLATE + 8);
            m.array(status + cbuilding_status::BUILDING_INDICES, indices, 1);
            m.array(status + cbuilding_status::BUILDINGS, list, 1);
            m.u64(list, building as u64);
            m.u64(building + cbuilding::TEMPLATE, template as u64);
            m.put(building + cbuilding::LEVEL, &5_i16.to_le_bytes());
            // Provinces, each with a route node; the last node carries a value.
            let provinces = m.alloc(8 * PROVINCES_PER_STATE);
            m.array(
                pointer + cstate::PROVINCES,
                provinces,
                PROVINCES_PER_STATE as u32,
            );
            for index in 0..PROVINCES_PER_STATE {
                let id = (id * PROVINCES_PER_STATE + index) as u32;
                let province = m.alloc(cprovince::ID + 4);
                m.u64(provinces + index * 8, province as u64);
                m.u64(province, m.rva(cprovince::VTABLE_RVA));
                m.u32(province + cprovince::ID, id);
                let pair = ordinal * PROVINCES_PER_STATE + index;
                m.u32(routes + pair * 8, id);
                m.u32(routes + pair * 8 + 4, pair as u32);
                let node = nodes + pair * supply_node::STRIDE;
                m.u32(node, pair as u32);
                m.u32(node + 4, id);
                m.u64(node + supply_node::ROUTE_VALUE, (index / 3) as u64);
            }
            // Five dynamic modifiers.
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

/// `cargo test --release -p engine raid_state_inputs_bench -- --ignored --nocapture`
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
#[test]
#[ignore]
fn raid_state_inputs_bench() {
    for (countries, states) in [(1_500, 3_000), (5_000, 10_000)] {
        let (arena, game_state) = scaled(countries, states);
        // SAFETY: immutable synthetic storage owned by this test.
        let memory = Counted::new(unsafe { crate::memory::LiveMemory::new() });
        let mut capacity = Capacity::default();
        let mut buffers = Buffers::new(&capacity).unwrap();
        loop {
            let walk = walk(
                &memory,
                arena.base,
                game_state,
                &mut buffers,
                Selection::ALL,
            );
            if walk
                .outcomes
                .iter()
                .all(|o| matches!(o, Outcome::Rows { .. }))
            {
                break;
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
            buffers = Buffers::new(&capacity).unwrap();
        }
        let mut times = Vec::new();
        let mut last = None;
        for _ in 0..20 {
            memory.reset();
            let started = std::time::Instant::now();
            let walk = walk(
                &memory,
                arena.base,
                game_state,
                &mut buffers,
                Selection::ALL,
            );
            times.push(started.elapsed().as_secs_f64() * 1_000.0);
            last = Some(walk);
        }
        times.sort_by(f64::total_cmp);
        let walk = last.unwrap();
        let rows = walk.outcomes.map(Outcome::len);
        assert_eq!(rows[0], states);
        assert!(buffers.inputs[..states].iter().all(|row| row.origin == 1
            && row.infrastructure_level == Some(5)
            && row.supply_route == 1));
        println!(
            "{countries} countries, {states} states: median {:.3} ms (min {:.3}, max {:.3}); \
             {} guarded calls, {} bytes; rows: inputs {}, resources {}, modifiers {}, overrides {}",
            times[10],
            times[0],
            times[19],
            memory.guards.get(),
            memory.bytes.get(),
            rows[0],
            rows[1],
            rows[2],
            rows[3],
        );
    }
}

/// Walk buffers grown to a complete walk of this image.
fn grown_walk(memory: &impl Memory, arena: &Arena, game_state: usize) -> Buffers {
    let mut capacity = Capacity::default();
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

/// A state hour as the collector runs it: the state census, then the
/// walk of one daily slice of states and countries (the first hour of a
/// session reads every owner). Cold: a full walk of another image first.
/// `cargo test --release -p engine raid_state_hour_bench -- --ignored --nocapture`
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
#[test]
#[ignore]
fn raid_state_hour_bench() {
    for (countries, states) in [(1_500, 3_000), (5_000, 10_000)] {
        let images = [scaled(countries, states), scaled(countries, states)];
        // SAFETY: immutable synthetic storage owned by this test.
        let memory = Counted::new(unsafe { crate::memory::LiveMemory::new() });
        let mut buffers = grown_walk(&memory, &images[0].0, images[0].1);
        let mut census = Vec::with_capacity(states + 1);
        let slice = |id: u32| id % 24 == 7;
        let every = |_: u32| true;
        for (label, chosen) in [
            ("first hour (every owner)", &every as &dyn Fn(u32) -> bool),
            ("steady hour (one slice)", &slice),
        ] {
            let select = Selection {
                states: chosen,
                countries: chosen,
            };
            let mut run = |image: usize, buffers: &mut Buffers| {
                let (arena, game_state) = &images[image];
                memory.reset();
                let started = std::time::Instant::now();
                state_census(&memory, *game_state, &mut census).unwrap();
                let walked = walk(&memory, arena.base, *game_state, buffers, select);
                let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
                assert!(
                    walked
                        .outcomes
                        .iter()
                        .all(|o| matches!(o, Outcome::Rows { .. }))
                );
                assert_eq!(census.len(), states);
                (elapsed, memory.guards.get(), memory.bytes.get())
            };
            let mut warm = Vec::new();
            let mut cold = Vec::new();
            let mut counts = (0, 0);
            run(0, &mut buffers);
            for _ in 0..20 {
                let (ms, calls, bytes) = run(0, &mut buffers);
                warm.push(ms);
                counts = (calls, bytes);
            }
            for index in 0..20 {
                let image = index % 2;
                let (arena, game_state) = &images[1 - image];
                walk(
                    &memory,
                    arena.base,
                    *game_state,
                    &mut buffers,
                    Selection::ALL,
                );
                cold.push(run(image, &mut buffers).0);
            }
            warm.sort_by(f64::total_cmp);
            cold.sort_by(f64::total_cmp);
            println!(
                "state {label} {countries} countries, {states} states: warm {:.3} ms, cold {:.3} ms                      (medians of 20); {} guarded calls, {} bytes",
                warm[10], cold[10], counts.0, counts.1,
            );
        }
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
