use super::*;
use replay_data::{
    rows::AirWingRow, rows::CountryLinkRow, rows::InvasionPreparationRow, rows::LandTheatreRow,
    rows::OrdersGroupPlanRow, rows::ParadropPreparationInputs, rows::ParadropPreparationTerms,
    rows::ParadropTransportModifiers, rows::ProvinceRow, rows::Recorded, rows::RowCodec,
    rows::TableId, rows::TableSchema,
};
use replay_frame::{frame::Metrics, frame::TableView};
use viewer_runtime::memo::TableKey;
use viewer_runtime::memo::project;

const GROUP: u64 = 0x1_0000_0035;
const UNIT: u64 = 0x2_0000_0061;

fn defines() -> Defines {
    Defines {
        naval_days: Ok(1),
        air_days: Ok(1),
        unit_weight: Ok(10 * ONE),
        minimum_planes: Ok(1),
    }
}

fn empty() -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour: 10,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "fixture deliberately omitted this table".into(),
            }
            .into()
        }),
    }
}

fn put<T: TableSchema>(frame: &mut DecodedFrame, mut rows: Vec<T::Row>) {
    rows.sort_by_key(RowCodec::key);
    frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows)).into();
}

fn order(instance: u32, kind: i32) -> ArmyOrderRow {
    ArmyOrderRow {
        owner_group: GROUP,
        instance,
        order_type: kind,
        active: 1,
        paradrop_target: 1,
        path: vec![2],
        ..Default::default()
    }
}

fn plan(instance: u32) -> ArmyOrderPlanRow {
    ArmyOrderPlanRow {
        owner_group: GROUP,
        instance,
        ..Default::default()
    }
}

fn evaluate(
    frame: &DecodedFrame,
    defines: &Defines,
    order: &ArmyOrderRow,
) -> Result<State, String> {
    evaluate_with(
        frame,
        defines,
        order,
        &paradrop_counts::Context::new(frame, None),
    )
}

fn evaluate_with<'a>(
    frame: &'a DecodedFrame,
    defines: &Defines,
    order: &ArmyOrderRow,
    transports: &paradrop_counts::Context<'a>,
) -> Result<State, String> {
    let orders = frame
        .complete::<tables::ArmyOrders>()
        .unwrap_or(&[])
        .iter()
        .map(|row| ((row.owner_group, row.instance), row))
        .collect();
    let plans = frame
        .complete::<tables::ArmyOrderPlans>()
        .unwrap_or(&[])
        .iter()
        .map(|row| ((row.owner_group, row.instance), row))
        .collect();
    let groups = frame
        .complete::<tables::OrdersGroups>()
        .unwrap_or(&[])
        .iter()
        .map(|row| (row.handle, row))
        .collect();
    Context::new(frame, defines, &orders, &plans, &groups, transports).callback(order)
}

fn prepared_frame() -> DecodedFrame {
    let mut frame = empty();
    let root = order(1, super::super::TYPE_MOVE);
    let mut drop = order(2, TYPE_PARADROP);
    drop.active = 0;
    let mut root_plan = plan(1);
    root_plan.successors = vec![2];
    let mut drop_plan = plan(2);
    drop_plan.assigned_divisions = vec![UNIT];
    drop_plan.execution = Recorded::Value(Some(OrderExecutionInputs {
        route_is_ok: Some(255),
        paradrop: Some(ParadropPreparationInputs {
            progress: 24 * ONE,
            terms: Some(ParadropPreparationTerms::default()),
        }),
    }));
    put::<tables::ArmyOrders>(&mut frame, vec![root, drop]);
    put::<tables::ArmyOrderPlans>(&mut frame, vec![root_plan, drop_plan]);
    put::<tables::OrdersGroups>(
        &mut frame,
        vec![OrdersGroupRow {
            handle: GROUP,
            theatre: 7,
            theatre_ordinal: Some(0),
            ..Default::default()
        }],
    );
    put::<tables::OrdersGroupPlans>(
        &mut frame,
        vec![OrdersGroupPlanRow {
            group: GROUP,
            primary_orders: vec![1, 2],
            ..Default::default()
        }],
    );
    put::<tables::LandTheatres>(
        &mut frame,
        vec![LandTheatreRow {
            id: 7,
            ..Default::default()
        }],
    );
    put::<tables::Divisions>(
        &mut frame,
        vec![DivisionRow {
            id: UNIT,
            province: 1,
            orders_group: GROUP,
            ..Default::default()
        }],
    );
    put::<tables::DivisionMovement>(
        &mut frame,
        vec![DivisionMovementRow {
            division: UNIT,
            order_transport: Recorded::Value(Some(DivisionOrderTransportInputs {
                paradrop_eligible: Some(1),
                paradrop_modifiers: Some(ParadropTransportModifiers::default()),
                ..Default::default()
            })),
            ..Default::default()
        }],
    );
    put::<tables::Provinces>(
        &mut frame,
        vec![ProvinceRow {
            id: 1,
            ..Default::default()
        }],
    );
    frame
}

