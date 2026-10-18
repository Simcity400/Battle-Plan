use super::*;
use replay_data::rows::OrderChildFrontInputs;

const GROUP: u64 = (1 << 32) | engine_native::corders_group::REFERENCE_TYPE as u64;
const CHILD: u64 = (2 << 32) | engine_native::corders_group::REFERENCE_TYPE as u64;
fn plan(area: i32, sections: Vec<OrderChildFront>) -> ArmyOrderPlanRow {
    ArmyOrderPlanRow {
        owner_group: GROUP,
        segment_start: 8,
        child_fronts: Recorded::Value(Some(OrderChildFrontInputs {
            area_province_count: area,
            count: Some(sections.len() as u32),
            group_child_count: Some(sections.len() as i32),
            sections: Some(sections),
        })),
        ..Default::default()
    }
}

#[test]
fn duplicate_slots_use_first_span_and_existing_parent_units_in_reverse_order() {
    let mut plan = plan(
        3,
        vec![
            OrderChildFront {
                group: CHILD,
                segment_start: 1,
                segment_end: 2,
            },
            OrderChildFront {
                group: CHILD,
                segment_start: 0,
                segment_end: 0,
            },
        ],
    );
    let Recorded::Value(Some(inputs)) = &mut plan.child_fronts else {
        unreachable!()
    };
    // Only the native-demand prefix is supplied; an unused raw stored tail
    // cannot change either indexed group's first matching span.
    inputs.count = Some(17);
    let pairs = [
        Pair {
            source: 1,
            target: 2,
            units: 3,
        },
        Pair {
            source: 2,
            target: 3,
            units: 4,
        },
        Pair {
            source: 3,
            target: 4,
            units: 5,
        },
    ];
    let result = children(&plan, &pairs).unwrap();
    assert_eq!(
        result
            .rows
            .iter()
            .map(|child| child.slot)
            .collect::<Vec<_>>(),
        [1, 0]
    );
    assert!(result.rows.iter().all(|child| child.pairs == &pairs[1..3]));
    assert!(result.refused.is_empty());
    assert_eq!(extra_units(&plan).unwrap(), 1);
}

#[test]
fn child_payload_must_cover_exactly_the_demanded_prefix() {
    let section = OrderChildFront {
        group: CHILD,
        segment_start: 0,
        segment_end: 0,
    };
    let pairs = [Pair {
        source: 1,
        target: 2,
        units: 3,
    }];
    for (sections, expected) in [
        (vec![section.clone()], true),
        (vec![], false),
        (vec![section.clone(), section], false),
    ] {
        let mut plan = plan(3, sections);
        let Recorded::Value(Some(inputs)) = &mut plan.child_fronts else {
            unreachable!()
        };
        inputs.count = Some(17);
        inputs.group_child_count = Some(1);
        assert_eq!(children(&plan, &pairs).is_ok(), expected);
        assert_eq!(extra_units(&plan).unwrap(), 1);
    }
}

#[test]
fn scalar_prefix_survives_payload_failure_and_small_area_shortcircuits_header() {
    let mut plan = plan(3, Vec::new());
    let Recorded::Value(Some(inputs)) = &mut plan.child_fronts else {
        unreachable!()
    };
    inputs.count = Some(2);
    inputs.group_child_count = Some(2);
    inputs.sections = None;
    assert_eq!(extra_units(&plan).unwrap(), 1);
    assert!(children(&plan, &[]).is_err());
    let Recorded::Value(Some(inputs)) = &mut plan.child_fronts else {
        unreachable!()
    };
    inputs.area_province_count = 2;
    inputs.count = None;
    inputs.group_child_count = None;
    assert_eq!(extra_units(&plan).unwrap(), 0);
    assert!(children(&plan, &[]).unwrap().rows.is_empty());
}

