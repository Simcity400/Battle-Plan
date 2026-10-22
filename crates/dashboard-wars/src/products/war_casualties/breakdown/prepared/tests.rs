use super::*;
use crate::products::wars;
use dashboard_base::prepared::series::tests::Fixture;
use dashboard_base::test_fixtures::{CAMPAIGN, continuation_records, records};
use replay_data::rows::TableRows;
use replay_frame::{frame::DecodedFrame, frame::TableView};

fn source(chunks: &[Vec<DecodedFrame>]) -> Source {
    let first = records(1, chunks[0].clone());
    let base = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&first)).unwrap();
    let mut segments = vec![first];
    segments.extend(
        chunks[1..]
            .iter()
            .map(|chunk| continuation_records(&base, 1, chunk.clone())),
    );
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "prepared-breakdown".into(),
            revision: chunks.len().to_string(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &segments).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn remove_country(frame: &mut DecodedFrame, id: u32) {
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows.retain(|row| row.id != id);
    }
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *frame.tables[TableId::WarRelations.index()]
    {
        rows.retain(|row| row.first_country != id && row.second_country != id);
    }
    if let TableView::Complete(TableRows::WarOwnedRelations(rows)) =
        &mut *frame.tables[TableId::WarOwnedRelations.index()]
    {
        rows.retain(|row| row.owner != id && row.first_country != id && row.second_country != id);
        let mut ordinals = BTreeMap::new();
        for row in rows {
            let ordinal = ordinals.entry(row.owner).or_insert(0);
            row.ordinal = *ordinal;
            *ordinal += 1;
        }
    }
    if let TableView::Complete(TableRows::WarOverviewCountries(rows)) =
        &mut *frame.tables[TableId::WarOverviewCountries.index()]
    {
        rows.retain(|row| row.country != id);
    }
}

fn publish(
    source: &Source,
    chunk: &[DecodedFrame],
    fixture: &mut Fixture,
    casualties: &mut super::super::super::prepared::Prepared,
    wars: &mut wars::prepared::Observer,
    append: bool,
) {
    let flag = AtomicBool::new(false);
    for frame in chunk {
        casualties
            .observe(
                frame,
                source.replay_ref(),
                source,
                &LoadContext { content: None },
                &flag,
            )
            .unwrap();
        wars.observe(frame).unwrap();
    }
    let mut writer = if append {
        fixture.append_writer()
    } else {
        fixture.writer()
    };
    casualties
        .finish_block(&mut writer, 1, chunk[0].hour, chunk.last().unwrap().hour)
        .unwrap();
    casualties.flush(&mut writer, 1, &flag).unwrap();
    wars.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    *casualties = serde_json::from_slice(&serde_json::to_vec(casualties).unwrap()).unwrap();
    *wars = wars::prepared::Observer::restore(
        series::read_catalogue(&fixture.store, &fixture.key, "wars", 1)
            .unwrap()
            .unwrap(),
    );
}

fn assert_parity(source: &Source, fixture: &Fixture, tags: &[&str]) {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    for tag in tags {
        for window in std::iter::once(None).chain((0..source.block_windows(1).len()).map(Some)) {
            let params = CasualtyBreakdownParams {
                branch_id: "1".into(),
                tag: (*tag).into(),
                window,
            };
            let expected = CasualtyBreakdown::load(source, &params, &context, &flag).unwrap();
            let actual = query(
                source,
                &params,
                &context,
                &fixture.store,
                &fixture.key,
                &flag,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected).unwrap(),
                "{tag}, window {window:?}"
            );
        }
    }
}

