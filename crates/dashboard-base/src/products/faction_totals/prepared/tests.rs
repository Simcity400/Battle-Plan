use super::*;
use crate::prepared::series::{Accumulator, tests::Fixture};
use crate::products::country_history::{CountryIdentityRun, HistoryScan, RecordedCountry};
use crate::test_fixtures::{CAMPAIGN, records};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{CountryFactoriesRow, ProvinceRow, TableId, TableRows},
};

fn frame(hour: i32) -> DecodedFrame {
    let mut frame =
        super::super::tests::recorded(hour, &[(401, "Pact", &[44, 9001]), (402, "Empty", &[])]);
    *frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
            id: 1,
            owner: 44,
            controller: 55,
            victory_points: -7,
            ..Default::default()
        }]));
    for table in [TableId::States, TableId::Provinces] {
        frame.metrics.table_ns[table.index()] = 1;
    }
    if hour >= 104
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
    {
        for row in rows {
            if row.id == 9001 {
                row.tag = b"renamed realm".to_vec();
            }
        }
    }
    *frame.tables[TableId::CountryFactories.index()] =
        TableView::Complete(TableRows::CountryFactories(vec![
            CountryFactoriesRow {
                country: 44,
                military_total_raw: 1_550_000,
                civilian_total_raw: 2_050_000,
                naval_total_raw: 1_250_000,
                ..Default::default()
            },
            CountryFactoriesRow {
                country: 9001,
                military_total_raw: i64::from(hour - 98) * 150_000,
                ..Default::default()
            },
        ]));
    if hour == 102 {
        for table in [TableId::States, TableId::CountryFactories] {
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: "synthetic supplying read missing".into(),
            };
        }
    }
    frame
}

