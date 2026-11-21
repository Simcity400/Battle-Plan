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

#[test]
fn owned_resource_sorting_keeps_independent_duplicate_refusals_and_ordinals() {
    let mut captured = Captured::with_capacities(&Capacities);
    captured.rights = vec![
        native::RightsRow {
            state: 9,
            resource: 2,
            ordinal: 7,
            ..Default::default()
        },
        native::RightsRow {
            state: 3,
            resource: 2,
            ordinal: 4,
            ..Default::default()
        },
    ];
    captured.oil = vec![
        native::OilRow {
            country: 3,
            factor: 17
        };
        2
    ];
    captured.temporary = vec![native::TemporaryRow {
        state: 2,
        resource: 4,
        amount: -5,
    }];
    captured.regional = vec![native::RegionalRow {
        country: 2,
        region: 4,
        efficiency: -7,
    }];
    captured.outcomes = [Outcome::Rows { len: 0 }; N];
    captured.outcomes[1] = Outcome::Rows { len: 2 };
    captured.outcomes[2] = Outcome::Rows { len: 2 };
    captured.outcomes[3] = Outcome::Rows { len: 1 };
    captured.outcomes[6] = Outcome::Rows { len: 1 };
    let mut expected = captured.rights.clone();
    let rights = sorted(&mut expected, captured.outcomes[1], |row| {
        (row.state, row.resource)
    });
    captured.pending_owned = true;
    captured.settle_owned();
    assert_eq!(captured.rights, expected);
    assert_eq!(
        captured
            .rights
            .iter()
            .map(|row| row.ordinal)
            .collect::<Vec<_>>(),
        [4, 7]
    );
    assert_eq!(captured.outcomes[1], rights);
    assert_eq!(
        captured.outcomes[2],
        Outcome::Unavailable {
            reason: "two resources map rows share one key"
        }
    );
    assert_eq!(captured.outcomes[3], Outcome::Rows { len: 1 });
    assert_eq!(captured.outcomes[6], Outcome::Rows { len: 1 });
    captured.settle_owned();
    assert_eq!(captured.rights, expected);
}

fn select(
    captured: &mut Captured,
    store: &mut SliceFingerprints,
    watches: &Watches,
    hour: i32,
    census: &[(RouteKey, usize)],
) -> (bool, Vec<u32>) {
    captured.selection.reset_hooked(hour, false);
    captured.census = census
        .iter()
        .map(|&(key, address)| (key, address, false))
        .collect();
    captured.select(
        &mut Stores {
            routes: store,
            changes: watches,
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

#[test]
fn removing_a_supply_client_marks_the_changed_census() {
    let watches = Watches::enabled(64, 64);
    let mut captured = Captured::with_capacities(&Capacities);
    let mut store = SliceFingerprints::new();
    let census = [
        (key(5, native::SUPPLY_ROUTE_CLASS, 9), 0x100),
        (key(5, native::SUPPLY_ROUTE_CLASS, 10), 0x200),
    ];
    select(&mut captured, &mut store, &watches, 0, &census);
    let (full, owners) = select(&mut captured, &mut store, &watches, 5, &census[..1]);
    assert!(!full);
    assert_eq!(owners, [5]);
    assert!(!watches.poisoned(crate::raid_changes::Family::Routes));
}

#[test]
fn supply_repath_with_unchanged_membership_reads_the_owner_immediately() {
    let watches = Watches::enabled(64, 64);
    let mut captured = Captured::with_capacities(&Capacities);
    let mut store = SliceFingerprints::new();
    let census = [(key(5, native::SUPPLY_ROUTE_CLASS, 9), 0x100)];
    select(&mut captured, &mut store, &watches, 0, &census);
    let hour = 5;
    assert!(
        !select(&mut captured, &mut store, &watches, hour, &census)
            .1
            .contains(&5)
    );
    let (index, (_, receiver)) = engine_mutations::raid_mutations::MUTATIONS
        .iter()
        .enumerate()
        .find(|(_, (site, _))| site.name == "update_supply_trade_route")
        .unwrap();
    assert_eq!(*receiver, engine_mutations::raid_mutations::Receiver::Route);
    drop(watches.notify(index, Target::Route(Some(5))));
    let (full, owners) = select(&mut captured, &mut store, &watches, hour, &census);
    assert!(!full);
    assert_eq!(owners, [5]);
    assert!(!watches.poisoned(crate::raid_changes::Family::Routes));
}

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
    assert_eq!(
        select(&mut captured, &mut store, &watches, 0, &census),
        (true, OWNERS.to_vec())
    );
    let hour = 1;
    for at in hour..hour + 24 {
        assert_eq!(
            select(&mut captured, &mut store, &watches, at, &census),
            (false, vec![])
        );
        assert_eq!(captured.selection.carried(), 3);
    }
    let hour = hour + 24;
    drop(watches.notify(FIRST_ROUTE_SITE, Target::Route(Some(5))));
    let next = select(&mut captured, &mut store, &watches, hour + 1, &census);
    assert_eq!(next, (false, vec![5]));
    let mut moved = census.clone();
    moved[2].1 = 0x400;
    assert_eq!(
        select(&mut captured, &mut store, &watches, hour + 2, &moved),
        (false, vec![7])
    );
    drop(watches.notify(FIRST_ROUTE_SITE, Target::Route(None)));
    assert_eq!(
        select(&mut captured, &mut store, &watches, hour + 3, &moved),
        (true, OWNERS.to_vec())
    );
}

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
fn market_endpoint_demand_changes_select_the_owner() {
    let watches = Watches::enabled(64, 64);
    let mut captured = Captured::with_capacities(&Capacities);
    let mut store = SliceFingerprints::new();
    let census = vec![(key(5, 1, 7), 0x100)];
    select(&mut captured, &mut store, &watches, 0, &census);
    captured.census[0].2 = true;
    captured.selection.reset_hooked(5, false);
    captured.select(
        &mut Stores {
            routes: &mut store,
            changes: &watches,
        },
        true,
        &Some,
    );
    assert!(captured.selection.contains(5));
}
