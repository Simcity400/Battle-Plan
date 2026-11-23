use super::*;
use replay_data::{
    rows::CountryCounterInputRow, rows::FloatingHarborDeploymentRow, rows::Recorded,
    rows::SupplyNodeRow, rows::TrainInventoryRow,
};

#[test]
fn supply_compares_province_census_and_routes_despite_membership_exclusions() {
    let membership = super::super::fixtures::membership(
        "state={id=1 provinces={1}} \
         state={id=2 provinces={2} history={free_operative={operative=a}}}",
        2,
    );
    let (mut frame, ..) = super::super::fixtures::fixture();
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
    let null = |report: &Report| {
        report
            .limitations
            .iter()
            .find(|(reason, _)| reason.contains("null province"))
            .map(|(_, subjects)| subjects.clone())
            .unwrap_or_default()
    };
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        null(&report),
        [format!("SupplyNodes {country}:2.province 99")]
    );
    let TableView::Complete(TableRows::SupplyNodes(rows)) =
        &mut *frame.tables[TableId::SupplyNodes.index()]
    else {
        unreachable!()
    };
    rows[2].province = 0;
    rows[2].parent = 98;
    rows.push(SupplyNodeRow {
        country,
        index: SupplyNodeRow::VERTEX | 98,
        province: 98,
        parent: -1,
        ..Default::default()
    });
    let mut report = Report::default();
    nodes(&frame, &membership, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        null(&report),
        [
            format!("SupplyNodes {country}:2.province 0"),
            format!("SupplyNodes {country}:2.parent 98"),
            format!(
                "SupplyNodes {country}:{}.province 98",
                SupplyNodeRow::VERTEX | 98
            ),
        ]
    );
    let TableView::Complete(TableRows::SupplyNodes(rows)) =
        &mut *frame.tables[TableId::SupplyNodes.index()]
    else {
        unreachable!()
    };
    rows.pop();
    rows[2].province = 99;
    rows[2].parent = -1;
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

fn harbour_fixture(fields: &str, deployments: &[(i32, u8)]) -> (DecodedFrame, Save) {
    let (mut frame, mut save, ..) = super::super::fixtures::fixture();
    let country = frame.complete::<tables::Countries>().unwrap()[0].clone();
    let input = format!(
        "countries={{}} supply_system_2={{countries={{{}={{{fields}}}}}}}",
        String::from_utf8_lossy(&country.tag)
    );
    let document = content_reader_parser::parser::parse(input.as_bytes()).unwrap();
    save.economy = save::economy::parse(&document.items).unwrap();
    *frame.tables[TableId::TrainInventory.index()] =
        TableView::Complete(TableRows::TrainInventory(vec![TrainInventoryRow {
            country: country.id,
            ..Default::default()
        }]));
    *frame.tables[TableId::FloatingHarborDeployments.index()] =
        TableView::Complete(TableRows::FloatingHarborDeployments(
            deployments
                .iter()
                .enumerate()
                .map(
                    |(ordinal, &(province, status))| FloatingHarborDeploymentRow {
                        country: country.canonical_slot,
                        ordinal: ordinal as u32,
                        province,
                        status,
                    },
                )
                .collect(),
        ));
    (frame, save)
}

#[test]
fn floating_harbour_rows_join_supply_systems_with_dense_ordinals() {
    let (mut frame, save) = harbour_fixture("temp_nodes={{province=-1 base=yes}}", &[(-1, 255)]);
    let mut report = Report::default();
    floating_harbors(&frame, &save, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["FloatingHarborDeployments"], 1);
    let TableView::Complete(TableRows::FloatingHarborDeployments(rows)) =
        &mut *frame.tables[TableId::FloatingHarborDeployments.index()]
    else {
        unreachable!()
    };
    rows[0].ordinal = 1;
    assert!(floating_harbors(&frame, &save, &mut Report::default()).is_err());
    let TableView::Complete(TableRows::FloatingHarborDeployments(rows)) =
        &mut *frame.tables[TableId::FloatingHarborDeployments.index()]
    else {
        unreachable!()
    };
    rows[0].ordinal = 0;
    rows[0].country += 1;
    assert!(floating_harbors(&frame, &save, &mut Report::default()).is_err());
    let (mut frame, save) = harbour_fixture("", &[(1, 0)]);
    *frame.tables[TableId::TrainInventory.index()] =
        TableView::Complete(TableRows::TrainInventory(vec![]));
    assert!(floating_harbors(&frame, &save, &mut Report::default()).is_err());
}