fn span(first_hour: i32, last_hour: i32) -> HourSpan {
    HourSpan {
        first_hour,
        last_hour,
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn faction_prepared_exact_matches_native_oracle_after_commits_resume_append_and_aliases() {
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let context = LoadContext { content: None };
    let mut territory = territory::prepared::Prepared::default();
    let mut factories = country_factories::prepared::Prepared::default();
    let mut identities = HistoryScan::new(false, &[]);
    let mut groups = None;
    let mut all = Vec::new();
    for (commit, hours) in [vec![100, 101], vec![102, 104], vec![105, 106]]
        .into_iter()
        .enumerate()
    {
        let tail: Vec<_> = hours.into_iter().map(frame).collect();
        all.extend(tail.iter().cloned());
        let replay =
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, all.clone())]).unwrap());
        let source = super::super::tests::opened(
            Arc::clone(&replay),
            "prepared-faction",
            Default::default(),
            &flag,
        );
        let scan = groups.get_or_insert_with(|| country_groups::Scan::new(&replay, 1));
        scan.begin_block();
        for frame in tail {
            territory
                .observe_owned_states(&frame, &mut identities, None)
                .unwrap();
            territory.configure(&source, &context, &flag).unwrap();
            territory
                .observe(
                    &frame,
                    source.replay_ref().format,
                    &flag,
                    &crate::prepared::frame_changes::FrameChangeTracker::new()
                        .observe(&frame, source.replay_ref().format),
                )
                .unwrap();
            factories
                .observe(
                    &frame,
                    &replay,
                    &crate::prepared::frame_changes::FrameChangeTracker::new()
                        .observe(&frame, replay.format),
                )
                .unwrap();
            scan.observe(&frame).unwrap();
        }
        scan.finish(&replay, 1);
        let saved_groups = scan.saved_bytes().unwrap();
        groups = Some(country_groups::Scan::read(saved_groups.as_slice()).unwrap());
        let history = groups.as_ref().unwrap().faction_history(&flag).unwrap();
        assert_eq!(
            history
                .factions
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["401", "402"],
            "resuming prepared country groups must retain the faction roster evidence, commit {commit}"
        );
        assert_eq!(
            history.faction_reads.last().unwrap().last_hour,
            all.last().unwrap().hour
        );
        let identity_history =
            serde_json::from_slice::<HistoryScan>(&serde_json::to_vec(&identities).unwrap())
                .unwrap()
                .finish();
        let mut writer = if commit == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        territory.flush(&mut writer, 1, &flag).unwrap();
        factories.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        territory = serde_json::from_slice(&serde_json::to_vec(&territory).unwrap()).unwrap();
        factories = serde_json::from_slice(&serde_json::to_vec(&factories).unwrap()).unwrap();
        identities = serde_json::from_slice(&serde_json::to_vec(&identities).unwrap()).unwrap();
        let (territory_keys, territory_meta) = series::read_catalogue::<territory::TerritoryReply>(
            &fixture.store,
            &fixture.key,
            "territory",
            1,
        )
        .unwrap()
        .unwrap();
        let (factory_keys, _) = series::read_catalogue::<country_factories::ScannedFactories>(
            &fixture.store,
            &fixture.key,
            "factories",
            1,
        )
        .unwrap()
        .unwrap();
        for source_measure in [
            FactionMeasure::Territory {
                measure: TerritoryMeasure::States,
                rule: TerritoryRule::Owned,
            },
            FactionMeasure::Territory {
                measure: TerritoryMeasure::VictoryPoints,
                rule: TerritoryRule::Controlled,
            },
            FactionMeasure::Factories {
                mode: FactoryCount::Built,
                groups: vec![],
            },
        ] {
            for membership in [
                FactionMembership::Hour,
                FactionMembership::End,
                FactionMembership::Ever,
            ] {
                let params = FactionTotalsParams {
                    branch_id: "1".into(),
                    source: source_measure.clone(),
                    membership,
                    top: 2,
                    faction_id: None,
                    parts: None,
                    inventory: Some(true),
                };
                let members =
                    NativeMembers::new(&history, &identity_history, membership, &flag).unwrap();
                let (keys, section, measures) = match &source_measure {
                    FactionMeasure::Territory { measure, rule } => (
                        territory_keys.clone(),
                        format!("territory-values-1-{}", measure.index(*rule)),
                        territory::faction_tracks(
                            &source,
                            1,
                            &members.tags(),
                            *measure,
                            *rule,
                            &context,
                            &flag,
                        )
                        .unwrap(),
                    ),
                    FactionMeasure::Factories { mode, groups } => {
                        let channel =
                            country_factories::prepared::channel(&country_factories::FactoryTop {
                                count: 1,
                                mode: *mode,
                                groups: groups.clone(),
                            });
                        let keys = identity_history
                            .countries
                            .keys()
                            .filter_map(|tag| {
                                factory_keys
                                    .get(&series::field_key(tag, ""))
                                    .map(|&key| (tag.clone(), key))
                            })
                            .collect();
                        (
                            keys,
                            format!("factories-values-1-{channel}"),
                            country_factories::faction_tracks(
                                &source,
                                1,
                                &members.tags(),
                                *mode,
                                groups,
                                &history,
                                &flag,
                            )
                            .unwrap(),
                        )
                    }
                };
                let full = sum_prepared(
                    &history,
                    &members.measures(&measures.tracks, &flag).unwrap(),
                    &members.rosters,
                    None,
                    &flag,
                )
                .unwrap();
                let old = FactionTotals::load(&source, &params, &context, &flag).unwrap();
                let mut values = Reader::new(&fixture.store, &fixture.key, section, &flag).unwrap();
                let last = all.last().unwrap().hour;
                for window in [span(100, last), span(last, last), span(101, last)] {
                    let (_, sums) =
                        exact(&mut values, &keys, &history, &members, &window, &flag).unwrap();
                    for (id, expected) in &full.series {
                        let expected: Vec<_> = expected
                            .iter()
                            .filter_map(|run| {
                                let first_hour = run.first_hour.max(window.first_hour);
                                let last_hour = run.last_hour.min(window.last_hour);
                                (first_hour <= last_hour).then_some(FactionTotalRun {
                                    first_hour,
                                    last_hour,
                                    starts_segment: run.starts_segment
                                        && first_hour == run.first_hour,
                                    value: run.value,
                                })
                            })
                            .collect();
                        assert_eq!(
                            sums.series[id], expected,
                            "{source_measure:?} {membership:?}, {window:?}, commit {commit}, faction {id}"
                        );
                    }
                    assert_eq!(sums.recorded, clipped(&full.recorded, &window));
                    assert_eq!(sums.disjoint, clipped(&full.disjoint, &window));
                    let measures = CountryMeasures {
                        tracks: Vec::new(),
                        unit: measures.unit.clone(),
                        unavailable_reason: measures.unavailable_reason.clone(),
                        warnings: measures.warnings.clone(),
                    };
                    let prepared = build_reply(
                        &source,
                        &params,
                        &context,
                        &history,
                        &identity_history,
                        &members,
                        &measures,
                        &sums,
                        Some(&window),
                        &flag,
                    )
                    .unwrap();
                    let mut expected = old.clone();
                    expected.recorded = clipped(&expected.recorded, &window);
                    expected.disjoint = clipped(&expected.disjoint, &window);
                    for series in &mut expected.series {
                        series.membership = clipped(&series.membership, &window);
                        series.members = clip_members(&series.members, Some(&window));
                        series.counted_members =
                            clip_members(&series.counted_members, Some(&window));
                        series.names.retain(|run| {
                            run.last_hour >= window.first_hour && run.first_hour <= window.last_hour
                        });
                        for run in &mut series.names {
                            run.first_hour = run.first_hour.max(window.first_hour);
                            run.last_hour = run.last_hour.min(window.last_hour);
                        }
                        series.runs.retain(|run| {
                            run.last_hour >= window.first_hour && run.first_hour <= window.last_hour
                        });
                        for run in &mut series.runs {
                            run.starts_segment &= run.first_hour >= window.first_hour;
                            run.first_hour = run.first_hour.max(window.first_hour);
                            run.last_hour = run.last_hour.min(window.last_hour);
                        }
                    }
                    expected
                        .series
                        .retain(|series| !series.membership.is_empty());
                    assert_eq!(
                        serde_json::to_value(&prepared).unwrap(),
                        serde_json::to_value(expected).unwrap(),
                        "bounded full reply: {source_measure:?} {membership:?}, commit {commit}"
                    );
                    assert!(prepared.series.iter().all(|series| {
                        series.membership.iter().all(|span| {
                            span.first_hour >= window.first_hour
                                && span.last_hour <= window.last_hour
                        })
                    }));
                }
            }
        }
        assert!(
            territory_meta.area_reason.is_some(),
            "missing map content is retained as an independent catalogue refusal"
        );
    }
}