#[test]
fn prepared_breakdown_matches_after_two_commits_resume_append_gaps_and_ended_enemy() {
    let mut frames: Vec<_> = [100, 101, 102, 103, 105, 106, 107]
        .into_iter()
        .map(wars::test_fixtures::recorded)
        .collect();
    for frame in &mut frames[4..] {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"mod renamed holder".to_vec();
        }
        if frame.hour >= 106 {
            remove_country(frame, 3);
        }
    }
    let chunks = vec![
        frames[..2].to_vec(),
        frames[2..4].to_vec(),
        frames[4..].to_vec(),
    ];
    let mut fixture = Fixture::new();
    let first = source(&chunks[..1]);
    let mut casualties = super::super::super::prepared::Prepared::new(first.replay_ref(), 1);
    let mut wars = wars::prepared::Observer::default();
    for ordinal in 0..chunks.len() {
        let current = source(&chunks[..=ordinal]);
        publish(
            &current,
            &chunks[ordinal],
            &mut fixture,
            &mut casualties,
            &mut wars,
            ordinal == 2,
        );
        assert_parity(
            &current,
            &fixture,
            &["ONE", "TWO", "mod renamed holder", "not recorded"],
        );
        if ordinal == 1 {
            let params = CasualtyBreakdownParams {
                branch_id: "1".into(),
                tag: "ONE".into(),
                window: Some(1),
            };
            let reply = query(
                &current,
                &params,
                &LoadContext { content: None },
                &fixture.store,
                &fixture.key,
                &AtomicBool::new(false),
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                reply.series[0].runs[0].read_hour, 100,
                "the original supplying boundary survives block commits"
            );
        }
    }
    let current = source(&chunks);
    let params = CasualtyBreakdownParams {
        branch_id: "1".into(),
        tag: "ONE".into(),
        window: Some(2),
    };
    let reply = query(
        &current,
        &params,
        &LoadContext { content: None },
        &fixture.store,
        &fixture.key,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    let ended = reply
        .series
        .iter()
        .find(|row| row.enemy_identity == "3:3")
        .unwrap();
    let run = ended
        .runs
        .iter()
        .find(|run| run.first_hour <= 107 && run.last_hour >= 107)
        .unwrap();
    assert!(!run.enemy_hourly);
    assert_eq!(run.enemy_hour, 105);
    assert_eq!(run.grouping_hour, 105);
    assert!(
        run.war_id.is_some(),
        "the typed projection retains membership before the last grouping read"
    );
}

#[test]
fn earlier_ambiguous_native_holders_refuse_a_later_unambiguous_window() {
    let mut frames: Vec<_> = (100..=105).map(wars::test_fixtures::recorded).collect();
    for frame in &mut frames {
        if let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut *frame.tables[TableId::WarRelations.index()]
        {
            rows.push(replay_data::rows::WarRelationRow {
                first_country: 2,
                second_country: 3,
                start_hour: 90,
                first_casualties: 5,
                ..Default::default()
            });
        }
        if let TableView::Complete(TableRows::WarOwnedRelations(rows)) =
            &mut *frame.tables[TableId::WarOwnedRelations.index()]
        {
            for owner in [2, 3] {
                rows.push(replay_data::rows::WarOwnedRelationRow {
                    owner,
                    ordinal: 1,
                    first_country: 2,
                    second_country: 3,
                    ..Default::default()
                });
            }
            rows.sort_by_key(|row| (row.owner, row.ordinal));
        }
    }
    for frame in &mut frames[2..] {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"renamed first holder".to_vec();
            rows[1].tag = b"ONE".to_vec();
        }
        if frame.hour >= 104 {
            remove_country(frame, 2);
        }
    }
    let chunks: Vec<_> = frames.chunks(2).map(<[DecodedFrame]>::to_vec).collect();
    let mut fixture = Fixture::new();
    let full = source(&chunks);
    let mut casualties = super::super::super::prepared::Prepared::new(full.replay_ref(), 1);
    let mut wars = wars::prepared::Observer::default();
    for chunk in &chunks {
        publish(
            &full,
            chunk,
            &mut fixture,
            &mut casualties,
            &mut wars,
            false,
        );
    }
    let params = CasualtyBreakdownParams {
        branch_id: "1".into(),
        tag: "ONE".into(),
        window: Some(2),
    };
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let expected = CasualtyBreakdown::load(&full, &params, &context, &flag).unwrap_err();
    let actual = query(
        &full,
        &params,
        &context,
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .err()
    .unwrap();
    assert_eq!(actual.to_string(), expected.to_string());
    assert_eq!(actual.to_string(), "Overlapping casualty country holders");
}