#[test]
fn floating_harbour_compares_boolean_status_and_first_match_order_through_aliases() {
    let (mut frame, save) = harbour_fixture(
        "temp_nodes={{province=-1 base=no} {province=-1 base=yes} {province=-2147483648 base=yes}}",
        &[(-1, 0), (-1, 1), (i32::MIN, 2)],
    );
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *frame.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    countries[0].id = 102;
    let mut report = Report::default();
    floating_harbors(&frame, &save, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["FloatingHarborDeployments"], 3);
    assert_eq!(report.limitations.len(), 1);
    assert!(
        report
            .limitations
            .keys()
            .next()
            .unwrap()
            .contains("magnitude")
    );
    let TableView::Complete(TableRows::FloatingHarborDeployments(rows)) =
        &mut *frame.tables[TableId::FloatingHarborDeployments.index()]
    else {
        unreachable!()
    };
    rows[0].status = 1;
    rows[1].status = 0;
    let mut reordered = Report::default();
    floating_harbors(&frame, &save, &mut reordered).unwrap();
    assert_eq!(reordered.differences.len(), 2);
    assert!(
        reordered
            .differences
            .iter()
            .all(|difference| difference.contains(".base"))
    );
}

#[test]
fn floating_harbour_empty_missing_extra_reordered_and_changed_rows_compare() {
    for fields in ["", "temp_nodes={}"] {
        let (frame, save) = harbour_fixture(fields, &[]);
        let mut report = Report::default();
        floating_harbors(&frame, &save, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["FloatingHarborDeployments"], 0);
    }
    let fields = "temp_nodes={{province=3 base=no} {province=9 base=yes}}";
    for (deployments, field) in [
        (vec![], ".count"),
        (vec![(3, 0)], ".count"),
        (vec![(3, 0), (9, 1), (9, 1)], ".count"),
        (vec![(9, 1), (3, 0)], ".province"),
        (vec![(4, 0), (9, 1)], ".province"),
        (vec![(3, 1), (9, 1)], ".base"),
    ] {
        let (frame, save) = harbour_fixture(fields, &deployments);
        let mut report = Report::default();
        floating_harbors(&frame, &save, &mut report).unwrap();
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains(field))
        );
    }
    let (frame, mut save) = harbour_fixture("temp_nodes={{province=3 base=no}}", &[(3, 0)]);
    save.economy.supply.as_mut().unwrap().clear();
    let mut report = Report::default();
    floating_harbors(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains(".count"));
}

#[test]
fn floating_harbour_requires_save_and_complete_identity_inputs() {
    let (mut frame, mut save) = harbour_fixture("", &[]);
    save.economy.supply = None;
    assert!(floating_harbors(&frame, &save, &mut Report::default()).is_err());
    for table in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::TrainInventory,
        TableId::FloatingHarborDeployments,
    ] {
        let (mut frame, save) = harbour_fixture("", &[]);
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "test".into(),
        };
        assert!(floating_harbors(&frame, &save, &mut Report::default()).is_err());
    }
    save.economy.supply = Some(BTreeMap::from([(b"UNKNOWN".to_vec(), Default::default())]));
    assert!(floating_harbors(&frame, &save, &mut Report::default()).is_err());
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![]));
    assert!(floating_harbors(&frame, &save, &mut Report::default()).is_err());
}

