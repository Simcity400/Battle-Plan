use super::*;

fn order_glyph(status: &NavalStatus) -> Option<OrderGlyph> {
    match status.no_orders? {
        true => Some(OrderGlyph::NoOrders),
        false => match status.invalid_orders? {
            true => Some(OrderGlyph::InvalidOrders),
            false => Some(OrderGlyph::None),
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OrderGlyph {
    None,
    NoOrders,
    InvalidOrders,
}

#[test]
fn the_strength_ratio_sums_the_roster_and_floors_each_raw_maximum() {
    assert_eq!(
        strength_ratio([(900_000, 1_000_000), (100_000, 1_000_000)]),
        50_000
    );
    assert_eq!(strength_ratio([(618_182, 1_500_000)]), 41_212);
    assert_eq!(strength_ratio([(50, 0)]), 50_000);
    assert_eq!(strength_ratio(std::iter::empty()), 0);
    assert_eq!(strength_ratio([(i64::MAX, 1)]), UNDEFINED);
}

#[test]
fn the_organisation_ratio_is_the_mean_of_each_ships_own_ratio() {
    assert_eq!(
        organisation_ratio([(3_000_000, 3_300_000), (3_300_000, 3_300_000)]),
        95_454
    );
    assert_eq!(ship_organisation_ratio(10, 0), UNDEFINED);
    assert_eq!(organisation_ratio(std::iter::empty()), 0);
}

#[test]
fn the_counter_mean_divides_by_the_members_it_summed() {
    assert_eq!(member_mean([100_000, 50_000]), Some(75_000));
    assert_eq!(member_mean([100_000, 50_001, 0]), Some(50_000));
    assert_eq!(member_mean(std::iter::empty()), None);
    assert_eq!(bar_fill(75_000), 0.75);
}

#[test]
fn a_bar_steps_by_half_its_length_and_draws_two_pixels_a_step() {
    assert_eq!(bar_step(bar_fill(84_870), 17.0), 7);
    assert_eq!(bar_pixels(bar_fill(84_870), 17.0), 16.0);
    assert_eq!(bar_pixels(1.0, 17.0), 18.0);
    assert_eq!(bar_step(bar_fill(94_110), 17.0), 7);
    assert_eq!(bar_step(bar_fill(94_120), 17.0), 8);
    assert_eq!(bar_pixels(bar_fill(80_880), 17.0), 14.0);
    assert_eq!(bar_pixels(0.0, 17.0), 2.0);
    assert_eq!(bar_pixels(bar_fill(UNDEFINED), 17.0), 18.0);
}

#[test]
fn a_mission_is_active_in_its_regions_except_hold_strike_and_reserve() {
    for mission in [1, 3, 4, 5, 6, 7] {
        assert!(mission_active(1, mission, 0), "{mission}");
        assert!(!mission_active(0, mission, 0), "{mission}");
    }
    for mission in [0, 2, 8, 10] {
        assert!(!mission_active(1, mission, 7), "{mission}");
    }
    assert!(!mission_active(1, 9, 0));
    assert!(mission_active(1, 9, 12));
    assert!(needs_regions(1) && needs_regions(9) && !needs_regions(7) && !needs_regions(0));
    assert_eq!(effective_mission(4, 0, 9), 4);
    assert_eq!(effective_mission(4, 1, 0), 0);
}

#[test]
fn engagement_refusal_reads_the_aggression_table_and_the_unsaved_byte() {
    let table = [0, 60_000, 125_000, 200_000, 1_000_000_000];
    assert_eq!(engagement_refused(0, 0, 2, &table), Some(true));
    assert_eq!(engagement_refused(0, 1, 2, &table), Some(false));
    assert_eq!(engagement_refused(1, 4, 3, &table), Some(true));
    assert_eq!(engagement_refused(1, 0, 1, &table), Some(false));
    assert_eq!(engagement_refused(0, 5, 2, &table), None);
    assert_eq!(engagement_refused(0, -1, 2, &table), None);
}

fn inputs() -> TaskForceCounterInputRow {
    TaskForceCounterInputRow {
        task_force: 1,
        is_in_regions: 1,
        mission_region_count: 3,
        engagement_rule: 2,
        ..Default::default()
    }
}

fn member(inputs: &TaskForceCounterInputRow, mission: u32) -> NavalMember<'_> {
    NavalMember {
        visible: true,
        own: true,
        mission,
        has_fleet: true,
        inputs: Some(inputs),
        fleet_regions: Some(0),
        pride: Some(false),
    }
}

#[test]
fn port_supply_takes_every_member_minimum_while_sea_takes_the_last_visible() {
    let ratio = |received_supply_ratio| TaskForceCounterInputRow {
        received_supply_ratio,
        ..inputs()
    };
    let (first, second, third) = (ratio(42_000), ratio(12_000), ratio(48_000));
    let mut hidden = member(&second, 1);
    hidden.visible = false;
    let units = [member(&first, 1), hidden, member(&third, 1)];
    assert_eq!(port_supply(&units), NavalSupply::Write(12_000));
    assert_eq!(sea_supply(&units), NavalSupply::Write(48_000));
    assert_eq!(sea_supply(&[hidden]), NavalSupply::Write(SCALE));
    assert_eq!(port_supply(&[hidden, units[0]]), NavalSupply::Retain);
    let missing = NavalMember {
        inputs: None,
        ..units[0]
    };
    assert_eq!(port_supply(&[missing]), NavalSupply::Unavailable);
    assert_eq!(sea_supply(&[missing]), NavalSupply::Unavailable);
}

#[test]
fn the_at_sea_counter_takes_order_glyphs_from_visible_own_unlinked_members() {
    let table = [0, 60_000, 125_000, 200_000, 1_000_000_000];
    let patrol = inputs();
    let status = at_sea_status(&[member(&patrol, 1)], &table);
    assert_eq!(order_glyph(&status), Some(OrderGlyph::None));
    assert_eq!(status.active, Some(true));
    let empty = TaskForceCounterInputRow {
        mission_region_count: 0,
        ..inputs()
    };
    let status = at_sea_status(&[member(&empty, 1)], &table);
    assert_eq!(order_glyph(&status), Some(OrderGlyph::NoOrders));
    let passive = TaskForceCounterInputRow {
        engagement_rule: 0,
        ..inputs()
    };
    let status = at_sea_status(&[member(&passive, 2)], &table);
    assert_eq!(order_glyph(&status), Some(OrderGlyph::InvalidOrders));
    assert_eq!(status.active, Some(false));
    let mut holding = member(&patrol, 0);
    holding.fleet_regions = Some(2);
    assert_eq!(
        order_glyph(&at_sea_status(&[holding], &table)),
        Some(OrderGlyph::InvalidOrders)
    );
    let linked = TaskForceCounterInputRow {
        repair_parent: 5,
        ..empty.clone()
    };
    assert_eq!(
        order_glyph(&at_sea_status(&[member(&linked, 1)], &table)),
        Some(OrderGlyph::None)
    );
    let mut foreign = member(&empty, 1);
    foreign.own = false;
    let mut hidden = member(&empty, 1);
    hidden.visible = false;
    assert_eq!(
        order_glyph(&at_sea_status(&[foreign, hidden], &table)),
        Some(OrderGlyph::None)
    );
    assert_eq!(
        order_glyph(&port_status(&[hidden], &table)),
        Some(OrderGlyph::NoOrders)
    );
    let mut unknown = member(&patrol, 1);
    unknown.inputs = None;
    let status = at_sea_status(&[unknown], &table);
    assert_eq!(order_glyph(&status), None);
    assert_eq!(status.active, None);
}

#[test]
fn unrecorded_fleet_regions_and_aggression_leave_the_order_terms_unknown() {
    let table = [0, 60_000, 125_000, 200_000, 1_000_000_000];
    let patrol = inputs();
    let mut holding = member(&patrol, 0);
    holding.fleet_regions = None;
    let status = at_sea_status(&[holding], &table);
    assert_eq!(status.invalid_orders, None);
    assert_eq!(status.no_orders, Some(false));
    assert_eq!(order_glyph(&status), None);
    let passive = TaskForceCounterInputRow {
        engagement_rule: 0,
        ..inputs()
    };
    let status = at_sea_status(&[member(&passive, 2)], &[]);
    assert_eq!(status.invalid_orders, None);
    assert_eq!(order_glyph(&status), None);
    assert_eq!(order_glyph(&port_status(&[holding], &table)), None);
    let settled = member(&patrol, 1);
    let status = at_sea_status(&[settled, holding], &table);
    assert_eq!(status.no_orders, Some(false));
    assert_eq!(status.invalid_orders, None);
    assert_eq!(order_glyph(&status), None);
}

#[test]
fn pride_needs_a_resolving_ship_of_this_task_force() {
    let ships = |ship: u64| (ship == 51).then_some(7);
    assert_eq!(pride(7, Some(51), ships, true), Some(true));
    assert_eq!(pride(8, Some(51), ships, true), Some(false));
    assert_eq!(pride(7, Some(52), ships, true), Some(false));
    assert_eq!(pride(7, Some(52), ships, false), None);
    assert_eq!(pride(7, Some(0), ships, false), Some(false));
    assert_eq!(pride(7, None, ships, true), None);
    let row = TaskForceCounterInputRow::default();
    let mut hidden = member(&row, 1);
    hidden.visible = false;
    hidden.pride = Some(true);
    assert_eq!(at_sea_status(&[hidden], &[]).pride, Some(false));
    assert_eq!(port_status(&[hidden], &[]).pride, Some(true));
}

#[test]
fn a_reserve_fleet_is_one_whose_first_rostered_task_force_is_in_reserve() {
    let fleet = FleetCounterInputRow {
        task_forces: vec![3, 4],
        ..Default::default()
    };
    let missions = |task_force: u64| Some(if task_force == 3 { 8 } else { 1 });
    assert_eq!(reserve_fleet(&fleet, missions), Some(true));
    let reordered = FleetCounterInputRow {
        task_forces: vec![4, 3],
        ..Default::default()
    };
    assert_eq!(reserve_fleet(&reordered, missions), Some(false));
    assert_eq!(
        reserve_fleet(&FleetCounterInputRow::default(), missions),
        Some(false)
    );
    assert_eq!(reserve_fleet(&fleet, |_| None), None);
    assert_eq!(
        color(1056898815, 0, 0, 1065353216),
        Some([f32::from_bits(1056898815), 0.0, 0.0, 1.0])
    );
}

fn order_membership_frame() -> DecodedFrame {
    use replay_data::{
        rows::ArmyOrderRow, rows::OrdersGroupPlanRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 0,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "fixture".into(),
            }
            .into()
        }),
    };
    *frame.tables[TableId::OrdersGroups.index()] = TableView::Complete(TableRows::OrdersGroups(
        [(10, 20, 0), (20, 30, 1), (30, 0, 1)]
            .into_iter()
            .map(|(handle, parent, is_army_group)| OrdersGroupRow {
                handle,
                parent,
                is_army_group,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::OrdersGroupPlans.index()] =
        TableView::Complete(TableRows::OrdersGroupPlans(
            [
                (10, vec![9, 1], vec![2]),
                (20, vec![3], vec![4]),
                (30, vec![5], vec![]),
            ]
            .into_iter()
            .map(
                |(group, primary_orders, fallback_orders)| OrdersGroupPlanRow {
                    group,
                    primary_orders,
                    fallback_orders,
                    ..Default::default()
                },
            )
            .collect(),
        ));
    let plans: Vec<_> = [
        (10, 9, vec![1]),
        (10, 1, vec![1]),
        (10, 2, vec![1, 2, 3]),
        (20, 3, vec![1, 2]),
        (20, 4, vec![1, 2, 3, 4]),
        (30, 5, vec![5]),
    ]
    .into_iter()
    .map(
        |(owner_group, instance, assigned_divisions)| ArmyOrderPlanRow {
            owner_group,
            instance,
            assigned_divisions,
            ..Default::default()
        },
    )
    .collect();
    *frame.tables[TableId::ArmyOrders.index()] = TableView::Complete(TableRows::ArmyOrders(
        plans
            .iter()
            .map(|row| ArmyOrderRow {
                owner_group: row.owner_group,
                instance: row.instance,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::ArmyOrderPlans.index()] =
        TableView::Complete(TableRows::ArmyOrderPlans(plans));
    frame
}

#[test]
fn order_membership_preserves_native_list_and_parent_precedence() {
    let frame = order_membership_frame();
    let mut index = OrdersIndex::read(&frame).unwrap();
    for (division, expected) in [(1, (10, 9)), (2, (20, 3)), (3, (10, 2)), (4, (20, 4))] {
        assert_eq!(resolved_order(&index, division, 10), Some(Some(expected)));
    }
    assert_eq!(
        resolved_order(&index, 5, 10),
        Some(None),
        "no grandparent lookup"
    );
    assert_eq!(resolved_order(&index, 1, 0), Some(None));
    assert_eq!(parent_branch(1, 10, &index), Some(false));
    assert_eq!(parent_branch(2, 10, &index), Some(true));
    assert_eq!(parent_branch(4, 10, &index), Some(true));
    index.has_child.insert((20, 3), true);
    assert_eq!(parent_branch(2, 10, &index), Some(false));
    assert_eq!(parent_branch(5, 10, &index), Some(false));
    assert_eq!(parent_branch(1, 0, &index), Some(false));
    index.groups.get_mut(&10).unwrap().parent = 10;
    assert_eq!(
        resolved_order(&index, 5, 10),
        Some(None),
        "self-cycle terminates"
    );
    index.groups.get_mut(&10).unwrap().parent = 20;
    index.groups.get_mut(&20).unwrap().parent = 10;
    assert_eq!(
        resolved_order(&index, 5, 10),
        Some(None),
        "parent cycle terminates"
    );
    index.colors = [
        (10, Some([1.0, 0.0, 0.0, 1.0])),
        (20, Some([0.0, 0.0, 1.0, 1.0])),
    ]
    .into();
    assert_eq!(index.marker(10, Some(true)), Ok(Some([0.0, 0.0, 1.0, 1.0])));
    assert_eq!(
        index.marker(10, Some(false)),
        Ok(Some([1.0, 0.0, 0.0, 1.0]))
    );
    assert_eq!(index.marker(10, None), Ok(None));
}

#[test]
fn ability_root_type_uses_primary_lists_without_fallback_or_grandparent() {
    let frame = order_membership_frame();
    let mut index = OrdersIndex::read(&frame).unwrap();
    index.order_types.insert((10, 9), -7);
    index.order_types.insert((20, 3), 10);
    assert_eq!(index.root_order_type(1, 10), Some(Some(-7)));
    assert_eq!(index.root_order_type(2, 10), Some(Some(10)));
    for division in [3, 4, 5] {
        assert_eq!(index.root_order_type(division, 10), Some(None));
    }
    assert_eq!(index.root_order_type(1, 0), Some(None));
}

#[test]
fn ability_root_type_keeps_unknown_membership_and_missing_order_class() {
    let frame = order_membership_frame();
    let mut index = OrdersIndex::read(&frame).unwrap();
    index.order_types.remove(&(10, 9));
    assert_eq!(index.root_order_type(1, 10), None);
    assert_eq!(index.root_order_type(2, 10), Some(Some(0)));
    index.memberships.remove(&20);
    assert_eq!(index.root_order_type(2, 10), None);
    index.groups.get_mut(&10).unwrap().parent = 0;
    assert_eq!(index.root_order_type(2, 10), Some(None));
    index.memberships.remove(&10);
    assert_eq!(index.root_order_type(2, 10), None);
}

#[test]
fn truncated_order_memberships_use_only_proven_prefix_matches() {
    use replay_data::{rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;
    let mut frame = order_membership_frame();
    for table in [
        TableId::ArmyOrderPlans,
        TableId::OrdersGroupPlans,
        TableId::OrdersGroups,
    ] {
        let TableView::Complete(rows) = (*frame.tables[table.index()]).clone() else {
            unreachable!()
        };
        *frame.tables[table.index()] = TableView::Truncated { rows, missing: 1 };
    }
    let index = OrdersIndex::read(&frame).unwrap();
    assert_eq!(resolved_order(&index, 2, 10), Some(Some((20, 3))));
    assert_eq!(resolved_order(&index, 6, 10), Some(None));
    let TableView::Truncated {
        rows: TableRows::ArmyOrderPlans(plans),
        ..
    } = &mut *frame.tables[TableId::ArmyOrderPlans.index()]
    else {
        unreachable!()
    };
    plans.retain(|row| (row.owner_group, row.instance) != (10, 1));
    let index = OrdersIndex::read(&frame).unwrap();
    assert_eq!(resolved_order(&index, 1, 10), Some(Some((10, 9))));
    assert_eq!(resolved_order(&index, 2, 10), None);
    let TableView::Truncated {
        rows: TableRows::OrdersGroupPlans(groups),
        ..
    } = &mut *frame.tables[TableId::OrdersGroupPlans.index()]
    else {
        unreachable!()
    };
    groups
        .iter_mut()
        .find(|row| row.group == 10)
        .unwrap()
        .primary_orders
        .reverse();
    assert_eq!(
        resolved_order(&OrdersIndex::read(&frame).unwrap(), 1, 10),
        None
    );
}

#[test]
fn empty_order_lists_are_known_but_missing_parent_operands_are_not() {
    use replay_data::{rows::OrdersGroupPlanRow, rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;
    let mut frame = order_membership_frame();
    *frame.tables[TableId::OrdersGroupPlans.index()] = TableView::Truncated {
        rows: TableRows::OrdersGroupPlans(vec![
            OrdersGroupPlanRow {
                group: 10,
                ..Default::default()
            },
            OrdersGroupPlanRow {
                group: 20,
                ..Default::default()
            },
        ]),
        missing: 1,
    };
    *frame.tables[TableId::ArmyOrderPlans.index()] = TableView::Unavailable {
        reason: "unused".into(),
    };
    let mut index = OrdersIndex::read(&frame).unwrap();
    assert_eq!(resolved_order(&index, 1, 10), Some(None));
    index.memberships.remove(&20);
    assert_eq!(resolved_order(&index, 1, 10), None);
    index.groups.get_mut(&10).unwrap().parent = 0;
    assert_eq!(resolved_order(&index, 1, 10), Some(None));
    index.memberships.remove(&10);
    assert_eq!(resolved_order(&index, 1, 10), None);
    assert_eq!(
        parent_branch(1, 10, &index),
        Some(false),
        "no parent needs no order lookup"
    );
}

#[test]
fn area_defence_keeps_positive_evidence_and_requires_complete_negatives() {
    let frame = order_membership_frame();
    let mut index = OrdersIndex::read(&frame).unwrap();
    assert_eq!(area_defence(1, 10, &index), Some(false));
    index.orders_complete = false;
    assert_eq!(area_defence(1, 10, &index), None);
    index.order_types.insert((10, 9), AREA_DEFENCE_ORDER);
    assert_eq!(area_defence(1, 10, &index), Some(true));
    index.order_types.insert((10, 9), 0);
    index.area_groups.insert(20);
    assert_eq!(
        area_defence(1, 10, &index),
        None,
        "parent area ignored with a resolved order"
    );
    assert_eq!(
        area_defence(5, 10, &index),
        Some(true),
        "no resolved order permits parent area"
    );
    index.area_groups.insert(10);
    assert_eq!(area_defence(1, 10, &index), Some(true));
    index.area_groups.clear();
    index.orders_complete = true;
    index.order_types.remove(&(10, 9));
    assert_eq!(
        area_defence(1, 10, &index),
        None,
        "missing referenced order stays unknown"
    );
    index.memberships.remove(&10);
    index.area_groups.insert(10);
    assert_eq!(
        area_defence(1, 10, &index),
        Some(true),
        "own-group targets decide the result even when membership is unknown"
    );
    assert_eq!(area_defence(1, 0, &index), Some(false));
}

#[test]
fn the_marker_hides_without_a_group_or_a_colour() {
    let index = OrdersIndex::default();
    assert_eq!(index.marker(0, Some(false)), Ok(None));
    assert_eq!(index.marker(10, Some(false)), Ok(None));
    assert_eq!(index.marker(10, None), Ok(None));
    let index = OrdersIndex {
        groups: [(
            10,
            GroupFacts {
                parent: 0,
                army_group: false,
            },
        )]
        .into(),
        has_child: BTreeMap::new(),
        colors: [(10, Some([1.0, 0.0, 0.0, 1.0]))].into(),
        ..Default::default()
    };
    assert_eq!(
        index.marker(10, Some(false)),
        Ok(Some([1.0, 0.0, 0.0, 1.0]))
    );
}

#[test]
fn non_finite_recorded_colors_are_refused() {
    use replay_data::{rows::OrdersGroupPlanRow, rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;
    let valid = [0.25f32, 0.5, 0.75, 1.0].map(f32::to_bits);
    assert_eq!(
        color(valid[0], valid[1], valid[2], valid[3]),
        Some([0.25, 0.5, 0.75, 1.0])
    );
    assert_eq!(
        color(
            (-0.0f32).to_bits(),
            0.0f32.to_bits(),
            1.5f32.to_bits(),
            1.0f32.to_bits()
        ),
        Some([-0.0, 0.0, 1.5, 1.0])
    );
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for channel in 0..4 {
            let mut raw = valid;
            raw[channel] = bad.to_bits();
            assert_eq!(color(raw[0], raw[1], raw[2], raw[3]), None);
        }
    }
    let mut frame = order_membership_frame();
    *frame.tables[TableId::OrdersGroupPlans.index()] =
        TableView::Complete(TableRows::OrdersGroupPlans(vec![
            OrdersGroupPlanRow {
                group: 10,
                red: f32::NAN.to_bits(),
                green: 0.25f32.to_bits(),
                blue: 0.5f32.to_bits(),
                alpha: 1.0f32.to_bits(),
                ..Default::default()
            },
            OrdersGroupPlanRow {
                group: 20,
                red: 0.1f32.to_bits(),
                green: f32::INFINITY.to_bits(),
                blue: 0.5f32.to_bits(),
                alpha: 1.0f32.to_bits(),
                ..Default::default()
            },
            OrdersGroupPlanRow {
                group: 30,
                red: 0.1f32.to_bits(),
                green: 0.2f32.to_bits(),
                blue: 0.3f32.to_bits(),
                alpha: 1.0f32.to_bits(),
                ..Default::default()
            },
        ]));
    let index = OrdersIndex::read(&frame).unwrap();
    assert_eq!(index.marker(10, Some(false)), Err(10));
    assert_eq!(index.marker(10, Some(true)), Err(20));
    assert_eq!(
        index.marker(30, Some(false)),
        Ok(Some([0.1, 0.2, 0.3, 1.0]))
    );
    assert_eq!(index.marker(99, Some(false)), Ok(None));
}