#[test]
fn inactive_arrows_need_no_recording_or_content_execution_inputs() {
    let frame = empty();
    let mut row = order(1, TYPE_PARADROP);
    row.active = 0;
    let unavailable = Defines {
        naval_days: Err("absent".into()),
        air_days: Err("absent".into()),
        unit_weight: Err("absent".into()),
        minimum_planes: Err("absent".into()),
    };
    assert_eq!(evaluate(&frame, &unavailable, &row), Ok(State::default()));
}

#[test]
fn route_zero_stops_before_missing_preparation_unit_or_wing_sources() {
    let mut frame = empty();
    let row = order(1, TYPE_PARADROP);
    let mut pack = plan(1);
    pack.execution = Recorded::Value(Some(OrderExecutionInputs {
        route_is_ok: Some(0),
        paradrop: None,
    }));
    put::<tables::ArmyOrders>(&mut frame, vec![row.clone()]);
    put::<tables::ArmyOrderPlans>(&mut frame, vec![pack.clone()]);
    assert_eq!(
        evaluate(&frame, &defines(), &row),
        Ok(State {
            active: true,
            prerequisites: false
        })
    );
    pack.execution = Recorded::NotRecorded;
    put::<tables::ArmyOrderPlans>(&mut frame, vec![pack.clone()]);
    assert!(
        evaluate(&frame, &defines(), &row)
            .unwrap_err()
            .contains("not recorded")
    );
    pack.execution = Recorded::Value(Some(OrderExecutionInputs::default()));
    put::<tables::ArmyOrderPlans>(&mut frame, vec![pack]);
    assert!(
        evaluate(&frame, &defines(), &row)
            .unwrap_err()
            .contains("route_is_ok")
    );
}

#[test]
fn active_ordinary_callback_uses_inactive_successors_current_transport_fields() {
    let mut frame = prepared_frame();
    let root = order(1, super::super::TYPE_MOVE);
    let first = frame.clone();
    assert!(!evaluate(&frame, &defines(), &root).unwrap().prerequisites);
    let mut rows = frame
        .complete::<tables::DivisionMovement>()
        .unwrap()
        .to_vec();
    rows[0].order_transport = Recorded::Value(Some(DivisionOrderTransportInputs {
        paradrop_eligible: Some(1),
        paradrop_modifiers: Some(ParadropTransportModifiers {
            country_term: -ONE,
            commander_term: None,
        }),
        ..Default::default()
    }));
    put::<tables::DivisionMovement>(&mut frame, rows);
    let tables = super::super::memo_tables(false);
    assert!(TableKey::of(&first, &tables) == TableKey::of(&frame, &tables));
    assert!(super::super::requires_full_frame(&frame, &BTreeMap::new(), 0, false, false).unwrap());
    assert!(evaluate(&frame, &defines(), &root).unwrap().prerequisites);
    let projected = project(&frame, &tables);
    assert!(evaluate(&projected, &defines(), &root).is_err());
}

#[test]
fn the_full_frame_guard_keeps_hidden_foreign_activity_outside_admitted_groups() {
    let mut frame = prepared_frame();
    let mut orders = frame.complete::<tables::ArmyOrders>().unwrap().to_vec();
    for row in &mut orders {
        row.active = 0;
        row.order_type = super::super::TYPE_MOVE;
    }
    let mut foreign = order(1, TYPE_PARADROP);
    foreign.owner_group = 999;
    orders.push(foreign);
    put::<tables::ArmyOrders>(&mut frame, orders);
    assert!(!super::super::requires_full_frame(&frame, &BTreeMap::new(), 0, false, false).unwrap());
}

