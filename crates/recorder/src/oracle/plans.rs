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
    report.attempt_table(TableId::OrdersGroupColors, |r| colors(frame, save, r));
    report.attempt_table(TableId::Fronts, |r| fronts(frame, save, r));
}
fn orders(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
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
    Ok(())
}
fn colors(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::OrdersGroupColors>()?;
    let actual: BTreeSet<_> = rows.iter().map(|r| r.group).collect();
    let expected: BTreeSet<_> = save.plans.colors.keys().copied().collect();
    eq(report, "OrdersGroupColors.group census", actual, expected);
    report.compared("OrdersGroupColors (group census)", rows.len());
    report.limitation("save color is quantized RGB and omits alpha; the four raw 32-bit CColor components are not independently compared by this oracle", "OrdersGroupColors raw RGBA");
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
        rows::{ArmyOrderPlanRow, FrontRow, OrdersGroupColorRow},
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
                ..Default::default()
            }]));
        *frame.tables[TableId::Fronts.index()] =
            TableView::Complete(TableRows::Fronts(vec![FrontRow {
                front,
                section_ids: vec![3],
                section_ends: vec![2],
                province_pairs: vec![4, 5],
            }]));
        *frame.tables[TableId::OrdersGroupColors.index()] =
            TableView::Complete(TableRows::OrdersGroupColors(vec![OrdersGroupColorRow {
                group,
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
        let mut report = Report::default();
        compare(&frame, &saved, &mut report);
        assert_eq!(report.differences.len(), 3);
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
}
