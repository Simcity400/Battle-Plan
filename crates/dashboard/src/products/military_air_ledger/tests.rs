use super::*;
use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::{
    frame::TableView,
    rows::{
        AirLossEventRow, BuildingBombingDamageRow, CountryLinkRow, CountryRow, LexicalTokenRow,
        TableRows,
    },
};

fn frame(
    hour: i32,
    losses: Vec<AirLossEventRow>,
    damage: Vec<BuildingBombingDamageRow>,
) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 9,
            canonical_slot: 3,
            tag: b"mod-country".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 9,
            canonical_slot: 3,
        }]));
    *frame.tables[TableId::AirLossEvents.index()] =
        TableView::Complete(TableRows::AirLossEvents(losses));
    *frame.tables[TableId::BuildingBombingDamage.index()] =
        TableView::Complete(TableRows::BuildingBombingDamage(damage));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 9001,
            literal: b"mod_building".to_vec(),
        }]));
    frame
}

#[test]
fn military_air_ledger_signed_corrections_zero_gap_and_native_dates() {
    let row = |id, hour, amount| AirLossEventRow {
        id,
        game_hour: hour,
        country: 9,
        region: 40001,
        native_cause: 1,
        family_base: 3,
        amount_raw: amount,
        ..Default::default()
    };
    let frames = vec![
        frame(10, vec![row(0, 9, i64::MAX)], vec![]),
        frame(11, vec![row(1, 11, -100_000)], vec![]),
        frame(13, vec![], vec![]),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let result = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        result.air.observed,
        [
            HourSpan {
                first_hour: 10,
                last_hour: 11
            },
            HourSpan {
                first_hour: 13,
                last_hour: 13
            }
        ]
    );
    let leaves = leaves(&result.air, None).unwrap();
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].identity.as_deref(), Some("3:9"));
    assert_eq!(
        leaves[0].points[0],
        MilitaryAirPoint {
            hour: 9,
            read_hour: 10,
            total_raw: i64::MAX.to_string()
        }
    );
    assert_eq!(
        leaves[0].points[1].total_raw,
        (i128::from(i64::MAX) - 100_000).to_string()
    );
    assert_eq!(
        result.air.countries.values().next().unwrap()[1].first_hour,
        13
    );
}

#[test]
fn military_air_ledger_bombing_keeps_unattributed_and_excludes_other_writers() {
    let damage = |id, cause| BuildingBombingDamageRow {
        id,
        game_hour: 10,
        attacker: None,
        victim: 9,
        state: 0,
        building_type: 9001,
        cause,
        amount_raw: 12_345_678,
        credited_raw: i64::MIN,
        ..Default::default()
    };
    let mut result = FamilyScan::default();
    bombing_frame(
        &mut result,
        &frame(
            10,
            vec![],
            vec![
                damage(0, BuildingBombingDamageCause::AirMission),
                damage(1, BuildingBombingDamageCause::LandCombat),
                damage(2, BuildingBombingDamageCause::Unknown),
            ],
        ),
    )
    .unwrap();
    let leaves = leaves(&result, None).unwrap();
    assert_eq!(leaves.len(), 2);
    assert!(leaves.iter().any(|leaf| leaf.dealt && leaf.tag.is_none()));
    assert!(
        leaves
            .iter()
            .any(|leaf| !leaf.dealt && leaf.tag.as_deref() == Some("mod-country"))
    );
    for leaf in leaves {
        assert_eq!(leaf.building_token.as_deref(), Some("mod_building"));
        assert_eq!(leaf.state, Some(0));
        assert_eq!(leaf.points[0].total_raw, "12345678");
    }
    assert!(
        result
            .warnings
            .keys()
            .any(|reason| reason.contains("unsupported caller"))
    );
    assert_eq!(
        result.incomplete,
        [HourSpan {
            first_hour: 10,
            last_hour: 10
        }]
    );
}

