use super::*;

fn eq<T: PartialEq + std::fmt::Debug>(report: &mut Report, label: &str, actual: T, expected: T) {
    report.scalar_checks += 1;
    if actual != expected {
        report.difference(label, actual, expected);
    }
}

pub fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) {
    report.attempt_table(TableId::ArmyOrderPlans, |r| orders(frame, save, r));
    report.attempt_table(TableId::OrdersGroupPlans, |r| group_plans(frame, save, r));
    report.attempt_table(TableId::Fronts, |r| fronts(frame, save, r));
    report.attempt_table(TableId::InvasionPreparations, |r| invasions(frame, save, r));
}
fn orders(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let saved_groups: BTreeMap<u64, (bool, usize)> = save
        .naval_history
        .theatre_countries
        .as_ref()
        .into_iter()
        .flat_map(|countries| countries.values())
        .flat_map(|country| &country.land)
        .flat_map(|theatre| &theatre.definitions)
        .map(|group| (group.id, (group.is_army_group, group.children.len())))
        .collect();
    let mut predecessors: BTreeMap<(u64, u32), Vec<u32>> = BTreeMap::new();
    for (&(group, instance), order) in &save.plans.orders {
        for &successor in &order.successors {
            predecessors
                .entry((group, successor))
                .or_default()
                .push(instance);
        }
    }
    let mut rows = BTreeMap::new();
    for row in frame.complete::<tables::ArmyOrderPlans>()? {
        if row.owner_group as u32 != engine_native::corders_group::REFERENCE_TYPE {
            return Err(invalid(
                "ArmyOrderPlans has invalid owner-group reference type",
            ));
        }
        if rows.insert((row.owner_group, row.instance), row).is_some() {
            return Err(invalid(format!(
                "duplicate ArmyOrderPlans identity {}:{}",
                row.owner_group, row.instance
            )));
        }
    }
    for (key, saved) in &save.plans.orders {
        let label = format!("ArmyOrderPlans {}:{}", key.0, key.1);
        let Some(row) = rows.remove(key) else {
            report.difference(label, "absent", "present");
            continue;
        };
        if saved.order_type == 3 {
            let expected = saved
                .invasion_transports
                .ok_or_else(|| invalid(format!("{label} save lacks allocated invasion convoys")))?;
            let name = format!("{label}.invasion_transports");
            if let Some(actual) = report.recorded(
                &row.invasion_transports,
                "ArmyOrderPlans.invasion_transports",
                &name,
            ) {
                eq(report, &name, *actual, Some(expected));
            }
        }
        execution(row, saved, &label, report);
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
        if let Some(recorded) =
            report.recorded(&row.child_fronts, "ArmyOrderPlans.child_fronts", &label)
        {
            let demanded = saved.front.is_some()
                && save
                    .army_orders
                    .get(key)
                    .is_none_or(|order| !order.path.is_empty());
            match recorded {
                None if demanded => report.difference(
                    format!("{label}.child_fronts"),
                    "unavailable",
                    "bound nonempty-path operands",
                ),
                None => {}
                Some(inputs) if inputs.area_province_count <= 2 => {}
                Some(inputs) => {
                    let stored_count = saved.child_fronts.len();
                    match inputs.count {
                        Some(count) => eq(
                            report,
                            &format!("{label}.child_fronts.count"),
                            count as usize,
                            stored_count,
                        ),
                        None => report.difference(
                            format!("{label}.child_fronts.count"),
                            "unavailable",
                            stored_count,
                        ),
                    }
                    let reached = stored_count > 0
                        && saved.order_type == engine_native::corder_instance::TYPE_LINE;
                    let mut expected_prefix = (stored_count == 0).then_some(0);
                    if reached {
                        match saved_groups.get(&key.0) {
                            Some((true, expected)) => {
                                expected_prefix = Some(stored_count.min(*expected));
                                match inputs.group_child_count {
                                    Some(count) => {
                                        eq(
                                            report,
                                            &format!("{label}.child_fronts.group_child_count"),
                                            i64::from(count),
                                            *expected as i64,
                                        );
                                        if count < 0 {
                                            report.limitation("negative native army-group child count is unsupported; its exact signed value requires a matched native witness", &label);
                                        }
                                    }
                                    None => report.difference(
                                        format!("{label}.child_fronts.group_child_count"),
                                        "unavailable",
                                        expected,
                                    ),
                                }
                            }
                            Some((false, _)) => {}
                            None => report.limitation(
                                "saved group kind/count is unavailable for child-payload demand",
                                &label,
                            ),
                        }
                    }
                    if let Some(sections) = &inputs.sections {
                        if let Some(prefix) = expected_prefix {
                            let actual: Vec<_> = sections
                                .iter()
                                .map(|child| (child.group, child.segment_start, child.segment_end))
                                .collect();
                            let expected: Vec<_> = saved
                                .child_fronts
                                .iter()
                                .take(prefix)
                                .map(|child| (child.group, child.segment_start, child.segment_end))
                                .collect();
                            eq(
                                report,
                                &format!("{label}.child_fronts.sections"),
                                actual,
                                expected,
                            );
                        } else if !sections.is_empty()
                            && (!reached || saved_groups.contains_key(&key.0))
                        {
                            report.difference(
                                format!("{label}.child_fronts.sections"),
                                "nonempty",
                                "outside saved army-group demand",
                            );
                        }
                    } else if expected_prefix.is_some_and(|prefix| prefix > 0) {
                        report.difference(
                            format!("{label}.child_fronts.sections"),
                            "unavailable",
                            "demanded stored army-group prefix",
                        );
                    }
                }
            }
        }
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
    report.limitation("first-path controller-area size is unsaved and history-dependent; it needs independent matched native/write witnesses. Stored child-front count, current army-group child count and ordered group/relative spans are compared only where their native demands are reached", "ArmyOrderPlans.child_fronts");
    report.limitation("the save writes route_is_ok as boolean truth and accumulated preparation as time; type4 progress is compared only when the captured route byte is known nonzero. Target, country/relation/leader modifier terms and preparation-days terms are runtime getter operands, not certified by those saved fields; they need independent matched native/read witnesses", "ArmyOrderPlans.execution.paradrop");
    report.limitation("naval stat weight, paradrop eligibility and effective roster-country/selected-commander transport modifier terms are unsaved runtime inputs; agreement of saved plan, division and wing fields cannot certify them. Independent matched native/read witnesses remain required", "DivisionMovement.order_transport");
    Ok(())
}

fn execution(
    row: &replay_data::rows::ArmyOrderPlanRow,
    saved: &save::plans::Order,
    label: &str,
    report: &mut Report,
) {
    let Some(recorded) = report.recorded(&row.execution, "ArmyOrderPlans.execution", label) else {
        return;
    };
    if !matches!(
        saved.order_type,
        engine_native::corder_instance::TYPE_INVASION
            | engine_native::corder_instance::TYPE_PARADROP
    ) {
        if recorded.is_some() {
            report.difference(
                format!("{label}.execution"),
                "present",
                "outside type3/4 scope",
            );
        }
        return;
    }
    let Some(inputs) = recorded else {
        report.difference(
            format!("{label}.execution"),
            "unavailable",
            "type3/4 route byte",
        );
        return;
    };
    match inputs.route_is_ok {
        Some(raw) => {
            eq(
                report,
                &format!("{label}.execution.route_is_ok"),
                raw != 0,
                saved.route_is_ok,
            );
            if raw > 1 {
                report.limitation(
                    "route_is_ok raw byte exceeds one; the saved yes/no establishes only its truth, so exact byte agreement requires an independent matched native/read witness",
                    format!("{label}.execution.route_is_ok raw={raw}"),
                );
            }
        }
        None => report.difference(
            format!("{label}.execution.route_is_ok"),
            "unavailable",
            saved.route_is_ok,
        ),
    }
    if saved.order_type != engine_native::corder_instance::TYPE_PARADROP {
        if inputs.paradrop.is_some() {
            report.difference(
                format!("{label}.execution.paradrop"),
                "present",
                "outside type4 scope",
            );
        }
        return;
    }
    if inputs.route_is_ok.is_none_or(|raw| raw == 0) {
        if inputs.paradrop.is_some() {
            report.difference(
                format!("{label}.execution.paradrop"),
                "present",
                "outside known-nonzero route demand",
            );
        }
        return;
    }
    let expected = saved.preparation.as_ref().map_or(0, |time| time.raw);
    match inputs.paradrop {
        Some(preparation) => eq(
            report,
            &format!("{label}.execution.paradrop.progress"),
            preparation.progress,
            expected,
        ),
        None => report.difference(
            format!("{label}.execution.paradrop.progress"),
            "unavailable",
            expected,
        ),
    }
}
fn group_plans(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::OrdersGroupPlans>()?;
    let actual: BTreeSet<_> = rows.iter().map(|r| r.group).collect();
    let expected: BTreeSet<_> = save.plans.colors.keys().copied().collect();
    eq(report, "OrdersGroupPlans.group census", actual, expected);
    let census: Option<BTreeSet<(u64, u32)>> = frame
        .complete::<tables::ArmyOrders>()
        .ok()
        .map(|orders| orders.iter().map(|r| (r.owner_group, r.instance)).collect());
    let empty = [Vec::new(), Vec::new()];
    for row in rows {
        let [saved_primary, saved_fallback] =
            save.plans.group_orders.get(&row.group).unwrap_or(&empty);
        let label = format!("OrdersGroupPlans {}", row.group);
        let name = (!row.name.is_empty()).then(|| content_reader_parser::parser::c_string(&row.name));
        if name.is_some_and(|prefix| prefix.len() < row.name.len()) {
            report.limitation(
                "native text serialization ends at the first NUL; captured trailing group-name bytes are not save-comparable",
                format!("{label}.name"),
            );
        }
        eq(
            report,
            &format!("{label}.name"),
            Some(name),
            save.plans
                .group_names
                .get(&row.group)
                .map(|name| name.as_deref()),
        );
        eq(
            report,
            &format!("{label}.fallback_orders"),
            &row.fallback_orders,
            saved_fallback,
        );
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
    report.limitation("save color is quantized RGB and omits alpha; the four raw 32-bit colour components are not independently compared by this oracle. Primary roots are checked as an ordered subsequence of the serialized order graphs led by the first saved instance. The saved virtual_fallback orders are not compared with any recorded list (UNVERIFIED until W2)", "OrdersGroupPlans");
    Ok(())
}
fn fronts(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::Fronts>()?;
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

fn invasions(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let mut rows = BTreeMap::new();
    for row in frame.complete::<tables::InvasionPreparations>()? {
        if rows.insert((row.owner_group, row.instance), row).is_some() {
            return Err(invalid(format!(
                "duplicate InvasionPreparations identity {}:{}",
                row.owner_group, row.instance
            )));
        }
    }
    for (key, saved) in save
        .plans
        .orders
        .iter()
        .filter(|(_, order)| order.order_type == engine_native::corder_instance::TYPE_INVASION)
    {
        let label = format!("InvasionPreparations {}:{}", key.0, key.1);
        let Some(row) = rows.remove(key) else {
            report.difference(label, "absent", "present");
            continue;
        };
        eq(
            report,
            &format!("{label}.progress"),
            row.progress,
            saved.preparation.as_ref().map_or(0, |time| time.raw),
        );
        report.compared("InvasionPreparations", 1);
    }
    for (key, _) in rows {
        report.difference(
            format!("InvasionPreparations {}:{}", key.0, key.1),
            "present",
            "absent",
        );
    }
    report.limitation("invasion preparation's operands are live getter inputs the order serializer does not write: the target (the first path province's state controller), the country, relation and leader modifier terms, and whether the leader term counts (the HQ provider's root order) are not compared; the invasion census and the accumulated preparation are", "InvasionPreparations");
    report.limitation("invasion routing's path-cost operands are runtime state the save never writes: the route unit's logical owner remains InvasionPreparations.unit_owner, while its owner's slot-0 war-pointer presence and retired byte and directed targets now come from shared CountryCounterInputs.zero_war/co_belligerents. Those runtime operands are not compared. The roster fallback unit is compared as CountryCounterInputs.first_division", "InvasionPreparations.unit_owner; CountryCounterInputs.zero_war/co_belligerents");
    Ok(())
}

#[cfg(test)]
#[path = "plans/execution_tests.rs"]
mod execution_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TABLE_COUNT;
    use replay_data::{
        rows::ArmyOrderPlanRow, rows::ArmyOrderRow, rows::FrontRow, rows::InvasionPreparationRow,
        rows::OrdersGroupPlanRow,
    };
    use replay_frame::frame::Metrics;
    #[test]
    fn invasion_allocation_is_compared_exactly_and_capture_absence_is_not_zero() {
        use replay_data::rows::Recorded;
        let (_, mut saved, _, _) = super::super::fixtures::fixture();
        let group = (8_u64 << 32) | 53;
        saved.plans.orders.clear();
        for expected in [0_i32, -3] {
            saved.plans.orders.insert(
                (group, 1),
                save::plans::Order {
                    order_type: 3,
                    has_child: Some(false),
                    invasion_transports: Some(expected),
                    ..Default::default()
                },
            );
            for (actual, differences) in [(Some(expected), 0), (Some(expected + 1), 1), (None, 1)] {
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
                        invasion_transports: Recorded::Value(actual),
                        execution: Recorded::Value(Some(replay_data::rows::OrderExecutionInputs {
                            route_is_ok: Some(0),
                            paradrop: None,
                        })),
                        ..Default::default()
                    }]));
                let mut report = Report::default();
                orders(&frame, &saved, &mut report).unwrap();
                assert_eq!(report.differences.len(), differences);
                let mut row = ArmyOrderPlanRow {
                    owner_group: group,
                    instance: 1,
                    ..Default::default()
                };
                row.invasion_transports = Recorded::NotRecorded;
                row.execution = Recorded::Value(Some(replay_data::rows::OrderExecutionInputs {
                    route_is_ok: Some(0),
                    paradrop: None,
                }));
                *frame.tables[TableId::ArmyOrderPlans.index()] =
                    TableView::Complete(TableRows::ArmyOrderPlans(vec![row]));
                let mut historical = Report::default();
                orders(&frame, &saved, &mut historical).unwrap();
                assert!(historical.differences.is_empty());
                assert!(
                    historical
                        .limitations
                        .contains_key("ArmyOrderPlans.invasion_transports not recorded")
                );
            }
        }
    }
    #[test]
    fn historical_invasion_suffix_keeps_its_prefix_and_later_orders_compared() {
        use replay_data::rows::Recorded;
        let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
        let group = (8_u64 << 32) | 53;
        saved.plans.orders.clear();
        saved.plans.orders.insert(
            (group, 1),
            save::plans::Order {
                order_type: 3,
                has_child: Some(false),
                invasion_transports: Some(0),
                front: Some((9_u64 << 32) | 66),
                assigned: vec![11],
                ..Default::default()
            },
        );
        saved.plans.orders.insert(
            (group, 2),
            save::plans::Order {
                order_type: 1,
                has_child: Some(false),
                assigned: vec![22],
                ..Default::default()
            },
        );
        *frame.tables[TableId::ArmyOrderPlans.index()] =
            TableView::Complete(TableRows::ArmyOrderPlans(vec![
                ArmyOrderPlanRow {
                    owner_group: group,
                    instance: 1,
                    invasion_transports: Recorded::NotRecorded,
                    execution: Recorded::NotRecorded,
                    ..Default::default()
                },
                ArmyOrderPlanRow {
                    owner_group: group,
                    instance: 2,
                    execution: Recorded::NotRecorded,
                    ..Default::default()
                },
            ]));
        let mut report = Report::default();
        orders(&frame, &saved, &mut report).unwrap();
        assert!(
            report
                .limitations
                .contains_key("ArmyOrderPlans.invasion_transports not recorded")
        );
        for field in [
            format!("{group}:1.front"),
            format!("{group}:1.assigned_divisions"),
            format!("{group}:2.assigned_divisions"),
        ] {
            assert!(
                report
                    .differences
                    .iter()
                    .any(|difference| difference.contains(&field)),
                "{:?}",
                report.differences
            );
        }
        assert_eq!(report.compared_tables["ArmyOrderPlans"], 2);
    }
    #[test]
    fn every_saved_plan_operand_and_pair_order_can_fail_the_oracle() {
        let (_, mut saved, _, _) = super::super::fixtures::fixture();
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
        saved
            .plans
            .group_names
            .insert(group, Some(b"Army 1".to_vec()));
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
                child_fronts: replay_data::rows::Recorded::NotRecorded,
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
        *frame.tables[TableId::InvasionPreparations.index()] =
            TableView::Complete(TableRows::InvasionPreparations(Vec::new()));
        let mut report = Report::default();
        compare(&frame, &saved, &mut report);
        assert!(report.differences.is_empty());
        assert!(report.failed_comparisons.is_empty());
        assert_eq!(
            report.limitations["ArmyOrderPlans.child_fronts not recorded"],
            vec![format!("ArmyOrderPlans {group}:1")],
        );
        saved
            .plans
            .orders
            .get_mut(&(group, 1))
            .unwrap()
            .assigned
            .reverse();
        saved.plans.orders.get_mut(&(group, 1)).unwrap().blitz = true;
        saved.plans.fronts.get_mut(&front).unwrap().pairs.reverse();
        saved
            .plans
            .orders
            .get_mut(&(group, 1))
            .unwrap()
            .names
            .swap(2, 3);
        saved.plans.orders.get_mut(&(group, 1)).unwrap().successors = vec![7];
        saved
            .plans
            .group_names
            .insert(group, Some(b"Army 2".to_vec()));
        let mut report = Report::default();
        compare(&frame, &saved, &mut report);
        assert_eq!(report.differences.len(), 7, "{:?}", report.differences);
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
    fn child_front_oracle_compares_ordered_spans_and_refuses_reached_capture_gaps() {
        use replay_data::{rows::OrderChildFront, rows::OrderChildFrontInputs, rows::Recorded};
        let (_, mut saved, _, _) = super::super::fixtures::fixture();
        let group = (8_u64 << 32) | 53;
        let child = (9_u64 << 32) | 53;
        let front = (9_u64 << 32) | 66;
        saved.plans.orders.clear();
        saved.plans.orders.insert(
            (group, 1),
            save::plans::Order {
                front: Some(front),
                order_type: engine_native::corder_instance::TYPE_LINE,
                has_child: Some(false),
                child_fronts: vec![
                    save::plans::ChildFront {
                        group: child,
                        segment_start: -1,
                        segment_end: -1,
                    },
                    save::plans::ChildFront {
                        group: child,
                        segment_start: 1,
                        segment_end: 3,
                    },
                ],
                ..Default::default()
            },
        );
        saved.naval_history.theatre_countries = Some(BTreeMap::from([(
            Vec::new(),
            save::naval_history::TheatreCountry {
                land: vec![save::naval_history::LandTheatre {
                    id: 1,
                    volunteers_country: None,
                    groups: Vec::new(),
                    units: Vec::new(),
                    area_representatives: Vec::new(),
                    definitions: vec![save::naval_history::OrdersGroup {
                        id: group,
                        is_army_group: true,
                        children: vec![child, child],
                        ..Default::default()
                    }],
                }],
                ..Default::default()
            },
        )]));
        let sections = vec![
            OrderChildFront {
                group: child,
                segment_start: -1,
                segment_end: -1,
            },
            OrderChildFront {
                group: child,
                segment_start: 1,
                segment_end: 3,
            },
        ];
        let inputs = OrderChildFrontInputs {
            area_province_count: 3,
            count: Some(2),
            group_child_count: Some(2),
            sections: Some(sections.clone()),
        };
        let mut reversed = sections;
        reversed.reverse();
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn::<_, TABLE_COUNT, _>(|_| TableView::Unavailable {
                reason: "fixture".into(),
            })
            .map(Into::into),
        };
        for (recorded, differences) in [
            (Recorded::Value(Some(inputs.clone())), 0),
            (
                Recorded::Value(Some(OrderChildFrontInputs {
                    sections: Some(reversed),
                    ..inputs.clone()
                })),
                1,
            ),
            (
                Recorded::Value(Some(OrderChildFrontInputs {
                    count: None,
                    group_child_count: None,
                    sections: None,
                    ..inputs.clone()
                })),
                3,
            ),
            (
                Recorded::Value(Some(OrderChildFrontInputs {
                    group_child_count: None,
                    sections: None,
                    ..inputs.clone()
                })),
                2,
            ),
            (
                Recorded::Value(Some(OrderChildFrontInputs {
                    group_child_count: Some(1),
                    ..inputs.clone()
                })),
                1,
            ),
            (
                Recorded::Value(Some(OrderChildFrontInputs {
                    group_child_count: Some(-1),
                    sections: None,
                    ..inputs.clone()
                })),
                2,
            ),
            (
                Recorded::Value(Some(OrderChildFrontInputs {
                    sections: None,
                    ..inputs
                })),
                1,
            ),
            (Recorded::Value(None), 1),
            (
                Recorded::Value(Some(OrderChildFrontInputs {
                    area_province_count: 2,
                    count: None,
                    group_child_count: None,
                    sections: None,
                })),
                0,
            ),
            (Recorded::NotRecorded, 0),
        ] {
            *frame.tables[TableId::ArmyOrderPlans.index()] =
                TableView::Complete(TableRows::ArmyOrderPlans(vec![ArmyOrderPlanRow {
                    owner_group: group,
                    instance: 1,
                    front,
                    front_valid: 1,
                    child_fronts: recorded,
                    ..Default::default()
                }]));
            let mut report = Report::default();
            orders(&frame, &saved, &mut report).unwrap();
            assert_eq!(
                report.differences.len(),
                differences,
                "{:?}",
                report.differences
            );
        }
        saved
            .naval_history
            .theatre_countries
            .as_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .land[0]
            .definitions[0]
            .children
            .truncate(1);
        let prefix = OrderChildFrontInputs {
            group_child_count: Some(1),
            sections: Some(inputs.sections.as_ref().unwrap()[..1].to_vec()),
            ..inputs.clone()
        };
        let mut altered = prefix.sections.clone().unwrap();
        altered[0].segment_end = 5;
        for (captured, differences) in [
            (prefix.clone(), 0),
            (
                OrderChildFrontInputs {
                    count: Some(0),
                    ..prefix.clone()
                },
                1,
            ),
            (
                OrderChildFrontInputs {
                    count: None,
                    group_child_count: None,
                    sections: None,
                    ..prefix.clone()
                },
                3,
            ),
            (
                OrderChildFrontInputs {
                    count: Some(0),
                    group_child_count: None,
                    sections: None,
                    ..prefix.clone()
                },
                3,
            ),
            (
                OrderChildFrontInputs {
                    group_child_count: Some(0),
                    sections: None,
                    ..prefix.clone()
                },
                2,
            ),
            (
                OrderChildFrontInputs {
                    group_child_count: Some(2),
                    ..prefix.clone()
                },
                1,
            ),
            (
                OrderChildFrontInputs {
                    sections: Some(Vec::new()),
                    ..prefix.clone()
                },
                1,
            ),
            (
                OrderChildFrontInputs {
                    sections: inputs.sections.clone(),
                    ..prefix.clone()
                },
                1,
            ),
            (
                OrderChildFrontInputs {
                    sections: Some(altered),
                    ..prefix
                },
                1,
            ),
        ] {
            *frame.tables[TableId::ArmyOrderPlans.index()] =
                TableView::Complete(TableRows::ArmyOrderPlans(vec![ArmyOrderPlanRow {
                    owner_group: group,
                    instance: 1,
                    front,
                    front_valid: 1,
                    child_fronts: Recorded::Value(Some(captured)),
                    ..Default::default()
                }]));
            let mut report = Report::default();
            orders(&frame, &saved, &mut report).unwrap();
            assert_eq!(
                report.differences.len(),
                differences,
                "{:?}",
                report.differences
            );
        }
        saved
            .naval_history
            .theatre_countries
            .as_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .land[0]
            .definitions[0]
            .is_army_group = false;
        *frame.tables[TableId::ArmyOrderPlans.index()] =
            TableView::Complete(TableRows::ArmyOrderPlans(vec![ArmyOrderPlanRow {
                owner_group: group,
                instance: 1,
                front,
                front_valid: 1,
                child_fronts: Recorded::Value(Some(inputs.clone())),
                ..Default::default()
            }]));
        let mut report = Report::default();
        orders(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        saved
            .naval_history
            .theatre_countries
            .as_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .land[0]
            .definitions[0]
            .is_army_group = true;
        saved.army_orders.insert(
            (group, 1),
            save::ArmyOrder {
                owner_group: group,
                instance: 1,
                country: Vec::new(),
                order_type: engine_native::corder_instance::TYPE_LINE,
                active: 0,
                paradrop_target: 0,
                path: Vec::new(),
                target_states: Vec::new(),
            },
        );
        *frame.tables[TableId::ArmyOrderPlans.index()] =
            TableView::Complete(TableRows::ArmyOrderPlans(vec![ArmyOrderPlanRow {
                owner_group: group,
                instance: 1,
                front,
                front_valid: 1,
                child_fronts: Recorded::Value(None),
                ..Default::default()
            }]));
        let mut report = Report::default();
        orders(&frame, &saved, &mut report).unwrap();
        assert!(report.differences.is_empty());
        saved.army_orders.get_mut(&(group, 1)).unwrap().path.push(1);
        saved
            .naval_history
            .theatre_countries
            .as_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .land[0]
            .definitions[0]
            .children
            .clear();
        *frame.tables[TableId::ArmyOrderPlans.index()] =
            TableView::Complete(TableRows::ArmyOrderPlans(vec![ArmyOrderPlanRow {
                owner_group: group,
                instance: 1,
                front,
                front_valid: 1,
                child_fronts: Recorded::Value(Some(OrderChildFrontInputs {
                    group_child_count: Some(0),
                    sections: None,
                    ..inputs
                })),
                ..Default::default()
            }]));
        let mut report = Report::default();
        orders(&frame, &saved, &mut report).unwrap();
        assert!(report.differences.is_empty());
    }

    #[test]
    fn group_name_compares_stored_length_gate_and_first_nul_prefix() {
        let (_, mut saved, _, _) = super::super::fixtures::fixture();
        let group = (13171_u64 << 32) | 53;
        saved.plans.colors.clear();
        saved.plans.colors.insert(group, None);
        saved.plans.group_names.clear();
        saved.plans.group_orders.clear();
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn::<_, TABLE_COUNT, _>(|_| TableView::Unavailable {
                reason: "fixture".into(),
            })
            .map(Into::into),
        };
        for (actual, expected, matches) in [
            (b"".as_slice(), None, true),
            (b"".as_slice(), Some(b"".as_slice()), false),
            (b"\0tail".as_slice(), Some(b"".as_slice()), true),
            (b"\0tail".as_slice(), None, false),
            (b"Army 1\0tail".as_slice(), Some(b"Army 1".as_slice()), true),
            (b"Army 1".as_slice(), None, false),
            (b"Army 2".as_slice(), Some(b"Army 1".as_slice()), false),
        ] {
            saved
                .plans
                .group_names
                .insert(group, expected.map(<[u8]>::to_vec));
            *frame.tables[TableId::OrdersGroupPlans.index()] =
                TableView::Complete(TableRows::OrdersGroupPlans(vec![OrdersGroupPlanRow {
                    group,
                    name: actual.to_vec(),
                    ..Default::default()
                }]));
            let mut report = Report::default();
            group_plans(&frame, &saved, &mut report).unwrap();
            assert_eq!(
                report.differences.is_empty(),
                matches,
                "actual={actual:?}, saved={expected:?}, differences={:?}",
                report.differences
            );
            if actual.contains(&0) {
                assert!(
                    report
                        .limitations
                        .keys()
                        .any(|reason| reason.contains("first NUL"))
                );
            }
        }
    }

    #[test]
    fn group_roots_lead_the_saved_list_and_belong_to_the_order_census() {
        let (_, mut saved, _, _) = super::super::fixtures::fixture();
        let group = (8_u64 << 32) | 53;
        saved.plans.colors.insert(group, None);
        saved.plans.group_names.insert(group, None);
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
        assert_eq!(run(&mut frame, &saved, group, vec![], vec![3]), 1);
        assert_eq!(run(&mut frame, &saved, group, vec![2], vec![3]), 1);
        saved
            .plans
            .group_orders
            .insert(group, [vec![1, 5, 2, 9], vec![3]]);
        assert_eq!(run(&mut frame, &saved, group, vec![1, 9], vec![3]), 1);
    }

    #[test]
    fn each_saved_invasion_needs_its_row_and_exact_progress() {
        let (_, mut saved, _, _) = super::super::fixtures::fixture();
        let group = (8_u64 << 32) | 53;
        let invasion = |preparation: Option<&[u8]>| save::plans::Order {
            order_type: 3,
            preparation: preparation.map(|text| Decimal::parse(text).unwrap()),
            invasion_transports: Some(0),
            ..Default::default()
        };
        saved
            .plans
            .orders
            .insert((group, 1), invasion(Some(b"12.34567")));
        saved.plans.orders.insert((group, 2), invasion(None));
        saved.plans.orders.insert(
            (group, 3),
            save::plans::Order {
                order_type: 1,
                ..Default::default()
            },
        );
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn::<_, TABLE_COUNT, _>(|_| TableView::Unavailable {
                reason: "fixture".into(),
            })
            .map(Into::into),
        };
        let mut run = |rows: Vec<(u32, i64)>| {
            *frame.tables[TableId::InvasionPreparations.index()] =
                TableView::Complete(TableRows::InvasionPreparations(
                    rows.into_iter()
                        .map(|(instance, progress)| InvasionPreparationRow {
                            owner_group: group,
                            instance,
                            progress,
                            relation_term: Some(5),
                            ..Default::default()
                        })
                        .collect(),
                ));
            let mut report = Report::default();
            invasions(&frame, &saved, &mut report).unwrap();
            report.differences.len()
        };
        assert_eq!(run(vec![(1, 1_234_567), (2, 0)]), 0);
        assert_eq!(run(vec![(1, 1_234_568), (2, 0)]), 1);
        assert_eq!(run(vec![(1, 1_234_567)]), 1);
        assert_eq!(run(vec![(1, 1_234_567), (2, 0), (3, 0)]), 1);
        assert_eq!(run(Vec::new()), 2);
    }
}
