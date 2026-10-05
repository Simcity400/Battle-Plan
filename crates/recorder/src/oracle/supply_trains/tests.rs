use super::*;
use replay::rows::{
    CountryCounterInputRow, FloatingHarborDeploymentRow, SupplyNodeRow, TrainInventoryRow,
};

#[test]
fn supply_compares_province_census_and_routes_despite_membership_exclusions() {
    let membership = super::super::state_history_tests::membership(
        "state={id=1 provinces={1}} \
         state={id=2 provinces={2} history={free_operative={operative=a}}}",
        2,
    );
    let (mut frame, ..) = super::super::tests::fixture();
    let country = frame.complete::<tables::Countries>().unwrap()[0].id;
    *frame.tables[TableId::SupplyNodes.index()] =
        TableView::Complete(TableRows::SupplyNodes(vec![
            SupplyNodeRow {
                country,
                index: 0,
                province: 2,
                parent: -1,
                ..Default::default()
            },
            SupplyNodeRow {
                country,
                index: 1,
                province: 1,
                parent: -1,
                path: vec![2],
                ..Default::default()
            },
            SupplyNodeRow {
                country,
                index: 2,
                province: 99,
                parent: -1,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::TrainInventory.index()] =
        TableView::Complete(TableRows::TrainInventory(vec![TrainInventoryRow {
            country,
            node_count: 3,
            ..Default::default()
        }]));
    let mut report = Report::default();
    nodes(&frame, &membership, &mut report).unwrap();
    assert_eq!(report.compared_tables["SupplyNodes"], 3);
    assert!(report.membership_skips.is_empty());
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains(".province"));
    // A route through the excluded state's province still has to match its
    // captured parent lookup.
    let TableView::Complete(TableRows::SupplyNodes(rows)) =
        &mut *frame.tables[TableId::SupplyNodes.index()]
    else {
        unreachable!()
    };
    rows[1].route_from = 2;
    rows[1].route_to = 1;
    assert!(
        nodes(&frame, &membership, &mut Report::default())
            .unwrap_err()
            .to_string()
            .contains("route endpoints differ from parent lookup")
    );
}

#[test]
fn floating_harbour_rows_join_supply_systems_with_dense_ordinals() {
    let (mut frame, ..) = super::super::tests::fixture();
    let [raw, country] = {
        let row = &frame.complete::<tables::Countries>().unwrap()[0];
        [row.id, row.canonical_slot]
    };
    *frame.tables[TableId::TrainInventory.index()] =
        TableView::Complete(TableRows::TrainInventory(vec![TrainInventoryRow {
            country: raw,
            ..Default::default()
        }]));
    let set = |frame: &mut DecodedFrame, country, ordinal| {
        *frame.tables[TableId::FloatingHarborDeployments.index()] =
            TableView::Complete(TableRows::FloatingHarborDeployments(vec![
                FloatingHarborDeploymentRow {
                    country,
                    ordinal,
                    province: -1,
                    status: 255,
                },
            ]));
    };
    set(&mut frame, country, 0);
    let mut report = Report::default();
    floating_harbors(&frame, &mut report).unwrap();
    assert_eq!(report.compared_tables["FloatingHarborDeployments"], 0);
    set(&mut frame, country, 1);
    assert!(floating_harbors(&frame, &mut Report::default()).is_err());
    set(&mut frame, country + 1, 0);
    assert!(floating_harbors(&frame, &mut Report::default()).is_err());
}

fn owned_states(frame: &mut DecodedFrame, countries: &[replay::rows::CountryRow], count: i32) {
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            countries
                .iter()
                .map(|c| CountryCounterInputRow {
                    country: c.id,
                    owned_state_count: count,
                    ..Default::default()
                })
                .collect(),
        ));
}
#[test]
fn supply_joins_fail_for_bad_censuses_but_do_not_compare_distinct_save_caches() {
    let (mut frame, mut save, mut membership, _) = super::super::tests::fixture();
    let countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    owned_states(&mut frame, &countries, 1);
    save.economy.supply = Some(
        countries
            .iter()
            .map(|c| (c.tag.clone(), Default::default()))
            .collect(),
    );
    // Province 1 is on the verified map but absent from the save's sparse
    // provinces block, as most node provinces are in real saves.
    let province = 1;
    assert!(!save.provinces.contains_key(&province));
    let country = countries[0].id;
    *frame.tables[TableId::SupplyNodes.index()] =
        TableView::Complete(TableRows::SupplyNodes(vec![SupplyNodeRow {
            country,
            index: 0,
            province,
            parent: -1,
            available: i64::MIN,
            consumed: i64::MAX,
            flag: 255,
            path: vec![province],
            ..Default::default()
        }]));
    *frame.tables[TableId::TrainInventory.index()] =
        TableView::Complete(TableRows::TrainInventory(
            countries
                .iter()
                .map(|c| TrainInventoryRow {
                    country: c.id,
                    node_count: u32::from(c.id == country),
                    controller_access: replay::rows::SparseBytes::from_dense(&[0, 255]).unwrap(),
                    ..Default::default()
                })
                .collect(),
        ));
    let mut report = Report::default();
    compare(&frame, &save, &Ok(&membership), &mut report);
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    // The serializer's CCountry+484 filter omits an authenticated supply
    // system. A parent province need not be a simulation node, but it must
    // still have a recorded vertex in the country's graph.
    save.economy.supply.as_mut().unwrap().clear();
    owned_states(&mut frame, &countries, 0);
    let parent = province + 1;
    if let TableView::Complete(TableRows::SupplyNodes(rows)) =
        &mut *frame.tables[TableId::SupplyNodes.index()]
    {
        rows[0].parent = parent as i32;
    }
    let mut report = Report::default();
    compare(&frame, &save, &Ok(&membership), &mut report);
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.contains_key("SupplyNodes"));
    // Every parent must have a vertex row, and vertices are bare edges.
    let TableView::Complete(TableRows::SupplyNodes(rows)) =
        &mut *frame.tables[TableId::SupplyNodes.index()]
    else {
        unreachable!()
    };
    rows.push(SupplyNodeRow {
        country,
        index: SupplyNodeRow::VERTEX | parent,
        province: parent,
        parent: -1,
        ..Default::default()
    });
    let mut report = Report::default();
    compare(&frame, &save, &Ok(&membership), &mut report);
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    assert_eq!(report.compared_tables["SupplyNodes"], 2);
    let TableView::Complete(TableRows::SupplyNodes(rows)) =
        &mut *frame.tables[TableId::SupplyNodes.index()]
    else {
        unreachable!()
    };
    rows[1].kind = 2;
    let mut report = Report::default();
    compare(&frame, &save, &Ok(&membership), &mut report);
    assert!(report.failed_comparisons.contains_key("SupplyNodes"));
    let TableView::Complete(TableRows::SupplyNodes(rows)) =
        &mut *frame.tables[TableId::SupplyNodes.index()]
    else {
        unreachable!()
    };
    rows.pop();
    membership.provinces.remove(&parent);
    let mut report = Report::default();
    compare(&frame, &save, &Ok(&membership), &mut report);
    assert!(report.failed_comparisons.contains_key("SupplyNodes"));
    membership
        .provinces
        .insert(parent, save::ProvinceMembership::Known(None));
    let mut report = Report::default();
    compare(&frame, &save, &Err("no content".to_string()), &mut report);
    assert!(report.failed_comparisons.contains_key("SupplyNodes"));
    save.economy
        .supply
        .as_mut()
        .unwrap()
        .insert(b"unknown".to_vec(), Default::default());
    let mut report = Report::default();
    compare(&frame, &save, &Ok(&membership), &mut report);
    assert!(!report.differences.is_empty());
    save.economy.supply.as_mut().unwrap().clear();
    // Repair the preceding missing-parent fixture so this case reaches
    // the independent inventory count and dense node-index checks.
    if let TableView::Complete(TableRows::SupplyNodes(rows)) =
        &mut *frame.tables[TableId::SupplyNodes.index()]
    {
        rows[0].parent = -1;
    }
    if let TableView::Complete(TableRows::TrainInventory(rows)) =
        &mut *frame.tables[TableId::TrainInventory.index()]
    {
        rows[0].node_count += 1;
    }
    let mut report = Report::default();
    compare(&frame, &save, &Ok(&membership), &mut report);
    assert_eq!(report.difference_groups.get("SupplyNodes"), Some(&1));
    assert!(
        report.failed_comparisons["SupplyNodes"].contains("native node index census is not dense")
    );
}

#[test]
fn owned_state_counts_make_saved_supply_membership_exact() {
    let (mut frame, mut save, _, _) = super::super::tests::fixture();
    let countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    *frame.tables[TableId::TrainInventory.index()] =
        TableView::Complete(TableRows::TrainInventory(
            countries
                .iter()
                .map(|c| TrainInventoryRow {
                    country: c.id,
                    ..Default::default()
                })
                .collect(),
        ));
    save.economy.supply = Some(Default::default());
    owned_states(&mut frame, &countries, 1);
    let mut report = Report::default();
    trains(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    owned_states(&mut frame, &countries, 0);
    let mut report = Report::default();
    trains(&frame, &save, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        !report
            .limitations
            .keys()
            .any(|reason| reason.contains("uncaptured"))
    );
    *frame.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
        reason: "country counter capture failed".into(),
    };
    let mut report = Report::default();
    trains(&frame, &save, &mut report).unwrap();
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("uncaptured"))
    );
}
