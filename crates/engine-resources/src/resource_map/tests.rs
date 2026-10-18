use super::*;
use crate::{ccountry_resources::test_support::Fixture, cresource_exchange as exchange};

const GAME_STATE: usize = 0x1000;
const THRESHOLD: i64 = 80_000;

fn state(f: &mut Fixture, address: usize, id: u32) {
    f.u32(address + cstate::ID, id);
    // An empty country-modifier hash: the base accumulator, with no entries.
    f.u64(address + cstate::COUNTRY_MODIFIERS, 0x90000);
    f.u32(address + cstate::COUNTRY_MODIFIERS + 8, 0);
    f.u32(address + cstate::COUNTRY_MODIFIERS + 12, 0);
    f.put(address + cstate::COUNTRY_MODIFIERS + 16, &[0]);
    f.put(0x90000, &[0; 16]);
    f.array(address + cstate::BASE_MODIFIERS, 0, 0);
}

fn client(f: &mut Fixture, address: usize, vtable: usize, handle: u32, efficiency: [i64; 2]) {
    f.u64(address, vtable as u64);
    f.u32(address + CLIENT_CONVOYS, handle / 10);
    f.u32(address + CLIENT_CONVOYS + 4, handle / 10 + 1);
    f.u32(address + CLIENT_HANDLE_TYPE, CLIENT_TYPE);
    f.u32(address + CLIENT_HANDLE_ID, handle);
    f.u64(address + CLIENT_EFFICIENCY_A, efficiency[0] as u64);
    f.u64(address + CLIENT_EFFICIENCY_B, efficiency[1] as u64);
}

fn route(f: &mut Fixture, address: usize, kind: u8, ends: [usize; 2], path: &[usize]) {
    f.put(address, &[0; 0x70]);
    f.u64(address, ROUTE_VTABLE_RVA as u64);
    f.put(address + ROUTE_KIND, &[kind]);
    let (ends_at, list) = if kind == NAVAL {
        (ROUTE_FROM_PORT, ROUTE_SEA_REGIONS)
    } else {
        (ROUTE_FROM, ROUTE_LAND_STATES)
    };
    f.u64(address + ends_at, ends[0] as u64);
    f.u64(address + ends_at + 8, ends[1] as u64);
    let data = address + 0x100;
    f.array(
        address + list,
        if path.is_empty() { 0 } else { data },
        path.len() as u32,
    );
    for (index, &item) in path.iter().enumerate() {
        f.u64(data + index * 8, item as u64);
    }
}