#[test]
fn faction_prepared_dense_empty_roster_zero_does_not_require_country_reads() {
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let mut values = Reader::new(&fixture.store, &fixture.key, "no-values".into(), &flag).unwrap();
    let mut present =
        Reader::new(&fixture.store, &fixture.key, "no-presence".into(), &flag).unwrap();
    let mut rosters = Reader::new(
        &fixture.store,
        &fixture.key,
        "no-country-read".into(),
        &flag,
    )
    .unwrap();
    let native = NativeMembers::new(
        &History::default(),
        &RecordedHistories::default(),
        FactionMembership::Hour,
        &flag,
    )
    .unwrap();
    let result = dense(
        &mut values,
        &BTreeMap::new(),
        &mut present,
        &BTreeMap::new(),
        &mut rosters,
        &native,
        &BTreeMap::new(),
        &[span(0, 99)],
        &[],
        (0.0, 99.0, 2),
        &flag,
    )
    .unwrap();
    assert!(result.runs.is_none());
    assert!(result.buckets.iter().all(|bucket| {
        bucket.is_some_and(|bucket| {
            bucket.min_value == 0.0 && bucket.max_value == 0.0 && !bucket.has_break
        })
    }));
    let unknown = dense(
        &mut values,
        &BTreeMap::new(),
        &mut present,
        &BTreeMap::new(),
        &mut rosters,
        &native,
        &BTreeMap::new(),
        &[],
        &[span(0, 99)],
        (0.0, 99.0, 2),
        &flag,
    )
    .unwrap();
    assert!(unknown.buckets.iter().all(Option::is_none));
}

