use super::*;
use replay::{
    frame::{DecodedFrame, TableView},
    rows::{
        CountryCollaborationRow, CountryLinkRow, CountryRow, StateGarrisonRow, StateResistanceRow,
        StateRow, TableId, TableRows,
    },
};

#[test]
fn occupation_trend_manual_roots_clip_aliases_to_all_selected_native_holders() {
    let mut histories = RecordedHistories::default();
    for (tag, entries) in [
        ("root", vec![(100, 101, 7, 11), (104, 105, 8, 22)]),
        ("alias", vec![(98, 99, 7, 11), (102, 103, 9, 33)]),
        ("before its measure", vec![(96, 97, 7, 11)]),
    ] {
        let mut recorded = country_history::RecordedCountry::default();
        recorded.identities = std::sync::Arc::new(
            entries
                .into_iter()
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
        histories.countries.insert(tag.into(), recorded);
    }
    let run = OccupationTrendRun {
        first_hour: 96,
        last_hour: 105,
        read_hour: 96,
        starts_segment: true,
        resistance_raw: "12345".into(),
        compliance_raw: "0".into(),
        states: 1,
        held_states: 1,
        held_complete: true,
        collaboration: vec![],
    };
    let history = History {
        series: BTreeMap::from([
            ("root".into(), vec![run.clone()]),
            ("alias".into(), vec![run]),
        ]),
        ..Default::default()
    };
    let params = OccupationTrendParams {
        branch_id: "1".into(),
        tags: vec!["root".into()],
        top: None,
        collaboration: false,
    };
    let reply = reply_with_histories(
        "1",
        &history,
        &params,
        vec![],
        &AtomicBool::new(false),
        Some(&histories),
    )
    .unwrap();
    let alias = reply
        .series
        .iter()
        .find(|series| series.tag == "alias")
        .unwrap();
    assert_eq!(
        alias
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(98, 99)]
    );
    assert_eq!(
        alias
            .identities
            .as_ref()
            .unwrap()
            .iter()
            .map(|run| run.country_id)
            .collect::<Vec<_>>(),
        [11]
    );
    assert_eq!(
        reply
            .series
            .iter()
            .find(|series| series.tag == "root")
            .unwrap()
            .identities
            .as_ref()
            .unwrap()
            .len(),
        2
    );
    let before = reply
        .series
        .iter()
        .find(|series| series.tag == "before its measure")
        .unwrap();
    assert!(before.runs.is_empty());
    assert_eq!(before.identities.as_ref().unwrap()[0].first_hour, 96);
    assert!(
        reply_with_histories(
            "1",
            &history,
            &params,
            vec!["x".repeat(MAX_PRODUCT_REPLY_BYTES)],
            &AtomicBool::new(false),
            Some(&histories)
        )
        .is_err()
    );
}