/// Two countries (raw 7 at slot 1, raw 2 at slot 2): country 7 owns a state's origin, gives
/// one resource right to country 2 and sells to it on the market; country 2 imports from
/// country 7 and has one naval supply transfer.
fn world() -> Fixture {
    let mut f = Fixture::new();
    f.u64(BAD_EFFICIENCY_THRESHOLD_RVA, THRESHOLD as u64);
    for (country, resources) in [(0x2000, 0x6000), (0x4000, 0x7000)] {
        f.array(country + ccountry::MODIFIER_ACCUMULATOR, 0, 0);
        for list in [
            ccountry_resources::RESOURCE_ORIGINS,
            RIGHTS_ORIGINS,
            LEND_LEASE_GIVEN,
            LEND_LEASE_RECEIVED,
        ] {
            f.array(resources + list, 0, 0);
        }
    }
    state(&mut f, 0x30000, 101);
    state(&mut f, 0x31000, 102);
    // Country 7's origin of state 101: a bad production line on a naval route.
    f.array(0x6000 + ccountry_resources::RESOURCE_ORIGINS, 0x20800, 1);
    f.u64(0x20800, 0x20000);
    client(&mut f, 0x20000, ORIGIN_VTABLE_RVA, 41, [100_000, 70_000]);
    f.u64(0x20000 + ORIGIN_STATE, 0x30000);
    f.u64(0x20000 + ORIGIN_EFFICIENCY, 70_000);
    f.u64(0x20000 + ORIGIN_ROUTE_FACTOR, 110_000);
    f.array(0x20000 + ORIGIN_RIGHTS, 0x20900, 1);
    f.u32(0x20900, 3);
    f.u32(0x20904, 2);
    f.u32(0x32000 + PROVINCE_ID, 11);
    f.u32(0x33000 + PROVINCE_ID, 12);
    f.u32(0x34000 + REGION_ID, 501);
    f.u32(0x35000 + REGION_ID, 502);
    route(
        &mut f,
        0x20000 + ORIGIN_ROUTE,
        NAVAL,
        [0x32000, 0x33000],
        &[0x34000, 0x35000],
    );
    // Country 7's delivery route for country 2 (slot 2): a land route.
    f.array(0x6000 + DELIVERY_ROUTES, 0x40800, 3);
    f.put(0x40800, &[0; 24]);
    f.u64(0x40810, 0x40000);
    route(&mut f, 0x40000, 1, [0x30000, 0x31000], &[0x30000, 0x31000]);
    // Country 2's import from country 7 (the fixture's exchange).
    f.u32(0xa000 + exchange::REFERENCE, CLIENT_TYPE);
    f.u32(0xa000 + exchange::REFERENCE + 4, 0x33);
    f.u32(0xa000 + exchange::CONVOYS_ASSIGNED, 0);
    f.u32(0xa000 + exchange::CONVOYS_REQUIRED, 1);
    // Country 2's supply transfer: its embedded route draws nothing.
    f.u64(GAME_STATE + cgame_state::SUPPLY_SYSTEM, 0x50000);
    f.u64(0x50000 + SUPPLY_COUNTRY_SYSTEMS, 0x50800);
    f.put(0x50800, &[0; 24]);
    f.u64(0x50810, 0x51000);
    f.array(0x51000 + SUPPLY_TRANSFERS, 0x51800, 1);
    f.put(0x51800, &[0; SUPPLY_TRANSFER_STRIDE]);
    f.u64(0x51800 + SUPPLY_TRANSFER_CLIENT, 0x52000);
    client(&mut f, 0x52000, SUPPLY_VTABLE_RVA, 77, [100_000, 100_000]);
    route(&mut f, 0x52000 + SUPPLY_ROUTE, 0, [0, 0], &[]);
    // A market contract from country 7 to country 2 on the same delivery route.
    f.u64(GAME_STATE + cgame_state::MARKET_MANAGER, 0x60000);
    f.array(0x60000 + MARKET_OWNED_CONTRACTS, 0x60800, 1);
    f.u64(0x60800, 0x61000);
    f.u32(0x61000 + CONTRACT_SELLER, 7);
    f.u32(0x61000 + CONTRACT_BUYER, 2);
    client(
        &mut f,
        0x61000 + CONTRACT_CLIENT,
        CONTRACT_CLIENT_VTABLE_RVA,
        88,
        [100_000, 50_000],
    );
    f.u64(0x61000 + CONTRACT_CLIENT + CONTRACT_CLIENT_ROUTE, 0x40000);
    for (country, status) in [(0x2000, 0x62000), (0x4000, 0x62200)] {
        f.u64(country + ccountry::EQUIPMENT_MARKET_STATUS, status);
        f.u64(status as usize + MARKET_STATUS_MANAGER, 0x60000);
    }
    for (table, data) in [(MARKET_SELLERS, 0x63000), (MARKET_BUYERS, 0x64000)] {
        f.array(0x60000 + table, data, 3);
        for slot in 0..3 {
            f.array(data + slot * MARKET_COUNTRY_STRIDE, 0, 0);
        }
    }
    f.array(0x63000 + MARKET_COUNTRY_STRIDE, 0x65000, 1);
    f.u64(0x65000, 0x61000);
    f.array(0x64000 + 2 * MARKET_COUNTRY_STRIDE, 0x65010, 1);
    f.u64(0x65010, 0x61000);

    f
}

struct Rows {
    origins: Vec<OriginRow>,
    rights: Vec<RightsRow>,
    oil: Vec<OilRow>,
    clients: Vec<ClientRow>,
}

