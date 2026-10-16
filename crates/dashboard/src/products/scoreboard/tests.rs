use super::*;
use crate::test_fixtures::{CAMPAIGN, records, source as frame};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{
        CountryCounterInputRow, CountryFactoriesRow, CountryLinkRow, CountryRow, TableId,
        TableRows, tables,
    },
};
use serde_json::{Value, json};
use std::sync::Arc;

fn countries(hour: i32, count: u32) -> DecodedFrame {
    let mut out = frame(1, hour, Some(1));
    *out.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("country-{id:05}").into_bytes(),
                political_power_raw: (id != 2).then_some(i64::from(id) * 100_001),
                command_power_raw: i64::from(id) * 100_003,
                ..Default::default()
            })
            .collect(),
    ));
    *out.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=count)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    *out.tables[TableId::CountryFactories.index()] =
        TableView::Complete(TableRows::CountryFactories(
            (1..=count)
                .map(|country| CountryFactoriesRow {
                    country,
                    civilian_total_raw: i64::from(country) * 100_000,
                    civilian_damaged_raw: 100_000,
                    consumer_goods: 1,
                    civilian_unavailable: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    *out.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            (1..=count)
                .map(|country| CountryCounterInputRow {
                    country,
                    owned_state_count: (country * 2) as i32,
                    ..Default::default()
                })
                .collect(),
        ));
    out
}

fn opened(frames: Vec<DecodedFrame>) -> Source {
    Source::from_replay(
        crate::SourceKey {
            source_id: "scoreboard-synthetic".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn request(
    columns: Vec<ScoreboardColumnKey>,
    tags: Vec<String>,
    top: Option<ScoreboardTop>,
) -> ScoreboardParams {
    ScoreboardParams {
        branch_id: "1".into(),
        columns,
        tags,
        top,
    }
}

#[test]
fn explicit_casualty_columns_share_exact_signed_subtotals_and_reads() {
    use replay::rows::WarRelationRow;
    let source = opened(
        (100..=145)
            .filter(|hour| !(127..130).contains(hour))
            .map(|hour| {
                let mut out = countries(hour, 3);
                *out.tables[TableId::WarRelations.index()] =
                    TableView::Complete(TableRows::WarRelations(if hour < 130 {
                        (2..=3)
                            .map(|second_country| WarRelationRow {
                                first_country: 1,
                                second_country,
                                start_hour: 90,
                                first_casualties: i64::MAX,
                                second_casualties: if hour == 126 { -3 } else { 7 },
                                ..Default::default()
                            })
                            .collect()
                    } else {
                        Vec::new()
                    }));
                out
            })
            .collect(),
    );
    let cancelled = AtomicBool::new(false);
    let tag = "country-00001".to_owned();
    let shared = war_casualties::scanned_branch(&source, 1, &cancelled).unwrap();
    let reply = Scoreboard::load(
        &source,
        &request(
            vec![
                ScoreboardColumnKey::CasualtiesDealt,
                ScoreboardColumnKey::CasualtiesTaken,
            ],
            vec![tag.clone()],
            None,
        ),
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    let columns = &reply.series[0].columns;
    for hour in 100..=145 {
        let expected = shared.country_at(&tag, hour);
        for (key, dealt) in [
            (ScoreboardColumnKey::CasualtiesDealt, true),
            (ScoreboardColumnKey::CasualtiesTaken, false),
        ] {
            let column = columns.iter().find(|column| column.key == key).unwrap();
            let value = column
                .runs
                .iter()
                .find(|run| run.first_hour <= hour && hour <= run.last_hour);
            let read = column
                .read_runs
                .iter()
                .find(|run| run.first_hour <= hour && hour <= run.last_hour);
            assert_eq!(
                value
                    .and_then(|run| run.amount.as_ref())
                    .map(|raw| raw.parse::<i128>().unwrap()),
                expected
                    .as_ref()
                    .map(|measure| if dealt { measure.dealt } else { measure.taken })
            );
            assert_eq!(
                read.map(|run| run.read_hour),
                expected.as_ref().map(|measure| measure.read_hour)
            );
            if let (Some(value), Some(expected)) = (value, &expected) {
                assert_eq!(value.at_least, expected.at_least);
            }
        }
    }
    assert_eq!(
        columns
            .iter()
            .find(|column| column.key == ScoreboardColumnKey::CasualtiesTaken)
            .unwrap()
            .runs[0]
            .amount
            .as_deref(),
        Some((i128::from(i64::MAX) * 2).to_string().as_str())
    );
    assert!(
        columns
            .iter()
            .flat_map(|column| &column.runs)
            .any(|run| run.at_least)
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("entirely inside a gap"))
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("negative corrections"))
    );
    assert!(
        columns
            .iter()
            .find(|column| column.key == ScoreboardColumnKey::CasualtiesDealt)
            .unwrap()
            .runs
            .iter()
            .any(|run| run.at_least && run.amount.as_deref() == Some("-6"))
    );
    for key in [
        ScoreboardColumnKey::CasualtiesDealt,
        ScoreboardColumnKey::CasualtiesTaken,
    ] {
        let column = reply
            .columns
            .iter()
            .find(|column| column.key == key)
            .unwrap();
        assert!(column.available);
        let note = column.note.as_deref().unwrap();
        assert!(note.contains("across land, naval and air"));
        assert!(note.contains("† marks a partial recorded subtotal"));
        assert!(
            Scoreboard::validate(&request(
                vec![key],
                vec![],
                Some(ScoreboardTop {
                    count: 5000,
                    sort: key,
                    descending: true,
                })
            ))
            .is_ok()
        );
    }
    let ranked = Scoreboard::load(
        &source,
        &request(
            vec![ScoreboardColumnKey::CasualtiesTaken],
            vec![],
            Some(ScoreboardTop {
                count: 1,
                sort: ScoreboardColumnKey::CasualtiesTaken,
                descending: true,
            }),
        ),
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    assert_eq!(ranked.series.len(), 1);
    assert_eq!(ranked.series[0].tag, tag);
    assert!(
        ranked.series[0].columns[0]
            .runs
            .iter()
            .any(|run| run.at_least)
    );
}

#[test]
fn scoreboard_columns_agree_with_single_stat_shared_products() {
    use super::super::country_measure::{CountryMeasure, Measure, MeasureParams};
    let source = opened(vec![countries(100, 3), countries(101, 3)]);
    let tags = vec!["country-00001".into(), "country-00002".into()];
    let keys = vec![
        ScoreboardColumnKey::Factories,
        ScoreboardColumnKey::FactoriesBuilt,
        ScoreboardColumnKey::FactoriesTopbar,
        ScoreboardColumnKey::PoliticalPower,
        ScoreboardColumnKey::CommandPower,
        ScoreboardColumnKey::Territory,
    ];
    let reply = Scoreboard::load(
        &source,
        &request(keys, tags.clone(), None),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    let factory = country_factories::scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    for row in &reply.series {
        for (key, mode) in [
            (ScoreboardColumnKey::Factories, FactoryCount::Free),
            (ScoreboardColumnKey::FactoriesBuilt, FactoryCount::Built),
            (ScoreboardColumnKey::FactoriesTopbar, FactoryCount::TopBar),
        ] {
            let column = row.columns.iter().find(|column| column.key == key).unwrap();
            for (actual, recorded) in column.runs.iter().zip(&factory.series[&row.tag]) {
                let expected: i64 = FactoryGroupKey::ALL
                    .iter()
                    .map(|group| country_factories::group_count(&recorded.values, *group, mode))
                    .sum();
                assert_eq!(
                    actual.amount.as_deref(),
                    Some(expected.to_string().as_str())
                );
                assert_eq!(
                    (actual.first_hour, actual.last_hour),
                    (recorded.first_hour, recorded.last_hour)
                );
            }
        }
        for (key, measure) in [
            (ScoreboardColumnKey::PoliticalPower, Measure::PoliticalPower),
            (ScoreboardColumnKey::CommandPower, Measure::CommandPower),
        ] {
            let single = CountryMeasure::load(
                &source,
                &MeasureParams {
                    branch_id: "1".into(),
                    tags: vec![row.tag.clone()],
                    measure,
                },
                &LoadContext { content: None },
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(
                row.columns
                    .iter()
                    .find(|column| column.key == key)
                    .unwrap()
                    .runs,
                fixed_runs(&single.series[0].runs)
            );
        }
    }
    assert!(
        reply.series[1]
            .columns
            .iter()
            .find(|column| column.key == ScoreboardColumnKey::PoliticalPower)
            .unwrap()
            .runs
            .is_empty()
    );
    assert_eq!(
        reply.series[0]
            .columns
            .iter()
            .find(|column| column.key == ScoreboardColumnKey::Territory)
            .unwrap()
            .runs[0]
            .amount
            .as_deref(),
        Some("2")
    );
    // All three Scoreboard factory counts and the single-stat request share one walk.
    assert_eq!(
        source
            .state::<country_factories::Scans>()
            .lock()
            .unwrap()
            .runs,
        1
    );
}

#[test]
fn scoreboard_native_owned_count_missing_and_ambiguous_is_never_zero() {
    let mut second = countries(101, 2);
    *second.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(vec![
            CountryCounterInputRow {
                country: 1,
                owned_state_count: 0,
                ..Default::default()
            },
        ]));
    let mut third = countries(102, 2);
    *third.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
        reason: "synthetic missing read".into(),
    };
    let source = opened(vec![countries(100, 2), second, third]);
    let reply = Scoreboard::load(
        &source,
        &request(
            vec![ScoreboardColumnKey::Territory],
            vec!["country-00001".into(), "country-00002".into()],
            None,
        ),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply.series[0].columns[0]
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.amount.as_deref()))
            .collect::<Vec<_>>(),
        [(100, 100, Some("2")), (101, 101, Some("0"))]
    );
    assert_eq!(
        reply.series[1].columns[0]
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100)]
    );
    assert_eq!(
        reply.series[1].segments,
        [HourSpan {
            first_hour: 100,
            last_hour: 102
        }]
    );
}