#[test]
fn insufficient_preparation_stops_before_unavailable_units_and_air() {
    let mut frame = prepared_frame();
    let mut plans = frame.complete::<tables::ArmyOrderPlans>().unwrap().to_vec();
    plans[1].execution = Recorded::Value(Some(OrderExecutionInputs {
        route_is_ok: Some(1),
        paradrop: Some(ParadropPreparationInputs {
            progress: 0,
            terms: Some(ParadropPreparationTerms::default()),
        }),
    }));
    put::<tables::ArmyOrderPlans>(&mut frame, plans);
    frame.tables[tables::Divisions::ID.index()] = TableView::Unavailable {
        reason: "not demanded".into(),
    }
    .into();
    assert!(
        !evaluate(&frame, &defines(), &order(1, super::super::TYPE_MOVE))
            .unwrap()
            .prerequisites
    );
}

#[test]
fn own_primary_order_precedes_the_later_paradrop_primary() {
    let mut frame = prepared_frame();
    let mut plans = frame.complete::<tables::ArmyOrderPlans>().unwrap().to_vec();
    plans[0].assigned_divisions = vec![UNIT];
    put::<tables::ArmyOrderPlans>(&mut frame, plans);
    assert!(
        evaluate(&frame, &defines(), &order(1, super::super::TYPE_MOVE))
            .unwrap()
            .prerequisites
    );
}

fn air_fixture() -> paradrop_counts::tests::Fixture {
    let mut fixture = paradrop_counts::tests::Fixture::new();
    let mut frame = prepared_frame();
    for table in [
        TableId::Provinces,
        TableId::CountryLinks,
        TableId::AirBases,
        TableId::AirBaseRegistrations,
        TableId::AirWings,
        TableId::RaidCountryInputs,
    ] {
        frame.tables[table.index()] = fixture.frame.tables[table.index()].clone();
    }
    let mut groups = frame.complete::<tables::OrdersGroups>().unwrap().to_vec();
    groups[0].country = 77;
    put::<tables::OrdersGroups>(&mut frame, groups);
    let mut theatres = frame.complete::<tables::LandTheatres>().unwrap().to_vec();
    theatres[0].owner = 77;
    put::<tables::LandTheatres>(&mut frame, theatres);
    fixture.frame = frame;
    fixture
}

#[test]
fn accepted_wing_capacity_threshold_and_duplicate_unit_demand_change_execution() {
    content_reader_script::test_game::enter();
    let mut fixture = air_fixture();
    let root = order(1, super::super::TYPE_MOVE);
    let defs = defines();
    assert!(
        !evaluate_with(&fixture.frame, &defs, &root, &fixture.context())
            .unwrap()
            .prerequisites
    );
    let first = fixture.frame.clone();
    let mut wings = fixture
        .frame
        .complete::<tables::AirWings>()
        .unwrap()
        .to_vec();
    wings[0].planes = 10;
    put::<tables::AirWings>(&mut fixture.frame, wings);
    assert!(
        evaluate_with(&fixture.frame, &defs, &root, &fixture.context())
            .unwrap()
            .prerequisites
    );
    let memo_tables = super::super::memo_tables(false);
    assert!(TableKey::of(&first, &memo_tables) == TableKey::of(&fixture.frame, &memo_tables));
    assert!(
        super::super::requires_full_frame(
            &fixture.frame,
            &BTreeMap::from([(77, 4)]),
            4,
            false,
            false
        )
        .unwrap()
    );
    let mut minimum = defines();
    minimum.minimum_planes = Ok(11);
    assert!(
        !evaluate_with(&fixture.frame, &minimum, &root, &fixture.context())
            .unwrap()
            .prerequisites
    );
    let mut plans = fixture
        .frame
        .complete::<tables::ArmyOrderPlans>()
        .unwrap()
        .to_vec();
    plans[1].assigned_divisions.push(UNIT);
    put::<tables::ArmyOrderPlans>(&mut fixture.frame, plans);
    assert!(
        !evaluate_with(&fixture.frame, &defs, &root, &fixture.context())
            .unwrap()
            .prerequisites
    );
}