fn walk(memory: &Fixture) -> (Outcomes, Rows) {
    let mut rows = Rows {
        origins: Vec::new(),
        rights: Vec::new(),
        oil: Vec::new(),
        clients: Vec::new(),
    };
    let outcomes = walk_buffer(
        memory,
        0,
        GAME_STATE,
        Outputs {
            origins: Buffer::Growing(&mut rows.origins),
            rights: Buffer::Growing(&mut rows.rights),
            oil: Buffer::Growing(&mut rows.oil),
            clients: Buffer::Growing(&mut rows.clients),
        },
    );
    (outcomes, rows)
}

#[test]
fn every_client_names_its_route_and_whether_its_line_is_bad() {
    let f = world();
    let (outcomes, rows) = walk(&f);
    assert_eq!(
        outcomes,
        Outcomes {
            origins: Outcome::Rows { len: 1 },
            rights: Outcome::Rows { len: 1 },
            oil: Outcome::Rows { len: 0 },
            clients: Outcome::Rows { len: 3 },
        }
    );
    // The icon's stored product, not A × B; the line reads A × B against the threshold.
    assert_eq!(
        rows.origins[..1],
        [OriginRow {
            ordinal: 0,
            state: 101,
            country: 7,
            list: 0,
            efficiency: 70_000,
            route_factor: 110_000,
            local_resources_factor: 0,
            bad: true,
            convoys: [4, 5],
            route_address: 0x20000 + ORIGIN_ROUTE,
        }]
    );
    assert_eq!(
        rows.rights[..1],
        [RightsRow {
            ordinal: 0,
            state: 101,
            resource: 3,
            holder: 2,
        }]
    );
    let delivery = RouteKey {
        class: DELIVERY_ROUTE_CLASS,
        country: 7,
        other: 2,
    };
    assert_eq!(
        rows.clients[..3],
        [
            ClientRow {
                address: 0xa000,
                owner_ordinal: None,
                other_ordinal: None,
                handle: 0x33,
                category: IMPORT,
                owner: 2,
                other: 7,
                route: delivery,
                bad: false,
                convoys: [0, 1],
                route_address: 0x40000,
            },
            ClientRow {
                address: 0x52000,
                owner_ordinal: Some(0),
                other_ordinal: None,
                handle: 77,
                category: SUPPLY_TRANSFER,
                owner: 2,
                other: 0,
                route: RouteKey {
                    class: SUPPLY_ROUTE_CLASS,
                    country: 2,
                    other: 77,
                },
                bad: false,
                convoys: [7, 8],
                route_address: 0x52000 + SUPPLY_ROUTE,
            },
            ClientRow {
                address: 0x61000 + CONTRACT_CLIENT,
                owner_ordinal: Some(0),
                other_ordinal: Some(0),
                handle: 88,
                category: MARKET,
                owner: 7,
                other: 2,
                route: delivery,
                bad: true,
                convoys: [8, 9],
                route_address: 0x40000,
            },
        ]
    );
}

#[test]
fn routes_draw_ports_and_regions_by_sea_and_states_by_land() {
    let f = world();
    let mut path = Vec::new();
    let key = RouteKey {
        class: ORIGIN_ROUTE_CLASS,
        country: 7,
        other: 101,
    };
    let naval = read_route(&f, 0, key, 0x20000 + ORIGIN_ROUTE, None, &mut path).unwrap();
    assert_eq!(
        (naval.kind, naval.from, naval.to, naval.path_len),
        (NAVAL, 11, 12, 2)
    );
    let land = read_route(&f, 0, key, 0x40000, None, &mut path).unwrap();
    assert_eq!(
        (land.kind, land.from, land.to, land.path_len),
        (1, 101, 102, 2)
    );
    assert_eq!(path, [501, 502, 101, 102]);
    // A route with no ends or no path draws nothing.
    let none = read_route(&f, 0, key, 0x52000 + SUPPLY_ROUTE, None, &mut path).unwrap();
    assert_eq!((none.kind, none.from, none.to, none.path_len), (0, 0, 0, 0));
    assert_eq!(path.len(), 4);
}

#[test]
fn a_client_of_an_unexpected_type_refuses_the_tables_by_name() {
    let mut f = world();
    f.u64(0x52000, ROUTE_VTABLE_RVA as u64);
    let (outcomes, _) = walk(&f);
    assert_eq!(
        outcomes.clients,
        Outcome::Unavailable {
            reason: "convoy client of an unexpected type"
        }
    );
    // The origins, rights and oil factors do not read the clients.
    assert_eq!(outcomes.origins, Outcome::Rows { len: 1 });
    assert_eq!(outcomes.rights, Outcome::Rows { len: 1 });
    assert_eq!(outcomes.oil, Outcome::Rows { len: 0 });
}