#[test]
fn zero_group_count_shortcircuits_payload_but_unknown_and_negative_refuse() {
    let mut plan = plan(3, Vec::new());
    let Recorded::Value(Some(inputs)) = &mut plan.child_fronts else {
        unreachable!()
    };
    inputs.count = Some(7);
    inputs.sections = None;
    inputs.group_child_count = Some(0);
    assert!(children(&plan, &[]).unwrap().rows.is_empty());
    let Recorded::Value(Some(inputs)) = &mut plan.child_fronts else {
        unreachable!()
    };
    inputs.group_child_count = None;
    assert_eq!(
        children(&plan, &[]).err(),
        Some("army-group current child count unavailable")
    );
    let Recorded::Value(Some(inputs)) = &mut plan.child_fronts else {
        unreachable!()
    };
    inputs.group_child_count = Some(-1);
    assert_eq!(
        children(&plan, &[]).err(),
        Some("army-group current child count is negative")
    );
}

#[test]
fn child_assignment_filter_and_child_hq_follow_callback_override_rules() {
    let mut plan = plan(3, Vec::new());
    plan.assigned_divisions = vec![10, 11, 12];
    let group = OrdersGroupRow {
        handle: CHILD,
        leader: 99,
        hq_provider: 11,
        ..Default::default()
    };
    let groups = BTreeMap::from([(CHILD, &group)]);
    let group_plans = BTreeMap::new();
    let divisions = [
        DivisionRow {
            id: 10,
            orders_group: CHILD,
            ..Default::default()
        },
        DivisionRow {
            id: 11,
            orders_group: CHILD,
            ..Default::default()
        },
        DivisionRow {
            id: 12,
            orders_group: GROUP,
            ..Default::default()
        },
    ];
    let division_groups = DivisionGroups {
        rows: Some(&divisions),
        index: OnceCell::new(),
    };
    let display = Display {
        plan: &plan,
        groups: &groups,
        group_plans: &group_plans,
        division_groups: &division_groups,
        owner_is_ours: Some(true),
        kind: Default::default(),
    };
    assert_eq!(assigned_count(CHILD, &display).unwrap(), 1);
    assert!(division_groups.index.get().is_some());
    plan.owner_override = 7;
    let division_groups = DivisionGroups {
        rows: None,
        index: OnceCell::new(),
    };
    let display = Display {
        plan: &plan,
        groups: &groups,
        group_plans: &group_plans,
        division_groups: &division_groups,
        owner_is_ours: Some(true),
        kind: Default::default(),
    };
    assert_eq!(assigned_count(CHILD, &display).unwrap(), 2);
    assert!(division_groups.index.get().is_none());
}

#[test]
fn current_group_slots_past_the_stored_array_are_native_no_ribbon() {
    let mut plan = plan(
        3,
        vec![OrderChildFront {
            group: CHILD,
            segment_start: 0,
            segment_end: 0,
        }],
    );
    let Recorded::Value(Some(inputs)) = &mut plan.child_fronts else {
        unreachable!()
    };
    inputs.group_child_count = Some(i32::MAX);
    let pairs = [Pair {
        source: 1,
        target: 2,
        units: 4,
    }];
    let result = children(&plan, &pairs).unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].slot, 0);
    assert_eq!(result.rows[0].pairs, pairs);
    assert!(result.refused.is_empty());
}

#[test]
fn virtual_callback_never_demands_child_metadata_or_assignment_index() {
    let mut plan = plan(3, Vec::new());
    plan.virtual_order = 1;
    plan.assigned_divisions = vec![10];
    let groups = BTreeMap::new();
    let group_plans = BTreeMap::new();
    let division_groups = DivisionGroups {
        rows: None,
        index: OnceCell::new(),
    };
    let display = Display {
        plan: &plan,
        groups: &groups,
        group_plans: &group_plans,
        division_groups: &division_groups,
        owner_is_ours: None,
        kind: Default::default(),
    };
    assert_eq!(assigned_count(CHILD, &display).unwrap(), 0);
    assert!(division_groups.index.get().is_none());
}

#[test]
fn empty_assignment_count_never_demands_child_hq_or_filter_metadata() {
    let plan = plan(3, Vec::new());
    let groups = BTreeMap::new();
    let group_plans = BTreeMap::new();
    let division_groups = DivisionGroups {
        rows: None,
        index: OnceCell::new(),
    };
    let display = Display {
        plan: &plan,
        groups: &groups,
        group_plans: &group_plans,
        division_groups: &division_groups,
        owner_is_ours: Some(false),
        kind: Default::default(),
    };
    assert_eq!(assigned_count(CHILD, &display).unwrap(), 0);
    assert!(division_groups.index.get().is_none());
}
