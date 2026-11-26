use super::*;
use dashboard_core::test_fixtures::game;
use crate::triggers::Scopes;
use replay_data::{
    rows::ArmyOrderPlanRow, rows::ArmyOrderRow, rows::CountryLinkRow, rows::FactionMemberRow,
    rows::LandTheatreRow, rows::OrdersGroupPlanRow, rows::OrdersGroupRow, rows::Recorded,
    rows::ReservedCountryNamingRow, rows::TableId, rows::TableRows,
};

fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 0,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
        metrics: Default::default(),
    };
    set::<tables::CountryLinks>(
        &mut frame,
        vec![
            CountryLinkRow {
                id: 100,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 200,
                canonical_slot: 2,
            },
        ],
    );
    frame
}

fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(TableRows::from_rows::<T>(rows));
}

fn with_world<T>(frame: &DecodedFrame, run: impl FnOnce(World<'_>) -> T) -> T {
    let dlcs = BTreeSet::new();
    let triggers = TriggerNames::default();
    let aliases = Arc::default();
    run(World::read(
        frame,
        ContentFacts::content_only(&dlcs, &triggers, &aliases),
    ))
}

#[test]
fn genuine_member_zero_does_not_poison_positive_faction_absence() {
    game::enter();
    let mut frame = frame();
    set::<tables::FactionMembers>(
        &mut frame,
        vec![
            FactionMemberRow {
                country: 0,
                faction: 7,
                ordinal: 0,
            },
            FactionMemberRow {
                country: 100,
                faction: 7,
                ordinal: 1,
            },
        ],
    );
    with_world(&frame, |world| {
        assert_eq!(world.slot(0), None);
        assert_eq!(world.faction(1), Ok(Some(7)));
        assert_eq!(world.faction(2), Ok(None));
        assert!(world.faction(0).is_err());
    });
    set::<tables::FactionMembers>(
        &mut frame,
        vec![FactionMemberRow {
            country: 999,
            faction: 7,
            ordinal: 0,
        }],
    );
    with_world(&frame, |world| {
        let why = world.faction(2).unwrap_err();
        assert!(
            why.names("CountryLinks.canonical_slot")
                && why == "faction members names a country that no recorded country link holds",
            "{why}"
        );
    });
}

#[test]
fn reserved_faction_uses_its_fresh_getter_including_null_and_refusals() {
    game::enter();
    let mut frame = frame();
    for (captured, expected) in [
        (Recorded::Value(Some(7)), Some(Ok(Some(7)))),
        (Recorded::Value(Some(0)), Some(Ok(None))),
        (Recorded::Value(None), None),
        (Recorded::NotRecorded, None),
    ] {
        set::<tables::ReservedCountryNaming>(
            &mut frame,
            vec![ReservedCountryNamingRow {
                faction: captured,
                ..Default::default()
            }],
        );
        with_world(&frame, |world| {
            let result = world.faction(0);
            if let Some(expected) = expected {
                assert_eq!(result, expected);
            } else {
                assert!(result.is_err());
            }
            assert_eq!(world.faction(2), Ok(None));
        });
    }
}

fn theatres(frame: &mut DecodedFrame, count: Recorded<Option<u32>>) {
    set::<tables::ReservedCountryNaming>(
        frame,
        vec![ReservedCountryNamingRow {
            land_theatre_count: count,
            ..Default::default()
        }],
    );
    set::<tables::LandTheatres>(
        frame,
        vec![LandTheatreRow {
            id: 10,
            owner: 0,
            ordinal: 0,
            ..Default::default()
        }],
    );
}

fn empty_reserved_group_and_positive_invasion() -> DecodedFrame {
    let mut frame = frame();
    set::<tables::CountryLinks>(
        &mut frame,
        vec![
            CountryLinkRow {
                id: 100,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 200,
                canonical_slot: 2,
            },
            CountryLinkRow {
                id: 900,
                canonical_slot: 0,
            },
        ],
    );
    set::<tables::States>(
        &mut frame,
        vec![StateRow {
            id: 64,
            ..Default::default()
        }],
    );
    theatres(&mut frame, Recorded::Value(Some(1)));
    set::<tables::LandTheatres>(
        &mut frame,
        vec![
            LandTheatreRow {
                id: 10,
                owner: 0,
                ordinal: 0,
                ..Default::default()
            },
            LandTheatreRow {
                id: 20,
                owner: 100,
                ordinal: 0,
                ..Default::default()
            },
        ],
    );
    set::<tables::OrdersGroups>(
        &mut frame,
        vec![
            OrdersGroupRow {
                handle: 30,
                theatre: 10,
                theatre_ordinal: Some(0),
                ..Default::default()
            },
            OrdersGroupRow {
                handle: 50,
                theatre: 20,
                theatre_ordinal: Some(0),
                ..Default::default()
            },
        ],
    );
    set::<tables::OrdersGroupPlans>(
        &mut frame,
        vec![
            OrdersGroupPlanRow {
                group: 30,
                ..Default::default()
            },
            OrdersGroupPlanRow {
                group: 50,
                primary_orders: vec![60],
                ..Default::default()
            },
        ],
    );
    set::<tables::ArmyOrders>(
        &mut frame,
        vec![ArmyOrderRow {
            owner_group: 50,
            instance: 60,
            order_type: 3,
            ..Default::default()
        }],
    );
    set::<tables::ArmyOrderPlans>(
        &mut frame,
        vec![ArmyOrderPlanRow {
            owner_group: 50,
            instance: 60,
            ..Default::default()
        }],
    );
    frame
}

fn assert_reserved_empty(world: &World<'_>) {
    let reserved = world.slot(900).unwrap();
    assert_eq!(reserved, 0);
    assert!(world.complete_invasion_orders(reserved).unwrap().is_empty());
    let trigger = content_reader_script::triggers::read(
        b"has_naval_invasion_against_state = 64",
        &content_reader_script::key_names::KeyNames::default(),
    );
    assert_eq!(trigger.items.len(), 1);
    assert_eq!(
        world.evaluate(&trigger.items, Scopes::of(Scope::Country(reserved))),
        Eval::False
    );
}

#[test]
fn certified_empty_reserved_groups_ignore_unavailable_order_tables() {
    game::enter();
    for unavailable in [
        TableId::ArmyOrders,
        TableId::ArmyOrderPlans,
        TableId::OrdersGroupPlans,
    ] {
        let mut frame = empty_reserved_group_and_positive_invasion();
        set::<tables::OrdersGroups>(
            &mut frame,
            vec![
                OrdersGroupRow {
                    handle: 30,
                    theatre: 10,
                    theatre_ordinal: Some(0),
                    is_army_group: 1,
                    ..Default::default()
                },
                OrdersGroupRow {
                    handle: 31,
                    theatre: 10,
                    theatre_ordinal: None,
                    ..Default::default()
                },
                OrdersGroupRow {
                    handle: 50,
                    theatre: 20,
                    theatre_ordinal: Some(0),
                    ..Default::default()
                },
            ],
        );
        *frame.tables[unavailable.index()] = TableView::Unavailable {
            reason: "positive country's order capture failed".into(),
        };
        with_world(&frame, |world| {
            assert_reserved_empty(&world);
            let reason = world.complete_invasion_orders(1).err().unwrap();
            assert!(reason.contains(&format!("{unavailable:?}")), "{reason}");
            assert_reserved_empty(&world);
            assert!(world.complete_invasion_orders(2).unwrap().is_empty());
        });
        *frame.tables[TableId::OrdersGroups.index()] = TableView::Unavailable {
            reason: "reserved regular roster not proven".into(),
        };
        with_world(&frame, |world| {
            assert!(
                world
                    .complete_invasion_orders(0)
                    .err()
                    .unwrap()
                    .contains("OrdersGroups")
            );
        });
    }
}

#[test]
fn empty_primary_roots_ignore_unavailable_order_payloads() {
    game::enter();
    for unavailable in [TableId::ArmyOrders, TableId::ArmyOrderPlans] {
        let mut frame = empty_reserved_group_and_positive_invasion();
        *frame.tables[unavailable.index()] = TableView::Unavailable {
            reason: "unreached order payload unavailable".into(),
        };
        with_world(&frame, |world| {
            assert_reserved_empty(&world);
            let reason = world.complete_invasion_orders(1).err().unwrap();
            assert!(reason.contains(&format!("{unavailable:?}")), "{reason}");
            assert_reserved_empty(&world);
            assert!(world.complete_invasion_orders(2).unwrap().is_empty());
        });
    }
    let mut frame = empty_reserved_group_and_positive_invasion();
    for unavailable in [TableId::ArmyOrders, TableId::ArmyOrderPlans] {
        *frame.tables[unavailable.index()] = TableView::Unavailable {
            reason: "unreached order payload unavailable".into(),
        };
    }
    set::<tables::LandTheatres>(
        &mut frame,
        vec![
            LandTheatreRow {
                id: 10,
                owner: 0,
                ordinal: 0,
                ..Default::default()
            },
            LandTheatreRow {
                id: 20,
                owner: 100,
                ordinal: 0,
                ..Default::default()
            },
            LandTheatreRow {
                id: 21,
                owner: 999,
                ordinal: 0,
                ..Default::default()
            },
        ],
    );
    set::<tables::OrdersGroupPlans>(
        &mut frame,
        vec![
            OrdersGroupPlanRow {
                group: 30,
                ..Default::default()
            },
            OrdersGroupPlanRow {
                group: 50,
                ..Default::default()
            },
        ],
    );
    with_world(&frame, |world| {
        assert_reserved_empty(&world);
        let why = world.complete_invasion_orders(1).err().unwrap();
        assert!(
            why.names("CountryLinks.canonical_slot")
                && why == "land theatres names a country that no recorded country link holds",
            "{why}"
        );
        let why = world.complete_invasion_orders(2).err().unwrap();
        assert!(
            why.names("CountryLinks.canonical_slot")
                && why == "land theatres names a country that no recorded country link holds",
            "{why}"
        );
    });
}

#[test]
fn reserved_theatre_group_faults_stay_with_their_owner() {
    game::enter();
    let mut frame = frame();
    theatres(&mut frame, Recorded::Value(Some(1)));
    set::<tables::OrdersGroups>(
        &mut frame,
        vec![OrdersGroupRow {
            handle: 30,
            country: 100,
            theatre: 10,
            theatre_ordinal: Some(0),
            ..Default::default()
        }],
    );
    with_world(&frame, |world| {
        assert!(world.complete_invasion_orders(1).unwrap().is_empty());
        assert!(
            world
                .complete_invasion_orders(0)
                .err()
                .unwrap()
                .contains("group 0x1e")
        );
        assert!(world.complete_invasion_orders(2).unwrap().is_empty());
    });
}

#[test]
fn reserved_invasions_use_certified_roster_and_keep_unknown_positive_owners_unknown() {
    game::enter();
    let mut frame = frame();
    theatres(&mut frame, Recorded::Value(Some(1)));
    set::<tables::OrdersGroups>(
        &mut frame,
        vec![OrdersGroupRow {
            handle: 30,
            theatre: 10,
            theatre_ordinal: Some(0),
            ..Default::default()
        }],
    );
    set::<tables::OrdersGroupPlans>(
        &mut frame,
        vec![OrdersGroupPlanRow {
            group: 30,
            primary_orders: vec![40],
            ..Default::default()
        }],
    );
    set::<tables::ArmyOrders>(
        &mut frame,
        vec![ArmyOrderRow {
            owner_group: 30,
            instance: 40,
            order_type: 3,
            active: 1,
            path: vec![17],
            ..Default::default()
        }],
    );
    set::<tables::ArmyOrderPlans>(
        &mut frame,
        vec![ArmyOrderPlanRow {
            owner_group: 30,
            instance: 40,
            ..Default::default()
        }],
    );
    with_world(&frame, |world| {
        let orders = world.complete_invasion_orders(0).unwrap();
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].key, (30, 40));
        assert_eq!(orders[0].active, 1);
        assert_eq!(orders[0].path, &[17]);
        assert!(world.complete_invasion_orders(1).unwrap().is_empty());
    });
    set::<tables::LandTheatres>(
        &mut frame,
        vec![
            LandTheatreRow {
                id: 10,
                owner: 0,
                ordinal: 0,
                ..Default::default()
            },
            LandTheatreRow {
                id: 20,
                owner: 100,
                ordinal: 0,
                ..Default::default()
            },
        ],
    );
    set::<tables::OrdersGroups>(
        &mut frame,
        vec![
            OrdersGroupRow {
                handle: 30,
                theatre: 10,
                theatre_ordinal: Some(0),
                ..Default::default()
            },
            OrdersGroupRow {
                handle: 50,
                theatre: 20,
                theatre_ordinal: Some(0),
                ..Default::default()
            },
        ],
    );
    with_world(&frame, |world| {
        assert_eq!(world.complete_invasion_orders(0).unwrap().len(), 1);
        assert!(
            world
                .complete_invasion_orders(1)
                .err()
                .unwrap()
                .contains("group 0x32")
        );
        assert!(world.complete_invasion_orders(2).unwrap().is_empty());
    });
    set::<tables::LandTheatres>(
        &mut frame,
        vec![
            LandTheatreRow {
                id: 10,
                owner: 0,
                ordinal: 0,
                ..Default::default()
            },
            LandTheatreRow {
                id: 20,
                owner: 999,
                ordinal: 0,
                ..Default::default()
            },
        ],
    );
    with_world(&frame, |world| {
        assert_eq!(world.complete_invasion_orders(0).unwrap().len(), 1);
        let why = world.complete_invasion_orders(1).err().unwrap();
        assert!(
            why.names("CountryLinks.canonical_slot")
                && why == "land theatres names a country that no recorded country link holds",
            "{why}"
        );
    });
}