#[test]
fn replacement_holder_under_an_old_tag_keeps_native_lifetimes_separate() {
    let mut frames: Vec<_> = (100..=103).map(wars::test_fixtures::recorded).collect();
    for frame in &mut frames[2..] {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].id = 11;
            rows.sort_by_key(|row| row.id);
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            rows[0].id = 11;
            rows.sort_by_key(|row| row.id);
        }
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[0].country = 11;
            rows.sort_by_key(|row| row.country);
        }
        if let TableView::Complete(TableRows::Politics(rows)) =
            &mut *frame.tables[TableId::Politics.index()]
        {
            rows[0].country = 11;
            rows.sort_by_key(|row| row.country);
        }
        if let TableView::Complete(TableRows::LocalPlayer(rows)) =
            &mut *frame.tables[TableId::LocalPlayer.index()]
        {
            rows[0].country = 11;
        }
        if let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut *frame.tables[TableId::WarRelations.index()]
        {
            for row in rows.iter_mut() {
                row.first_country = 11;
                row.start_hour = 102;
                row.first_casualties = -17;
            }
            rows.sort_by_key(|row| (row.first_country, row.second_country));
        }
        if let TableView::Complete(TableRows::WarOwnedRelations(rows)) =
            &mut *frame.tables[TableId::WarOwnedRelations.index()]
        {
            for row in rows.iter_mut() {
                if row.owner == 1 {
                    row.owner = 11;
                }
                row.first_country = 11;
            }
            rows.sort_by_key(|row| (row.owner, row.ordinal));
        }
        if let TableView::Complete(TableRows::WarOverviewCountries(rows)) =
            &mut *frame.tables[TableId::WarOverviewCountries.index()]
        {
            rows[0].country = 11;
            rows.sort_by_key(|row| row.country);
        }
        if let TableView::Complete(TableRows::WarNameRules(rows)) =
            &mut *frame.tables[TableId::WarNameRules.index()]
        {
            rows[0].viewpoint = 11;
        }
    }
    let chunks: Vec<_> = frames.chunks(2).map(<[DecodedFrame]>::to_vec).collect();
    let full = source(&chunks);
    let mut fixture = Fixture::new();
    let mut casualties = super::super::super::prepared::Prepared::new(full.replay_ref(), 1);
    let mut wars = wars::prepared::Observer::default();
    for chunk in &chunks {
        publish(
            &full,
            chunk,
            &mut fixture,
            &mut casualties,
            &mut wars,
            false,
        );
    }
    assert_parity(&full, &fixture, &["ONE", "TWO"]);
    let reply = query(
        &full,
        &CasualtyBreakdownParams {
            branch_id: "1".into(),
            tag: "ONE".into(),
            window: Some(1),
        },
        &LoadContext { content: None },
        &fixture.store,
        &fixture.key,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    assert!(
        reply
            .series
            .iter()
            .all(|series| series.country_identity == "1:11")
    );
    assert!(
        reply
            .series
            .iter()
            .all(|series| series.runs.iter().all(|run| run.value == "-17"))
    );
}