#[test]
fn scoreboard_reranks_all_5000_countries_and_fills_missing_last() {
    let mut second = countries(101, 5000);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *second.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(9_000_000_000);
    }
    let source = opened(vec![countries(100, 5000), second]);
    let reply = Scoreboard::load(
        &source,
        &request(
            vec![ScoreboardColumnKey::PoliticalPower],
            vec![],
            Some(ScoreboardTop {
                count: 1,
                sort: ScoreboardColumnKey::PoliticalPower,
                descending: true,
            }),
        ),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.series.len(), 2);
    assert_eq!(reply.series[0].tag, "country-00001");
    assert_eq!(
        reply.series[0].membership,
        Some(vec![HourSpan {
            first_hour: 101,
            last_hour: 101
        }])
    );
    assert_eq!(reply.series[1].tag, "country-05000");
    assert_eq!(
        reply.series[1].membership,
        Some(vec![HourSpan {
            first_hour: 100,
            last_hour: 100
        }])
    );
    let ascending = Scoreboard::load(
        &source,
        &request(
            vec![ScoreboardColumnKey::PoliticalPower],
            vec![],
            Some(ScoreboardTop {
                count: 1,
                sort: ScoreboardColumnKey::PoliticalPower,
                descending: false,
            }),
        ),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        ascending
            .series
            .iter()
            .all(|row| row.tag != "country-00002")
    );
    assert!(json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_some());
}