#[test]
fn floating_harbour_keeps_each_saved_country_separate() {
    let (mut frame, mut save) = harbour_fixture("", &[]);
    let source = frame.complete::<tables::Countries>().unwrap()[0].clone();
    let countries: Vec<_> = (1..=4)
        .map(|slot| {
            let mut country = source.clone();
            country.id = slot + 10_000;
            country.canonical_slot = slot;
            country.tag = format!("C{slot}").into_bytes();
            country
        })
        .collect();
    save.economy.supply = Some(
        countries
            .iter()
            .map(|country| {
                (
                    country.tag.clone(),
                    save::economy::Supply {
                        temp_nodes: vec![save::economy::TemporaryNode {
                            ordinal: 0,
                            province: country.canonical_slot as i32,
                            base: true,
                        }],
                        ..Default::default()
                    },
                )
            })
            .collect(),
    );
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        countries
            .iter()
            .map(|country| replay_data::rows::CountryLinkRow {
                id: country.id,
                canonical_slot: country.canonical_slot,
            })
            .collect(),
    ));
    *frame.tables[TableId::TrainInventory.index()] =
        TableView::Complete(TableRows::TrainInventory(
            countries
                .iter()
                .map(|country| TrainInventoryRow {
                    country: country.id,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::FloatingHarborDeployments.index()] =
        TableView::Complete(TableRows::FloatingHarborDeployments(
            countries
                .iter()
                .map(|country| FloatingHarborDeploymentRow {
                    country: country.canonical_slot,
                    ordinal: 0,
                    province: country.canonical_slot as i32,
                    status: 2,
                })
                .collect(),
        ));
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    let mut report = Report::default();
    floating_harbors(&frame, &save, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["FloatingHarborDeployments"], 4);
    let TableView::Complete(TableRows::FloatingHarborDeployments(rows)) =
        &mut *frame.tables[TableId::FloatingHarborDeployments.index()]
    else {
        unreachable!()
    };
    rows[0].province = rows[1].province;
    let mut changed = Report::default();
    floating_harbors(&frame, &save, &mut changed).unwrap();
    assert_eq!(changed.differences.len(), 1);
    assert!(changed.differences[0].contains("C1.temp_nodes[0].province"));
}

fn owned_states(frame: &mut DecodedFrame, countries: &[replay_data::rows::CountryRow], count: i32) {
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            countries
                .iter()
                .map(|c| CountryCounterInputRow {
                    forward_visibility: replay_data::rows::Recorded::NotRecorded,
                    country: c.id,
                    owned_state_count: count,
                    ..Default::default()
                })
                .collect(),
        ));
}
#[test]
fn supply_joins_fail_for_bad_censuses_but_do_not_compare_distinct_save_caches() {
    let (mut frame, mut save, mut membership, _) = super::super::fixtures::fixture();
    let countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    owned_states(&mut frame, &countries, 1);
    save.economy.supply = Some(
        countries
            .iter()
            .map(|c| (c.tag.clone(), Default::default()))
            .collect(),
    );
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
                    controller_access: replay_data::rows::SparseBytes::from_dense(&[0, 255])
                        .unwrap(),
                    ..Default::default()
                })
                .collect(),
        ));
    let mut report = Report::default();
    compare(&frame, &save, &Ok(&membership), &mut report);
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
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
fn only_inventories_read_at_the_oracle_hour_compare_their_saved_variants() {
    let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
    let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    let mut second = countries[0].clone();
    second.id = countries[0].id + 50;
    second.canonical_slot += 50;
    second.tag = b"ENG".to_vec();
    countries.push(second);
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries.clone()));
    save.economy.supply = Some(Default::default());
    owned_states(&mut frame, &countries, 0);
    *frame.tables[TableId::TrainInventory.index()] =
        TableView::Complete(TableRows::TrainInventory(
            countries
                .iter()
                .map(|c| TrainInventoryRow {
                    country: c.id,
                    allocations: vec![replay_data::rows::TrainAllocation {
                        variant: 0xdead_0000_beef,
                        count: 1,
                    }],
                    ..Default::default()
                })
                .collect(),
        ));
    let unsaved = |report: &Report| {
        report
            .differences
            .iter()
            .filter(|line| line.contains("saved identity"))
            .count()
    };
    let mut report = Report::default();
    trains(&frame, &save, &mut report).unwrap();
    assert_eq!(unsaved(&report), 2);
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(vec![
        replay_data::rows::SliceReadRow {
            table: TableId::TrainInventory.index() as u32,
            owner: countries[0].id,
            sample_hour: frame.hour,
        },
        replay_data::rows::SliceReadRow {
            table: TableId::TrainInventory.index() as u32,
            owner: countries[1].id,
            sample_hour: frame.hour - 5,
        },
    ]));
    let mut report = Report::default();
    trains(&frame, &save, &mut report).unwrap();
    assert_eq!(unsaved(&report), 1, "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("owner decision 2026-10-11"))
    );
    let mut report = Report {
        forced_full: true,
        ..Default::default()
    };
    trains(&frame, &save, &mut report).unwrap();
    assert_eq!(unsaved(&report), 2);
}

