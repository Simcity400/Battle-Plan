use super::*;
use replay::{
    frame::{Metrics, TableView},
    rows::{CampaignRow, CountryLinkRow, CountryRow, ProvinceRow, StateRow, TableRows},
};

#[test]
fn territory_region_names_keep_successful_empty_text_distinct_from_failure() {
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "map/strategicregions/mod.txt",
            "strategic_region={id=11 name=EMPTY} strategic_region={id=12} strategic_region={id=13 name=ABSENT} strategic_region={id=14 name=REFUSED}",
        ),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n EMPTY:0 \"\"\n REFUSED:0 \"a\u{a4}b\"\n",
        ),
    ]);
    let regions = Regions::read(&catalogue).unwrap();
    for (region, name) in regions.names[..3].iter().zip(["", "", "ABSENT"]) {
        assert_eq!(region.name.as_deref(), Some(name));
        assert!(region.missing.is_none());
    }
    assert!(regions.names[3].name.is_none());
    assert!(regions.names[3].missing.is_some());
    assert!(
        regions
            .warnings
            .iter()
            .any(|warning| { warning.starts_with("1 strategic region names unavailable") })
    );
}

fn frame(hour: i32, count: u32) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "outside fixture".into(),
    });
    tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: b"analytics-test".to_vec(),
        }]));
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("realm {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=count)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        (1..=count)
            .map(|id| StateRow {
                id,
                owner: id,
                controller: id,
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        (1..=count)
            .map(|id| ProvinceRow {
                id,
                state: id,
                owner: id,
                controller: id,
                victory_points: -7,
                ..Default::default()
            })
            .collect(),
    ));
    let mut metrics = Metrics::default();
    metrics.table_ns[TableId::States.index()] = 1;
    metrics.table_ns[TableId::Provinces.index()] = 1;
    DecodedFrame {
        branch: 1,
        hour,
        tables: tables.map(Into::into),
        metrics,
    }
}
fn regions(count: u32) -> Regions {
    Regions {
        provinces: (1..=count).map(|id| (id, Some(id))).collect(),
        states: (1..=count)
            .map(|id| {
                (
                    id,
                    vec![StatePart {
                        region: Some(id),
                        numerator: 1,
                        denominator: 1,
                    }],
                )
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn every_country_region_and_signed_value_survives_without_a_count_ceiling() {
    let mut scan = Scan::default();
    observe(&mut scan, &frame(100, 5_000), &regions(5_000), None).unwrap();
    assert_eq!(scan.series.len(), 5_000);
    for series in scan.series.values() {
        assert!(series.tag.is_some());
        assert_eq!(
            series.runs[0].values,
            [
                Some("1".into()),
                Some("-7".into()),
                None,
                Some("1".into()),
                Some("-7".into()),
                None
            ]
        );
        assert_eq!(
            series.runs[0].state_shares[0].as_ref().unwrap(),
            &[TerritoryStateShare {
                numerator: "1".into(),
                denominator: 1
            }]
        );
    }
}
#[test]
fn gaps_and_reused_tags_start_new_stretches_without_inventing_zeroes() {
    let mut scan = Scan::default();
    let regions = regions(1);
    observe(&mut scan, &frame(100, 1), &regions, None).unwrap();
    observe(&mut scan, &frame(102, 1), &regions, None).unwrap();
    let mut renamed = frame(103, 1);
    *renamed.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 2,
            canonical_slot: 1,
            tag: b"realm 1".to_vec(),
            ..Default::default()
        }]));
    *renamed.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 1,
            },
        ]));
    observe(&mut scan, &renamed, &regions, None).unwrap();
    let runs = &scan.series.values().next().unwrap().runs;
    assert_eq!(
        runs.iter()
            .map(|run| (run.first_hour, run.last_hour, run.starts_segment))
            .collect::<Vec<_>>(),
        [(100, 100, true), (102, 102, true), (103, 103, true)]
    );
}
#[test]
fn multi_region_states_split_exactly_while_unknown_holders_keep_named_parts() {
    let mut frame = frame(100, 1);
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
        ProvinceRow {
            id: 1,
            state: 1,
            owner: 1,
            controller: 1,
            ..Default::default()
        },
        ProvinceRow {
            id: 2,
            state: 1,
            owner: 0,
            controller: 99,
            victory_points: -9,
            ..Default::default()
        },
    ]));
    let mut scan = Scan::default();
    let mut regions = regions(2);
    regions
        .states
        .insert(1, state_parts(&[1, 2], &regions.provinces).unwrap());
    observe(&mut scan, &frame, &regions, None).unwrap();
    for region in [1, 2] {
        let state = scan
            .series
            .values()
            .find(|series| {
                series.tag.as_deref() == Some("realm 1") && series.region == Some(region)
            })
            .unwrap();
        assert_eq!(
            state.runs[0].state_shares[0].as_ref().unwrap(),
            &[TerritoryStateShare {
                numerator: "1".into(),
                denominator: 2
            }]
        );
        assert!(state.missing.is_none());
    }
    let unknown = scan
        .series
        .values()
        .find(|series| {
            series.tag.is_none()
                && series
                    .missing
                    .as_deref()
                    .is_some_and(|m| m.starts_with("Country"))
        })
        .unwrap();
    assert_eq!(unknown.runs[0].values[1].as_deref(), Some("-9"));
    assert!(
        scan.series
            .values()
            .any(|series| series.tag.is_none() && series.missing.is_none())
    );
}