#[test]
fn scoreboard_registry_needs_no_scan_and_force_columns_allow_ranking() {
    let source = opened(vec![countries(100, 3)]);
    let reply = Scoreboard::load(
        &source,
        &request(vec![], vec![], None),
        &LoadContext { content: None },
        &AtomicBool::new(true),
    )
    .unwrap();
    assert!(reply.series.is_empty());
    assert_eq!(reply.columns.len(), ScoreboardColumnKey::ALL.len());
    assert_eq!(reply.default_sort, ScoreboardColumnKey::Factories);
    assert!(
        reply
            .default_columns
            .contains(&ScoreboardColumnKey::CasualtiesDealt)
    );
    assert!(
        reply
            .default_columns
            .contains(&ScoreboardColumnKey::CasualtiesTaken)
    );
    assert!(
        !reply
            .default_columns
            .contains(&ScoreboardColumnKey::Casualties)
    );
    for key in [
        ScoreboardColumnKey::ArmySize,
        ScoreboardColumnKey::Planes,
        ScoreboardColumnKey::Ships,
    ] {
        let column = reply
            .columns
            .iter()
            .find(|column| column.key == key)
            .unwrap();
        assert!(column.available);
        assert!(column.reason.is_none());
    }
    let singular = reply
        .columns
        .iter()
        .find(|column| column.key == ScoreboardColumnKey::Casualties)
        .unwrap();
    assert!(!singular.available);
    assert!(singular.reason.as_ref().unwrap().contains("dealt"));
    assert_eq!(
        source
            .state::<country_factories::Scans>()
            .lock()
            .unwrap()
            .runs,
        0
    );
    for key in [
        ScoreboardColumnKey::ArmySize,
        ScoreboardColumnKey::Planes,
        ScoreboardColumnKey::Ships,
    ] {
        assert!(
            Scoreboard::validate(&request(
                vec![key],
                vec![],
                Some(ScoreboardTop {
                    count: 2,
                    sort: key,
                    descending: true,
                })
            ))
            .is_ok()
        );
    }
    let reply = Scoreboard::load(
        &source,
        &request(
            vec![ScoreboardColumnKey::ArmySize],
            vec!["country-00001".into(), "country-00002".into()],
            None,
        ),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.series.len(), 2);
    assert!(
        reply
            .series
            .iter()
            .all(|row| row.columns[0].runs.iter().all(|run| run.amount.is_none()))
    );
}