#[test]
fn faction_prepared_navigation_skips_cancelling_changes_and_keeps_membership_edges() {
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let mut values = Accumulator::default();
    for hour in 0..=10 {
        values.observe(
            1,
            hour,
            Some(if hour < 5 {
                10
            } else if hour < 7 {
                20
            } else {
                22
            }),
            false,
        );
        values.observe(2, hour, Some(if hour < 5 { 20 } else { 10 }), false);
    }
    let mut writer = fixture.writer();
    values.flush(&mut writer, "values", 0, &flag).unwrap();
    writer.commit().unwrap();
    let mut history = History {
        factions: BTreeMap::from([(
            "401".into(),
            vec![
                RosterRun {
                    first_hour: 0,
                    last_hour: 8,
                    name: None,
                    members: vec!["A".into(), "B".into()],
                    complete: true,
                },
                RosterRun {
                    first_hour: 9,
                    last_hour: 10,
                    name: None,
                    members: vec!["A".into()],
                    complete: true,
                },
            ],
        )]),
        faction_reads: vec![span(0, 10)],
        last_hour: 10,
        end_recorded: true,
        ..History::default()
    };
    history.country_reads = vec![span(0, 10)];
    history.country_presence = HashMap::from([
        ("A".into(), vec![span(0, 10)]),
        ("B".into(), vec![span(0, 10)]),
    ]);
    let mut identities = RecordedHistories::default();
    identities.identity_reads = vec![span(0, 10)];
    for (tag, slot) in [("A", 1), ("B", 2)] {
        let mut country = RecordedCountry::default();
        country.identities = Arc::new(vec![CountryIdentityRun {
            first_hour: 0,
            last_hour: 10,
            slot,
            country_id: slot,
        }]);
        identities.countries.insert(tag.into(), country);
    }
    let params = FactionTotalsParams {
        branch_id: "1".into(),
        source: FactionMeasure::Territory {
            measure: TerritoryMeasure::States,
            rule: TerritoryRule::Owned,
        },
        membership: FactionMembership::Hour,
        top: 1,
        faction_id: Some("401".into()),
        parts: None,
        inventory: None,
    };
    let members = NativeMembers::new(&history, &identities, params.membership, &flag).unwrap();
    let keys = BTreeMap::from([("A".into(), 1), ("B".into(), 2)]);
    let mut values = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    for (key, from, first_hour, expected) in [
        (series::MeasureKey::Home, None, 0.0, 0.0),
        (series::MeasureKey::ArrowRight, Some(0.0), 0.0, 7.0),
        (series::MeasureKey::ArrowLeft, Some(7.0), 0.0, 0.0),
        (series::MeasureKey::ArrowRight, Some(7.0), 0.0, 9.0),
        (series::MeasureKey::End, None, 0.0, 9.0),
        (series::MeasureKey::Home, None, 1.5, 1.5),
        (series::MeasureKey::ArrowRight, Some(9.0), 0.0, 9.0),
    ] {
        let navigation = series::Navigation {
            key,
            from,
            first_hour,
            last_hour: 10.0,
        };
        assert_eq!(
            navigate(
                &mut values,
                &keys,
                &history,
                &members,
                &params,
                &navigation,
                &flag
            )
            .unwrap(),
            Some(expected),
            "{key:?} {from:?}"
        );
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn faction_prepared_exact_and_dense_keep_five_thousand_signed_member_values() {
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let tags: Vec<_> = (0..5_000)
        .map(|slot| format!("opaque country {slot}"))
        .collect();
    let keys: BTreeMap<_, _> = tags
        .iter()
        .enumerate()
        .map(|(slot, tag)| (tag.clone(), slot as u64 + 1))
        .collect();
    let mut values = Accumulator::default();
    let mut present = Accumulator::default();
    let mut rosters = Accumulator::default();
    for hours in [100..=101, 102..=103] {
        for hour in hours {
            rosters.observe(0, hour, Some(1), false);
            for (slot, tag) in tags.iter().enumerate() {
                values.observe(keys[tag], hour, Some(slot as i128 - 2_500), false);
                present.observe(keys[tag], hour, Some(1), false);
            }
        }
        let mut writer = fixture.writer();
        values.flush(&mut writer, "values", 0, &flag).unwrap();
        present.flush(&mut writer, "presence", 0, &flag).unwrap();
        rosters.flush(&mut writer, "rosters", 0, &flag).unwrap();
        writer.commit().unwrap();
        values = serde_json::from_slice(&serde_json::to_vec(&values).unwrap()).unwrap();
        present = serde_json::from_slice(&serde_json::to_vec(&present).unwrap()).unwrap();
        rosters = serde_json::from_slice(&serde_json::to_vec(&rosters).unwrap()).unwrap();
    }
    let history = History {
        factions: BTreeMap::from([(
            "401".into(),
            vec![RosterRun {
                first_hour: 100,
                last_hour: 103,
                name: None,
                members: tags.clone(),
                complete: true,
            }],
        )]),
        faction_reads: vec![span(100, 103)],
        country_reads: vec![span(100, 103)],
        country_presence: tags
            .iter()
            .map(|tag| (tag.clone(), vec![span(100, 103)]))
            .collect(),
        last_hour: 103,
        end_recorded: true,
        ..History::default()
    };
    let mut identities = RecordedHistories::default();
    identities.identity_reads = vec![span(100, 103)];
    for (slot, tag) in tags.iter().enumerate() {
        let mut country = RecordedCountry::default();
        country.identities = Arc::new(vec![CountryIdentityRun {
            first_hour: 100,
            last_hour: 103,
            slot: slot as u32,
            country_id: slot as u32 + 20_000,
        }]);
        identities.countries.insert(tag.clone(), country);
    }
    let members =
        NativeMembers::new(&history, &identities, FactionMembership::Hour, &flag).unwrap();
    let mut values = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let (tracks, sums) = exact(
        &mut values,
        &keys,
        &history,
        &members,
        &span(103, 103),
        &flag,
    )
    .unwrap();
    assert_eq!(tracks.len(), 5_000);
    assert_eq!(sums.series["401"][0].value, -2_500);
    let mut present = Reader::new(&fixture.store, &fixture.key, "presence".into(), &flag).unwrap();
    let mut rosters = Reader::new(&fixture.store, &fixture.key, "rosters".into(), &flag).unwrap();
    let (bindings, known, unknown) = bindings(&members, "401", &flag).unwrap();
    let summary = dense(
        &mut values,
        &keys,
        &mut present,
        &keys,
        &mut rosters,
        &members,
        &bindings,
        &known,
        &unknown,
        (100.0, 103.0, 1),
        &flag,
    )
    .unwrap();
    assert_eq!(summary.buckets[0].unwrap().first_value, -2_500.0);
    assert_eq!(summary.buckets[0].unwrap().last_value, -2_500.0);
    eprintln!(
        "5,000-member faction bounded summary bytes: {}",
        serde_json::to_vec(&summary).unwrap().len()
    );
}

#[test]
fn faction_native_counted_reused_alias_is_dated_before_merging_dense_bindings() {
    let flag = AtomicBool::new(false);
    let history = History {
        factions: BTreeMap::from([(
            "401".into(),
            vec![RosterRun {
                first_hour: 0,
                last_hour: 3,
                name: None,
                members: vec!["shared".into()],
                complete: true,
            }],
        )]),
        faction_reads: vec![span(0, 3)],
        last_hour: 3,
        end_recorded: true,
        ..History::default()
    };
    let mut identities = RecordedHistories::default();
    identities.identity_reads = vec![span(-1, 3)];
    for (tag, runs) in [
        ("AA future", vec![(0, 1, 2, 20)]),
        ("shared", vec![(0, 1, 1, 10), (2, 3, 2, 20)]),
        ("Z old", vec![(-1, -1, 1, 10)]),
    ] {
        let mut country = RecordedCountry::default();
        country.identities = Arc::new(
            runs.into_iter()
                .map(
                    |(first_hour, last_hour, slot, country_id)| CountryIdentityRun {
                        first_hour,
                        last_hour,
                        slot,
                        country_id,
                    },
                )
                .collect(),
        );
        identities.countries.insert(tag.into(), country);
    }
    let members =
        NativeMembers::new(&history, &identities, FactionMembership::Ever, &flag).unwrap();
    let shared: Vec<_> = members.counted["401"]
        .iter()
        .filter(|run| run.tag == "shared")
        .map(|run| span(run.first_hour, run.last_hour))
        .collect();
    assert_eq!(
        shared,
        [span(0, 3)],
        "native census order cannot discard an earlier holder of an alias"
    );
    let (native, _, _) = bindings(&members, "401", &flag).unwrap();
    assert_eq!(native.len(), 2);
    assert!(native.values().all(|spans| *spans == [span(0, 3)]));
    let fixture = Fixture::new();
    let keys = BTreeMap::from([
        ("AA future".into(), 1),
        ("shared".into(), 2),
        ("Z old".into(), 3),
    ]);
    let mut values = Accumulator::default();
    let mut present = Accumulator::default();
    let mut rosters = Accumulator::default();
    for hour in 0..=3 {
        values.observe(1, hour, (hour < 2).then_some(20), false);
        values.observe(2, hour, Some(if hour < 2 { 10 } else { 20 }), hour == 2);
        present.observe(1, hour, (hour < 2).then_some(1), false);
        present.observe(2, hour, Some(1), hour == 2);
        rosters.observe(0, hour, Some(1), false);
    }
    let mut writer = fixture.writer();
    values.flush(&mut writer, "values", 0, &flag).unwrap();
    present.flush(&mut writer, "presence", 0, &flag).unwrap();
    rosters.flush(&mut writer, "rosters", 0, &flag).unwrap();
    writer.commit().unwrap();
    let mut values = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let mut present = Reader::new(&fixture.store, &fixture.key, "presence".into(), &flag).unwrap();
    let mut rosters = Reader::new(&fixture.store, &fixture.key, "rosters".into(), &flag).unwrap();
    let (native, known, unknown) = bindings(&members, "401", &flag).unwrap();
    let summary = dense(
        &mut values,
        &keys,
        &mut present,
        &keys,
        &mut rosters,
        &members,
        &native,
        &known,
        &unknown,
        (0.0, 3.0, 1),
        &flag,
    )
    .unwrap();
    let bucket = summary.buckets[0].unwrap();
    assert_eq!(
        (
            bucket.first_value,
            bucket.last_value,
            bucket.min_value,
            bucket.max_value
        ),
        (30.0, 20.0, 20.0, 30.0),
        "aliases contribute once per native holder; proved absence contributes zero"
    );
}

#[test]
fn faction_prepared_dense_top_keeps_mid_column_leaders_and_exact_reached_ranks() {
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let frames: Vec<_> = (100..=119)
        .map(|hour| {
            let mut frame = super::super::tests::recorded(
                hour,
                &[
                    (401, "Brief leader", &[44]),
                    (402, "Steady leader", &[9001]),
                    (403, "Signed total", &[55]),
                    (404, "Empty roster", &[]),
                ],
            );
            *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
                [
                    (1, 44, if hour == 101 { 40 } else { 0 }),
                    (2, 9001, 10),
                    (3, 55, -5),
                ]
                .into_iter()
                .map(|(id, country, victory_points)| ProvinceRow {
                    id,
                    owner: country,
                    controller: country,
                    victory_points,
                    ..Default::default()
                })
                .collect(),
            ));
            frame
        })
        .collect();
    let replay =
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap());
    let source = super::super::tests::opened(
        Arc::clone(&replay),
        "dense-faction-top",
        Default::default(),
        &flag,
    );
    let context = LoadContext { content: None };
    let mut territory = territory::prepared::Prepared::default();
    let mut identities = HistoryScan::new(false, &[]);
    let mut groups = country_groups::Scan::new(&replay, 1);
    groups.begin_block();
    for frame in &frames {
        territory
            .observe_owned_states(frame, &mut identities, None)
            .unwrap();
        territory.configure(&source, &context, &flag).unwrap();
        territory
            .observe(
                frame,
                source.replay_ref().format,
                &flag,
                &crate::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(frame, source.replay_ref().format),
            )
            .unwrap();
        groups.observe(frame).unwrap();
    }
    groups.finish(&replay, 1);
    let history = groups.faction_history(&flag).unwrap();
    let identities = identities.finish();
    let mut writer = fixture.writer();
    territory.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let (keys, _) = series::read_catalogue::<territory::TerritoryReply>(
        &fixture.store,
        &fixture.key,
        "territory",
        1,
    )
    .unwrap()
    .unwrap();
    let params = FactionTotalsParams {
        branch_id: "1".into(),
        source: FactionMeasure::Territory {
            measure: TerritoryMeasure::VictoryPoints,
            rule: TerritoryRule::Controlled,
        },
        membership: FactionMembership::Hour,
        top: 1,
        faction_id: None,
        parts: None,
        inventory: None,
    };
    let members = NativeMembers::new(&history, &identities, params.membership, &flag).unwrap();
    let mut values = Reader::new(
        &fixture.store,
        &fixture.key,
        "territory-values-1-2".into(),
        &flag,
    )
    .unwrap();
    for top in [1, 2] {
        let params = FactionTotalsParams {
            top,
            ..params.clone()
        };
        let reply = dense_reply(
            &source,
            &context,
            &params,
            &history,
            &identities,
            &members,
            CountryMeasures {
                tracks: Vec::new(),
                unit: Some("victory points".into()),
                unavailable_reason: None,
                warnings: Vec::new(),
            },
            &mut values,
            &keys,
            &keys,
            &fixture.store,
            &fixture.key,
            1,
            (100.0, 119.0, 2),
            &flag,
        )
        .unwrap();
        let summaries: BTreeMap<String, ChartSummary> =
            serde_json::from_value(reply["summaries"].clone()).unwrap();
        let leader = summaries["401"].buckets[0].unwrap();
        assert_eq!(
            (leader.min_value, leader.max_value, leader.last_value),
            (0.0, 40.0, 0.0)
        );
        assert!(summaries["402"].buckets.iter().all(Option::is_some));
        assert!(
            !summaries.contains_key("403"),
            "a signed maximum below the cutoff is excluded"
        );
        if top == 1 {
            assert!(summaries["401"].buckets[1].is_none());
            assert!(!summaries.contains_key("404"));
        } else {
            assert!(
                summaries["404"].buckets.iter().all(Option::is_some),
                "equal cutoff keeps additional dense candidates"
            );
        }
        let end = summaries["401"]
            .buckets
            .iter()
            .rev()
            .flatten()
            .next()
            .unwrap()
            .last_hour;
        let (_, exact_end) = exact(
            &mut values,
            &keys,
            &history,
            &members,
            &span(end, end),
            &flag,
        )
        .unwrap();
        let drawn = reply["series"]
            .as_array()
            .unwrap()
            .iter()
            .find(|series| series["id"] == "401")
            .unwrap();
        assert_eq!(drawn["runs"][0]["value"], exact_end.series["401"][0].value);
        assert_eq!(drawn["runs"][0]["lastHour"], end);
        assert_eq!(
            drawn["runs"][0]["value"], 0,
            "end labels cannot show the bucket's peak"
        );
    }
    for (hour, id, value) in [(100, "402", 10), (101, "401", 40), (119, "402", 10)] {
        let (_, sums) = exact(
            &mut values,
            &keys,
            &history,
            &members,
            &span(hour, hour),
            &flag,
        )
        .unwrap();
        assert_eq!(
            signature(&sums, &params, hour),
            BTreeMap::from([(id.into(), value)]),
            "reached rank remains exact at {hour}"
        );
    }
}