#[test]
fn military_air_ledger_missing_country_census_is_not_empty_and_duplicate_events_refuse() {
    let mut missing = frame(10, vec![], vec![]);
    *missing.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "missing links".into(),
    };
    let mut scan = FamilyScan::default();
    air_frame(&mut scan, &missing).unwrap();
    assert!(scan.observed.is_empty());
    let row = AirLossEventRow {
        id: 7,
        game_hour: 11,
        country: 9,
        region: 40001,
        family_base: 3,
        amount_raw: 100_000,
        ..Default::default()
    };
    let read = frame(11, vec![row], vec![]);
    air_frame(&mut scan, &read).unwrap();
    assert!(air_frame(&mut scan, &read).is_err());
}

#[test]
fn military_air_ledger_captures_every_identity_without_a_human_predicate() {
    let mut read = frame(10, vec![], vec![]);
    *read.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=5000)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("mod_{id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *read.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=5000)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    let mut scan = FamilyScan::default();
    air_frame(&mut scan, &read).unwrap();
    assert_eq!(scan.countries.len(), 5000);
    assert_eq!(scan.observed.len(), 1);
    assert!(scan.leaves.is_empty());
    let ranked = ranked_membership(
        &scan.countries,
        &[],
        10,
        &MilitaryAirTop {
            count: 5000,
            metric: MilitaryAirMetric::Taken,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(ranked.len(), 5000);
}

#[test]
fn military_air_ledger_publication_order_does_not_backfill_old_native_dates() {
    let row = |id, hour, amount| AirLossEventRow {
        id,
        game_hour: hour,
        country: 9,
        region: 40001,
        native_cause: 1,
        family_base: 3,
        amount_raw: amount,
        ..Default::default()
    };
    let mut scan = FamilyScan::default();
    air_frame(&mut scan, &frame(10, vec![row(0, 9, 300_000)], vec![])).unwrap();
    air_frame(&mut scan, &frame(11, vec![row(1, 8, -100_000)], vec![])).unwrap();
    let leaves = leaves(&scan, None).unwrap();
    assert_eq!(
        leaves[0].points,
        [
            MilitaryAirPoint {
                hour: 9,
                read_hour: 10,
                total_raw: "300000".into()
            },
            MilitaryAirPoint {
                hour: 8,
                read_hour: 11,
                total_raw: "200000".into()
            },
        ]
    );
}

#[test]
fn military_air_ledger_top_follows_signed_values_and_dated_holder_aliases() {
    let countries = BTreeMap::from([
        (
            ("old tag".into(), "3:9".into()),
            vec![HourSpan {
                first_hour: 10,
                last_hour: 10,
            }],
        ),
        (
            ("new tag".into(), "3:9".into()),
            vec![HourSpan {
                first_hour: 11,
                last_hour: 12,
            }],
        ),
        (
            ("zero".into(), "4:10".into()),
            vec![HourSpan {
                first_hour: 10,
                last_hour: 12,
            }],
        ),
    ]);
    let mut scan = FamilyScan::default();
    let key = LeafKey {
        country: Some(("old tag".into(), "3:9".into())),
        dealt: false,
        opponent: None,
        region: Some(40001),
        building: None,
        state: None,
        cause: 1,
    };
    scan.add(key.clone(), 9, 10, i64::MAX).unwrap();
    scan.add(key, 8, 11, i64::MIN).unwrap();
    let leaves = leaves(&scan, None).unwrap();
    let ranked = ranked_membership(
        &countries,
        &leaves,
        10,
        &MilitaryAirTop {
            count: 1,
            metric: MilitaryAirMetric::Taken,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        ranked.get("old tag"),
        Some(&vec![HourSpan {
            first_hour: 10,
            last_hour: 10
        }])
    );
    assert_eq!(
        ranked.get("zero"),
        Some(&vec![HourSpan {
            first_hour: 11,
            last_hour: 12
        }])
    );
    assert!(!ranked.contains_key("new tag"));
    let ranged_leaves = super::leaves(&scan, Some(11)).unwrap();
    let ranged = ranked_membership(
        &countries,
        &ranged_leaves,
        11,
        &MilitaryAirTop {
            count: 1,
            metric: MilitaryAirMetric::Taken,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        ranged.get("new tag"),
        Some(&vec![HourSpan {
            first_hour: 11,
            last_hour: 12
        }])
    );
}

#[test]
fn military_air_ledger_ranges_select_native_dates_before_publication_prefixes() {
    let mut scan = FamilyScan::default();
    let key = LeafKey {
        country: Some(("mod country".into(), "3:9".into())),
        dealt: false,
        opponent: None,
        region: Some(40001),
        building: None,
        state: None,
        cause: 1,
    };
    scan.add(key.clone(), 9, 10, 100_000).unwrap();
    scan.add(key.clone(), 10, 11, 200_000).unwrap();
    scan.add(key.clone(), 9, 12, -50_000).unwrap();
    scan.add(key, 11, 13, -25_000).unwrap();
    let leaves = leaves(&scan, Some(10)).unwrap();
    assert_eq!(
        leaves[0].points,
        [
            MilitaryAirPoint {
                hour: 10,
                read_hour: 11,
                total_raw: "200000".into()
            },
            MilitaryAirPoint {
                hour: 11,
                read_hour: 13,
                total_raw: "175000".into()
            },
        ]
    );
    assert!(leaves[0].points.iter().all(|point| point.read_hour > 10));
}

#[test]
fn military_air_ledger_block_keyframes_keep_branch_census_and_missing_hours_separate() {
    use replay::{frame::FrameEncoder, spool::Record};
    let row = |id, country, hour, amount| AirLossEventRow {
        id,
        game_hour: hour,
        country,
        region: 40001,
        native_cause: 1,
        family_base: 3,
        amount_raw: amount,
        ..Default::default()
    };
    let mut first = records(1, vec![frame(10, vec![row(1, 9, 9, 100_000)], vec![])]);
    let mut encoder = FrameEncoder::new();
    for hour in [14, 15] {
        let mut next = frame(
            hour,
            if hour == 14 {
                vec![row(2, 77, 14, 200_000)]
            } else {
                vec![]
            },
            vec![],
        );
        *next.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 77,
                canonical_slot: 4,
                tag: b"new holder".to_vec(),
                ..Default::default()
            }]));
        *next.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 77,
                canonical_slot: 4,
            }]));
        first.push(Record::Frame(encoder.encode(next, false).unwrap()));
    }
    let mut other = frame(14, vec![row(99, 9, 14, 900_000)], vec![]);
    other.branch = 2;
    let replay = Replay::from_records(CAMPAIGN.into(), &[first, records(2, vec![other])]).unwrap();
    assert_eq!(
        replay
            .index
            .iter()
            .filter(|block| block.branch == 1)
            .count(),
        2
    );
    let result = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        result.air.observed,
        [
            HourSpan {
                first_hour: 10,
                last_hour: 10
            },
            HourSpan {
                first_hour: 14,
                last_hour: 15
            }
        ]
    );
    let leaves = leaves(&result.air, None).unwrap();
    assert_eq!(leaves.len(), 2);
    assert!(leaves.iter().any(
        |leaf| leaf.identity.as_deref() == Some("3:9") && leaf.points[0].total_raw == "100000"
    ));
    assert!(
        leaves
            .iter()
            .any(|leaf| leaf.identity.as_deref() == Some("4:77")
                && leaf.points[0].read_hour == 14
                && leaf.points[0].total_raw == "200000")
    );
    assert_eq!(
        result.air.countries[&("mod-country".into(), "3:9".into())],
        [HourSpan {
            first_hour: 10,
            last_hour: 10
        }]
    );
}

