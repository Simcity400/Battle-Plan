use super::test_fixtures::frame;
use super::*;
use dashboard_base::{
    source::SourceKey,
    test_fixtures::{CAMPAIGN, records},
};
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::DivisionCensusRow, rows::DivisionTemplateIconRow,
    rows::LexicalTokenRow, rows::TableRows, rows::TemplateSubunit,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};

pub(super) fn opened(frames: Vec<DecodedFrame>) -> Source {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    Source::from_replay(
        SourceKey {
            source_id: "army".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn params(measure: ArmyMeasure, count_for: ArmyAttribution) -> ArmySizeParams {
    ArmySizeParams {
        template_at: None,
        branch_id: "1".into(),
        tags: vec!["origin".into(), "host".into(), "empty".into()],
        top: None,
        measure,
        count_for,
        by_type: false,
        window: Some(0),
    }
}

#[test]
fn army_size_manual_selection_excludes_unrelated_alias_holders() {
    let first = frame(100);
    let mut renamed = frame(101);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"later alias".to_vec();
    }
    let mut reused = frame(102);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *reused.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"current alias".to_vec();
        rows[1].tag = b"later alias".to_vec();
    }
    let source = opened(vec![first, renamed, reused]);
    for (tag, last) in [("origin", 101), ("later alias", 102)] {
        let mut request = params(ArmyMeasure::Divisions, ArmyAttribution::Owner);
        request.tags = vec![tag.into()];
        let reply = ArmySize::load(
            &source,
            &request,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        let alias = reply
            .series
            .iter()
            .find(|series| series.tag == "later alias")
            .unwrap();
        assert_eq!(alias.runs.first().unwrap().first_hour, 101);
        assert_eq!(alias.runs.last().unwrap().last_hour, last);
        assert_eq!(alias.segments.last().unwrap().last_hour, last);
        assert_eq!(
            alias.identities.as_ref().unwrap().last().unwrap().last_hour,
            last
        );
        let current = reply
            .series
            .iter()
            .find(|series| series.tag == "current alias")
            .unwrap();
        assert_eq!(current.runs[0].raw, "3");
    }
}

#[test]
fn army_size_large_selections_and_branch_cache_keep_every_recorded_country() {
    let mut observation = frame(100);
    let tags: Vec<_> = (1..=300).map(|id| format!("recorded {id}")).collect();
    *observation.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        tags.iter()
            .enumerate()
            .map(|(index, tag)| CountryRow {
                id: index as u32 + 1,
                canonical_slot: index as u32 + 1,
                tag: tag.as_bytes().to_vec(),
                ..Default::default()
            })
            .collect(),
    ));
    *observation.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(
            (1..=300)
                .map(|id| CountryLinkRow {
                    id,
                    canonical_slot: id,
                })
                .collect(),
        ));
    *observation.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(
            (1..=300)
                .map(|country| DivisionCensusRow {
                    country,
                    grouped: country,
                    ungrouped: 1,
                })
                .collect(),
        ));
    let branches: Vec<_> = (1..=4)
        .map(|branch| {
            let mut observation = observation.clone();
            observation.branch = branch;
            records(branch, vec![observation])
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &branches).unwrap();
    let source = Source::from_replay(
        SourceKey {
            source_id: "many armies".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let mut request = params(ArmyMeasure::Divisions, ArmyAttribution::Owner);
    request.tags = tags;
    ArmySize::validate(&request).unwrap();
    let reply = ArmySize::load(&source, &request, &LoadContext { content: None }, &cancel).unwrap();
    assert_eq!(reply.series.len(), 300);
    request.tags.clear();
    request.top = Some(300);
    ArmySize::validate(&request).unwrap();
    let ranked =
        ArmySize::load(&source, &request, &LoadContext { content: None }, &cancel).unwrap();
    assert_eq!(ranked.series.len(), 300);
    for branch in [2, 3, 4, 1] {
        assert_eq!(
            army_value_at(
                &source,
                branch,
                "recorded 300",
                100,
                ArmyMeasure::Divisions,
                ArmyAttribution::Owner,
                &cancel
            )
            .unwrap()
            .unwrap()
            .value,
            301.0
        );
    }
    assert_eq!(source.state::<Scans>().lock().unwrap().runs, 5);
    assert_eq!(source.state::<Scans>().lock().unwrap().entries.len(), 2);
    assert!(
        army_value_at(
            &source,
            999,
            "recorded 300",
            100,
            ArmyMeasure::Divisions,
            ArmyAttribution::Owner,
            &cancel
        )
        .unwrap_err()
        .to_string()
        .contains("Unknown branch")
    );
    assert_eq!(source.state::<Scans>().lock().unwrap().runs, 5);
}

#[test]
fn army_size_scoreboard_agrees_with_product_for_every_measure_and_attribution() {
    use replay_data::{rows::DivisionLandInputsRow, rows::DivisionManpowerContributor};
    let source = opened(
        [100, 101, 103]
            .into_iter()
            .map(|hour| {
                let mut frame = frame(hour);
                *frame.tables[TableId::DivisionLandInputs.index()] =
                    TableView::Complete(TableRows::DivisionLandInputs(vec![
                        DivisionLandInputsRow {
                            division: 1,
                            manpower: vec![DivisionManpowerContributor {
                                country: 20,
                                amount: 1000,
                            }],
                            ..Default::default()
                        },
                        DivisionLandInputsRow {
                            division: 2,
                            manpower: vec![DivisionManpowerContributor {
                                country: 30,
                                amount: 750,
                            }],
                            ..Default::default()
                        },
                    ]));
                frame
            })
            .collect(),
    );
    let cancel = AtomicBool::new(false);
    for measure in [
        ArmyMeasure::Divisions,
        ArmyMeasure::Strength,
        ArmyMeasure::Battalions,
        ArmyMeasure::Manpower,
    ] {
        for attribution in [ArmyAttribution::Owner, ArmyAttribution::Host] {
            let reply = ArmySize::load(
                &source,
                &params(measure, attribution),
                &LoadContext { content: None },
                &cancel,
            )
            .unwrap();
            for series in &reply.series {
                for hour in [100, 101, 102, 103] {
                    let plotted = series
                        .runs
                        .iter()
                        .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                        .and_then(|run| run.total);
                    let shared =
                        army_value_at(&source, 1, &series.tag, hour, measure, attribution, &cancel)
                            .unwrap();
                    assert_eq!(shared.map(|value| value.value), plotted);
                    assert!(shared.is_none_or(|value| value.read_hour == hour));
                }
            }
            let origin = &reply
                .series
                .iter()
                .find(|series| series.tag == "origin")
                .unwrap()
                .runs;
            assert_eq!(origin.len(), 2);
            assert_eq!(origin[1].first_hour, 103);
            assert_eq!(
                origin[0].total,
                Some(match (measure, attribution) {
                    (ArmyMeasure::Manpower, ArmyAttribution::Owner) => 1750.0,
                    (ArmyMeasure::Manpower, ArmyAttribution::Host) => 750.0,
                    (ArmyMeasure::Divisions, ArmyAttribution::Owner) => 3.0,
                    (ArmyMeasure::Divisions, ArmyAttribution::Host) => 1.0,
                    (ArmyMeasure::Strength, ArmyAttribution::Owner) => 32.3,
                    (ArmyMeasure::Strength, ArmyAttribution::Host) => 20.0,
                    (ArmyMeasure::Battalions, ArmyAttribution::Owner) => 12.0,
                    (ArmyMeasure::Battalions, ArmyAttribution::Host) => 6.0,
                })
            );
        }
    }
    // The cheap owner count scan is upgraded once when details are requested.
    assert_eq!(source.state::<Scans>().lock().unwrap().runs, 2);
}

#[test]
fn army_size_absent_country_identity_does_not_hide_other_known_totals() {
    let mut absent_host = frame(101);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *absent_host.tables[TableId::Countries.index()]
    {
        rows.retain(|row| row.id != 20);
    }
    let source = opened(vec![frame(100), absent_host]);
    let cancel = AtomicBool::new(false);
    let origin = army_value_at(
        &source,
        1,
        "origin",
        101,
        ArmyMeasure::Divisions,
        ArmyAttribution::Host,
        &cancel,
    )
    .unwrap()
    .unwrap();
    assert_eq!(origin.value, 1.0);
    assert_eq!(
        army_value_at(
            &source,
            1,
            "host",
            101,
            ArmyMeasure::Divisions,
            ArmyAttribution::Host,
            &cancel
        )
        .unwrap(),
        None
    );
    let reply = ArmySize::load(
        &source,
        &params(ArmyMeasure::Divisions, ArmyAttribution::Host),
        &LoadContext { content: None },
        &cancel,
    )
    .unwrap();
    assert_eq!(
        reply
            .series
            .iter()
            .find(|series| series.tag == "host")
            .unwrap()
            .runs
            .last()
            .unwrap()
            .last_hour,
        100
    );
    assert_eq!(
        reply
            .series
            .iter()
            .find(|series| series.tag == "origin")
            .unwrap()
            .runs
            .last()
            .unwrap()
            .last_hour,
        101
    );
}

#[test]
fn army_size_inventory_only_hours_agree_before_and_after_detail_requests() {
    let mut inventory_only = frame(101);
    *inventory_only.tables[TableId::DivisionCensus.index()] = TableView::Unavailable {
        reason: "census read unavailable".into(),
    };
    let source = opened(vec![frame(100), inventory_only, frame(102)]);
    let cancel = AtomicBool::new(false);
    let request = params(ArmyMeasure::Divisions, ArmyAttribution::Owner);
    let reply = ArmySize::load(&source, &request, &LoadContext { content: None }, &cancel).unwrap();
    let plotted = reply
        .series
        .iter()
        .find(|series| series.tag == "origin")
        .unwrap()
        .runs
        .iter()
        .find(|run| run.first_hour <= 101 && 101 <= run.last_hour)
        .unwrap()
        .total;
    assert_eq!(plotted, Some(2.0));
    assert!(
        army_value_at(
            &source,
            1,
            "origin",
            101,
            ArmyMeasure::Strength,
            ArmyAttribution::Owner,
            &cancel
        )
        .unwrap()
        .is_some()
    );
    let shared = army_value_at(
        &source,
        1,
        "origin",
        101,
        ArmyMeasure::Divisions,
        ArmyAttribution::Owner,
        &cancel,
    )
    .unwrap()
    .unwrap();
    assert_eq!(Some(shared.value), plotted);
    assert_eq!(shared.read_hour, 101);
    assert_eq!(source.state::<Scans>().lock().unwrap().runs, 2);
}

#[test]
fn army_size_census_aliases_add_missing_rows_are_unknown_and_census_avoids_detail_decode() {
    let mut missing = frame(101);
    if let TableView::Complete(TableRows::DivisionCensus(rows)) =
        &mut *missing.tables[TableId::DivisionCensus.index()]
    {
        rows.retain(|row| row.country != 30);
    }
    let mut unknown = frame(102);
    if let TableView::Complete(TableRows::DivisionCensus(rows)) =
        &mut *unknown.tables[TableId::DivisionCensus.index()]
    {
        rows.push(DivisionCensusRow {
            country: 999,
            grouped: 1,
            ungrouped: 0,
        });
    }
    let source = opened(vec![frame(100), missing, unknown]);
    let cancel = AtomicBool::new(false);
    assert_eq!(
        army_value_at(
            &source,
            1,
            "origin",
            100,
            ArmyMeasure::Divisions,
            ArmyAttribution::Owner,
            &cancel
        )
        .unwrap()
        .unwrap()
        .value,
        3.0
    );
    assert_eq!(
        army_value_at(
            &source,
            1,
            "empty",
            100,
            ArmyMeasure::Divisions,
            ArmyAttribution::Owner,
            &cancel
        )
        .unwrap()
        .unwrap()
        .value,
        0.0
    );
    assert_eq!(
        army_value_at(
            &source,
            1,
            "empty",
            101,
            ArmyMeasure::Divisions,
            ArmyAttribution::Owner,
            &cancel
        )
        .unwrap(),
        None
    );
    assert_eq!(
        army_value_at(
            &source,
            1,
            "origin",
            102,
            ArmyMeasure::Divisions,
            ArmyAttribution::Owner,
            &cancel
        )
        .unwrap(),
        None
    );
    let scan = scanned(&source, 1, false, &cancel).unwrap();
    assert!(scan.strength); // advertised from wire status without reconstruction
    assert_eq!(scan.tracks["origin"].runs[0].totals.values[1], None);
}

#[test]
fn army_size_invalid_hit_points_unresolved_owners_and_identity_changes_never_guess() {
    let mut bad_hp = frame(101);
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *bad_hp.tables[TableId::Divisions.index()]
    {
        rows[0].hit_points = i64::MAX;
    }
    let mut owner_unknown = frame(102);
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *owner_unknown.tables[TableId::Divisions.index()]
    {
        rows[0].roster_owner = 999;
        rows[0].expeditionary_owner = 999;
    }
    *owner_unknown.tables[TableId::DivisionCensus.index()] = TableView::Unavailable {
        reason: "unavailable".into(),
    };
    let mut replaced = frame(103);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *replaced.tables[TableId::Countries.index()]
    {
        rows[0].id = 11;
    }
    let source = opened(vec![frame(100), bad_hp, owner_unknown, replaced]);
    let cancel = AtomicBool::new(false);
    assert_eq!(
        army_value_at(
            &source,
            1,
            "origin",
            101,
            ArmyMeasure::Strength,
            ArmyAttribution::Owner,
            &cancel
        )
        .unwrap(),
        None
    );
    assert_eq!(
        army_value_at(
            &source,
            1,
            "host",
            102,
            ArmyMeasure::Strength,
            ArmyAttribution::Host,
            &cancel
        )
        .unwrap(),
        None
    );
    let scan = scanned(&source, 1, true, &cancel).unwrap();
    assert_eq!(scan.tracks["origin"].segments.len(), 2);
    assert!(scan.tracks["origin"].runs.last().unwrap().starts);
}

#[test]
fn army_size_top_ranks_known_zero_and_omits_unknown_and_absent_countries() {
    let source = opened(vec![frame(100)]);
    let mut request = params(ArmyMeasure::Divisions, ArmyAttribution::Owner);
    request.tags.clear();
    request.top = Some(2);
    let reply = ArmySize::load(
        &source,
        &request,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply
            .series
            .iter()
            .map(|series| series.tag.as_str())
            .collect::<Vec<_>>(),
        ["empty", "origin"]
    );
    assert!(
        reply
            .series
            .iter()
            .all(|series| series.membership.as_ref().unwrap()[0].first_hour == 100)
    );
    for series in &reply.series {
        assert_eq!(
            series.runs[0].total,
            army_value_at(
                &source,
                1,
                &series.tag,
                100,
                ArmyMeasure::Divisions,
                ArmyAttribution::Owner,
                &AtomicBool::new(false)
            )
            .unwrap()
            .map(|value| value.value)
        );
    }
}

#[test]
fn army_size_recording_availability_and_cancel_are_explicit() {
    let mut absent = frame(100);
    for table in [
        TableId::DivisionCensus,
        TableId::Divisions,
        TableId::DivisionTemplateIcons,
    ] {
        *absent.tables[table.index()] = TableView::Unavailable {
            reason: "not recorded".into(),
        };
    }
    let source = opened(vec![absent]);
    let reply = ArmySize::load(
        &source,
        &params(ArmyMeasure::Divisions, ArmyAttribution::Owner),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!reply.available);
    assert!(reply.measures.is_empty());
    let mut census_only = frame(100);
    *census_only.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    };
    let census_source = opened(vec![census_only]);
    for attribution in [ArmyAttribution::Owner, ArmyAttribution::Host] {
        let reply = ArmySize::load(
            &census_source,
            &params(ArmyMeasure::Divisions, attribution),
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(reply.available, attribution == ArmyAttribution::Owner);
        assert_eq!(reply.measures, [ArmyMeasure::Divisions]);
    }
    assert_eq!(
        scan(source.replay_ref(), 1, true, &AtomicBool::new(true))
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::Interrupted
    );
}

/// Before preparation reaches the hours recording a measure, an hour the
/// manifest samples without it leaves it unknown, not absent, and an exact
/// window read finds its recorded values.
#[test]
fn army_manifest_keeps_a_measure_its_sampled_hour_lacks_unknown_while_preparing() {
    use crate::products::parts_race_prepared::test_fixtures::{frame as forces, tag};
    use dashboard_base::test_fixtures::continuation_records;
    let mut sampled = forces(2, 2);
    *sampled.tables[TableId::DivisionLandInputs.index()] = TableView::Unavailable {
        reason: "not read at this hour".into(),
    };
    let first =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![forces(0, 2), forces(1, 2)])])
            .unwrap();
    let replay = first
        .append(&[continuation_records(&first, 1, vec![sampled, forces(3, 2)])])
        .unwrap();
    let (source, root) = dashboard_base::test_fixtures::file_source(replay);
    let context = LoadContext { content: None };
    let cancel = AtomicBool::new(false);
    let mut request = ArmySizeParams {
        template_at: None,
        branch_id: "1".into(),
        tags: vec![tag(0)],
        top: None,
        measure: ArmyMeasure::Divisions,
        count_for: ArmyAttribution::Owner,
        by_type: false,
        window: None,
    };
    let manifest = prepared::query(&source, &request, &context, &cancel)
        .unwrap()
        .unwrap();
    assert!(manifest.measures.contains(&ArmyMeasure::Divisions));
    assert!(
        !manifest.measures.contains(&ArmyMeasure::Manpower),
        "the sampled hour does not read it"
    );
    assert!(
        manifest.unknown_measures.contains(&ArmyMeasure::Manpower),
        "nor does it prove the recording lacks it"
    );
    request.measure = ArmyMeasure::Manpower;
    request.window = Some(0);
    let window = prepared::query(&source, &request, &context, &cancel)
        .unwrap()
        .unwrap();
    assert!(window.measures.contains(&ArmyMeasure::Manpower));
    assert!(window.series.iter().any(|series| !series.runs.is_empty()));
    let whole = ArmySize::load(&source, &request, &context, &cancel).unwrap();
    assert!(whole.measures.contains(&ArmyMeasure::Manpower));
    assert!(
        whole.unknown_measures.is_empty(),
        "a whole read leaves nothing unknown"
    );
    let (store, _) = dashboard_base::prepared::files(&source, None).unwrap();
    let _ = store.clear();
    drop(source);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