#[test]
fn reserved_invasion_certificate_never_substitutes_zero_for_refusal() {
    game::enter();
    let mut frame = frame();
    for count in [
        Recorded::NotRecorded,
        Recorded::Value(None),
        Recorded::Value(Some(2)),
    ] {
        theatres(&mut frame, count);
        with_world(&frame, |world| {
            assert!(world.complete_invasion_orders(0).is_err());
            assert!(world.complete_invasion_orders(1).unwrap().is_empty());
        });
    }
    theatres(&mut frame, Recorded::Value(Some(0)));
    with_world(&frame, |world| {
        assert!(world.complete_invasion_orders(0).is_err())
    });
    set::<tables::LandTheatres>(&mut frame, Vec::new());
    *frame.tables[tables::OrdersGroups::ID.index()] = TableView::Unavailable {
        reason: "unused empty roster".into(),
    };
    with_world(&frame, |world| {
        assert!(world.complete_invasion_orders(0).unwrap().is_empty())
    });
    *frame.tables[tables::LandTheatres::ID.index()] = TableView::Unavailable {
        reason: "unrelated positive theatre read refused".into(),
    };
    with_world(&frame, |world| {
        assert!(world.complete_invasion_orders(0).unwrap().is_empty());
        assert!(
            world
                .complete_invasion_orders(1)
                .err()
                .unwrap()
                .contains("LandTheatres")
        );
    });
}