#[test]
fn an_unreadable_origin_refuses_the_origins_and_rights_alone() {
    let mut f = world();
    f.u64(0x20000, ROUTE_VTABLE_RVA as u64);
    let (outcomes, rows) = walk(&f);
    assert_eq!(
        outcomes.origins,
        Outcome::Unavailable {
            reason: "origin vtable mismatch"
        }
    );
    assert_eq!(outcomes.rights, outcomes.origins);
    assert_eq!(outcomes.clients, Outcome::Rows { len: 3 });
    assert_eq!(rows.clients.len(), 3);
}

#[test]
fn an_unreadable_oil_factor_refuses_the_oil_factors_alone() {
    let mut f = world();
    f.bytes.retain(|&address, _| {
        !(0x2000 + ccountry::MODIFIER_ACCUMULATOR..0x2010 + ccountry::MODIFIER_ACCUMULATOR)
            .contains(&address)
    });
    let (outcomes, _) = walk(&f);
    assert!(matches!(outcomes.oil, Outcome::Unavailable { .. }));
    assert_eq!(outcomes.origins, Outcome::Rows { len: 1 });
    assert_eq!(outcomes.clients, Outcome::Rows { len: 3 });
}

#[test]
fn an_import_whose_exporter_resources_are_not_the_exporters_own_is_refused() {
    let mut f = world();
    // A copy of country 7's resources that names country 7 as its owner, but that country 7
    // does not hold: its owner word alone used to be enough.
    f.u64(0x6800, ccountry_resources::VTABLE_RVA as u64);
    f.u32(0x6800 + ccountry_resources::OWNER, 7);
    f.array(0x6800 + DELIVERY_ROUTES, 0x40800, 3);
    f.u64(0xa000 + exchange::EXPORTER_RESOURCES, 0x6800);
    let (outcomes, _) = walk(&f);
    assert_eq!(
        outcomes.clients,
        Outcome::Unavailable {
            reason: "trade exporter resources backlink mismatch"
        }
    );
    // An exchange in country 2's list whose importer is another country is refused too.
    let mut f = world();
    f.u32(0xa000 + exchange::IMPORTER, 7);
    let (outcomes, _) = walk(&f);
    assert!(matches!(outcomes.clients, Outcome::Unavailable { .. }));
}

#[test]
fn a_rights_holder_origin_reads_its_own_route_and_line() {
    let mut f = world();
    // Country 2 holds state 102's origin by rights, on a land route.
    f.array(0x7000 + RIGHTS_ORIGINS, 0x21800, 1);
    f.put(0x21800, &[0; RIGHTS_ORIGIN_STRIDE]);
    f.u64(0x21800, 0x21000);
    client(&mut f, 0x21000, ORIGIN_VTABLE_RVA, 55, [100_000, 60_000]);
    f.u64(0x21000 + ORIGIN_STATE, 0x31000);
    f.u64(0x21000 + ORIGIN_EFFICIENCY, 60_000);
    f.u64(0x21000 + ORIGIN_ROUTE_FACTOR, 100_000);
    f.array(0x21000 + ORIGIN_RIGHTS, 0, 0);
    route(
        &mut f,
        0x21000 + ORIGIN_ROUTE,
        1,
        [0x31000, 0x30000],
        &[0x31000, 0x30000],
    );
    let (outcomes, rows) = walk(&f);
    assert_eq!(outcomes.origins, Outcome::Rows { len: 2 });
    let held = rows.origins.iter().find(|row| row.list == 1).unwrap();
    assert_eq!(
        (held.state, held.country, held.bad, held.route_address),
        (102, 2, true, 0x21000 + ORIGIN_ROUTE)
    );
}