#[ignore = "large transport fixture; filtered explicit run only"]
fn army_size_refuses_reply_budget() {
    let runs = (0..500_000)
        .map(|hour| RawRun {
            first: hour,
            last: hour,
            starts: hour == 0,
            totals: Totals {
                values: [Some(i128::from(hour)); 8],
            },
        })
        .collect();
    let mut scan = Scan {
        tracks: BTreeMap::from([(
            "origin".into(),
            Track {
                runs,
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let request = params(ArmyMeasure::Divisions, ArmyAttribution::Owner);
    let cancel = AtomicBool::new(false);
    // Compact totals are counted exactly at the transport boundary.
    let allowed = reply_series(
        &scan,
        &request,
        &HourSpan {
            first_hour: 0,
            last_hour: 1_999_999,
        },
        None,
        &cancel,
        None,
    )
    .unwrap()
    .0;
    assert_eq!(allowed[0].runs.len(), 500_000);
    assert!(
        dashboard_core::transport::json_len(
            &allowed,
            dashboard_base::products::MAX_PRODUCT_REPLY_BYTES
        )
        .is_some()
    );
    scan.tracks
        .get_mut("origin")
        .unwrap()
        .runs
        .extend((500_000..2_000_000).map(|hour| RawRun {
            first: hour,
            last: hour,
            starts: false,
            totals: Totals {
                values: [Some(i128::from(hour)); 8],
            },
        }));
    let error = reply_series(
        &scan,
        &request,
        &HourSpan {
            first_hour: 0,
            last_hour: 1_999_999,
        },
        None,
        &cancel,
        None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("transport reply budget"));
}

#[test]
fn army_size_inventory_without_census_is_counted() {
    let mut inventory_only = frame(100);
    *inventory_only.tables[TableId::DivisionCensus.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    };
    let source = opened(vec![inventory_only]);
    let value = army_value_at(
        &source,
        1,
        "origin",
        100,
        ArmyMeasure::Divisions,
        ArmyAttribution::Owner,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    assert_eq!(value.value, 2.0);
}

#[test]
fn army_size_battalions_include_support_and_keep_unknown_groups() {
    let source = opened(vec![frame(100), frame(101), frame(103)]);
    let mut request = params(ArmyMeasure::Battalions, ArmyAttribution::Owner);
    request.by_type = true;
    let reply = ArmySize::load(
        &source,
        &request,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(reply.available);
    assert_eq!(reply.types.len(), 3);
    assert!(
        reply
            .types
            .iter()
            .all(|kind| kind.name == "Unit group name unavailable")
    );
    let series = reply
        .series
        .iter()
        .find(|series| series.tag == "origin")
        .unwrap();
    assert_eq!(series.mix.as_ref().unwrap().len(), 2);
    assert_eq!(
        series.mix.as_ref().unwrap()[0]
            .values
            .iter()
            .flatten()
            .sum::<f64>(),
        12.0
    );
    assert_eq!(series.runs[0].total, Some(12.0));
    let json = serde_json::to_value(&reply).unwrap();
    assert_eq!(json["series"][2]["runs"][0].as_array().unwrap().len(), 4);
    assert_eq!(
        json["series"][2]["runs"][0][3],
        serde_json::to_value(&reply.series[2].runs[0].raw).unwrap()
    );
    request.measure = ArmyMeasure::Strength;
    let hp = ArmySize::load(
        &source,
        &request,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(hp.types.is_empty());
    assert!(hp.series.iter().all(|series| series.mix.is_none()));
}

#[test]
fn army_size_missing_or_invalid_composition_ends_only_battalion_values() {
    let mut missing = frame(101);
    *missing.tables[TableId::DivisionTemplateIcons.index()] = TableView::Unavailable {
        reason: "template read missing".into(),
    };
    let mut invalid = frame(102);
    if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
        &mut *invalid.tables[TableId::DivisionTemplateIcons.index()]
    {
        rows[0].subunits[0].count = -1;
    }
    let source = opened(vec![frame(100), missing, invalid, frame(103)]);
    let cancel = AtomicBool::new(false);
    for hour in [101, 102] {
        assert_eq!(
            army_value_at(
                &source,
                1,
                "origin",
                hour,
                ArmyMeasure::Battalions,
                ArmyAttribution::Owner,
                &cancel
            )
            .unwrap(),
            None
        );
        assert_eq!(
            army_value_at(
                &source,
                1,
                "origin",
                hour,
                ArmyMeasure::Divisions,
                ArmyAttribution::Owner,
                &cancel
            )
            .unwrap()
            .unwrap()
            .value,
            3.0
        );
    }
    assert_eq!(
        army_value_at(
            &source,
            1,
            "origin",
            103,
            ArmyMeasure::Battalions,
            ArmyAttribution::Owner,
            &cancel
        )
        .unwrap()
        .unwrap()
        .value,
        12.0
    );
}

#[test]
fn army_size_compact_wire_contract_has_optional_composition() {
    let run = ArmyRun {
        first_hour: 7,
        last_hour: 7,
        total: Some(1.23456),
        raw: "123456".into(),
    };
    assert_eq!(
        serde_json::to_value(&run).unwrap(),
        serde_json::json!([7, 1.23456, null, "123456"])
    );
    let mut series = ArmySeries {
        tag: "opaque".into(),
        runs: vec![run],
        identities: None,
        segments: vec![HourSpan {
            first_hour: 7,
            last_hour: 7,
        }],
        membership: None,
        mix: None,
    };
    assert!(serde_json::to_value(&series).unwrap().get("mix").is_none());
    series.mix = Some(vec![ArmyMixRun {
        first_hour: 7,
        last_hour: 7,
        values: vec![Some(1.0)],
        raw_values: Some(vec![Some("1".into())]),
    }]);
    assert_eq!(
        serde_json::to_value(&series).unwrap()["mix"],
        serde_json::json!([
            { "firstHour": 7, "lastHour": 7, "values": [1.0], "rawValues": ["1"] }
        ])
    );
}

#[test]
fn army_size_incremental_hp_and_template_edits_do_only_changed_row_work() {
    use replay_frame::frame::FrameEncoder;
    let mut hp = frame(101);
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *hp.tables[TableId::Divisions.index()]
    {
        rows[0].hit_points = 3_000_000;
    }
    let mut template = hp.clone();
    template.hour = 102;
    if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
        &mut *template.tables[TableId::DivisionTemplateIcons.index()]
    {
        rows[0].subunits[0].count = 5;
    }
    let mut unrelated_name = template.clone();
    unrelated_name.hour = 103;
    if let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *unrelated_name.tables[TableId::LexicalTokens.index()]
    {
        rows.push(LexicalTokenRow {
            token: 99,
            literal: b"unrelated_equipment".to_vec(),
        });
    }
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&scan_tables(true));
    let mut state = ScanState::new(true);
    for observation in [frame(100), hp, template, unrelated_name] {
        let force = observation.hour == 103;
        let wire = encoder.encode(observation, force).unwrap();
        let decoded = decoder.advance(&wire).unwrap();
        state.apply_changes(&wire, decoded);
        state.compose(decoded);
        let inventory = state.inventory.as_ref().unwrap();
        let total = inventory.totals(1, inventory.composition_known());
        match decoded.hour {
            100 => assert_eq!(
                (total.values[1], total.values[4]),
                (Some(3_230_000), Some(12))
            ),
            101 => assert_eq!(
                (total.values[1], total.values[4]),
                (Some(5_000_000), Some(12))
            ),
            102 | 103 => assert_eq!(
                (total.values[1], total.values[4]),
                (Some(5_000_000), Some(16))
            ),
            _ => unreachable!(),
        }
        if decoded.hour >= 101 {
            assert_eq!(
                inventory.applied_divisions,
                if decoded.hour == 103 { 5 } else { 3 }
            );
        }
        assert_eq!(
            inventory.compiled_templates,
            if decoded.hour >= 102 { 2 } else { 1 }
        );
    }
}

#[test]
fn army_size_incremental_transfers_deletions_and_missing_templates_preserve_totals() {
    use replay_frame::frame::FrameEncoder;
    let mut transferred = frame(101);
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *transferred.tables[TableId::Divisions.index()]
    {
        rows[0].expeditionary_owner = u32::MAX;
    }
    let mut deleted = transferred.clone();
    deleted.hour = 102;
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *deleted.tables[TableId::Divisions.index()]
    {
        rows.remove(1);
    }
    let mut missing_template = deleted.clone();
    missing_template.hour = 103;
    *missing_template.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![]));
    let mut restored_template = deleted.clone();
    restored_template.hour = 104;
    if let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *restored_template.tables[TableId::LexicalTokens.index()]
    {
        rows[0].literal = b"renamed_unit".to_vec();
    }
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&scan_tables(true));
    let mut state = ScanState::new(true);
    for observation in [
        frame(100),
        transferred,
        deleted,
        missing_template,
        restored_template,
    ] {
        let wire = encoder.encode(observation, false).unwrap();
        let decoded = decoder.advance(&wire).unwrap();
        state.apply_changes(&wire, decoded);
        state.compose(decoded);
        let inventory = state.inventory.as_ref().unwrap();
        let origin = inventory.totals(1, true);
        let host = inventory.totals(2, true);
        match decoded.hour {
            100 => assert_eq!((origin.values[0], host.values[0]), (Some(2), Some(0))),
            101 => assert_eq!((origin.values[0], host.values[0]), (Some(1), Some(1))),
            102..=104 => {
                assert_eq!((origin.values[0], host.values[0]), (Some(0), Some(1)));
                assert_eq!(
                    (origin.values[1], host.values[1]),
                    (Some(0), Some(1_230_000))
                );
                assert_eq!(origin.values[4], Some(0));
                assert_eq!(
                    host.values[4],
                    if decoded.hour == 103 { None } else { Some(6) }
                );
            }
            _ => unreachable!(),
        }
        if decoded.hour == 104 {
            assert_eq!(
                inventory.parts_at(2)[0].as_ref().unwrap()["renamed_unit"],
                3
            );
            assert!(
                !inventory.parts_at(2)[0]
                    .as_ref()
                    .unwrap()
                    .contains_key("foot")
            );
            assert_eq!(inventory.applied_divisions, 4);
        }
    }
}

#[test]
fn army_size_incremental_bad_baselines_stay_missing_until_snapshot_recovery() {
    use replay_frame::frame::FrameEncoder;
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&scan_tables(true));
    let mut state = ScanState::new(true);
    for hour in 100..=103 {
        let mut observation = frame(hour);
        if hour >= 101
            && let TableView::Complete(TableRows::Divisions(rows)) =
                &mut *observation.tables[TableId::Divisions.index()]
        {
            rows[0].hit_points = 3_000_000;
        }
        let mut wire = encoder.encode(observation, hour == 103).unwrap();
        if hour == 101 {
            let TableUpdate::Delta { baseline, .. } = &mut wire.tables[TableId::Divisions.index()]
            else {
                panic!("expected HP delta");
            };
            *baseline = 99;
        }
        let decoded = decoder.advance(&wire).unwrap();
        state.apply_changes(&wire, decoded);
        state.compose(decoded);
        let inventory = state.inventory.as_ref().unwrap();
        match hour {
            100 => assert_eq!(inventory.totals(1, true).values[1], Some(3_230_000)),
            101 | 102 => assert!(!inventory.known()),
            103 => {
                assert_eq!(inventory.totals(1, true).values[1], Some(5_000_000));
                assert_eq!(inventory.totals(1, true).values[4], Some(12));
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn army_size_windows_rank_mid_window_entries_and_never_rank_outside_history() {
    use replay::spool::Record;
    use replay_frame::frame::FrameEncoder;
    let mut middle = frame(101);
    if let TableView::Complete(TableRows::DivisionCensus(rows)) =
        &mut *middle.tables[TableId::DivisionCensus.index()]
    {
        rows[2].grouped = 4;
    }
    let mut outside = frame(200);
    if let TableView::Complete(TableRows::DivisionCensus(rows)) =
        &mut *outside.tables[TableId::DivisionCensus.index()]
    {
        rows[3].grouped = 99;
    }
    let mut wire = records(1, vec![frame(100)]);
    wire.truncate(2); // Keep the synthetic session and branch headers.
    let mut encoder = FrameEncoder::new();
    for observation in [frame(100), middle, frame(102), outside] {
        let force = observation.hour == 200;
        wire.push(Record::Frame(encoder.encode(observation, force).unwrap()));
    }
    let source = Source::from_replay(
        SourceKey {
            source_id: "army windows".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[wire]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let mut request = params(ArmyMeasure::Divisions, ArmyAttribution::Owner);
    request.tags.clear();
    request.top = Some(1);
    request.window = None;
    let manifest =
        ArmySize::load(&source, &request, &LoadContext { content: None }, &cancel).unwrap();
    assert_eq!(manifest.windows.len(), 2);
    assert!(manifest.series.is_empty());
    request.window = Some(0);
    let reply = ArmySize::load(&source, &request, &LoadContext { content: None }, &cancel).unwrap();
    assert!(reply.windows.is_empty());
    assert_eq!(
        reply.interval,
        Some(HourSpan {
            first_hour: 100,
            last_hour: 102
        })
    );
    assert_eq!(
        reply
            .series
            .iter()
            .map(|series| series.tag.as_str())
            .collect::<Vec<_>>(),
        ["host", "origin"]
    );
    let host = reply
        .series
        .iter()
        .find(|series| series.tag == "host")
        .unwrap();
    assert_eq!(
        host.membership,
        Some(vec![HourSpan {
            first_hour: 101,
            last_hour: 101
        }])
    );
    assert_eq!(host.runs[0].total, Some(4.0));
    let origin = reply
        .series
        .iter()
        .find(|series| series.tag == "origin")
        .unwrap();
    assert_eq!(
        origin.membership,
        Some(vec![
            HourSpan {
                first_hour: 100,
                last_hour: 100
            },
            HourSpan {
                first_hour: 102,
                last_hour: 102
            }
        ])
    );
    request.window = Some(1);
    let later = ArmySize::load(&source, &request, &LoadContext { content: None }, &cancel).unwrap();
    assert_eq!(later.series[0].tag, "empty");
    assert_eq!(later.series[0].runs[0].first_hour, 200);
    request.window = Some(2);
    assert!(
        ArmySize::load(&source, &request, &LoadContext { content: None }, &cancel)
            .unwrap_err()
            .to_string()
            .contains("Unknown army history window")
    );
    assert_eq!(source.state::<Scans>().lock().unwrap().runs, 1);
}

// Recompute directly from the full decoder's rows, independently of sparse
// deltas, template weights, compiled compositions and cached versions.
fn full_inventory_at(
    decoded: &DecodedFrame,
    slot: u32,
) -> (Totals, [Option<BTreeMap<String, i64>>; 2]) {
    let (Ok(rows), Ok(links)) = (
        decoded.complete::<tables::Divisions>(),
        decoded.complete::<tables::CountryLinks>(),
    ) else {
        return (Totals::default(), [None, None]);
    };
    let templates = decoded.complete::<tables::DivisionTemplateIcons>().ok();
    let lexical = decoded.complete::<tables::LexicalTokens>().ok();
    let mut total = Totals::default();
    let mut mix = [None, None];
    for (index, attribution) in [ArmyAttribution::Owner, ArmyAttribution::Host]
        .into_iter()
        .enumerate()
    {
        let raw = |row: &DivisionRow| {
            if attribution == ArmyAttribution::Owner {
                owner(row)
            } else {
                row.roster_owner
            }
        };
        if rows.iter().any(|row| link_slot(links, raw(row)).is_none()) {
            continue;
        }
        let scalar = channel(ArmyMeasure::Divisions, attribution);
        total.values[scalar] = Some(0);
        total.values[scalar + 1] = Some(0);
        let mut parts = templates.map(|_| BTreeMap::<String, i64>::new());
        for row in rows
            .iter()
            .filter(|row| link_slot(links, raw(row)) == Some(slot))
        {
            total.values[scalar] = total.values[scalar].map(|value| value + 1);
            if dashboard_base::products::finite_fixed64(row.hit_points) && row.hit_points >= 0 {
                total.values[scalar + 1] =
                    total.values[scalar + 1].map(|value| value + i128::from(row.hit_points));
            } else {
                total.values[scalar + 1] = None;
            }
            let template = templates.and_then(|templates| {
                templates
                    .iter()
                    .find(|template| template.template == row.template)
            });
            if let (Some(value), Some(template)) = (parts.as_mut(), template)
                && template.model_unreadable == 0
            {
                let mut valid = true;
                for unit in &template.subunits {
                    if unit.token == 0 {
                        continue;
                    }
                    if unit.count < 0 {
                        valid = false;
                        break;
                    }
                    let name = lexical
                        .and_then(|lexical| lexical.iter().find(|name| name.token == unit.token))
                        .and_then(|name| std::str::from_utf8(&name.literal).ok())
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("Unresolved sub-unit token {}", unit.token));
                    *value.entry(name).or_default() += i64::from(unit.count);
                }
                if !valid {
                    parts = None;
                }
            } else {
                parts = None;
            }
        }
        total.values[channel(ArmyMeasure::Battalions, attribution)] = parts
            .as_ref()
            .map(|parts| parts.values().copied().map(i128::from).sum());
        mix[index] = parts;
    }
    (total, mix)
}

#[test]
fn army_size_inventory_matches_full_recompute_at_every_step() {
    use replay_frame::frame::FrameEncoder;
    let mut initial = frame(100);
    if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
        &mut *initial.tables[TableId::DivisionTemplateIcons.index()]
    {
        rows[0].subunits.push(TemplateSubunit {
            token: 0,
            local: 0,
            count: 99,
        });
    }
    let mut observations = vec![initial];
    for step in 1..=12 {
        let mut next = observations.last().unwrap().clone();
        next.hour = 100 + step;
        if let TableView::Complete(TableRows::Divisions(rows)) =
            &mut *next.tables[TableId::Divisions.index()]
        {
            match step {
                1 | 10 => rows[0].hit_points += 123_457,
                2 => {
                    rows[0].roster_owner = 30;
                    rows[0].logical_owner = 30;
                }
                3 => rows[0].expeditionary_owner = 20,
                4 => rows.push(DivisionRow {
                    id: 3,
                    roster_owner: 30,
                    logical_owner: 30,
                    expeditionary_owner: u32::MAX,
                    template: 9,
                    hit_points: 800_000,
                    ..Default::default()
                }),
                5 => {
                    rows.remove(1);
                }
                _ => {}
            }
        }
        if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
            &mut *next.tables[TableId::DivisionTemplateIcons.index()]
        {
            match step {
                4 | 12 => rows.push(DivisionTemplateIconRow {
                    template: 9,
                    subunits: vec![TemplateSubunit {
                        token: 20,
                        local: 1,
                        count: 5,
                    }],
                    ..Default::default()
                }),
                6 => rows[0].subunits[0].count += 2,
                7 => {
                    rows.remove(1);
                }
                _ => {}
            }
        }
        if step == 8
            && let TableView::Complete(TableRows::LexicalTokens(rows)) =
                &mut *next.tables[TableId::LexicalTokens.index()]
        {
            rows[0].literal = b"renamed_foot".to_vec();
        }
        if step == 9
            && let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *next.tables[TableId::CountryLinks.index()]
        {
            rows[0].canonical_slot = 2;
            rows[1].canonical_slot = 2;
            rows[2].canonical_slot = 3;
        }
        observations.push(next);
    }
    let mut encoder = FrameEncoder::new();
    let mut tables = scan_tables(true);
    tables.extend([TableId::Divisions, TableId::DivisionTemplateIcons]);
    let mut full = FrameDecoder::for_tables(&tables);
    let mut sparse = Inventory::default();
    for observation in observations {
        let force = observation.hour == 112;
        let mut wire = encoder.encode(observation, force).unwrap();
        if wire.hour == 110 {
            if let TableUpdate::Delta { baseline, .. } =
                &mut wire.tables[TableId::Divisions.index()]
            {
                *baseline = 99;
            } else {
                panic!("expected changed HP delta");
            }
        }
        let decoded = full.advance(&wire).unwrap();
        sparse.apply_changes(&wire, decoded);
        sparse.compose(decoded);
        for slot in 1..=3 {
            let expected = full_inventory_at(decoded, slot);
            let actual_mix = if sparse.composition_known() {
                sparse.parts_at(slot)
            } else {
                [None, None]
            };
            assert_eq!(
                sparse.totals(slot, sparse.composition_known()),
                expected.0,
                "totals at hour {}, slot {slot}",
                decoded.hour
            );
            assert_eq!(
                actual_mix, expected.1,
                "mix at hour {}, slot {slot}",
                decoded.hour
            );
        }
        if decoded.hour == 110 || decoded.hour == 111 {
            assert!(!sparse.known());
            assert!(
                sparse
                    .errors()
                    .any(|(_, reason)| reason == "missing table baseline")
            );
        }
        if decoded.hour == 112 {
            assert!(sparse.composition_known());
        }
    }
}

#[test]
fn army_size_invalid_sparse_rows_name_the_gap_and_preserve_census() {
    use replay_frame::frame::FrameEncoder;
    let source = opened(vec![frame(100), frame(101)]);
    let mut encoder = FrameEncoder::new();
    let mut bad = encoder.encode(frame(100), false).unwrap();
    if let TableUpdate::Snapshot(rows) = &mut bad.tables[TableId::Divisions.index()] {
        let rows = rows.rows_mut::<tables::Divisions>().unwrap();
        rows.push(rows[0].clone());
    }
    let mut decoder = FrameDecoder::for_tables(&scan_tables(true));
    let mut state = ScanState::new(true);
    let mut result = Scan::default();
    let decoded = decoder.advance(&bad).unwrap();
    state.apply_changes(&bad, decoded);
    state.compose(decoded);
    observe_totals(
        &mut result,
        decoded,
        &bad,
        source.replay_ref().format,
        false,
        &mut state,
        None,
    );
    assert_eq!(
        result.tracks["origin"].runs[0].totals.values,
        [Some(3), None, None, None, None, None, None, None]
    );
    assert_eq!(
        result
            .invalid_table_reads
            .get("Divisions: row keys must be unique and ascending"),
        Some(&1)
    );
    let recovery = encoder.encode(frame(101), true).unwrap();
    let decoded = decoder.advance(&recovery).unwrap();
    state.apply_changes(&recovery, decoded);
    state.compose(decoded);
    observe_totals(
        &mut result,
        decoded,
        &recovery,
        source.replay_ref().format,
        false,
        &mut state,
        None,
    );
    let values = result.tracks["origin"].runs.last().unwrap().totals.values;
    assert_eq!(values[0], Some(3));
    assert_eq!(values[1], Some(3_230_000));
    assert_eq!(values[4], Some(12));
}

#[test]
fn army_size_manpower_native_total_owner_host_and_missing_input() {
    use replay_data::{rows::DivisionLandInputsRow, rows::DivisionManpowerContributor};
    let mut first = frame(100);
    *first.tables[TableId::DivisionLandInputs.index()] =
        TableView::Complete(TableRows::DivisionLandInputs(vec![
            DivisionLandInputsRow {
                division: 1,
                manpower: vec![
                    DivisionManpowerContributor {
                        country: 20,
                        amount: i32::MAX,
                    },
                    DivisionManpowerContributor {
                        country: 30,
                        amount: 1,
                    },
                ],
                ..Default::default()
            },
            DivisionLandInputsRow {
                division: 2,
                manpower: vec![DivisionManpowerContributor {
                    country: 30,
                    amount: 250,
                }],
                ..Default::default()
            },
        ]));
    let mut next = first.clone();
    next.hour = 101;
    if let TableView::Complete(TableRows::DivisionLandInputs(rows)) =
        &mut *next.tables[TableId::DivisionLandInputs.index()]
    {
        rows.remove(0);
    }
    let source = opened(vec![first, next]);
    let cancel = AtomicBool::new(false);
    assert_eq!(
        army_value_at(
            &source,
            1,
            "origin",
            100,
            ArmyMeasure::Manpower,
            ArmyAttribution::Owner,
            &cancel
        )
        .unwrap()
        .unwrap()
        .raw,
        i128::from(i32::MIN) + 250
    );
    assert_eq!(
        army_value_at(
            &source,
            1,
            "host",
            100,
            ArmyMeasure::Manpower,
            ArmyAttribution::Host,
            &cancel
        )
        .unwrap()
        .unwrap()
        .raw,
        i128::from(i32::MIN)
    );
    assert_eq!(
        army_value_at(
            &source,
            1,
            "empty",
            100,
            ArmyMeasure::Manpower,
            ArmyAttribution::Owner,
            &cancel
        )
        .unwrap()
        .unwrap()
        .raw,
        0
    );
    assert!(
        army_value_at(
            &source,
            1,
            "origin",
            101,
            ArmyMeasure::Manpower,
            ArmyAttribution::Owner,
            &cancel
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(
        army_value_at(
            &source,
            1,
            "origin",
            101,
            ArmyMeasure::Strength,
            ArmyAttribution::Owner,
            &cancel
        )
        .unwrap()
        .unwrap()
        .raw,
        3_230_000
    );
    let mut runs = Vec::new();
    visit_army_runs(
        &source,
        1,
        &["origin".into()],
        ArmyMeasure::Manpower,
        ArmyAttribution::Owner,
        &cancel,
        |tag, first, last, starts, raw| {
            runs.push((tag.to_owned(), first, last, starts, raw));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        runs,
        [
            (
                "origin".into(),
                100,
                100,
                true,
                Some(i128::from(i32::MIN) + 250)
            ),
            ("origin".into(), 101, 101, false, None)
        ]
    );
    assert_eq!(source.state::<Scans>().lock().unwrap().runs, 1);
}

#[test]
fn army_size_exact_hp_wire_retains_adjacent_and_large_finite_totals() {
    let scan = Scan {
        tracks: BTreeMap::from([(
            "origin".into(),
            Track {
                runs: [
                    9_007_199_254_740_986i128,
                    9_007_199_254_740_987,
                    i128::from(i64::MAX) + 1,
                ]
                .into_iter()
                .enumerate()
                .map(|(index, raw)| {
                    let mut totals = Totals::default();
                    totals.values[1] = Some(raw);
                    RawRun {
                        first: index as i32,
                        last: index as i32,
                        starts: index == 0,
                        totals,
                    }
                })
                .collect(),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let reply = reply_series(
        &scan,
        &params(ArmyMeasure::Strength, ArmyAttribution::Owner),
        &HourSpan {
            first_hour: 0,
            last_hour: 2,
        },
        None,
        &AtomicBool::new(false),
        None,
    )
    .unwrap()
    .0;
    assert_eq!(
        reply[0]
            .runs
            .iter()
            .map(|run| run.raw.as_str())
            .collect::<Vec<_>>(),
        [
            "9007199254740986",
            "9007199254740987",
            "9223372036854775808"
        ]
    );
}

#[test]
fn army_size_composition_wire_retains_adjacent_wide_counts() {
    let raw = [
        9_007_199_254_740_992i64,
        9_007_199_254_740_993,
        9_007_199_254_740_993,
    ];
    assert_eq!(raw[0] as f64, raw[1] as f64);
    let range = HourSpan {
        first_hour: 7,
        last_hour: 9,
    };
    let scan = Scan {
        tracks: BTreeMap::from([(
            "origin".into(),
            Track {
                segments: vec![range],
                runs: raw
                    .into_iter()
                    .enumerate()
                    .map(|(index, raw)| {
                        let mut totals = Totals::default();
                        totals.values[4] = Some(i128::from(raw));
                        RawRun {
                            first: 7 + index as i32,
                            last: 7 + index as i32,
                            starts: index == 0,
                            totals,
                        }
                    })
                    .collect(),
                mix: raw
                    .into_iter()
                    .enumerate()
                    .map(|(index, raw)| RawMixRun {
                        first: 7 + index as i32,
                        last: 7 + index as i32,
                        parts: [Some(BTreeMap::from([("synthetic_unit".into(), raw)])), None],
                    })
                    .collect(),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let source = opened(vec![frame(100)]);
    let request = params(ArmyMeasure::Battalions, ArmyAttribution::Owner);
    let cancelled = AtomicBool::new(false);
    let (mut series, _) = reply_series(&scan, &request, &range, None, &cancelled, None).unwrap();
    let types = composition_reply(
        &source,
        &LoadContext { content: None },
        &scan,
        &request,
        &range,
        &mut series,
        &mut Vec::new(),
        &cancelled,
    )
    .unwrap();
    assert_eq!(types.len(), 1);
    let runs = series[0].mix.as_ref().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!((runs[0].first_hour, runs[0].last_hour), (7, 7));
    assert_eq!((runs[1].first_hour, runs[1].last_hour), (8, 9));
    assert_eq!(runs[0].values, runs[1].values);
    let wire = serde_json::to_value(runs).unwrap();
    assert_eq!(wire[0]["rawValues"][0], "9007199254740992");
    assert_eq!(wire[1]["rawValues"][0], "9007199254740993");
}

#[test]
fn army_size_identity_windows_clip_native_dates() {
    let identity = dashboard_base::products::country_history::CountryIdentityRun {
        first_hour: 10,
        last_hour: 29,
        slot: 4,
        country_id: 0xffff_ffff,
    };
    let mut country = dashboard_base::products::country_history::RecordedCountry::default();
    country.identities = Arc::new(vec![identity]);
    let found = window_identities(
        &country,
        &HourSpan {
            first_hour: 20,
            last_hour: 39,
        },
    );
    assert_eq!(found.len(), 1);
    assert_eq!(
        (
            found[0].first_hour,
            found[0].last_hour,
            found[0].slot,
            found[0].country_id
        ),
        (20, 29, 4, 0xffff_ffff)
    );
}