#[test]
fn unchanged_tables_compress_but_unavailable_tables_cannot_reuse_a_known_measure() {
    let first = frame(100, 1);
    let mut same = first.clone();
    same.hour = 101;
    let mut stale = same.clone();
    stale.hour = 102;
    *stale.tables[TableId::States.index()] = TableView::Unavailable {
        reason: "unreadable states".into(),
    };
    let mut scan = Scan::default();
    for read in [&first, &same, &stale] {
        observe(&mut scan, read, &regions(1), None).unwrap();
    }
    let runs = &scan.series.values().next().unwrap().runs;
    assert_eq!((runs[0].first_hour, runs[0].last_hour), (100, 101));
    assert_eq!(runs[1].state_shares[0], None);
    assert_eq!(runs[1].values[0].as_deref(), Some("1"));
}

#[test]
fn native_content_roster_keeps_repeats_and_unattributed_shares_without_a_province_read() {
    let mut regions = regions(2);
    regions
        .states
        .insert(1, state_parts(&[1, 1, 2, 99], &regions.provinces).unwrap());
    let mut read = frame(100, 1);
    *read.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "province read failed".into(),
    };
    let mut scan = Scan::default();
    observe(&mut scan, &read, &regions, None).unwrap();
    let shares: BTreeMap<_, _> = scan
        .series
        .values()
        .map(|series| (series.region, series.runs[0].state_shares[0].clone()))
        .collect();
    for (region, numerator) in [(Some(1), "2"), (Some(2), "1"), (None, "1")] {
        assert_eq!(
            shares[&region].as_ref().unwrap(),
            &[TerritoryStateShare {
                numerator: numerator.into(),
                denominator: 4
            }]
        );
    }
    assert!(
        scan.series
            .values()
            .all(|series| series.runs[0].values[0].is_none())
    );
}

#[test]
fn state_terms_combine_by_denominator_without_rounding_or_lcm_limits() {
    let mut regions = regions(2);
    regions
        .states
        .insert(1, state_parts(&[1, 2], &regions.provinces).unwrap());
    regions
        .states
        .insert(2, state_parts(&[1, 1, 2], &regions.provinces).unwrap());
    let mut read = frame(100, 1);
    *read.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 1,
            owner: 1,
            controller: 1,
            ..Default::default()
        },
        StateRow {
            id: 2,
            owner: 1,
            controller: 1,
            ..Default::default()
        },
    ]));
    let mut scan = Scan::default();
    observe(&mut scan, &read, &regions, None).unwrap();
    let first = scan
        .series
        .values()
        .find(|series| series.region == Some(1))
        .unwrap();
    assert_eq!(
        first.runs[0].state_shares[0].as_ref().unwrap(),
        &[
            TerritoryStateShare {
                numerator: "1".into(),
                denominator: 2
            },
            TerritoryStateShare {
                numerator: "2".into(),
                denominator: 3
            },
        ]
    );
}