#[test]
fn military_air_same_branch_accepts_published_events_after_refusal_and_partial_recovery() {
    let loss = |id, hour, amount| AirLossEventRow {
        id,
        game_hour: hour,
        country: 9,
        region: 40001,
        native_cause: 1,
        family_base: 3,
        amount_raw: amount,
        ..Default::default()
    };
    let damage = |id, hour, amount| BuildingBombingDamageRow {
        id,
        game_hour: hour,
        attacker: Some(9),
        victim: 9,
        state: 0,
        building_type: 9001,
        cause: BuildingBombingDamageCause::AirMission,
        amount_raw: amount,
        ..Default::default()
    };
    let first = frame(2, vec![loss(0, 1, 300_000)], vec![damage(0, 1, 10_000_000)]);
    let mut missed = frame(3, vec![], vec![]);
    let mut partial = frame(4, vec![], vec![]);
    for (read, reason) in [
        (
            &mut missed,
            "event ledger active or unwound native writer at drain",
        ),
        (
            &mut partial,
            "event ledger first/branch/recovery interval is partial",
        ),
    ] {
        for table in [TableId::AirLossEvents, TableId::BuildingBombingDamage] {
            *read.tables[table.index()] = TableView::Unavailable {
                reason: reason.into(),
            };
        }
    }
    // The real producer regression proves these recovered IDs remain newer.
    let recovered = frame(5, vec![loss(1, 4, -100_000)], vec![damage(1, 4, 5_000_000)]);
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![first, missed, partial, recovered])],
    )
    .unwrap();
    assert_eq!(replay.branches.len(), 1);
    let result = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let observed = [
        HourSpan {
            first_hour: 2,
            last_hour: 2,
        },
        HourSpan {
            first_hour: 5,
            last_hour: 5,
        },
    ];
    assert_eq!(result.air.observed, observed);
    assert_eq!(result.bombing.observed, observed);
    let air = leaves(&result.air, None).unwrap();
    assert_eq!(
        air[0].points,
        [
            MilitaryAirPoint {
                hour: 1,
                read_hour: 2,
                total_raw: "300000".into()
            },
            MilitaryAirPoint {
                hour: 4,
                read_hour: 5,
                total_raw: "200000".into()
            },
        ]
    );
    let bombing = leaves(&result.bombing, None).unwrap();
    assert_eq!(bombing.len(), 2);
    assert!(
        bombing
            .iter()
            .all(|leaf| leaf.points.last().unwrap().total_raw == "15000000")
    );
}