#[test]
fn scoreboard_force_columns_share_roster_counts_zero_and_hourly_reads_at_scale() {
    use replay::rows::{AirWingRow, ShipRow, TaskForceRow, WingEquipmentRow};
    let force_frame = |hour, planes| {
        let mut out = countries(hour, 5000);
        *out.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![AirWingRow {
                id: 1,
                owner: 1,
                country_override: 2,
                planes,
                ..Default::default()
            }]));
        *out.tables[TableId::WingEquipment.index()] =
            TableView::Complete(TableRows::WingEquipment(vec![WingEquipmentRow {
                wing: 1,
                ordinal: 0,
                variant: 1,
                amount_raw: i64::from(planes) * 100_000,
            }]));
        *out.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
                id: 1,
                roster_owner: 1,
                logical_owner: 2,
                ..Default::default()
            }]));
        *out.tables[TableId::Ships.index()] =
            TableView::Complete(TableRows::Ships(vec![ShipRow {
                id: 1,
                task_force: 1,
                ..Default::default()
            }]));
        out
    };
    let mut missing = force_frame(101, 5);
    *missing.tables[TableId::AirWings.index()] = TableView::Unavailable {
        reason: "unread census".into(),
    };
    *missing.tables[TableId::Ships.index()] = TableView::Unavailable {
        reason: "unread census".into(),
    };
    let source = opened(vec![force_frame(100, 5), missing, force_frame(103, 10)]);
    let cancelled = AtomicBool::new(false);
    let keys = vec![ScoreboardColumnKey::Planes, ScoreboardColumnKey::Ships];
    let tags: Vec<_> = (1..=5000).map(|id| format!("country-{id:05}")).collect();
    let reply = Scoreboard::load(
        &source,
        &request(keys.clone(), tags, None),
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    assert_eq!(reply.series.len(), 5000);
    for hour in [100, 101, 102, 103] {
        for series in &reply.series {
            for key in &keys {
                let expected = matches!(hour, 100 | 103).then(|| {
                    if series.tag != "country-00001" {
                        0
                    } else if *key == ScoreboardColumnKey::Ships {
                        1
                    } else if hour == 100 {
                        5
                    } else {
                        10
                    }
                });
                let column = series
                    .columns
                    .iter()
                    .find(|column| column.key == *key)
                    .unwrap();
                let run = column
                    .runs
                    .iter()
                    .find(|run| run.first_hour <= hour && hour <= run.last_hour);
                assert_eq!(
                    run.and_then(|run| run.amount.as_ref())
                        .map(|amount| amount.parse::<i64>().unwrap()),
                    expected,
                    "{} {key:?} at {hour}",
                    series.tag
                );
                let read = column
                    .read_runs
                    .iter()
                    .find(|run| run.first_hour <= hour && hour <= run.last_hour);
                assert_eq!(read.is_some(), run.is_some());
                if let Some(read) = read {
                    assert!(read.hourly);
                    assert_eq!(read.read_hour, read.first_hour);
                }
                if matches!(hour, 100 | 103) {
                    assert!(read.is_some());
                    assert_eq!(read.unwrap().read_hour, hour);
                }
                if hour == 102 {
                    assert!(read.is_none());
                }
            }
        }
    }
    let first = &reply.series[0];
    assert_eq!(first.columns[0].runs[0].amount.as_deref(), Some("5"));
    assert_eq!(
        reply.series[1].columns[0].runs[0].amount.as_deref(),
        Some("0")
    );
    assert!(
        first
            .columns
            .iter()
            .flat_map(|column| &column.runs)
            .all(|run| !(run.first_hour <= 102 && 102 <= run.last_hour))
    );
    let ranked = Scoreboard::load(
        &source,
        &request(
            keys,
            vec![],
            Some(ScoreboardTop {
                count: 1,
                sort: ScoreboardColumnKey::Planes,
                descending: true,
            }),
        ),
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    assert_eq!(ranked.series[0].tag, "country-00001");
}

#[test]
fn scoreboard_army_size_uses_exact_owner_manpower_and_preserves_missing_input() {
    use replay::rows::{
        DivisionCensusRow, DivisionLandInputsRow, DivisionManpowerContributor, DivisionRow,
    };
    let mut first = countries(100, 2);
    *first.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: 1,
            roster_owner: 2,
            logical_owner: 2,
            expeditionary_owner: 1,
            ..Default::default()
        }]));
    *first.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(vec![
            DivisionCensusRow {
                country: 1,
                grouped: 1,
                ungrouped: 0,
            },
            DivisionCensusRow {
                country: 2,
                grouped: 0,
                ungrouped: 0,
            },
        ]));
    *first.tables[TableId::DivisionLandInputs.index()] =
        TableView::Complete(TableRows::DivisionLandInputs(vec![DivisionLandInputsRow {
            division: 1,
            manpower: vec![
                DivisionManpowerContributor {
                    country: 2,
                    amount: i32::MAX,
                },
                DivisionManpowerContributor {
                    country: 1,
                    amount: 1,
                },
            ],
            ..Default::default()
        }]));
    let mut missing = first.clone();
    missing.hour = 101;
    *missing.tables[TableId::DivisionLandInputs.index()] =
        TableView::Complete(TableRows::DivisionLandInputs(vec![]));
    let source = opened(vec![first, missing]);
    let cancelled = AtomicBool::new(false);
    let reply = Scoreboard::load(
        &source,
        &request(
            vec![ScoreboardColumnKey::ArmySize],
            vec!["country-00001".into(), "country-00002".into()],
            None,
        ),
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    assert_eq!(
        reply.series[0].columns[0].runs[0].amount.as_deref(),
        Some(i32::MIN.to_string().as_str())
    );
    assert!(
        reply.series[0].columns[0]
            .runs
            .iter()
            .any(|run| run.first_hour == 101 && run.amount.is_none())
    );
    assert_eq!(
        reply.series[1].columns[0].runs[0].amount.as_deref(),
        Some("0")
    );
    assert_eq!(
        (
            reply.series[1].columns[0].runs[0].first_hour,
            reply.series[1].columns[0].runs[0].last_hour,
        ),
        (100, 101)
    );
    let column = reply
        .columns
        .iter()
        .find(|column| column.key == ScoreboardColumnKey::ArmySize)
        .unwrap();
    assert_eq!(column.unit, "people");
    assert!(column.note.as_ref().unwrap().contains("owner"));
    let absent = Scoreboard::load(
        &source,
        &request(
            vec![ScoreboardColumnKey::ArmySize],
            vec!["absent-country".into()],
            None,
        ),
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    assert!(absent.series.is_empty());
}

#[test]
fn scoreboard_identity_does_not_bridge_missing_hours_tag_reuse_or_disappearance() {
    let first = countries(100, 2);
    let mut second = countries(101, 1);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *second.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"renamed".to_vec();
    }
    let third = countries(103, 1);
    let source = opened(vec![first, second, third]);
    let history =
        country_history::branch_histories(&source, 1, false, &[], &AtomicBool::new(false)).unwrap();
    assert_eq!(
        history.countries["country-00002"].segments.as_slice(),
        [HourSpan {
            first_hour: 100,
            last_hour: 100
        }]
    );
    assert_eq!(
        history.countries["country-00001"].segments.as_slice(),
        [
            HourSpan {
                first_hour: 100,
                last_hour: 100
            },
            HourSpan {
                first_hour: 103,
                last_hour: 103
            }
        ]
    );
    assert_eq!(
        history.countries["country-00001"].identities[0].slot,
        history.countries["renamed"].identities[0].slot
    );
    assert_eq!(
        history.countries["country-00001"].identities[0].country_id,
        history.countries["renamed"].identities[0].country_id
    );
    let reply = Scoreboard::load(
        &source,
        &request(
            vec![ScoreboardColumnKey::PoliticalPower],
            vec!["country-00001".into()],
            None,
        ),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply
            .series
            .iter()
            .map(|row| row.tag.as_str())
            .collect::<Vec<_>>(),
        ["country-00001", "renamed"]
    );
}