#[test]
fn faction_dense_top_candidates_keep_ties_negative_values_and_missing_columns() {
    let flag = AtomicBool::new(false);
    let summary = |buckets: Vec<Option<ChartBucket>>| ChartSummary {
        approximate: Vec::new(),
        from: 0.0,
        to: 3.0,
        width: 2,
        buckets,
        runs: None,
    };
    let column = |hour, first, last| {
        merge(
            Some(ChartBucket::hour(hour, first, false)),
            Some(ChartBucket::hour(hour + 1, last, false)),
        )
    };
    let original = BTreeMap::from([
        ("A".into(), summary(vec![column(0, -2.0, -2.0), None])),
        (
            "B".into(),
            summary(vec![column(0, -10.0, -2.0), column(2, 3.0, 3.0)]),
        ),
        ("C".into(), summary(vec![column(0, -10.0, -3.0), None])),
        ("unknown".into(), summary(vec![None, None])),
    ]);
    let mut top = original.clone();
    retain_dense_top_candidates(&mut top, 1, &flag).unwrap();
    assert_eq!(
        top.keys().map(String::as_str).collect::<Vec<_>>(),
        ["A", "B"]
    );
    assert_eq!(
        top["B"], original["B"],
        "max equal to the largest minimum is retained"
    );
    assert!(
        top["A"].buckets[1].is_none(),
        "a missing column never becomes zero"
    );
    let mut all = original.clone();
    retain_dense_top_candidates(&mut all, u64::MAX, &flag).unwrap();
    assert_eq!(
        all.len(),
        3,
        "fewer known candidates than Top N preserves every known column"
    );
    for id in ["A", "B", "C"] {
        assert_eq!(all[id], original[id]);
    }
}