#[test]
fn selecting_both_endpoints_keeps_the_existing_first_endpoint_priority() {
    let mut frames: Vec<_> = (100..=103).map(wars::test_fixtures::recorded).collect();
    for frame in &mut frames {
        remove_country(frame, 3);
        if frame.hour >= 102
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"first renamed".to_vec();
            rows[1].tag = b"ONE".to_vec();
        }
    }
    let chunks: Vec<_> = frames.chunks(2).map(<[DecodedFrame]>::to_vec).collect();
    let full = source(&chunks);
    let mut fixture = Fixture::new();
    let mut casualties = super::super::super::prepared::Prepared::new(full.replay_ref(), 1);
    let mut wars = wars::prepared::Observer::default();
    for chunk in &chunks {
        publish(
            &full,
            chunk,
            &mut fixture,
            &mut casualties,
            &mut wars,
            false,
        );
    }
    assert_parity(&full, &fixture, &["ONE"]);
}

#[test]
fn uncertainty_before_the_window_preserves_other_lifetimes_original_read_boundaries() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = super::super::super::Scan::new();
    let a = scan.country((1, 10));
    let b = scan.country((2, 20));
    let c = scan.country((3, 30));
    let live = BTreeMap::from([("one".into(), a), ("two".into(), b), ("three".into(), c)]);
    let mut reference = pairs::PairHistory::default();
    let mut prepared = pairs::PairHistory::default();
    let windows = vec![
        HourSpan {
            first_hour: 100,
            last_hour: 102,
        },
        HourSpan {
            first_hour: 103,
            last_hour: 103,
        },
    ];
    for hour in 100..=103 {
        scan.begin_hour(hour, &live, hour == 100, true);
        if hour >= 102 {
            scan.countries[b].at_least = true;
        }
        scan.relation(a, b, 90, [if hour == 103 { 11 } else { 10 }, 0, 0, 0]);
        scan.relation(a, c, 90, [7, 0, 0, 0]);
        scan.end_hour(true);
        reference.observe(&scan);
        prepared.observe(&scan);
        if hour >= 102 {
            let ordinal = u64::from(hour == 103);
            let mut writer = fixture.writer();
            prepared
                .publish_window(
                    &mut writer,
                    1,
                    ordinal,
                    windows[ordinal as usize],
                    &scan.observed,
                )
                .unwrap();
            writer.commit().unwrap();
        }
    }
    let wars = WarsReply {
        branch_id: "1".into(),
        available: true,
        viewpoints: Vec::new(),
        country_names: Vec::new(),
        wars: Vec::new(),
        unobserved: Vec::new(),
        capitulations_unobserved: Vec::new(),
        warnings: Vec::new(),
    };
    let selected = HashSet::from([(1, 10)]);
    let expected = project(reference.runs(), &wars, &selected, &flag, None).unwrap();
    let window: pairs::Window =
        series::read_reply(&fixture.store, &fixture.key, "casualty-pairs-1", 1)
            .unwrap()
            .unwrap();
    let actual = project(
        window
            .records
            .iter()
            .map(|(key, runs)| (key, runs.as_slice())),
        &wars,
        &selected,
        &flag,
        Some(countries(&prepared.coverage, &selected, &flag).unwrap()),
    )
    .unwrap();
    let params = CasualtyBreakdownParams {
        branch_id: "1".into(),
        tag: "one".into(),
        window: Some(1),
    };
    let expected = build_reply(
        &params,
        windows.clone(),
        &scan.observed,
        false,
        &wars,
        Some(&expected),
        &flag,
    )
    .unwrap();
    let actual = build_reply(
        &params,
        windows,
        &scan.observed,
        false,
        &wars,
        Some(&actual),
        &flag,
    )
    .unwrap();
    let unchanged = actual
        .series
        .iter()
        .find(|series| series.enemy_identity == "3:30")
        .unwrap();
    assert_eq!(unchanged.runs[0].read_hour, 102);
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn coverage_joins_publications_and_skips_unobserved_intervals() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut coverage = series::Accumulator::default();
    for hour in [100, 101, 103, 104, 105] {
        coverage.observe(0, hour, Some(1), false);
        let mut writer = fixture.writer();
        coverage.flush(&mut writer, "coverage", 0, &flag).unwrap();
        writer.commit().unwrap();
    }
    let mut reader = Reader::new(&fixture.store, &fixture.key, "coverage".into(), &flag).unwrap();
    assert_eq!(
        observed(&mut reader, (99, 107)).unwrap(),
        [
            HourSpan {
                first_hour: 100,
                last_hour: 101
            },
            HourSpan {
                first_hour: 103,
                last_hour: 105
            }
        ]
    );
}