#[test]
fn scoreboard_validation_has_no_country_cap_and_refuses_duplicates() {
    assert!(Scoreboard::validate(&request(vec![], vec![], None)).is_ok());
    assert!(
        Scoreboard::validate(&request(
            vec![ScoreboardColumnKey::Factories],
            (0..5000).map(|i| i.to_string()).collect(),
            None
        ))
        .is_ok()
    );
    assert!(
        Scoreboard::validate(&request(
            vec![ScoreboardColumnKey::Factories; 2],
            vec![],
            None
        ))
        .is_err()
    );
    assert!(
        Scoreboard::validate(&request(
            vec![ScoreboardColumnKey::Factories],
            vec!["selected-country".into()],
            Some(ScoreboardTop {
                count: 10,
                sort: ScoreboardColumnKey::Factories,
                descending: true
            })
        ))
        .is_err()
    );
    assert!(
        Scoreboard::validate(&request(
            vec![ScoreboardColumnKey::Factories],
            vec![],
            Some(ScoreboardTop {
                count: 0,
                sort: ScoreboardColumnKey::Factories,
                descending: true
            })
        ))
        .is_err()
    );
}

#[test]
fn scoreboard_balance_cache_materializes_only_requested_fields_and_preserves_upgrades() {
    use super::super::country_measure::Measure;
    let source = opened(vec![countries(100, 3), countries(101, 3)]);
    let cancel = AtomicBool::new(false);
    Scoreboard::load(
        &source,
        &request(
            vec![ScoreboardColumnKey::Factories],
            vec!["country-00001".into()],
            None,
        ),
        &LoadContext { content: None },
        &cancel,
    )
    .unwrap();
    let identities = country_history::branch_histories(&source, 1, false, &[], &cancel).unwrap();
    assert!(
        identities
            .countries
            .values()
            .all(|country| country.political_power.runs.is_empty()
                && country.command_power.runs.is_empty())
    );
    let pp =
        country_history::branch_histories(&source, 1, false, &[Measure::PoliticalPower], &cancel)
            .unwrap();
    assert_eq!(
        pp.countries["country-00001"].political_power.runs[0].value,
        100001
    );
    assert!(
        pp.countries
            .values()
            .all(|country| country.command_power.runs.is_empty())
    );
    let cp =
        country_history::branch_histories(&source, 1, false, &[Measure::CommandPower], &cancel)
            .unwrap();
    for (tag, country) in &pp.countries {
        assert!(Arc::ptr_eq(
            &cp.countries[tag].political_power,
            &country.political_power
        ));
        assert!(Arc::ptr_eq(
            &cp.countries[tag].identities,
            &country.identities
        ));
    }
    assert_eq!(
        cp.countries["country-00001"].command_power.runs[0].value,
        100003
    );
    let again = country_history::branch_histories(
        &source,
        1,
        false,
        &[Measure::PoliticalPower, Measure::CommandPower],
        &cancel,
    )
    .unwrap();
    assert!(Arc::ptr_eq(&again, &cp));
}

#[test]
fn scoreboard_held_stability_read_at_maximum_hour_stays_disjoint() {
    let first_hour = i32::MAX - 1;
    let first = crate::test_fixtures::stability_frame(
        first_hour,
        crate::test_fixtures::stability_inputs(),
        first_hour,
        vec![],
    );
    let tag = std::str::from_utf8(&first.complete::<tables::Countries>().unwrap()[0].tag)
        .unwrap()
        .to_owned();
    let source = opened(vec![
        first,
        crate::test_fixtures::stability_frame(
            i32::MAX,
            crate::test_fixtures::stability_inputs(),
            first_hour,
            vec![],
        ),
    ]);
    let reply = Scoreboard::load(
        &source,
        &request(vec![ScoreboardColumnKey::Stability], vec![tag], None),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply.series[0].columns[0].read_runs,
        [
            ScoreboardReadRun {
                first_hour,
                last_hour: first_hour,
                read_hour: first_hour,
                hourly: true,
            },
            ScoreboardReadRun {
                first_hour: i32::MAX,
                last_hour: i32::MAX,
                read_hour: first_hour,
                hourly: false,
            },
        ]
    );
}

