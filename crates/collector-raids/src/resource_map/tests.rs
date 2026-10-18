use super::*;
use crate::raid_changes::{Target, Watches};
use engine_mutations::raid_mutations::FIRST_ROUTE_SITE;

fn key(country: u32, class: u8, other: u32) -> RouteKey {
    RouteKey {
        class,
        country,
        other,
    }
}

const OWNERS: [u32; 3] = [3, 5, 7];

/// One hour's selection over `census`: whether every owner is read, and which are.
fn select(
    captured: &mut Captured,
    store: &mut SliceFingerprints,
    watches: &Watches,
    hour: i32,
    census: &[(RouteKey, usize)],
) -> (bool, Vec<u32>) {
    captured.selection.reset(hour, false);
    captured.census = census
        .iter()
        .map(|&(key, address)| (key, address, false))
        .collect();
    captured.select(
        &mut Stores {
            routes: store,
            changes: watches,
            verify: false,
        },
        true,
        &Some,
    );
    let selection = &captured.selection;
    (
        selection.force_full,
        OWNERS
            .into_iter()
            .filter(|&owner| selection.contains(owner))
            .collect(),
    )
}

fn slice(hour: i32) -> Vec<u32> {
    OWNERS
        .into_iter()
        .filter(|&owner| replay_data::cadence::slice_active(owner, hour, false))
        .collect()
}

/// A route owner a hooked route writer marked is read within the hour, outside its daily
/// slice, and so is one whose referenced routes changed; an unmarked, unchanged owner
/// outside the slice is not. An hour whose take does not return the previous hour's epoch
/// reads every owner.
#[test]
fn touched_route_owners_are_read_within_the_hour_and_an_uncertified_hour_reads_all() {
    let watches = Watches::enabled(64, 64);
    let mut captured = Captured::with_capacities(&Capacities);
    let mut store = SliceFingerprints::new();
    let census = vec![
        (key(3, 1, 5), 0x100),
        (key(5, 0, 101), 0x200),
        (key(7, 2, 9), 0x300),
    ];
    // The first take certifies no earlier read.
    assert_eq!(
        select(&mut captured, &mut store, &watches, 0, &census),
        (true, OWNERS.to_vec())
    );
    // A quiet hour reads its slice alone.
    let hour = (1..24)
        .find(|&hour| slice(hour).len() < OWNERS.len() && !slice(hour).contains(&5))
        .unwrap();
    assert_eq!(
        select(&mut captured, &mut store, &watches, hour, &census),
        (false, slice(hour))
    );
    // A route of country 5 was repathed: it is read beside the slice.
    drop(watches.notify(FIRST_ROUTE_SITE, Target::Route(Some(5))));
    let mut expected = slice(hour + 1);
    expected.push(5);
    expected.sort_unstable();
    expected.dedup();
    let next = select(&mut captured, &mut store, &watches, hour + 1, &census);
    assert_eq!(next, (false, expected));
    // Country 7's route now lives elsewhere: its changed print reads it.
    let mut moved = census.clone();
    moved[2].1 = 0x400;
    let mut expected = slice(hour + 2);
    expected.push(7);
    expected.sort_unstable();
    expected.dedup();
    assert_eq!(
        select(&mut captured, &mut store, &watches, hour + 2, &moved),
        (false, expected)
    );
    // A route writer whose country does not read moves the epoch: every owner.
    drop(watches.notify(FIRST_ROUTE_SITE, Target::Route(None)));
    assert_eq!(
        select(&mut captured, &mut store, &watches, hour + 3, &moved),
        (true, OWNERS.to_vec())
    );
}

/// The census holds every referenced route once, in TradeRoutes' key order: own and
/// rights-holder origins' routes and every client's; two clients of one delivery route share
/// its row, and a key naming two routes refuses the census.
#[test]
fn the_route_census_is_in_key_order_with_one_row_per_route() {
    let mut captured = Captured::with_capacities(&Capacities);
    let delivery = key(7, engine_resources::resource_map::DELIVERY_ROUTE_CLASS, 2);
    captured.origins = vec![
        native::OriginRow {
            state: 101,
            country: 7,
            list: 0,
            route_address: 0x200,
            ..Default::default()
        },
        // Country 2 holds state 101 by rights: its origin's route is its own.
        native::OriginRow {
            state: 101,
            country: 2,
            list: 1,
            route_address: 0x250,
            ..Default::default()
        },
    ];
    captured.clients = vec![
        native::ClientRow {
            handle: 1,
            route: delivery,
            route_address: 0x100,
            ..Default::default()
        },
        native::ClientRow {
            handle: 2,
            route: delivery,
            route_address: 0x100,
            ..Default::default()
        },
        native::ClientRow {
            handle: 3,
            route: key(2, engine_resources::resource_map::SUPPLY_ROUTE_CLASS, 3),
            route_address: 0x300,
            ..Default::default()
        },
    ];
    captured.outcomes[0] = Outcome::Rows { len: 2 };
    captured.outcomes[4] = Outcome::Rows { len: 3 };
    assert_eq!(captured.census(), Ok(()));
    assert_eq!(
        captured.census,
        [
            (
                key(2, engine_resources::resource_map::ORIGIN_ROUTE_CLASS, 101),
                0x250,
                false
            ),
            (
                key(2, engine_resources::resource_map::SUPPLY_ROUTE_CLASS, 3),
                0x300,
                false
            ),
            (
                key(7, engine_resources::resource_map::ORIGIN_ROUTE_CLASS, 101),
                0x200,
                false
            ),
            (delivery, 0x100, false),
        ]
    );
    captured.clients[1].category = native::MARKET;
    assert_eq!(captured.census(), Ok(()));
    assert_eq!(captured.census.last().unwrap(), &(delivery, 0x100, true));
    captured.clients[1].route_address = 0x180;
    assert_eq!(
        captured.census(),
        Err("one trade route key names two routes")
    );
    captured.outcomes[4] = Outcome::Unavailable { reason: "x" };
    assert!(captured.census().is_err());
}

#[test]
fn market_endpoint_demand_changes_select_the_owner_and_endpoint_values_change_its_hash() {
    let watches = Watches::enabled(64, 64);
    let mut captured = Captured::with_capacities(&Capacities);
    let mut store = SliceFingerprints::new();
    let census = vec![(key(5, 1, 7), 0x100)];
    select(&mut captured, &mut store, &watches, 0, &census);
    let hour = (1..24).find(|&hour| !slice(hour).contains(&5)).unwrap();
    captured.census[0].2 = true;
    captured.selection.reset(hour, false);
    captured.select(
        &mut Stores {
            routes: &mut store,
            changes: &watches,
            verify: false,
        },
        true,
        &Some,
    );
    assert!(captured.selection.contains(5));
    captured.routes = vec![native::RouteRow {
        key: key(5, 1, 7),
        source_state: Some(10),
        destination_state: Some(20),
        ..Default::default()
    }];
    assert!(captured.route_hashes());
    let initial = captured.hashes.clone();
    captured.routes[0].source_state = Some(11);
    assert!(captured.route_hashes());
    assert_ne!(captured.hashes, initial);
    let source_changed = captured.hashes.clone();
    captured.routes[0].destination_state = Some(21);
    assert!(captured.route_hashes());
    assert_ne!(captured.hashes, source_changed);
}