#[test]
fn signed_invalid_source_and_ineligible_units_skip_later_demand_operands() {
    content_reader_script::test_game::enter();
    let mut fixture = air_fixture();
    let root = order(1, super::super::TYPE_MOVE);
    let mut orders = fixture
        .frame
        .complete::<tables::ArmyOrders>()
        .unwrap()
        .to_vec();
    orders[1].paradrop_target = u32::MAX;
    put::<tables::ArmyOrders>(&mut fixture.frame, orders.clone());
    fixture.frame.tables[TableId::DivisionMovement.index()] = TableView::Unavailable {
        reason: "invalid source reaches no unit or wing demand".into(),
    }
    .into();
    assert!(
        evaluate_with(&fixture.frame, &defines(), &root, &fixture.context())
            .unwrap()
            .prerequisites
    );
    orders[1].paradrop_target = 1;
    put::<tables::ArmyOrders>(&mut fixture.frame, orders);
    put::<tables::DivisionMovement>(
        &mut fixture.frame,
        vec![DivisionMovementRow {
            division: UNIT,
            order_transport: Recorded::Value(Some(DivisionOrderTransportInputs {
                paradrop_eligible: Some(0),
                ..Default::default()
            })),
            ..Default::default()
        }],
    );
    let mut defs = defines();
    defs.unit_weight = Err("ineligible unit needs no weight define".into());
    assert!(
        evaluate_with(&fixture.frame, &defs, &root, &fixture.context())
            .unwrap()
            .prerequisites
    );
}

#[test]
fn naval_transfer_skips_weight_but_convoy_shortage_still_reaches_preparation() {
    let mut frame = prepared_frame();
    let row = order(1, TYPE_INVASION);
    let mut pack = plan(1);
    pack.assigned_divisions = vec![UNIT, UNIT];
    pack.invasion_transports = Recorded::Value(Some(1));
    pack.execution = Recorded::Value(Some(OrderExecutionInputs {
        route_is_ok: Some(1),
        paradrop: None,
    }));
    put::<tables::ArmyOrders>(&mut frame, vec![row.clone()]);
    put::<tables::ArmyOrderPlans>(&mut frame, vec![pack.clone()]);
    let mut movement = DivisionMovementRow {
        division: UNIT,
        order_transport: Recorded::Value(Some(DivisionOrderTransportInputs {
            naval_weight: Some(60_000),
            ..Default::default()
        })),
        ..Default::default()
    };
    put::<tables::DivisionMovement>(&mut frame, vec![movement.clone()]);
    assert!(
        evaluate(&frame, &defines(), &row)
            .unwrap_err()
            .contains("InvasionPreparations")
    );
    put::<tables::InvasionPreparations>(
        &mut frame,
        vec![InvasionPreparationRow {
            owner_group: GROUP,
            instance: 1,
            progress: 24 * ONE,
            ..Default::default()
        }],
    );
    assert!(!evaluate(&frame, &defines(), &row).unwrap().prerequisites);
    movement.naval_transfer = 1;
    movement.order_transport = Recorded::NotRecorded;
    put::<tables::DivisionMovement>(&mut frame, vec![movement.clone()]);
    assert!(evaluate(&frame, &defines(), &row).unwrap().prerequisites);
    pack.execution = Recorded::Value(Some(OrderExecutionInputs {
        route_is_ok: Some(0),
        paradrop: None,
    }));
    put::<tables::ArmyOrderPlans>(&mut frame, vec![pack.clone()]);
    assert!(!evaluate(&frame, &defines(), &row).unwrap().prerequisites);
    movement.naval_transfer = 0;
    movement.order_transport = Recorded::Value(Some(DivisionOrderTransportInputs {
        naval_weight: Some(-100_001),
        ..Default::default()
    }));
    put::<tables::DivisionMovement>(&mut frame, vec![movement]);
    pack.invasion_transports = Recorded::Value(Some(-1));
    pack.execution = Recorded::Value(Some(OrderExecutionInputs {
        route_is_ok: Some(1),
        paradrop: None,
    }));
    put::<tables::ArmyOrderPlans>(&mut frame, vec![pack]);
    assert!(evaluate(&frame, &defines(), &row).unwrap().prerequisites);
}