fn multi_measure_frame(hour: i32, read: i32, delivered: Option<i64>) -> DecodedFrame {
    use replay::rows::{
        LexicalTokenRow, NukeRow, ProductionLineOutputRow, ProductionLineRow,
        RocketProductionOutputRow, VariantDefinitionRow,
    };
    let mut out = crate::test_fixtures::stability_frame(
        hour,
        crate::test_fixtures::stability_inputs(),
        read,
        vec![],
    );
    *out.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(vec![NukeRow {
        country: 11,
        nuke_type: 0,
        amount_raw: 299_999_999,
        ready: 97,
    }]));
    *out.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 1,
            literal: b"synthetic_mod_equipment".to_vec(),
        }]));
    *out.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 10,
            equipment_token: 1,
            creator_link: 9001,
            ..Default::default()
        }]));
    *out.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(vec![ProductionLineRow {
            handle: 1000,
            country: 11,
            variant: 10,
            kind: 0,
            ..Default::default()
        }]));
    *out.tables[TableId::ProductionLineOutput.index()] = match delivered {
        Some(units) => TableView::Complete(TableRows::ProductionLineOutput(vec![
            ProductionLineOutputRow {
                line: 1000,
                country: 11,
                variant: 10,
                units,
                since_hour: 100,
                exact: true,
                observed_since_creation: true,
                ..Default::default()
            },
        ])),
        None => TableView::Unavailable {
            reason: "synthetic missing output".into(),
        },
    };
    let deliveries = if hour < 102 { 1 } else { 2 };
    let rocket = RocketProductionOutputRow {
        since_hour: 100,
        exact: true,
        units_exact: true,
        deliveries,
        ..Default::default()
    };
    *out.tables[TableId::RocketProductionOutput.index()] =
        TableView::Complete(TableRows::RocketProductionOutput(vec![
            rocket.clone(),
            RocketProductionOutputRow {
                country: 11,
                ..rocket.clone()
            },
            RocketProductionOutputRow {
                country: 11,
                variant: 10,
                units: if hour < 102 { 2 } else { 4 },
                ..rocket
            },
        ]));
    if hour == 100 || hour == 102 || delivered.is_none() {
        out.metrics.table_ns[TableId::RocketProductionOutput.index()] = 1;
        out.metrics.table_ns[TableId::ProductionLines.index()] = 1;
        out.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
    }
    out
}

#[test]
fn scoreboard_stability_war_support_nukes_and_equipment_match_public_products_and_read_dates() {
    use super::super::{
        country_nukes::{CountryNukes, NukesParams},
        country_stability::{CountryStability, CountryStabilityParams},
        equipment_produced::{
            EquipmentProduced, ProducedGrouping, ProducedMeasure, ProducedParams,
        },
    };
    let source = opened(vec![
        multi_measure_frame(100, 100, Some(3)),
        multi_measure_frame(101, 100, Some(3)),
        multi_measure_frame(102, 102, Some(5)),
        multi_measure_frame(103, 102, None),
    ]);
    let context = LoadContext { content: None };
    let cancel = AtomicBool::new(false);
    let reply = Scoreboard::load(
        &source,
        &request(
            vec![
                ScoreboardColumnKey::Stability,
                ScoreboardColumnKey::WarSupport,
                ScoreboardColumnKey::Nukes,
                ScoreboardColumnKey::EquipmentProduced,
            ],
            vec!["GER".into()],
            None,
        ),
        &context,
        &cancel,
    )
    .unwrap();
    let row = &reply.series[0];
    let column = |key| row.columns.iter().find(|column| column.key == key).unwrap();
    let stability = CountryStability::load(
        &source,
        &CountryStabilityParams {
            branch_id: "1".into(),
            tags: vec!["GER".into()],
            hour: None,
            breakdown_only: false,
        },
        &context,
        &cancel,
    )
    .unwrap();
    assert_eq!(
        column(ScoreboardColumnKey::Stability).runs,
        fixed_runs(&stability.series[0].stability)
    );
    assert_eq!(
        column(ScoreboardColumnKey::WarSupport).runs,
        fixed_runs(&stability.series[0].war_support)
    );
    for key in [
        ScoreboardColumnKey::Stability,
        ScoreboardColumnKey::WarSupport,
    ] {
        let read = column(key)
            .read_runs
            .iter()
            .find(|run| run.first_hour <= 101 && run.last_hour >= 101)
            .unwrap();
        assert_eq!((read.hourly, read.read_hour), (false, 100));
        let read = column(key)
            .read_runs
            .iter()
            .find(|run| run.first_hour <= 102 && run.last_hour >= 102)
            .unwrap();
        assert!(read.hourly);
    }
    let nukes = CountryNukes::load(
        &source,
        &NukesParams {
            branch_id: "1".into(),
            tags: vec!["GER".into()],
            top: None,
            origin_hour: None,
        },
        &context,
        &cancel,
    )
    .unwrap();
    assert_eq!(
        column(ScoreboardColumnKey::Nukes)
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.amount.clone()))
            .collect::<Vec<_>>(),
        nukes.series[0]
            .runs
            .iter()
            .map(|run| (
                run.first_hour,
                run.last_hour,
                run.total.map(|value| value.to_string())
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        column(ScoreboardColumnKey::Nukes).runs[0].amount.as_deref(),
        Some("2")
    );
    assert!(
        column(ScoreboardColumnKey::Nukes)
            .read_runs
            .iter()
            .all(|run| run.hourly)
    );
    let produced = EquipmentProduced::load(
        &source,
        &ProducedParams {
            branch_id: "1".into(),
            tags: vec!["GER".into()],
            top: None,
            measure: ProducedMeasure::Total,
            count: crate::products::equipment_produced::ProducedCount::Units,
            grouping: ProducedGrouping::Type,
            included: vec![],
            refits: false,
        },
        &context,
        &cancel,
    )
    .unwrap();
    let unread_warning = produced
        .warnings
        .iter()
        .find(|warning| warning.contains("reads of the production lines have no readable output"))
        .expect("the missed publication has a counted reason");
    assert!(reply.warnings.contains(unread_warning));
    let series = &produced.series[0];
    let decode = |index: usize| {
        let Some(total) = &series.total else {
            return Some(0);
        };
        if total
            .unknown
            .chunks_exact(2)
            .any(|range| range[0] as usize <= index && index < range[1] as usize)
        {
            return None;
        }
        if index < total.from as usize {
            return Some(0);
        }
        Some(
            total
                .deltas
                .iter()
                .take(index - total.from as usize + 1)
                .sum::<i64>(),
        )
    };
    for (sample, first) in series.first_hours.iter().enumerate() {
        let actual = column(ScoreboardColumnKey::EquipmentProduced)
            .runs
            .iter()
            .find(|run| run.first_hour <= *first && run.last_hour >= *first)
            .unwrap();
        assert_eq!(
            actual
                .amount
                .as_ref()
                .and_then(|raw| raw.parse::<i64>().ok()),
            decode(sample)
        );
    }
    for (hour, amount) in [(100, "5"), (102, "9")] {
        let run = column(ScoreboardColumnKey::EquipmentProduced)
            .runs
            .iter()
            .find(|run| run.first_hour <= hour && run.last_hour >= hour)
            .unwrap();
        assert_eq!(run.amount.as_deref(), Some(amount));
    }
    let carried = column(ScoreboardColumnKey::EquipmentProduced)
        .read_runs
        .iter()
        .find(|run| run.first_hour <= 101 && run.last_hour >= 101)
        .unwrap();
    assert_eq!((carried.hourly, carried.read_hour), (false, 100));
    // A missed publication never restarts an exact total, even with later rows.
    assert!(
        column(ScoreboardColumnKey::EquipmentProduced)
            .runs
            .iter()
            .filter(|run| run.first_hour >= 103)
            .all(|run| run.amount.is_none())
    );
}

