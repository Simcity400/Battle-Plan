use super::*;
use dashboard_base::{prepared::series::tests::Fixture, test_fixtures};
use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::TableRows};
use replay_frame::frame::TableView;

fn frame(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = test_fixtures::source(1, hour, None);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|slot| CountryRow {
                id: slot,
                canonical_slot: slot,
                tag: format!("Mod agency | {slot}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=count)
            .map(|slot| CountryLinkRow {
                id: slot,
                canonical_slot: slot,
            })
            .collect(),
    ));
    *frame.tables[TableId::IntelligenceAgencies.index()] =
        TableView::Complete(TableRows::IntelligenceAgencies(
            (1..=count)
                .map(|slot| IntelligenceAgencyRow {
                    country: slot,
                    created: 1,
                    name: b"Mod literal agency".to_vec(),
                    defense_raw: 125_000,
                    active_count: 1,
                    unknown_f0: 5,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::AgencyUpgradeLevels.index()] =
        TableView::Complete(TableRows::AgencyUpgradeLevels(Vec::new()));
    *frame.tables[TableId::OperativeMemberships.index()] =
        TableView::Complete(TableRows::OperativeMemberships(
            (1..=count)
                .map(|slot| OperativeMembershipRow {
                    id: u64::from(slot),
                    roster_country: slot,
                    roster_kind: 0,
                    ..Default::default()
                })
                .collect(),
        ));
    frame.metrics.table_ns[TableId::OperativeMemberships.index()] = u64::from(hour % 24 == 0);
    frame
}

fn source(frame: DecodedFrame) -> Source {
    let replay = Replay::from_records(
        test_fixtures::CAMPAIGN.into(),
        &[test_fixtures::records(1, vec![frame])],
    )
    .unwrap();
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "prepared-agencies".into(),
            revision: "synthetic".into(),
        },
        "fixtures/agencies.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn params() -> Params {
    Params {
        selection: AgencyComparisonParams {
            branch_id: "1".into(),
            tags: Vec::new(),
            include_uncreated: true,
            top: None,
        },
        first_hour: None,
        last_hour: None,
    }
}

#[test]
fn prepared_agencies_match_two_commits_resume_append_gaps_and_identity_changes() {
    let mut fixture = Fixture::new();
    let source = source(frame(0, 2));
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let mut state = Prepared::default();
    let mut old = ScannedAgencies::default();
    for phase in 0..3 {
        let mut writer = if phase == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        for hour in phase * 48..(phase + 1) * 48 {
            let mut input = frame(hour, 2);
            if hour == 61 {
                *input.tables[TableId::IntelligenceAgencies.index()] = TableView::Unavailable {
                    reason: "synthetic agency gap".into(),
                };
            }
            if hour >= 75 {
                let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *input.tables[TableId::Countries.index()]
                else {
                    panic!("countries");
                };
                rows[1].id = 7;
                rows[1].tag = b"New mod holder".to_vec();
                let TableView::Complete(TableRows::CountryLinks(links)) =
                    &mut *input.tables[TableId::CountryLinks.index()]
                else {
                    panic!("links");
                };
                links.push(CountryLinkRow {
                    id: 7,
                    canonical_slot: 2,
                });
            }
            old.observe_frame(&input, source.replay_ref()).unwrap();
            state.observe(&source, &input).unwrap();
        }
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        let actual = query(
            &source,
            &context,
            &fixture.store,
            &fixture.key,
            1,
            &params(),
            &flag,
        )
        .unwrap()
        .unwrap();
        let expected =
            super::super::reply(&source, &params().selection, &context, &flag, &old, None).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        state = series::read_reply(&fixture.store, &fixture.key, "agency-comparison-1", 0)
            .unwrap()
            .unwrap();
        assert!(
            state
                .scan
                .tracks
                .values()
                .all(|track| track.entries.len() <= 1 && track.rosters.len() <= 1)
        );
    }
}

#[cfg(feature = "slow-tests")]
#[test]
fn prepared_agencies_keep_five_thousand_countries_and_existing_top_projection() {
    let fixture = Fixture::new();
    let source = source(frame(0, 5_000));
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let mut state = Prepared::default();
    let mut old = ScannedAgencies::default();
    for hours in [0..2, 2..4] {
        for hour in hours {
            let input = frame(hour, 5_000);
            state.observe(&source, &input).unwrap();
            old.observe_frame(&input, source.replay_ref()).unwrap();
        }
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
    }
    let mut params = params();
    params.selection.top = Some(AgencyTop {
        count: 5_000,
        field: AgencyRankField::Defense,
    });
    let actual = query(
        &source,
        &context,
        &fixture.store,
        &fixture.key,
        1,
        &params,
        &flag,
    )
    .unwrap()
    .unwrap();
    let expected =
        super::super::reply(&source, &params.selection, &context, &flag, &old, None).unwrap();
    assert_eq!(actual.series.len(), 5_000);
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[cfg(feature = "slow-tests")]
#[test]
fn prepared_agency_checkpoint_retains_changes_without_daily_dates() {
    let fixture = Fixture::new();
    let source = source(frame(0, 2));
    let flag = AtomicBool::new(false);
    let mut state = Prepared::default();
    let mut short = 0;
    for day in 0..100 {
        for hour in day * 24..(day + 1) * 24 {
            state.observe(&source, &frame(hour, 2)).unwrap();
        }
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        if day == 1 {
            short = serde_json::to_vec(&state).unwrap().len();
        }
    }
    let long = serde_json::to_vec(&state).unwrap().len();
    eprintln!("agency checkpoint bytes: 2 days={short}, 100 days={long}");
    assert!(long < short * 2);
    assert_eq!(state.scan.operative_read_hours.len(), 1);
}

#[test]
fn intel_shared_driver_agencies_equal_reads_are_flat_and_exact() {
    use crate::products::{intelligence_prepared::measured, operative_rows::CountryInputs};
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let scaled = observer_tests::scaled_work(8, |hours| {
        let first = frame(0, 2);
        let source = source(first.clone());
        let flag = AtomicBool::new(false);
        let mut fixture = Fixture::new();
        let mut state = Prepared::default();
        let mut countries = CountryInputs::default();
        let changes = FrameChangeTracker::new().observe(&first, source.replay_ref().format);
        countries.update(&first, Some(&changes));
        state.observe_changes(&source, &first, &changes, &countries)?;
        let mut old = ScannedAgencies::default();
        old.observe_frame(&first, source.replay_ref())?;
        // Oracle work is outside the measured callback.
        for offset in 1..=hours {
            let mut input = first.clone();
            input.hour += offset as i32;
            old.observe_frame(&input, source.replay_ref())?;
        }
        let delta = observer_tests::unchanged_frames(
            measured(),
            &first,
            hours as i32,
            source.replay_ref().format,
            |input, changes| {
                let restored = if input.hour == first.hour + (hours / 2) as i32 {
                    let mut writer = fixture.writer();
                    state.flush(&mut writer, 1, &flag)?;
                    writer.commit()?;
                    state =
                        series::read_reply(&fixture.store, &fixture.key, "agency-comparison-1", 0)?
                            .unwrap();
                    countries = CountryInputs::default();
                    Some(FrameChangeTracker::new().observe(input, source.replay_ref().format))
                } else {
                    None
                };
                let changes = restored.as_ref().unwrap_or(changes);
                countries.update(input, Some(changes));
                state.observe_changes(&source, input, changes, &countries)?;
                Ok(measured())
            },
        )?;
        let mut writer = fixture.append_writer();
        state.flush(&mut writer, 1, &flag)?;
        writer.commit()?;
        let context = LoadContext { content: None };
        let actual = query(
            &source,
            &context,
            &fixture.store,
            &fixture.key,
            1,
            &params(),
            &flag,
        )?
        .unwrap();
        let expected =
            super::super::reply(&source, &params().selection, &context, &flag, &old, None)?;
        observer_tests::assert_reply_parity(&actual, &expected)?;
        Ok(delta)
    })
    .unwrap();
    observer_tests::assert_flat(scaled);
    assert_eq!(scaled[0].row_visits, 4);
    assert_eq!(scaled[0].history_visits, 4);
}

#[test]
fn intel_shared_driver_agencies_one_row_ignores_ten_times_closed_holders() {
    use crate::products::intelligence_prepared::{measured, work};
    use dashboard_base::prepared::observer_tests;
    let scaled = observer_tests::scaled_work(8, |history| {
        let mut input = frame(0, 1);
        let source = source(input.clone());
        let mut state = Prepared::default();
        input.metrics.table_ns[TableId::OperativeMemberships.index()] = 0;
        for index in 0..history {
            input.hour = index as i32;
            let TableView::Complete(TableRows::Countries(rows)) =
                &mut *input.tables[TableId::Countries.index()]
            else {
                unreachable!()
            };
            rows[0].tag = format!("holder-{index}").into_bytes();
            state.observe(&source, &input)?;
        }
        assert_eq!(state.scan.tracks.len(), history);
        input.hour += 1;
        let TableView::Complete(TableRows::IntelligenceAgencies(rows)) =
            &mut *input.tables[TableId::IntelligenceAgencies.index()]
        else {
            unreachable!()
        };
        rows[0].defense_raw += 1;
        work(|work| *work = Default::default());
        state.observe(&source, &input)?;
        Ok(measured())
    })
    .unwrap();
    observer_tests::assert_flat(scaled);
    assert_eq!(scaled[0].row_visits, 1);
    assert_eq!(scaled[0].emitted, 1);
}

#[test]
#[cfg(feature = "slow-tests")]
fn first_view_keeps_all_five_thousand_agencies_and_matches_hourly_values() {
    let source = source(frame(0, 5_000));
    let context = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let mut params = params();
    params.first_hour = Some(0);
    params.last_hour = Some(0);
    let actual = first_view(&source, &params, &context, &cancelled).unwrap();
    let scanned = super::super::oracle::scan(source.replay_ref(), 1, &cancelled).unwrap();
    let expected = super::super::reply(
        &source,
        &params.selection,
        &context,
        &cancelled,
        &scanned,
        None,
    )
    .unwrap();
    assert_eq!(actual["series"].as_array().unwrap().len(), 5_000);
    assert_eq!(actual, serde_json::to_value(expected).unwrap());
}