#[test]
fn ordered_graph_stops_at_first_unknown_or_false_and_memoises_shared_successors() {
    let rows: Vec<_> = (1..=4).map(|id| order(id, 1)).collect();
    let mut packs: Vec<_> = (1..=4).map(plan).collect();
    packs[0].successors = vec![2, 3];
    packs[1].successors = vec![4];
    packs[2].successors = vec![4];
    let orders = rows
        .iter()
        .map(|row| ((GROUP, row.instance), row))
        .collect();
    let plans = packs
        .iter()
        .map(|row| ((GROUP, row.instance), row))
        .collect();
    let mut calls = Vec::new();
    let mut memo = BTreeMap::new();
    assert_eq!(
        ordered_conditions((GROUP, 1), &orders, &plans, &mut memo, |row, _| {
            calls.push(row.instance);
            Ok(true)
        }),
        Ok(true)
    );
    assert_eq!(calls, [1, 2, 4, 3]);
    calls.clear();
    memo.clear();
    assert_eq!(
        ordered_conditions((GROUP, 1), &orders, &plans, &mut memo, |row, _| {
            calls.push(row.instance);
            if row.instance == 2 {
                Err("first unknown".into())
            } else {
                Ok(false)
            }
        }),
        Ok(false)
    );
    assert_eq!(calls, [1]);
    calls.clear();
    memo.clear();
    assert_eq!(
        ordered_conditions((GROUP, 1), &orders, &plans, &mut memo, |row, _| {
            calls.push(row.instance);
            match row.instance {
                2 => Err("first unknown".into()),
                3 => Ok(false),
                _ => Ok(true),
            }
        }),
        Err("first unknown".into())
    );
    assert_eq!(calls, [1, 2]);
}

#[test]
fn cycles_and_missing_successors_refuse_at_the_reached_native_position() {
    let rows = [order(1, 1), order(2, 1)];
    let mut packs = [plan(1), plan(2)];
    packs[0].successors = vec![1, 2];
    let orders = rows
        .iter()
        .map(|row| ((GROUP, row.instance), row))
        .collect();
    {
        let plans = packs
            .iter()
            .map(|row| ((GROUP, row.instance), row))
            .collect();
        assert!(
            ordered_conditions(
                (GROUP, 1),
                &orders,
                &plans,
                &mut BTreeMap::new(),
                |row, _| Ok(row.instance != 2)
            )
            .unwrap_err()
            .contains("cycles")
        );
    }
    packs[0].successors = vec![999, 2];
    let plans = packs
        .iter()
        .map(|row| ((GROUP, row.instance), row))
        .collect();
    assert!(
        ordered_conditions(
            (GROUP, 1),
            &orders,
            &plans,
            &mut BTreeMap::new(),
            |row, _| Ok(row.instance != 2)
        )
        .unwrap_err()
        .contains("no order row")
    );
}

#[test]
fn reserved_native_id_zero_preserves_the_relation_alias_zero_exception() {
    assert_eq!(
        relation_gate(77, 0, |_| panic!("id0 skips same-country links")),
        Ok(true)
    );
    assert_eq!(relation_gate(77, 9, |_| Ok(0)), Ok(false));
    assert_eq!(
        relation_gate(u32::MAX, 0, |_| panic!("negative target skips links")),
        Ok(false)
    );
    assert_eq!(
        relation_gate(9, 9, |_| panic!("equal raw id skips links")),
        Ok(false)
    );
    assert!(relation_gate(77, 9, |_| Err("missing link".into())).is_err());
}