fn fixed_runs(runs: &[FixedRun]) -> Vec<ScoreboardRun> {
    runs.iter()
        .map(|run| ScoreboardRun {
            first_hour: run.first_hour,
            last_hour: run.last_hour,
            amount: Some(run.value_raw.clone()),
            starts_segment: run.starts_segment,
            at_least: false,
        })
        .collect()
}

#[test]
fn scoreboard_top_accepts_five_thousand_and_only_reply_size_refuses() {
    assert!(
        Scoreboard::validate(&request(
            vec![ScoreboardColumnKey::PoliticalPower],
            vec![],
            Some(ScoreboardTop {
                count: 5000,
                sort: ScoreboardColumnKey::PoliticalPower,
                descending: true
            })
        ))
        .is_ok()
    );
    let source = opened(vec![countries(100, 5000)]);
    let reply = Scoreboard::load(
        &source,
        &request(
            vec![ScoreboardColumnKey::PoliticalPower],
            vec![],
            Some(ScoreboardTop {
                count: 5000,
                sort: ScoreboardColumnKey::PoliticalPower,
                descending: true,
            }),
        ),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.series.len(), 5000);
    assert!(
        reply.series.iter().all(|row| row
            .membership
            .as_ref()
            .is_some_and(|spans| spans.len() == 1
                && spans[0].first_hour == 100
                && spans[0].last_hour == 100))
    );
    let mut large = reply;
    large.warnings.push("x".repeat(MAX_PRODUCT_REPLY_BYTES));
    assert!(
        checked_reply(large)
            .unwrap_err()
            .to_string()
            .contains("MAX_PRODUCT_REPLY_BYTES")
    );
}

#[test]
fn scoreboard_stability_read_merge_preserves_many_runs_and_held_intersections() {
    let runs: Vec<_> = (0..4)
        .map(|i| ValueRun {
            first_hour: i * 2,
            last_hour: i * 2,
            amount: Some(i128::from(i)),
            starts_segment: true,
            at_least: false,
        })
        .collect();
    let carried: Vec<_> = (0..2)
        .map(|i| country_stability::ReadRun {
            first_hour: i * 4,
            last_hour: i * 4 + 2,
            read_hour: i * 4 - 1,
        })
        .collect();
    let actual = stability_reads(&runs, &carried);
    assert_eq!(actual.len(), runs.len());
    for (run, read) in runs.iter().zip(&actual) {
        assert_eq!(
            (read.first_hour, read.last_hour),
            (run.first_hour, run.last_hour)
        );
        assert!(!read.hourly);
        assert_eq!(read.read_hour, run.first_hour / 4 * 4 - 1);
    }
    // Ranking calculates only values. Supplying dates are constructed after membership.
    let source = opened(vec![crate::test_fixtures::stability_frame(
        100,
        crate::test_fixtures::stability_inputs(),
        100,
        vec![],
    )]);
    let cancel = AtomicBool::new(false);
    let history = country_history::branch_histories(&source, 1, false, &[], &cancel).unwrap();
    let tags: Vec<_> = history.countries.keys().cloned().collect();
    let mut values = values(
        &source,
        1,
        ScoreboardColumnKey::Stability,
        &tags,
        &history,
        &cancel,
        &mut vec![],
    )
    .unwrap();
    assert!(values.values().all(|column| column.read_runs.is_empty()));
    fill_read_runs(
        &source,
        1,
        ScoreboardColumnKey::Stability,
        &tags,
        &mut values,
        &cancel,
    )
    .unwrap();
    assert!(values.values().all(|column| !column.read_runs.is_empty()));
}