#[test]
fn unprepared_inputs_remain_pending_and_complete_empty_relations_are_available() {
    let mut frame = wars::test_fixtures::recorded(100);
    frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(Vec::new())).into();
    frame.tables[TableId::WarOwnedRelations.index()] =
        TableView::Complete(TableRows::WarOwnedRelations(Vec::new())).into();
    let chunks = vec![vec![frame]];
    let full = source(&chunks);
    let mut fixture = Fixture::new();
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let mut params = CasualtyBreakdownParams {
        branch_id: "1".into(),
        tag: "ONE".into(),
        window: None,
    };
    assert!(
        query(
            &full,
            &params,
            &context,
            &fixture.store,
            &fixture.key,
            &flag
        )
        .unwrap()
        .is_none()
    );
    let mut casualties = super::super::super::prepared::Prepared::new(full.replay_ref(), 1);
    let mut wars = wars::prepared::Observer::default();
    publish(
        &full,
        &chunks[0],
        &mut fixture,
        &mut casualties,
        &mut wars,
        false,
    );
    let reply = query(
        &full,
        &params,
        &context,
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .unwrap()
    .unwrap();
    assert!(reply.available);
    assert!(reply.series.is_empty());
    assert_eq!(
        reply.observed,
        [HourSpan {
            first_hour: 100,
            last_hour: 100
        }]
    );
    params.window = Some(4);
    assert!(
        query(
            &full,
            &params,
            &context,
            &fixture.store,
            &fixture.key,
            &flag
        )
        .is_err()
    );
    assert_parity(&full, &fixture, &["ONE"]);
}

/// While the joint prefix ends before the requested window, the window
/// answers from the prefix's newest window, as that window's own reply
/// gives it, instead of waiting; observation stops at the prefix's end.
#[test]
fn a_window_past_the_prepared_prefix_answers_the_prefix_newest_window() {
    let frames: Vec<_> = (100..=103).map(wars::test_fixtures::recorded).collect();
    let chunks = vec![frames[..2].to_vec(), frames[2..].to_vec()];
    let full = source(&chunks);
    let mut fixture = Fixture::new();
    let mut casualties = super::super::super::prepared::Prepared::new(full.replay_ref(), 1);
    let mut wars = wars::prepared::Observer::default();
    publish(
        &full,
        &chunks[0],
        &mut fixture,
        &mut casualties,
        &mut wars,
        false,
    );
    let through = chunks[0].last().unwrap().hour;
    let ask = |window, through| {
        let params = CasualtyBreakdownParams {
            branch_id: "1".into(),
            tag: "ONE".into(),
            window,
        };
        let context = LoadContext { content: None };
        let flag = AtomicBool::new(false);
        query_through(
            &full,
            &params,
            &context,
            &fixture.store,
            &fixture.key,
            through,
            &flag,
        )
        .unwrap()
    };
    assert!(ask(Some(1), None).is_none(), "an unpublished window waits");
    let newest = ask(Some(0), None).unwrap();
    let ahead = ask(Some(1), Some(through)).unwrap();
    assert_eq!(ahead.window, Some(0));
    assert!(!ahead.series.is_empty());
    assert_eq!(
        serde_json::to_value(&ahead).unwrap(),
        serde_json::to_value(&newest).unwrap()
    );
    let manifest = ask(None, Some(through)).unwrap();
    assert!(!manifest.observed.is_empty());
    assert!(
        manifest
            .observed
            .iter()
            .all(|span| span.last_hour <= through)
    );
}