fn frame(hour: i32, values: &[(u32, i64, i64)], held: &[(u32, u32)]) -> DecodedFrame {
    let mut frame = dashboard_base::test_fixtures::source(1, hour, None);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=3)
            .map(|id| CountryRow {
                id,
                canonical_slot: id + 10,
                tag: format!("Opaque country {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=3)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id + 10,
            })
            .collect(),
    ));
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        held.iter()
            .map(|&(id, controller)| StateRow {
                id,
                controller,
                owner: controller,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(
            values
                .iter()
                .map(|&(state, level, compliance)| StateResistanceRow {
                    state,
                    country: 3,
                    active: 1,
                    level,
                    compliance,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::StateGarrisons.index()] =
        TableView::Complete(TableRows::StateGarrisons(
            held.iter()
                .map(|&(state, controller)| StateGarrisonRow {
                    controller,
                    occupied: 3,
                    state,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::CountryCollaboration.index()] =
        TableView::Complete(TableRows::CountryCollaboration(vec![
            CountryCollaborationRow {
                source: 3,
                target: 1,
                progress: 12345,
            },
        ]));
    frame.metrics.table_ns[TableId::StateResistance.index()] = 1;
    frame.metrics.table_ns[TableId::StateGarrisons.index()] = 1;
    frame.metrics.table_ns[TableId::CountryCollaboration.index()] = 1;
    frame
}

fn link_table(links: &[(u32, u32)]) -> TableView {
    TableView::Complete(TableRows::CountryLinks(
        links
            .iter()
            .map(|&(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect(),
    ))
}

fn scan(frames: Vec<DecodedFrame>) -> History {
    let replay = dashboard_base::test_fixtures::replay(frames);
    occupied_states::scan_history(&replay, 1, None, &AtomicBool::new(false))
        .unwrap()
        .trend
}

#[test]
fn occupation_trend_gain_loss_unread_states_and_empty_occupier_are_honest() {
    let first = frame(100, &[(1, 1, 10), (2, 2, 20)], &[(1, 1), (2, 1)]);
    let gained = frame(
        101,
        &[(1, 1, 10), (2, 2, 20), (3, 9, 90)],
        &[(1, 1), (2, 1), (3, 1)],
    );
    let mut partial = frame(102, &[(1, 1, 10)], &[(1, 1), (2, 1), (3, 1)]);
    let rows = partial
        .complete::<replay::rows::tables::StateResistance>()
        .unwrap()
        .to_vec();
    *partial.tables[TableId::StateResistance.index()] = TableView::Truncated {
        rows: TableRows::StateResistance(rows),
        missing: 2,
    };
    let lost = frame(103, &[(2, 2, 20)], &[(2, 2)]);
    let empty = frame(104, &[], &[]);
    let history = scan(vec![first, gained, partial, lost, empty]);
    let a = &history.series["Opaque country 1"];
    assert_eq!(
        a.iter()
            .map(|run| (
                run.resistance_raw.as_str(),
                run.states,
                run.held_states,
                run.held_complete
            ))
            .collect::<Vec<_>>(),
        [("1", 2, 2, true), ("4", 3, 3, true), ("1", 1, 3, true)]
    );
    assert_eq!(a[0].collaboration[0].raw.as_deref(), Some("12345"));
    assert_eq!(a[0].collaboration[0].read_hour, Some(100));
    let b = &history.series["Opaque country 2"];
    assert_eq!(b.len(), 1);
    assert_eq!((b[0].first_hour, b[0].last_hour), (103, 103));
    assert_eq!(b[0].collaboration[0].raw, None); // never synthesize native missing pair zero
}

#[test]
fn occupation_trend_dates_reads_gaps_and_disappearing_occupiers() {
    let first = frame(100, &[(1, 3000000, 9000000)], &[(1, 1)]);
    let mut carried = first.clone();
    carried.hour = 101;
    carried.metrics.table_ns.fill(0);
    let mut disappeared = carried.clone();
    disappeared.hour = 102;
    *disappeared.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![]));
    let mut returned = first.clone();
    returned.hour = 103;
    if let TableView::Complete(TableRows::Countries(countries)) =
        &mut *returned.tables[TableId::Countries.index()]
    {
        countries[0].id = 7; // same tag, different native identity
        countries.sort_unstable_by_key(|country| country.id);
    }
    *returned.tables[TableId::CountryLinks.index()] =
        link_table(&[(1, 11), (2, 12), (3, 13), (7, 11)]);
    let mut gap = returned.clone();
    gap.hour = 105;
    gap.metrics.table_ns.fill(0);
    let mut fresh = returned.clone();
    fresh.hour = 106;
    let history = scan(vec![first, carried, disappeared, returned, gap, fresh]);
    let runs = &history.series["Opaque country 1"];
    assert_eq!(
        runs.iter()
            .map(|run| (run.first_hour, run.last_hour, run.read_hour))
            .collect::<Vec<_>>(),
        [(100, 101, 100), (103, 103, 103), (106, 106, 106)]
    );
    assert!(runs.iter().all(|run| run.starts_segment));
    assert_eq!(runs[2].first_hour, 106);
}

#[test]
fn occupation_trend_held_collaboration_keeps_its_native_actor_after_tag_reuse() {
    let first = frame(100, &[(1, 3000000, 9000000)], &[(1, 1)]);
    let mut reused = first.clone();
    reused.hour = 101;
    reused.metrics.table_ns.fill(0);
    if let TableView::Complete(TableRows::Countries(countries)) =
        &mut *reused.tables[TableId::Countries.index()]
    {
        countries[2].id = 7;
    }
    *reused.tables[TableId::CountryLinks.index()] =
        link_table(&[(1, 11), (2, 12), (3, 13), (7, 13)]);
    let history = scan(vec![first, reused]);
    let runs = &history.series["Opaque country 1"];
    assert_eq!(runs.len(), 2);
    let initial = &runs[0].collaboration[0];
    let held = &runs[1].collaboration[0];
    assert_eq!(initial.actor.as_ref().unwrap().name_hour, Some(100));
    assert_eq!(held.tag.as_deref(), Some("Opaque country 3"));
    assert_eq!(held.raw.as_deref(), Some("12345"));
    assert_eq!(held.read_hour, Some(100));
    assert_eq!(
        held.actor.as_ref().unwrap(),
        &OccupationCollaborationActor {
            slot: 13,
            country_id: 3,
            name_hour: None,
        }
    );
    assert_eq!(runs[1].read_hour, 100);
    assert_eq!(runs[1].resistance_raw, runs[0].resistance_raw);
    assert!(
        history
            .collaboration_warnings
            .iter()
            .any(|warning| warning
                .contains("occupied native country naming identity is unavailable"))
    );
}

#[test]
fn occupation_trend_unnamed_collaboration_retains_the_numeric_row() {
    let mut first = frame(100, &[(1, 10, 20)], &[(1, 1)]);
    if let TableView::Complete(TableRows::Countries(countries)) =
        &mut *first.tables[TableId::Countries.index()]
    {
        countries[2].tag.clear();
    }
    let history = scan(vec![first]);
    let collaboration = &history.series["Opaque country 1"][0].collaboration;
    assert_eq!(collaboration.len(), 1);
    assert_eq!(collaboration[0].tag, None);
    assert_eq!(collaboration[0].raw.as_deref(), Some("12345"));
    assert_eq!(collaboration[0].read_hour, Some(100));
    assert_eq!(
        collaboration[0].actor.as_ref().unwrap(),
        &OccupationCollaborationActor {
            slot: 13,
            country_id: 3,
            name_hour: None,
        }
    );
}

#[test]
fn occupation_trend_all_5000_countries_top_ranking_and_budget() {
    let mut first = dashboard_base::test_fixtures::scale_frame(1, 100, 5000);
    *first.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(
            (0..5000)
                .map(|index| StateResistanceRow {
                    state: index + 1,
                    country: 100000 + index,
                    active: 1,
                    level: i64::from(index),
                    ..Default::default()
                })
                .collect(),
        ));
    *first.tables[TableId::StateGarrisons.index()] =
        TableView::Complete(TableRows::StateGarrisons(
            (0..5000)
                .map(|index| StateGarrisonRow {
                    state: index + 1,
                    controller: 100000 + index,
                    occupied: 100000 + index,
                    ..Default::default()
                })
                .collect(),
        ));
    first.metrics.table_ns[TableId::StateResistance.index()] = 1;
    first.metrics.table_ns[TableId::StateGarrisons.index()] = 1;
    let history = scan(vec![first]);
    let mut params = OccupationTrendParams {
        branch_id: "1".into(),
        tags: vec![],
        top: None,
        collaboration: false,
    };
    let all = reply("1", &history, &params, vec![], &AtomicBool::new(false)).unwrap();
    assert_eq!(all.series.len(), 5000);
    assert!(all.series.iter().all(|series| series.runs[0].held_complete));
    params.tags = history.series.keys().cloned().collect();
    assert!(OccupationTrend::validate(&params).is_ok());
    let chosen = reply("1", &history, &params, vec![], &AtomicBool::new(false)).unwrap();
    assert_eq!(chosen.series.len(), 5000);
    params.tags.clear();
    params.top = Some(OccupationTop {
        count: 10,
        measure: OccupationMeasure::Resistance,
    });
    let top = reply("1", &history, &params, vec![], &AtomicBool::new(false)).unwrap();
    assert_eq!(top.series.len(), 10);
    assert!(
        top.series
            .iter()
            .all(|series| series.runs[0].resistance_raw.parse::<i64>().unwrap() >= 4990)
    );
    assert_eq!(
        reply(
            "1",
            &history,
            &params,
            vec!["x".repeat(MAX_PRODUCT_REPLY_BYTES)],
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .to_string(),
        "Occupation trend reply exceeds MAX_PRODUCT_REPLY_BYTES"
    );
    assert!(reply("1", &history, &params, vec![], &AtomicBool::new(true)).is_err());
}

#[test]
fn occupation_trend_request_bounds_are_uncapped_with_shared_tag_validation() {
    let mut params = OccupationTrendParams {
        branch_id: "1".into(),
        tags: vec![],
        top: None,
        collaboration: false,
    };
    assert!(OccupationTrend::validate(&params).is_ok());
    params.tags = (0..5000)
        .map(|id| format!("Opaque space tag {id}"))
        .collect();
    assert!(OccupationTrend::validate(&params).is_ok());
    params.tags.push("extra".into());
    assert!(OccupationTrend::validate(&params).is_ok());
    params.tags.clear();
    params.top = Some(OccupationTop {
        count: 0,
        measure: OccupationMeasure::Resistance,
    });
    assert!(OccupationTrend::validate(&params).is_err());
    params.top.as_mut().unwrap().count = u32::MAX;
    assert!(OccupationTrend::validate(&params).is_ok());
    params.tags.push("chosen".into());
    assert!(OccupationTrend::validate(&params).is_err());
}

#[test]
fn occupation_trend_requires_matching_inventory_read_and_fresh_returning_country_read() {
    let first = frame(100, &[(1, 10, 20)], &[(1, 1), (2, 1)]);
    let mut unread_inventory = first.clone();
    unread_inventory.hour = 101;
    *unread_inventory.tables[TableId::StateGarrisons.index()] = TableView::Unavailable {
        reason: "synthetic inventory read failure".into(),
    };
    let mut absent = unread_inventory.clone();
    absent.hour = 102;
    absent.metrics.table_ns.fill(0);
    *absent.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![]));
    let mut returned = first.clone();
    returned.hour = 103;
    returned.metrics.table_ns.fill(0);
    let mut fresh = first.clone();
    fresh.hour = 104;
    let history = scan(vec![first, unread_inventory, absent, returned, fresh]);
    let runs = &history.series["Opaque country 1"];
    assert_eq!(runs[0].held_states, 2);
    assert!(runs[0].held_complete);
    assert_eq!(runs[1].held_states, 1);
    assert!(!runs[1].held_complete);
    assert_eq!(
        runs.iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (101, 101), (104, 104)]
    );
}

#[test]
fn occupation_trend_controller_changes_wait_for_the_supplying_read() {
    let first = frame(100, &[(1, 10, 20)], &[(1, 1)]);
    let mut moved = frame(101, &[(1, 10, 20)], &[(1, 2)]);
    moved.metrics.table_ns.fill(0);
    let fresh = frame(102, &[(1, 30, 40)], &[(1, 2)]);
    let history = scan(vec![first, moved, fresh]);
    let a = &history.series["Opaque country 1"];
    assert_eq!(
        (a[0].first_hour, a[0].last_hour, a[0].read_hour),
        (100, 101, 100)
    );
    let b = &history.series["Opaque country 2"];
    assert_eq!(
        (
            b[0].first_hour,
            b[0].read_hour,
            b[0].resistance_raw.as_str()
        ),
        (102, 102, "30")
    );
}

#[test]
fn occupation_trend_refusals_are_named_counted_and_do_not_change_occupied_states() {
    let mut first = frame(100, &[(1, i64::MAX, 20)], &[(1, 1)]);
    *first.tables[TableId::CountryCollaboration.index()] =
        TableView::Complete(TableRows::CountryCollaboration(vec![
            CountryCollaborationRow {
                source: 999,
                target: 1,
                progress: 12345,
            },
        ]));
    let replay = dashboard_base::test_fixtures::replay(vec![first]);
    let shared = occupied_states::scan_history(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    assert!(shared.trend.series.is_empty());
    assert_eq!(
        shared.states.states[0].runs[0].value.resistance_raw,
        i64::MAX
    );
    assert_eq!(
        shared.trend.warnings,
        [
            "1 recorded occupation trend facts not shown: native occupation mean is non-finite or not representable; country slot 11, read 100"
        ]
    );
    assert_eq!(
        shared.trend.collaboration_warnings,
        [
            "1 recorded occupation trend facts not shown: CountryCollaboration link is unresolved; 999 → 1"
        ]
    );
    assert!(
        !shared
            .states
            .warnings
            .iter()
            .any(|warning| warning.contains("CountryCollaboration"))
    );
}

#[test]
fn occupation_trend_incomplete_membership_is_local_to_its_occupier() {
    let mut first = frame(100, &[(1, 10, 20), (2, 30, 40)], &[(1, 1), (2, 2)]);
    if let TableView::Complete(TableRows::StateGarrisons(rows)) =
        &mut *first.tables[TableId::StateGarrisons.index()]
    {
        rows.retain(|row| row.controller != 1);
    }
    let history = scan(vec![first]);
    assert!(!history.series["Opaque country 1"][0].held_complete);
    assert!(history.series["Opaque country 2"][0].held_complete);
    assert_eq!(history.series["Opaque country 2"][0].held_states, 1);
}

#[test]
fn occupation_trend_unresolved_controller_leaves_every_held_denominator_unknown() {
    let first = frame(100, &[(1, 10, 20), (2, 30, 40)], &[(1, 1), (2, 999)]);
    let history = scan(vec![first]);
    assert!(!history.series["Opaque country 1"][0].held_complete);
    assert!(history.warnings.iter().any(|line| line == "1 recorded occupation trend facts not shown: held controller link is unresolved; state 2, raw link 999"));
}

#[test]
fn occupation_trend_unnamed_countries_are_counted_instead_of_silently_dropped() {
    let mut first = frame(100, &[(1, 10, 20)], &[(1, 1)]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *first.tables[TableId::Countries.index()]
    {
        rows[0].tag.clear();
        rows[1].tag = b"Invalid\0tag".to_vec();
        rows[2].tag = vec![0xff];
    }
    let history = scan(vec![first]);
    assert!(history.series.is_empty());
    assert_eq!(
        history.warnings,
        [
            "3 recorded occupation country identities not shown: country tag is empty, contains NUL or is not UTF-8; country 1, country 2, country 3"
        ]
    );
}

#[test]
fn occupation_trend_repeated_tags_name_the_refusal_in_both_occupation_products() {
    let mut first = frame(100, &[(1, 10, 20), (2, 30, 40)], &[(1, 1), (2, 2)]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *first.tables[TableId::Countries.index()]
    {
        rows[1].tag = rows[0].tag.clone();
    }
    let mut changed_owner = first.clone();
    changed_owner.hour = 101;
    changed_owner.metrics.table_ns.fill(0);
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *changed_owner.tables[TableId::States.index()]
    {
        rows[0].owner = 2;
    }
    let replay = dashboard_base::test_fixtures::replay(vec![first, changed_owner]);
    let shared = occupied_states::scan_history(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(shared.trend.series.len(), 1);
    assert_eq!(
        shared.trend.series["Opaque country 1"][0].resistance_raw,
        "10"
    );
    assert_eq!(
        shared.trend.warnings,
        [
            "1 recorded occupation country identities not shown: country tag repeats an earlier country; country 2"
        ]
    );
    let second = shared
        .states
        .states
        .iter()
        .find(|series| series.state == 2)
        .unwrap();
    assert!(second.runs[0].value.owner.is_none());
    assert!(second.runs[0].value.occupier.is_none());
    let first = shared
        .states
        .states
        .iter()
        .find(|series| series.state == 1)
        .unwrap();
    assert_eq!(
        first.runs[0].value.owner.as_deref(),
        Some("Opaque country 1")
    );
    assert!(first.runs[1].value.owner.is_none());
    assert!(shared.states.warnings.iter().any(|line| line == "2 recorded occupation facts not shown: country tag repeats an earlier country; state 1, country 2, state 2, country 2"));
    assert!(
        !shared
            .states
            .warnings
            .iter()
            .any(|line| line.contains("cannot resolve through Countries and CountryLinks"))
    );
}

#[test]
fn occupation_trend_unresolved_occupied_links_do_not_change_held_coverage() {
    for resistance_unresolved in [false, true] {
        let mut first = frame(100, &[(1, 10, 20)], &[(1, 1)]);
        if let TableView::Complete(TableRows::StateGarrisons(rows)) =
            &mut *first.tables[TableId::StateGarrisons.index()]
        {
            rows[0].occupied = 999;
        }
        if resistance_unresolved
            && let TableView::Complete(TableRows::StateResistance(rows)) =
                &mut *first.tables[TableId::StateResistance.index()]
        {
            rows[0].country = 999;
        }
        let history = scan(vec![first]);
        let run = &history.series["Opaque country 1"][0];
        assert_eq!(
            (run.states, run.held_states, run.held_complete),
            (1, 1, true)
        );
        assert_eq!(
            (run.resistance_raw.as_str(), run.compliance_raw.as_str()),
            ("10", "20")
        );
        assert!(history.warnings.is_empty());
        let mut expected = Vec::new();
        if resistance_unresolved {
            expected.push("1 recorded occupation trend facts not shown: active resistance occupied-country link is unresolved; state 1, raw link 999");
        }
        expected.push("1 recorded occupation trend facts not shown: held occupied-country link is unresolved; state 1, raw link 999");
        assert_eq!(history.collaboration_warnings, expected);
    }
}

#[test]
fn occupation_trend_collaboration_is_only_shipped_and_split_when_requested() {
    let first = frame(100, &[(1, 10, 20)], &[(1, 1)]);
    let mut renamed_occupied = first.clone();
    renamed_occupied.hour = 101;
    renamed_occupied.metrics.table_ns.fill(0);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed_occupied.tables[TableId::Countries.index()]
    {
        rows[2].tag = b"Changed occupied tag".to_vec();
    }
    let history = scan(vec![first, renamed_occupied]);
    let mut params = OccupationTrendParams {
        branch_id: "1".into(),
        tags: vec![],
        top: None,
        collaboration: false,
    };
    let without = reply("1", &history, &params, vec![], &AtomicBool::new(false)).unwrap();
    assert_eq!(without.series[0].runs.len(), 1);
    assert_eq!(
        (
            without.series[0].runs[0].first_hour,
            without.series[0].runs[0].last_hour
        ),
        (100, 101)
    );
    assert!(without.series[0].runs[0].collaboration.is_empty());
    params.collaboration = true;
    let with = reply("1", &history, &params, vec![], &AtomicBool::new(false)).unwrap();
    assert_eq!(with.series[0].runs.len(), 2);
    assert_eq!(
        with.series[0].runs[0].collaboration[0].tag.as_deref(),
        Some("Opaque country 3")
    );
    assert_eq!(
        with.series[0].runs[1].collaboration[0].tag.as_deref(),
        Some("Changed occupied tag")
    );
    assert_eq!(with.series[0].runs[1].collaboration[0].read_hour, Some(100));
}

#[test]
fn occupation_trend_name_changes_keep_the_same_series_and_hour_specific_looks() {
    use dashboard_base::products::country_styles::HourStyles;
    use replay::rows::{CountryAppearanceRow, PoliticsRow};
    let mut first = frame(100, &[(1, 10, 20)], &[(1, 1)]);
    *first.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 1,
            cosmetic_tag: b"Before name".to_vec(),
            ..Default::default()
        }]));
    *first.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![PoliticsRow {
            country: 1,
            ..Default::default()
        }]));
    let mut renamed = first.clone();
    renamed.hour = 101;
    renamed.metrics.table_ns.fill(0);
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *renamed.tables[TableId::CountryAppearance.index()]
    {
        rows[0].cosmetic_tag = b"After name".to_vec();
    }
    let localisation = [
        ("Before name".into(), "First display name".into()),
        ("After name".into(), "Renamed display name".into()),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        HourStyles::read(&first, Some(&localisation))
            .unwrap()
            .get("Opaque country 1")
            .unwrap()
            .name
            .as_deref(),
        Some("First display name")
    );
    assert_eq!(
        HourStyles::read(&renamed, Some(&localisation))
            .unwrap()
            .get("Opaque country 1")
            .unwrap()
            .name
            .as_deref(),
        Some("Renamed display name")
    );
    let history = scan(vec![first, renamed]);
    assert_eq!(history.series.len(), 1);
    let runs = &history.series["Opaque country 1"];
    assert_eq!(runs.len(), 1);
    assert_eq!(
        (runs[0].first_hour, runs[0].last_hour, runs[0].read_hour),
        (100, 101, 100)
    );
}

#[test]
fn occupation_trend_diagnostics_exclude_state_laws_and_disabled_collaboration() {
    let mut first = frame(100, &[(1, 10, 20)], &[(1, 1)]);
    *first.tables[TableId::CountryCollaboration.index()] =
        TableView::Complete(TableRows::CountryCollaboration(vec![
            CountryCollaborationRow {
                source: 999,
                target: 1,
                progress: 12345,
            },
        ]));
    let replay = std::sync::Arc::new(dashboard_base::test_fixtures::replay(vec![first]));
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "occupation".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let context = LoadContext { content: None };
    let mut params = OccupationTrendParams {
        branch_id: "1".into(),
        tags: vec![],
        top: None,
        collaboration: false,
    };
    let without =
        OccupationTrend::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    assert!(without.warnings.is_empty());
    params.collaboration = true;
    let with = OccupationTrend::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        with.warnings,
        [
            "1 recorded occupation trend facts not shown: CountryCollaboration link is unresolved; 999 → 1"
        ]
    );
}