#[test]
fn scoreboard_many_hour_stability_and_political_top_five_thousand_countries() {
    use replay::rows::{CountryStabilityInputRow, SliceReadRow};
    // One held-read stretch and the next supplying read; every country remains present.
    let frames = (0..5)
        .map(|offset| {
            let hour = 100 + offset;
            let read_offset = offset / 4 * 4;
            let mut out = countries(hour, 5000);
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *out.tables[TableId::Countries.index()]
            {
                for row in rows {
                    row.political_power_raw =
                        Some(i64::from((row.id + offset as u32) % 5000) * 100_000);
                }
            }
            *out.tables[TableId::CountryFactories.index()] =
                TableView::Complete(TableRows::CountryFactories(vec![]));
            *out.tables[TableId::CountryCounterInputs.index()] =
                TableView::Complete(TableRows::CountryCounterInputs(vec![]));
            *out.tables[TableId::CountryStabilityInputs.index()] =
                TableView::Complete(TableRows::CountryStabilityInputs(
                    (1..=5000)
                        .map(|id| CountryStabilityInputRow {
                            country: id,
                            stability_raw: i64::from((id + read_offset as u32) % 5000) * 10,
                            ruling_party_popularity_raw: 0,
                            propaganda_stability_q15: 0,
                            coastal_protection_stability_raw: 0,
                            stability_factor: Some(0),
                            party_popularity_stability_factor: Some(0),
                            ..crate::test_fixtures::stability_inputs()
                        })
                        .collect(),
                ));
            *out.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(
                (1..=5000)
                    .map(|owner| SliceReadRow {
                        table: TableId::CountryStabilityInputs.index() as u32,
                        owner,
                        sample_hour: 100 + read_offset,
                    })
                    .collect(),
            ));
            out
        })
        .collect();
    let source = opened(frames);
    let cancel = AtomicBool::new(false);
    for key in [
        ScoreboardColumnKey::PoliticalPower,
        ScoreboardColumnKey::Stability,
    ] {
        let reply = Scoreboard::load(
            &source,
            &request(
                vec![key],
                vec![],
                Some(ScoreboardTop {
                    count: 50,
                    sort: key,
                    descending: true,
                }),
            ),
            &LoadContext { content: None },
            &cancel,
        )
        .unwrap();
        for offset in 0..5 {
            let hour = 100 + offset;
            let shifted = if key == ScoreboardColumnKey::Stability {
                offset / 4 * 4
            } else {
                offset
            };
            let scale = if key == ScoreboardColumnKey::Stability {
                10
            } else {
                100_000
            };
            let mut expected: Vec<_> = (1..=5000)
                .map(|id| {
                    (
                        i64::from((id + shifted as u32) % 5000) * scale,
                        format!("country-{id:05}"),
                    )
                })
                .collect();
            expected.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            expected.truncate(50);
            let mut actual: Vec<_> = reply
                .series
                .iter()
                .filter(|row| {
                    row.membership
                        .as_ref()
                        .unwrap()
                        .iter()
                        .any(|span| span.first_hour <= hour && span.last_hour >= hour)
                })
                .map(|row| {
                    let column = &row.columns[0];
                    let run = column
                        .runs
                        .iter()
                        .find(|run| run.first_hour <= hour && run.last_hour >= hour)
                        .unwrap();
                    let read = column
                        .read_runs
                        .iter()
                        .find(|read| read.first_hour <= hour && read.last_hour >= hour)
                        .unwrap();
                    assert_eq!(
                        if read.hourly { hour } else { read.read_hour },
                        100 + shifted
                    );
                    (
                        run.amount.as_ref().unwrap().parse::<i64>().unwrap(),
                        row.tag.clone(),
                    )
                })
                .collect();
            actual.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            assert_eq!(actual, expected, "{key:?} at {hour}");
        }
        // A small single-stat request retains the large shared Stability cache.
        if key == ScoreboardColumnKey::Stability {
            let before =
                country_stability::histories(&source, 1, &["country-04999".into()], &cancel)
                    .unwrap();
            let again = country_stability::histories(
                &source,
                1,
                &["country-04999".into()],
                &AtomicBool::new(true),
            )
            .unwrap();
            assert!(Arc::ptr_eq(
                &before["country-04999"],
                &again["country-04999"]
            ));
        }
    }
}

#[path = "prepared_tests.rs"]
mod prepared_tests;