#[test]
fn captured_reserved_preparation_relation_is_independent_of_naming_and_positive_country_census() {
    let mut frame = prepared_frame();
    let mut orders = frame.complete::<tables::ArmyOrders>().unwrap().to_vec();
    orders[1].paradrop_target = 0;
    put::<tables::ArmyOrders>(&mut frame, orders);
    let mut plans = frame.complete::<tables::ArmyOrderPlans>().unwrap().to_vec();
    plans[1].execution = Recorded::Value(Some(OrderExecutionInputs {
        route_is_ok: Some(1),
        paradrop: Some(ParadropPreparationInputs {
            progress: 48 * ONE,
            terms: Some(ParadropPreparationTerms {
                target: 77,
                relation_term: Some(ONE),
                ..Default::default()
            }),
        }),
    }));
    put::<tables::ArmyOrderPlans>(&mut frame, plans);
    let root = order(1, super::super::TYPE_MOVE);
    assert!(evaluate(&frame, &defines(), &root).unwrap().prerequisites);
    let mut groups = frame.complete::<tables::OrdersGroups>().unwrap().to_vec();
    groups[0].country = 99;
    put::<tables::OrdersGroups>(&mut frame, groups);
    put::<tables::CountryLinks>(
        &mut frame,
        vec![
            CountryLinkRow {
                id: 77,
                canonical_slot: 0,
            },
            CountryLinkRow {
                id: 99,
                canonical_slot: 0,
            },
        ],
    );
    assert!(evaluate(&frame, &defines(), &root).unwrap().prerequisites);
}

#[test]
fn native_integer_rounding_signed_weights_and_extreme_sentinels_remain_exact() {
    assert_eq!([1, -1, 100_001, -100_001].map(naval_count), [1, 0, 2, -1]);
    assert_eq!(ceil_fixed(i64::MAX), -9_223_372_036_854_751_616);
    assert_eq!(preparation_hours(1, -50_000, 0, 0, 0), 0);
    assert_eq!(
        preparation_hours(60, 10_000, 20_000, -50_000, 10 * ONE),
        56 * 24
    );
    assert_eq!(transport_weight(10 * ONE, -50_000, -50_000), 0);
    assert_eq!(missing_transports(0, 0), 0);
    assert_eq!(missing_transports(1, 0), 0);
    assert_eq!(missing_transports(100, 0), 1);
    assert_eq!(missing_transports(-100, 0), 0);
    for delta in [i64::MIN, i64::MIN + 1, i64::MAX] {
        assert_eq!(missing_transports(delta, 0), 42_950);
    }
}

#[test]
fn shader_gates_keep_inner_active_fx_independent_from_blocked_texture_speed() {
    let epsilon = f32::from_bits(0x38d1_b717);
    let above = f32::from_bits(0x38d1_b718);
    let blocked = State {
        active: true,
        prerequisites: false,
    };
    let ready = State {
        active: true,
        prerequisites: true,
    };
    assert_eq!(blocked.shader_terms(2.0, false), [0.0, 1.0]);
    assert_eq!(blocked.shader_terms(2.0, true), [0.0, 0.0]);
    assert_eq!(ready.shader_terms(2.0, false), [2.0, 1.0]);
    assert_eq!(State::default().shader_terms(2.0, false), [2.0, 0.0]);
    for value in [epsilon, -epsilon, 0.0, -0.0] {
        assert_eq!(ready.shader_terms(value, false), [0.0, 1.0]);
    }
    assert_eq!(ready.shader_terms(-above, false), [-above, 1.0]);
}

#[test]
fn accepted_capacity_keeps_signed_plane_counts_and_duplicate_positive_geometry() {
    struct Wings([AirWingRow; 2]);
    impl transport_counts::Sources for Wings {
        fn wing(&self, handle: u64) -> Result<&AirWingRow, String> {
            self.0
                .iter()
                .find(|row| row.id == handle)
                .ok_or("missing".into())
        }
        fn is_transport_pool(&self, _: &AirWingRow) -> Result<bool, String> {
            Ok(true)
        }
        fn range(&self, _: &AirWingRow) -> Result<i64, String> {
            Ok(100_000)
        }
        fn transfer_resolves(&self, _: u64) -> Result<bool, String> {
            panic!("not reached")
        }
        fn region_admits(&self, _: &AirWingRow) -> Result<bool, String> {
            panic!("not reached")
        }
    }
    let wings = Wings([
        AirWingRow {
            id: 1,
            planes: 7,
            ..Default::default()
        },
        AirWingRow {
            id: 2,
            planes: -3,
            ..Default::default()
        },
    ]);
    assert_eq!(
        transport_counts::totals(&[1, 2, 1], 0, false, &wings),
        Ok(transport_counts::AcceptedTotals {
            plane_count: 11,
            capacity_raw: 14 * ONE
        })
    );
    assert_eq!(
        transport_counts::count(&[1, 2, 1], 0, false, &wings),
        Ok(11)
    );
}