#[test]
fn a_line_is_bad_only_below_the_threshold() {
    let mut f = world();
    for (factors, bad) in [
        ([100_000, 80_000], false),
        ([100_000, 79_999], true),
        ([-100_000, 100_000], true),
    ] {
        f.u64(0x52000 + CLIENT_EFFICIENCY_A, factors[0] as u64);
        f.u64(0x52000 + CLIENT_EFFICIENCY_B, factors[1] as u64);
        assert_eq!(client_bad(&f, 0x52000, THRESHOLD), Ok(bad), "{factors:?}");
    }
}

#[test]
fn only_a_gated_state_is_read_for_temporary_resources() {
    let mut f = world();
    let descriptors = f.descriptors();
    f.u32(0xd000 + cstrategic_resource::TEMPORARY_KEY, 0x55);
    f.array(GAME_STATE + cgame_state::STATES, 0x70000, 3);
    f.put(0x70000, &[0; 24]);
    f.u64(0x70008, 0x30000);
    f.u64(0x70010, 0x31000);
    for state in [0x30000, 0x31000] {
        f.u32(state + STATE_TEMPORARY_LIST_COUNT, 0);
        f.put(state + STATE_TEMPORARY_FLAG, &[0]);
    }
    let read = |f: &Fixture| {
        let mut rows = Vec::new();
        let outcome = walk_temporary(f, 0, GAME_STATE, &descriptors, Buffer::Growing(&mut rows));
        (outcome, rows.len())
    };
    // An ungated state's modifiers are not read, readable or not.
    f.bytes.retain(|&address, _| {
        !(0x31000 + cstate::BASE_MODIFIERS..0x31010 + cstate::BASE_MODIFIERS).contains(&address)
    });
    assert_eq!(read(&f), (Outcome::Rows { len: 0 }, 0));
    // A gated state is read: its unreadable modifiers refuse the table.
    f.put(0x31000 + STATE_TEMPORARY_FLAG, &[1]);
    assert!(matches!(read(&f).0, Outcome::Unavailable { .. }));
    // Readable and without the key: no row.
    f.array(0x31000 + cstate::BASE_MODIFIERS, 0, 0);
    assert_eq!(read(&f), (Outcome::Rows { len: 0 }, 0));
}

#[test]
fn the_temporary_walk_holds_every_resource_the_database_has() {
    let mut f = world();
    let one = f.descriptors()[0];
    f.u32(0xd000 + cstrategic_resource::TEMPORARY_KEY, 0);
    // More resources than any fixed table would hold: every key is read.
    let descriptors: Vec<_> = (0..70).map(|_| one).collect();
    f.array(GAME_STATE + cgame_state::STATES, 0x70000, 2);
    f.put(0x70000, &[0; 16]);
    f.u64(0x70008, 0x31000);
    f.u32(0x31000 + STATE_TEMPORARY_LIST_COUNT, 1);
    f.put(0x31000 + STATE_TEMPORARY_FLAG, &[0]);
    let mut rows = Vec::new();
    let outcome = walk_temporary(&f, 0, GAME_STATE, &descriptors, Buffer::Growing(&mut rows));
    assert_eq!(outcome, Outcome::Rows { len: 0 });
}

#[test]
fn a_region_outside_the_records_is_full_and_one_inside_reads_its_efficiency() {
    let mut f = Fixture::new();
    let stride = crate::cstrategic_navy::REGIONAL_CONVOYS_STRIDE;
    let efficiency = crate::cstrategic_navy::REGIONAL_CONVOY_EFFICIENCY;
    f.u64(0x80000 + 2 * stride + efficiency, 45_000);
    let records = Array {
        data: 0x80000,
        count: 3,
    };
    let read = |region| crate::cstrategic_navy::regional_convoy_efficiency(&f, records, region);
    assert_eq!(read(2), Ok(45_000));
    assert_eq!(read(3), Ok(100_000));
    // A record inside the array that does not read is refused, not taken as full.
    assert!(read(1).is_err());
}