#[test]
fn owned_state_counts_make_saved_supply_membership_exact() {
    let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
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

#[test]
fn supply_vehicle_limits_name_each_unsaved_scalar() {
    let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
    let source = frame.complete::<tables::Countries>().unwrap()[0].clone();
    let saved = save.countries.values().next().unwrap().clone();
    let countries: Vec<_> = (1..=4)
        .map(|id| {
            let mut country = source.clone();
            country.id = id;
            country.canonical_slot = id;
            country.tag = format!("C{id}").into_bytes();
            country
        })
        .collect();
    save.countries = countries
        .iter()
        .map(|country| (country.tag.clone(), saved.clone()))
        .collect();
    save.economy.supply = Some(Default::default());
    owned_states(&mut frame, &countries, 0);
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries.clone()));
    *frame.tables[TableId::TrainInventory.index()] =
        TableView::Complete(TableRows::TrainInventory(
            countries
                .iter()
                .map(|country| TrainInventoryRow {
                    country: country.id,
                    allocated_trucks: Recorded::Value((country.id % 2 == 0).then_some(0)),
                    required_trucks_raw: Recorded::Value((country.id % 2 == 0).then_some(i64::MAX)),
                    required_trains: Recorded::Value((country.id % 2 == 0).then_some(i32::MIN)),
                    ..Default::default()
                })
                .collect(),
        ));
    let mut report = Report::default();
    trains(&frame, &save, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["TrainInventory"], 4);
    for field in ["allocated_trucks", "required_trucks_raw", "required_trains"] {
        let subject =
            format!("TrainInventory.{field}: 2 rows captured; 2 native captures unavailable");
        assert!(
            report
                .limitations
                .values()
                .any(|subjects| subjects.contains(&subject)),
            "{field} limitation absent"
        );
        let reason = format!("TrainInventory.{field} native capture is unavailable");
        assert_eq!(report.limitations[&reason].len(), 2);
        assert!(
            !report
                .limitations
                .contains_key(&format!("TrainInventory.{field} not recorded"))
        );
    }
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("without first clearing"))
    );
    let TableView::Complete(TableRows::TrainInventory(rows)) =
        &mut *frame.tables[TableId::TrainInventory.index()]
    else {
        unreachable!()
    };
    for row in rows {
        row.allocated_trucks = Recorded::NotRecorded;
        row.required_trucks_raw = Recorded::NotRecorded;
        row.required_trains = Recorded::NotRecorded;
    }
    let mut historical = Report::default();
    trains(&frame, &save, &mut historical).unwrap();
    assert!(historical.differences.is_empty());
    assert_eq!(historical.compared_tables["TrainInventory"], 4);
    for field in ["allocated_trucks", "required_trucks_raw", "required_trains"] {
        let reason = format!("TrainInventory.{field} not recorded");
        assert_eq!(historical.limitations[&reason].len(), 4);
    }
    assert!(
        !historical
            .limitations
            .keys()
            .any(|reason| reason.contains("native capture is unavailable"))
    );
    assert!(
        !historical
            .limitations
            .keys()
            .any(|reason| reason.contains("without first clearing"))
    );
}

#[test]
fn required_trucks_oracle_checks_non_negative_and_keeps_other_scalars_when_withheld() {
    let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
    let country = frame.complete::<tables::Countries>().unwrap()[0].clone();
    save.economy.supply = Some(Default::default());
    owned_states(&mut frame, std::slice::from_ref(&country), 0);
    *frame.tables[TableId::TrainInventory.index()] =
        TableView::Complete(TableRows::TrainInventory(vec![TrainInventoryRow {
            country: country.id,
            allocated_trucks: Recorded::Value(Some(40)),
            required_trucks_raw: Recorded::Value(Some(9_000_000)),
            required_trains: Recorded::Value(Some(5)),
            ..Default::default()
        }]));
    let mut ok = Report::default();
    trains(&frame, &save, &mut ok).unwrap();
    assert!(ok.differences.is_empty(), "{:?}", ok.differences);
    assert!(
        ok.limitations
            .keys()
            .any(|reason| reason.contains("same-hour supply nodes"))
    );

    let TableView::Complete(TableRows::TrainInventory(rows)) =
        &mut *frame.tables[TableId::TrainInventory.index()]
    else {
        unreachable!()
    };
    rows[0].required_trucks_raw = Recorded::Value(Some(-1));
    let mut bad = Report::default();
    trains(&frame, &save, &mut bad).unwrap();
    assert!(
        bad.differences
            .iter()
            .any(|line| line.contains("required_trucks_raw non-negative")),
        "{:?}",
        bad.differences
    );

    let TableView::Complete(TableRows::TrainInventory(rows)) =
        &mut *frame.tables[TableId::TrainInventory.index()]
    else {
        unreachable!()
    };
    rows[0].required_trucks_raw = Recorded::Value(None);
    let mut withheld = Report::default();
    trains(&frame, &save, &mut withheld).unwrap();
    assert!(withheld.differences.is_empty(), "{:?}", withheld.differences);
    assert!(
        withheld
            .limitations
            .contains_key("TrainInventory.required_trucks_raw native capture is unavailable")
    );
    assert!(
        !withheld
            .limitations
            .contains_key("TrainInventory.allocated_trucks native capture is unavailable")
    );
    assert!(
        !withheld
            .limitations
            .contains_key("TrainInventory.required_trains native capture is unavailable")
    );
}
