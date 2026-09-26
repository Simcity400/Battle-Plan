//! Exact saved plan operands; each table is compared independently.
use super::*;

fn eq<T: PartialEq + std::fmt::Debug>(report: &mut Report, label: &str, actual: T, expected: T) {
    report.scalar_checks += 1;
    if actual != expected {
        report.difference(label, actual, expected);
    }
}

pub(super) fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) {
    report.attempt_table(TableId::ArmyOrderPlans, |r| orders(frame, save, r));
    report.attempt_table(TableId::OrdersGroupPlans, |r| group_plans(frame, save, r));
    report.attempt_table(TableId::Fronts, |r| fronts(frame, save, r));
}
fn orders(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    // Only successors are serialized (COrderInstance::serialize@1410362c0);
    // reciprocal predecessor membership is exact, its order is not saved.
    let mut predecessors: BTreeMap<(u64, u32), Vec<u32>> = BTreeMap::new();
    for (&(group, instance), order) in &save.plans.orders {
        for &successor in &order.successors {
            predecessors
                .entry((group, successor))
                .or_default()
                .push(instance);
        }
    }
    let mut rows: BTreeMap<_, _> = frame
        .complete::<tables::ArmyOrderPlans>()?
        .iter()
        .map(|r| ((r.owner_group, r.instance), r))
        .collect();
    for (key, saved) in &save.plans.orders {
        let label = format!("ArmyOrderPlans {}:{}", key.0, key.1);
        let Some(row) = rows.remove(key) else {
            report.difference(label, "absent", "present");
            continue;
        };
        for (name, actual, expected) in [
            ("blitz", row.blitz != 0, saved.blitz),
            ("withdraw", row.withdraw != 0, saved.withdraw),
            ("fallback", row.fallback != 0, saved.fallback),
            ("virtual_order", row.virtual_order != 0, saved.virtual_order),
        ] {
            eq(report, &format!("{label}.{name}"), actual, expected);
        }
        let child = saved.has_child.ok_or_else(|| {
            invalid(format!(
                "{label} save lacks unconditional manage_child_sections"
            ))
        })?;
        eq(
            report,
            &format!("{label}.has_child"),
            row.has_child != 0,
            child,
        );
        eq(
            report,
            &format!("{label}.front"),
            (row.front_valid != 0).then_some(row.front),
            saved.front,
        );
        if saved.front.is_some() {
            eq(
                report,
                &format!("{label}.section"),
                row.section,
                saved.section,
            );
        }
        if let Some(value) = saved.segment_start {
            eq(
                report,
                &format!("{label}.segment_start"),
                row.segment_start,
                value,
            );
        }
        if let Some(value) = saved.segment_end {
            eq(
                report,
                &format!("{label}.segment_end"),
                row.segment_end,
                value,
            );
        }
        eq(
            report,
            &format!("{label}.assigned_divisions"),
            &row.assigned_divisions,
            &saved.assigned,
        );
        eq(
            report,
            &format!("{label}.owner_override"),
            row.owner_override,
            saved.owner_override,
        );
        eq(
            report,
            &format!("{label}.successors"),
            &row.successors,
            &saved.successors,
        );
        let mut actual = row.predecessors.clone();
        actual.sort_unstable();
        let mut expected = predecessors.remove(key).unwrap_or_default();
        expected.sort_unstable();
        eq(
            report,
            &format!("{label}.predecessor membership"),
            actual,
            expected,
        );
        // Names are recorded only for the arrows GfxCommonArrowUpdateCallback
        // names; every other order records none.
        let named = matches!(saved.order_type, 1 | 3 | 4);
        let recorded = [
            &row.name_prefix,
            &row.name,
            &row.name_first,
            &row.name_second,
            &row.name_unique,
            &row.name_postfix,
        ];
        for ((field, actual), saved_name) in [
            "name_prefix",
            "name",
            "name_first",
            "name_second",
            "name_unique",
            "name_postfix",
        ]
        .into_iter()
        .zip(recorded)
        .zip(&saved.names)
        {
            let expected: &[u8] = if named { saved_name } else { &[] };
            eq(
                report,
                &format!("{label}.{field}"),
                actual.as_slice(),
                expected,
            );
        }
        report.compared("ArmyOrderPlans", 1);
    }
    for (key, _) in rows {
        report.difference(
            format!("ArmyOrderPlans {}:{}", key.0, key.1),
            "present",
            "absent",
        );
    }
    report.limitation("save yes/no fields distinguish zero from nonzero, not distinct nonzero raw bytes; absent bindings do not preserve the invalid sentinel bits or unused section word; segment endpoints are compared only when serialized; same-hour post-sample commands may change plans", "ArmyOrderPlans");
    report.limitation("the save stores ordinary successors in native order but not predecessor order; reciprocal predecessor membership and multiplicity are compared, native predecessor order is not", "ArmyOrderPlans.predecessors");
    Ok(())
}
fn group_plans(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::OrdersGroupPlans>()?;
    let actual: BTreeSet<_> = rows.iter().map(|r| r.group).collect();
    let expected: BTreeSet<_> = save.plans.colors.keys().copied().collect();
    eq(report, "OrdersGroupPlans.group census", actual, expected);
    // Every recorded root is an instance of the group's own order census.
    let census: Option<BTreeSet<(u64, u32)>> = frame
        .complete::<tables::ArmyOrders>()
        .ok()
        .map(|orders| orders.iter().map(|r| (r.owner_group, r.instance)).collect());
    let empty = [Vec::new(), Vec::new()];
    for row in rows {
        let [saved_primary, saved_fallback] =
            save.plans.group_orders.get(&row.group).unwrap_or(&empty);
        let label = format!("OrdersGroupPlans {}", row.group);
        eq(
            report,
            &format!("{label}.name"),
            Some(&row.name),
            save.plans.group_names.get(&row.group),
        );
        eq(
            report,
            &format!("{label}.fallback_orders"),
            &row.fallback_orders,
            saved_fallback,
        );
        // The serializer writes each primary root followed by its successor
        // graph, so the roots are an ordered subsequence of the saved list and
        // the first saved instance is the first root.
        if let Some(first) = saved_primary.first() {
            eq(
                report,
                &format!("{label}.primary_orders[0]"),
                row.primary_orders.first(),
                Some(first),
            );
        }
        let mut saved = saved_primary.iter();
        let ordered = row
            .primary_orders
            .iter()
            .all(|root| saved.any(|instance| instance == root));
        eq(
            report,
            &format!("{label}.primary_orders order"),
            ordered,
            true,
        );
        if let Some(census) = &census {
            for instance in row.primary_orders.iter().chain(&row.fallback_orders) {
                eq(
                    report,
                    &format!("{label} root {instance} in ArmyOrders"),
                    census.contains(&(row.group, *instance)),
                    true,
                );
            }
        }
    }
    if census.is_none() {
        report.limitation(
            "ArmyOrders is not complete this hour, so the root ids were not checked against the order census",
            "OrdersGroupPlans",
        );
    }
    report.compared("OrdersGroupPlans", rows.len());
    report.limitation("save color is quantized RGB and omits alpha; the four raw 32-bit CColor components are not independently compared by this oracle. Primary roots are checked as an ordered subsequence of the serialized order graphs led by the first saved instance. The saved virtual_fallback orders are not compared with any recorded list (UNVERIFIED until W2)", "OrdersGroupPlans");
    Ok(())
}
fn fronts(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::Fronts>()?;
    // whose CFront was destroyed before its order; only live theatre fronts
    // have sections, so only those bindings require a Fronts row.
    let required: BTreeSet<_> = save
        .plans
        .orders
        .values()
        .filter_map(|r| r.front)
        .filter(|front| save.plans.fronts.contains_key(front))
        .collect();
    let actual: BTreeSet<_> = rows.iter().map(|r| r.front).collect();
    eq(report, "Fronts.bound front census", &actual, &required);
    for row in rows {
        let label = format!("Fronts {}", row.front);
        let Some(expected) = save.plans.fronts.get(&row.front) else {
            report.difference(label, "present", "absent from save fronts");
            continue;
        };
        eq(
            report,
            &format!("{label}.section_ids"),
            &row.section_ids,
            &expected.section_ids,
        );
        eq(
            report,
            &format!("{label}.section_ends"),
            &row.section_ends,
            &expected.section_ends,
        );
        eq(
            report,
            &format!("{label}.province_pairs"),
            &row.province_pairs,
            &expected.pairs,
        );
        report.compared("Fronts", 1);
    }
    report.limitation("records all sections of fronts bound by the complete ArmyOrders census; unbound theatre fronts, per-country section metadata and unbound order cached sorted_pairs are outside this table's contract. A binding to a destroyed front is saved as root_front but has no live front or row; the save cannot show whether it resolved in the registry", "Fronts scope");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::{Metrics, TABLE_COUNT},
        rows::{ArmyOrderPlanRow, ArmyOrderRow, FrontRow, OrdersGroupPlanRow},
    };
    #[test]
    fn every_saved_plan_operand_and_pair_order_can_fail_the_oracle() {
        let (_, mut saved, _, _) = super::super::tests::fixture();
        let group = (8_u64 << 32) | 53;
        let front = (9_u64 << 32) | 66;
        saved.plans.orders.insert(
            (group, 1),
            save::plans::Order {
                front: Some(front),
                section: 3,
                has_child: Some(true),
                assigned: vec![11, 12],
                order_type: 1,
                successors: vec![],
                names: [
                    b"o_operation".to_vec(),
                    Vec::new(),
                    b"o_x".to_vec(),
                    b"o_y".to_vec(),
                    Vec::new(),
                    Vec::new(),
                ],
                ..Default::default()
            },
        );
        saved.plans.fronts.insert(
            front,
            save::plans::Front {
                section_ids: vec![3],
                section_ends: vec![2],
                pairs: vec![4, 5],
            },
        );
        saved.plans.colors.insert(group, Some(vec![127, 0, 0]));
        saved.plans.group_names.insert(group, b"Army 1".to_vec());
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn::<_, TABLE_COUNT, _>(|_| TableView::Unavailable {
                reason: "fixture".into(),
            })
            .map(Into::into),
        };
        *frame.tables[TableId::ArmyOrderPlans.index()] =
            TableView::Complete(TableRows::ArmyOrderPlans(vec![ArmyOrderPlanRow {
                owner_group: group,
                instance: 1,
                front,
                front_valid: 1,
                section: 3,
                has_child: 255,
                assigned_divisions: vec![11, 12],
                name_prefix: b"o_operation".to_vec(),
                name_first: b"o_x".to_vec(),
                name_second: b"o_y".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::Fronts.index()] =
            TableView::Complete(TableRows::Fronts(vec![FrontRow {
                front,
                section_ids: vec![3],
                section_ends: vec![2],
                province_pairs: vec![4, 5],
            }]));
        *frame.tables[TableId::OrdersGroupPlans.index()] =
            TableView::Complete(TableRows::OrdersGroupPlans(vec![OrdersGroupPlanRow {
                group,
                name: b"Army 1".to_vec(),
                ..Default::default()
            }]));
        let mut report = Report::default();
        compare(&frame, &saved, &mut report);
        assert!(report.differences.is_empty());
        assert!(report.failed_comparisons.is_empty());
        saved
            .plans
            .orders
            .get_mut(&(group, 1))
            .unwrap()
            .assigned
            .reverse();
        saved.plans.orders.get_mut(&(group, 1)).unwrap().blitz = true;
        saved.plans.fronts.get_mut(&front).unwrap().pairs.reverse();
        // A reordered name, a successor and a renamed group each differ too.
        saved
            .plans
            .orders
            .get_mut(&(group, 1))
            .unwrap()
            .names
            .swap(2, 3);
        saved.plans.orders.get_mut(&(group, 1)).unwrap().successors = vec![7];
        saved.plans.group_names.insert(group, b"Army 2".to_vec());
        let mut report = Report::default();
        compare(&frame, &saved, &mut report);
        assert_eq!(report.differences.len(), 7, "{:?}", report.differences);
        // A saved binding to a front absent from the theatres requires no row.
        let stale = (10_u64 << 32) | 66;
        saved.plans.orders.insert(
            (group, 2),
            save::plans::Order {
                front: Some(stale),
                ..Default::default()
            },
        );
        let mut report = Report::default();
        fronts(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        saved.plans.orders.remove(&(group, 2));
    }

    #[test]
    fn group_roots_lead_the_saved_list_and_belong_to_the_order_census() {
        let (_, mut saved, _, _) = super::super::tests::fixture();
        let group = (8_u64 << 32) | 53;
        saved.plans.colors.insert(group, None);
        // The group's name matches, so only the roots can differ.
        saved.plans.group_names.insert(group, Vec::new());
        // Root 1 is saved with its successor 5 before root 2; 3 is a fallback.
        saved
            .plans
            .group_orders
            .insert(group, [vec![1, 5, 2], vec![3]]);
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn::<_, TABLE_COUNT, _>(|_| TableView::Unavailable {
                reason: "fixture".into(),
            })
            .map(Into::into),
        };
        *frame.tables[TableId::ArmyOrders.index()] = TableView::Complete(TableRows::ArmyOrders(
            [1, 2, 3, 5]
                .into_iter()
                .map(|instance| ArmyOrderRow {
                    owner_group: group,
                    instance,
                    ..Default::default()
                })
                .collect(),
        ));
        fn run(
            frame: &mut DecodedFrame,
            saved: &Save,
            group: u64,
            primary: Vec<u32>,
            fallback: Vec<u32>,
        ) -> usize {
            *frame.tables[TableId::OrdersGroupPlans.index()] =
                TableView::Complete(TableRows::OrdersGroupPlans(vec![OrdersGroupPlanRow {
                    group,
                    primary_orders: primary,
                    fallback_orders: fallback,
                    ..Default::default()
                }]));
            let mut report = Report::default();
            group_plans(frame, saved, &mut report).unwrap();
            report.differences.len()
        }
        assert_eq!(run(&mut frame, &saved, group, vec![1, 2], vec![3]), 0);
        // A recording without the saved first root fails, empty or not.
        assert_eq!(run(&mut frame, &saved, group, vec![], vec![3]), 1);
        assert_eq!(run(&mut frame, &saved, group, vec![2], vec![3]), 1);
        // A root outside the census fails, even where the save lists it.
        saved
            .plans
            .group_orders
            .insert(group, [vec![1, 5, 2, 9], vec![3]]);
        assert_eq!(run(&mut frame, &saved, group, vec![1, 9], vec![3]), 1);
    }
}