#[test]
fn only_a_bad_naval_line_reads_regional_efficiency_and_an_unread_navy_refuses() {
    let f = world();
    let (_, rows) = walk(&f);
    let regional = |origins: &[OriginRow], clients: &[ClientRow]| {
        let mut rows = Vec::new();
        let outcome = walk_regional(
            &f,
            0,
            GAME_STATE,
            origins,
            clients,
            &mut Vec::new(),
            Buffer::Growing(&mut rows),
        );
        (outcome, rows.len())
    };
    // The bad land contract and the good import read no navy.
    assert_eq!(regional(&[], &rows.clients), (Outcome::Rows { len: 0 }, 0));
    // The bad naval production line needs country 7's navy, which this world lacks.
    assert!(matches!(
        regional(&rows.origins, &rows.clients).0,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn an_unreadable_origin_efficiency_preserves_its_resource_rights() {
    let mut f = world();
    f.bytes.retain(|&address, _| {
        !(0x20000 + ORIGIN_EFFICIENCY..0x20008 + ORIGIN_EFFICIENCY).contains(&address)
    });
    let (outcomes, _) = walk(&f);
    assert!(matches!(outcomes.origins, Outcome::Unavailable { .. }));
    assert_eq!(outcomes.rights, Outcome::Rows { len: 1 });
    assert_eq!(outcomes.clients, Outcome::Rows { len: 3 });
}

#[test]
fn market_directional_positions_do_not_follow_global_contract_or_handle_order() {
    let mut f = world();
    let copy: Vec<_> = (0..0x200)
        .map(|offset| *f.bytes.get(&(0x61000 + offset)).unwrap_or(&0))
        .collect();
    f.put(0x66000, &copy);
    client(
        &mut f,
        0x66000 + CONTRACT_CLIENT,
        CONTRACT_CLIENT_VTABLE_RVA,
        89,
        [100_000, 50_000],
    );
    f.array(0x60000 + MARKET_OWNED_CONTRACTS, 0x60800, 2);
    f.u64(0x60808, 0x66000);
    f.array(0x63000 + MARKET_COUNTRY_STRIDE, 0x65000, 2);
    f.u64(0x65000, 0x66000);
    f.u64(0x65008, 0x61000);
    f.array(0x64000 + 2 * MARKET_COUNTRY_STRIDE, 0x65010, 2);
    f.u64(0x65010, 0x61000);
    f.u64(0x65018, 0x66000);
    let (outcomes, rows) = walk(&f);
    assert_eq!(outcomes.clients, Outcome::Rows { len: 4 });
    let orders: Vec<_> = rows.clients[..4]
        .iter()
        .filter(|row| row.category == MARKET)
        .map(|row| (row.handle, row.owner_ordinal, row.other_ordinal))
        .collect();
    assert_eq!(orders, [(88, Some(1), Some(0)), (89, Some(0), Some(1))]);
}

#[test]
fn market_route_states_are_separate_from_ports_and_authenticate_against_the_state_roster() {
    let mut f = world();
    let address = 0x20000 + ORIGIN_ROUTE;
    f.u64(address + ROUTE_FROM, 0x30000);
    f.u64(address + ROUTE_TO, 0x31000);
    for state in [0x30000, 0x31000] {
        f.u64(state, cstate::VTABLE_RVA as u64);
    }
    f.array(GAME_STATE + cgame_state::STATES, 0x80000, 103);
    f.u64(0x80000 + 101 * 8, 0x30000);
    f.u64(0x80000 + 102 * 8, 0x31000);
    let mut path = Vec::new();
    let row = read_route(
        &f,
        0,
        RouteKey::default(),
        address,
        Some(GAME_STATE),
        &mut path,
    )
    .unwrap();
    assert_eq!((row.from, row.to), (11, 12));
    assert_eq!(
        (row.source_state, row.destination_state),
        (Some(101), Some(102))
    );
    let row = read_route(&f, 0, RouteKey::default(), address, None, &mut path).unwrap();
    assert_eq!((row.source_state, row.destination_state), (None, None));
    f.u64(0x80000 + 102 * 8, 0x30000);
    assert_eq!(
        read_route(
            &f,
            0,
            RouteKey::default(),
            address,
            Some(GAME_STATE),
            &mut path
        ),
        Err("market route state roster backlink mismatch")
    );
    f.u64(0x31000, 0);
    assert_eq!(
        read_route(
            &f,
            0,
            RouteKey::default(),
            address,
            Some(GAME_STATE),
            &mut path
        ),
        Err("market route state vtable mismatch")
    );
}