#[test]
fn military_air_resumed_attachment_keeps_the_existing_branch_and_prefixes() {
    use crate::test_fixtures::continuation_records;
    let loss = |id, hour, amount| AirLossEventRow {
        id,
        game_hour: hour,
        country: 9,
        region: 40001,
        native_cause: 1,
        family_base: 3,
        amount_raw: amount,
        ..Default::default()
    };
    let damage = |id, hour, amount| BuildingBombingDamageRow {
        id,
        game_hour: hour,
        attacker: Some(9),
        victim: 9,
        state: 0,
        building_type: 9001,
        cause: BuildingBombingDamageCause::AirMission,
        amount_raw: amount,
        ..Default::default()
    };
    let first = records(
        1,
        vec![frame(
            10,
            vec![loss(0, 9, 100_000)],
            vec![damage(0, 9, 10_000_000)],
        )],
    );
    let retained = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&first)).unwrap();
    let mut partial = frame(11, vec![], vec![]);
    for table in [TableId::AirLossEvents, TableId::BuildingBombingDamage] {
        *partial.tables[table.index()] = TableView::Unavailable {
            reason: "event ledger first/branch/recovery interval is partial".into(),
        };
    }
    // The retained-history/producer fixture proves the first resumed keys are max+1.
    let next = frame(
        12,
        vec![loss(1, 11, -25_000)],
        vec![damage(1, 11, 5_000_000)],
    );
    let continued = continuation_records(&retained, 1, vec![partial, next]);
    let replay = Replay::from_records(CAMPAIGN.into(), &[first, continued]).unwrap();
    assert_eq!(replay.branches.len(), 1);
    let result = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let air = leaves(&result.air, None).unwrap();
    assert_eq!(
        air[0].points.last().unwrap(),
        &MilitaryAirPoint {
            hour: 11,
            read_hour: 12,
            total_raw: "75000".into(),
        }
    );
    let bombing = leaves(&result.bombing, None).unwrap();
    assert_eq!(bombing.len(), 2);
    assert!(
        bombing
            .iter()
            .all(|leaf| leaf.points.last().unwrap().total_raw == "15000000")
    );
    assert_eq!(
        result.air.observed,
        [
            HourSpan {
                first_hour: 10,
                last_hour: 10
            },
            HourSpan {
                first_hour: 12,
                last_hour: 12
            }
        ]
    );
}
