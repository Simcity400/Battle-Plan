use super::test_fixtures::{frame, outcome_frames, reference, set};
use super::*;
use dashboard_base::prepared::frame_changes::FrameChangeTracker;
use dashboard_base::prepared::series::tests::Fixture;
use dashboard_core::test_fixtures::{CAMPAIGN, records};
use replay_data::{
    rows::LandCombatDetailsRow, rows::LandCombatResultsRow, rows::LandCombatSideDetails,
    rows::LandResultSide, rows::OrdersGroupRow, rows::Recorded, rows::TableRows,
};
use replay_frame::frame::TableView;

fn replay(frames: &[DecodedFrame]) -> Replay {
    Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap()
}
fn publish(state: &mut Prepared, fixture: &Fixture, cancelled: &AtomicBool) {
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, None, &[], cancelled).unwrap();
    writer.commit().unwrap();
}
fn resume(fixture: &Fixture) -> Prepared {
    let (keys, metadata) =
        series::read_catalogue::<Metadata>(&fixture.store, &fixture.key, "unit-leaders", 1)
            .unwrap()
            .unwrap();
    Prepared::read(
        fixture
            .store
            .read(&fixture.key, "unit-leaders-1", 0)
            .unwrap()
            .unwrap(),
        keys,
        metadata,
    )
    .unwrap()
}
fn observe(state: &mut Prepared, replay: &Replay, frames: &[DecodedFrame]) {
    let mut tracker = FrameChangeTracker::new();
    for frame in frames {
        let changes = tracker.observe(frame, replay.format);
        state.observe(frame, replay, None, &changes).unwrap();
    }
}
fn assert_parity(fixture: &Fixture, replay: &Replay) {
    let cancelled = AtomicBool::new(false);
    let full = scan(replay, &full_params("1"), None, &cancelled).unwrap();
    let history = dashboard_base::products::country_history::scan_histories(
        replay,
        1,
        false,
        &[],
        &cancelled,
    )
    .unwrap();
    for params in [
        UnitLeadersParams {
            view: LeaderView::Leaderboard,
            ..full_params("1")
        },
        UnitLeadersParams {
            selection: LeaderSelection::Leader {
                reference: reference(0).to_string(),
            },
            ..full_params("1")
        },
        UnitLeadersParams {
            selection: LeaderSelection::Countries {
                tags: vec![String::from_utf8(dashboard_core::test_fixtures::scale_tag(0)).unwrap()],
            },
            view: LeaderView::Leaderboard,
            ..full_params("1")
        },
    ] {
        let actual = query(
            &fixture.store,
            &fixture.key,
            &params,
            Some(&history),
            &cancelled,
        )
        .unwrap()
        .unwrap();
        dashboard_base::prepared::observer_tests::assert_reply_parity(
            &actual,
            &project(&full, &params, Some(&history)),
        )
        .unwrap();
    }
    for params in [
        UnitLeadersParams {
            view: LeaderView::Choices,
            ..full_params("1")
        },
        UnitLeadersParams {
            selection: LeaderSelection::Countries {
                tags: vec![String::from_utf8(dashboard_core::test_fixtures::scale_tag(0)).unwrap()],
            },
            view: LeaderView::Choices,
            ..full_params("1")
        },
        UnitLeadersParams {
            selection: LeaderSelection::All,
            leaders: LeaderKinds::Land,
            view: LeaderView::Choices,
            ..full_params("1")
        },
    ] {
        let actual = choices(
            &fixture.store,
            &fixture.key,
            &params,
            Some(&history),
            &cancelled,
        )
        .unwrap()
        .unwrap();
        assert_eq!(actual, choices_from(&full, &params, Some(&history)));
    }
}

#[test]
fn active_refusals_continue_identically_after_serialized_save_and_resume() {
    let frames: Vec<_> = (100..=109)
        .map(|hour| {
            let mut input = frame(hour, 2);
            if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *input.tables[TableId::UnitLeaders.index()]
            {
                rows[1].owner = if hour < 104 { u32::MAX } else { 0 };
                if hour >= 105 {
                    rows[1].kind = 2;
                }
                if hour == 107 {
                    rows.pop();
                }
            }
            input
        })
        .collect();
    let replay = replay(&frames);
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut states = [Prepared::new(&replay, 1), Prepared::new(&replay, 1)];
    let cancelled = AtomicBool::new(false);
    for (state, fixture) in states.iter_mut().zip(&fixtures) {
        observe(state, &replay, &frames[..3]);
        assert_eq!(state.scan.withheld_active.len(), 1);
        publish(state, fixture, &cancelled);
    }
    states[1] = resume(&fixtures[1]);
    assert_eq!(
        states[1].scan.withheld_active,
        states[0].scan.withheld_active
    );
    assert_eq!(states[1].scan.reply.withheld, states[0].scan.reply.withheld);
    for (state, fixture) in states.iter_mut().zip(&fixtures) {
        observe(state, &replay, &frames[3..]);
        publish(state, fixture, &cancelled);
        assert_parity(fixture, &replay);
    }
    dashboard_base::prepared::series::tests::assert_payloads_equal(
        &fixtures[1].payloads(),
        &fixtures[0].payloads(),
        "active leader refusals after serialized resume",
    );
}

#[test]
fn prepared_leaders_match_projection_after_two_commits_resume_gaps_and_identity_changes() {
    let mut frames: Vec<_> = [100, 101, 103, 104]
        .into_iter()
        .map(|hour| frame(hour, 2))
        .collect();
    for (index, frame) in frames.iter_mut().enumerate() {
        if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *frame.tables[TableId::UnitLeaders.index()]
        {
            rows[0].components[0] += index as i32;
            rows[0].skill_level += index as i32;
            if index >= 2 {
                rows[0].owner = 100_001;
                rows[0].category = 0;
            }
        }
        if index == 1 {
            *frame.tables[TableId::UnitLeaderTraits.index()] = TableView::Unavailable {
                reason: "Synthetic missing traits".into(),
            };
        } else {
            let tokens: &[&str] = match index {
                0 => &["synthetic_trait_a"],
                2 => &["synthetic_trait_a", "synthetic_trait_b"],
                _ => &["synthetic_trait_b"],
            };
            set(
                frame,
                TableRows::UnitLeaderTraits(
                    tokens
                        .iter()
                        .enumerate()
                        .map(|(ordinal, token)| UnitLeaderTraitRow {
                            leader_reference: reference(0),
                            ordinal: ordinal as u32,
                            token: token.as_bytes().to_vec(),
                            ..Default::default()
                        })
                        .collect(),
                ),
            );
        }
        if index >= 2
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].id += 500;
            rows.sort_by_key(|row| row.id);
        }
        set(
            frame,
            TableRows::OrdersGroups(vec![OrdersGroupRow {
                handle: 2,
                leader: reference(0),
                name: Recorded::Value(Some(b"Synthetic command".to_vec())),
                ..Default::default()
            }]),
        );
    }
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut state = Prepared::new(&replay(&frames), 1);
    for end in [2, 4] {
        let replay = replay(&frames[..end]);
        observe(&mut state, &replay, &frames[end - 2..end]);
        publish(&mut state, &fixture, &cancelled);
        assert_parity(&fixture, &replay);
        state = resume(&fixture);
        assert!(state.scan.battle_hours.is_empty());
        assert!(state.scan.stored_runs.len() <= state.scan.leaders.len() * 2);
    }
}

#[test]
fn prepared_leader_outcome_after_commander_changed_keeps_both_careers_and_first_read() {
    let frames = outcome_frames(false);
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut state = Prepared::new(&replay(&frames), 1);
    for end in [2, 3, 4] {
        let replay = replay(&frames[..end]);
        let begin = if end == 2 { 0 } else { end - 1 };
        observe(&mut state, &replay, &frames[begin..end]);
        publish(&mut state, &fixture, &cancelled);
        assert_parity(&fixture, &replay);
        state = resume(&fixture);
    }
    let reply = query(
        &fixture.store,
        &fixture.key,
        &UnitLeadersParams {
            selection: LeaderSelection::Leader {
                reference: reference(0).to_string(),
            },
            ..full_params("1")
        },
        None,
        &cancelled,
    )
    .unwrap()
    .unwrap();
    assert_eq!(reply.leaders[0].battle_won, Some(vec![Some(true)]));
    assert_eq!(
        reply.leaders[0].battle_result_read_hours,
        Some(vec![Some(102)])
    );
}
#[test]
fn prepared_leader_conflicting_repeated_outcomes_remain_unresolved_after_resume() {
    let frames = outcome_frames(true);
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut state = Prepared::new(&replay(&frames), 1);
    for (begin, end) in [(0, 3), (3, 4)] {
        let replay = replay(&frames[..end]);
        observe(&mut state, &replay, &frames[begin..end]);
        publish(&mut state, &fixture, &cancelled);
        assert_parity(&fixture, &replay);
        state = resume(&fixture);
    }
    let reply = query(
        &fixture.store,
        &fixture.key,
        &UnitLeadersParams {
            selection: LeaderSelection::Leader {
                reference: reference(0).to_string(),
            },
            ..full_params("1")
        },
        None,
        &cancelled,
    )
    .unwrap()
    .unwrap();
    assert_eq!(reply.leaders[0].battle_won, Some(vec![None]));
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("Conflicting native land battle outcomes"))
    );
}

#[test]
fn prepared_naval_leader_source_links_join_late_results_and_permanent_conflicts_after_resume() {
    use replay_data::{
        rows::NavalCombatDetailsRow, rows::NavalCombatResultDetailsRow,
        rows::NavalCombatSideDetails, rows::NavalResultRow, rows::NavalResultSideDetails,
        rows::NavalResultSourceRow,
    };
    let mut frames: Vec<_> = (100..=104).map(|hour| frame(hour, 2)).collect();
    for frame in &mut frames {
        if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *frame.tables[TableId::UnitLeaders.index()]
        {
            for row in rows {
                row.kind = 2;
                row.category = 2;
            }
        }
    }
    set(
        &mut frames[0],
        TableRows::NavalCombatDetails(vec![NavalCombatDetailsRow {
            combat: 91,
            custom_sea: Some(Vec::new()),
            sides: [
                NavalCombatSideDetails {
                    commander: reference(0),
                    ..Default::default()
                },
                Default::default(),
            ],
            ..Default::default()
        }]),
    );
    set(
        &mut frames[1],
        TableRows::NavalResultSources(vec![NavalResultSourceRow {
            id: 1,
            combat: 91,
            game_hour: 101,
            result: 71,
            ..Default::default()
        }]),
    );
    for (index, frame) in frames.iter_mut().enumerate().skip(2) {
        set(
            frame,
            TableRows::NavalResults(vec![NavalResultRow {
                id: 71,
                winning_side: if index == 3 { 2 } else { 1 },
                ..Default::default()
            }]),
        );
        set(
            frame,
            TableRows::NavalCombatResultDetails(vec![NavalCombatResultDetailsRow {
                result: 71,
                sides: [
                    NavalResultSideDetails {
                        leader: reference(1),
                        ..Default::default()
                    },
                    Default::default(),
                ],
            }]),
        );
    }
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut state = Prepared::new(&replay(&frames), 1);
    for end in 1..=frames.len() {
        let replay = replay(&frames[..end]);
        observe(&mut state, &replay, &frames[end - 1..end]);
        publish(&mut state, &fixture, &cancelled);
        assert_parity(&fixture, &replay);
        state = resume(&fixture);
        assert!(state.scan.naval_sources.is_empty());
        assert!(state.scan.outcomes.is_empty());
        if end == 3 || end == 5 {
            let params = UnitLeadersParams {
                selection: LeaderSelection::Leader {
                    reference: reference(0).to_string(),
                },
                ..full_params("1")
            };
            let reply = query(&fixture.store, &fixture.key, &params, None, &cancelled)
                .unwrap()
                .unwrap();
            assert_eq!(
                reply.leaders[0].battle_won,
                Some(vec![if end == 3 { Some(true) } else { None }])
            );
            if end == 3 {
                assert_eq!(
                    reply.leaders[0].battle_result_read_hours,
                    Some(vec![Some(102)])
                );
            }
        }
    }
}
#[test]
fn prepared_leader_long_career_moves_closed_commands_and_battles_out_of_continuation() {
    let first = frame(100, 2);
    let replay = replay(std::slice::from_ref(&first));
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut state = Prepared::new(&replay, 1);
    for batch in 0..4 {
        let mut tracker = FrameChangeTracker::new();
        for offset in 0..256 {
            let hour = 100 + batch * 256 + offset;
            let mut next = first.clone();
            next.hour = hour;
            if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *next.tables[TableId::UnitLeaders.index()]
            {
                rows[0].components[0] = hour;
            }
            let combat = hour as u64;
            set(
                &mut next,
                TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
                    combat,
                    sides: [
                        LandCombatSideDetails {
                            commander: reference(0),
                            ..Default::default()
                        },
                        Default::default(),
                    ],
                    ..Default::default()
                }]),
            );
            set(
                &mut next,
                TableRows::LandCombatResults(vec![LandCombatResultsRow {
                    id: combat,
                    combat,
                    end_hour: hour,
                    winners: 1,
                    ..Default::default()
                }]),
            );
            set(
                &mut next,
                TableRows::OrdersGroups(vec![OrdersGroupRow {
                    handle: combat,
                    leader: reference(0),
                    name: Recorded::Value(Some(b"Synthetic order".to_vec())),
                    ..Default::default()
                }]),
            );
            let changes = tracker.observe(&next, replay.format);
            state.observe(&next, &replay, None, &changes).unwrap();
        }
        publish(&mut state, &fixture, &cancelled);
        assert!(state.scan.stored_runs.len() <= 4);
        assert!(state.scan.leaders.iter().all(
            |leader| leader.career.commands.len() <= 1 && leader.career.trait_gains.is_empty()
        ));
        assert!(state.scan.battle_hours.is_empty());
        assert!(state.scan.outcomes.is_empty());
        assert!(state.scan.naval_sources.is_empty());
        assert!(state.scan.reply.observed.len() <= 1);
        eprintln!(
            "unit-leaders continuation after {} dated commands/battles: {} JSON / {} compressed bytes",
            (batch + 1) * 256,
            serde_json::to_vec(&state).unwrap().len(),
            state.saved_bytes().unwrap().len()
        );
        state = resume(&fixture);
    }
    let params = UnitLeadersParams {
        selection: LeaderSelection::Leader {
            reference: reference(0).to_string(),
        },
        ..full_params("1")
    };
    let reply = query(&fixture.store, &fixture.key, &params, None, &cancelled)
        .unwrap()
        .unwrap();
    assert_eq!(reply.leaders[0].commands.len(), 1024);
    assert_eq!(reply.leaders[0].battle_starts.as_ref().unwrap().len(), 1024);
    assert_eq!(reply.leaders[0].runs.len(), 1024);
}

#[test]
fn prepared_leader_choices_keep_historical_kind_and_country_admission_without_career_reads() {
    let mut frames: Vec<_> = (100..=103).map(|hour| frame(hour, 2)).collect();
    for (index, frame) in frames.iter_mut().enumerate() {
        if index >= 2 {
            if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *frame.tables[TableId::UnitLeaders.index()]
            {
                rows[0].owner = 100_001;
                rows[0].kind = 2;
                rows[0].category = 2;
            }
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].id += 500;
                rows.sort_by_key(|row| row.id);
            }
        }
    }
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let replay = replay(&frames);
    let mut state = Prepared::new(&replay, 1);
    for (begin, end) in [(0, 2), (2, 4)] {
        observe(&mut state, &replay, &frames[begin..end]);
        publish(&mut state, &fixture, &cancelled);
        state = resume(&fixture);
    }
    let full = scan(&replay, &full_params("1"), None, &cancelled).unwrap();
    let history = dashboard_base::products::country_history::scan_histories(
        &replay,
        1,
        false,
        &[],
        &cancelled,
    )
    .unwrap();
    // Choices must still succeed when opening any career batch would fail.
    let mut writer = fixture.writer();
    writer
        .write_at(&section(1), 0, &b"Unreadable synthetic career batch"[..])
        .unwrap();
    writer.commit().unwrap();
    for selection in [
        LeaderSelection::All,
        LeaderSelection::Countries {
            tags: vec![String::from_utf8(dashboard_core::test_fixtures::scale_tag(0)).unwrap()],
        },
        LeaderSelection::Leader {
            reference: reference(0).to_string(),
        },
    ] {
        for leaders in [LeaderKinds::All, LeaderKinds::Land, LeaderKinds::Navy] {
            let params = UnitLeadersParams {
                selection: selection.clone(),
                view: LeaderView::Choices,
                leaders,
                ..full_params("1")
            };
            let actual = choices(
                &fixture.store,
                &fixture.key,
                &params,
                Some(&history),
                &cancelled,
            )
            .unwrap()
            .unwrap();
            assert_eq!(actual, choices_from(&full, &params, Some(&history)));
        }
    }
    let params = UnitLeadersParams {
        selection: LeaderSelection::Leader {
            reference: reference(0).to_string(),
        },
        ..full_params("1")
    };
    assert!(query(&fixture.store, &fixture.key, &params, None, &cancelled).is_err());
}

fn work(state: &Prepared) -> dashboard_base::prepared::observer_tests::Work {
    dashboard_base::prepared::observer_tests::Work {
        content_loads: super::super::content_loads(),
        row_visits: state.work.row_visits + state.scan.work.row_visits,
        history_visits: state.work.history_visits + state.scan.work.history_visits,
        interpreter_calls: state.scan.work.interpreter_calls,
        ..state.work
    }
}

fn admission_work(
    closed: usize,
    hours: usize,
    countries: u32,
    change: Option<TableId>,
    content: Option<&Content>,
) -> io::Result<dashboard_base::prepared::observer_tests::Work> {
    use dashboard_base::prepared::observer_tests::{self, Work};
    use replay_data::{
        rows::NavalCombatResultDetailsRow, rows::NavalResultRow, rows::NavalResultSideDetails,
        rows::NavalResultSourceRow,
    };
    let mut template = frame(100, countries);
    for rows in [
        TableRows::LandCombatResults(vec![LandCombatResultsRow {
            id: 1,
            combat: 91,
            end_hour: 100,
            winners: 1,
            sides: [
                LandResultSide {
                    commander: reference(0),
                    ..Default::default()
                },
                Default::default(),
            ],
            ..Default::default()
        }]),
        TableRows::NavalResultSources(vec![NavalResultSourceRow {
            id: 1,
            combat: 92,
            game_hour: 100,
            result: 71,
            ..Default::default()
        }]),
        TableRows::NavalResults(vec![NavalResultRow {
            id: 71,
            winning_side: 1,
            ..Default::default()
        }]),
        TableRows::NavalCombatResultDetails(vec![NavalCombatResultDetailsRow {
            result: 71,
            sides: [
                NavalResultSideDetails {
                    leader: reference(1),
                    ..Default::default()
                },
                Default::default(),
            ],
        }]),
        TableRows::OrdersGroups(vec![
            OrdersGroupRow {
                handle: 1,
                leader: reference(1),
                name: Recorded::Value(Some(b"Carried command".to_vec())),
                ..Default::default()
            },
            OrdersGroupRow {
                handle: 2,
                leader: reference(1),
                name: Recorded::Value(None),
                ..Default::default()
            },
        ]),
    ] {
        set(&mut template, rows);
    }
    let mut frames: Vec<_> = (0..=closed)
        .map(|index| {
            let mut frame = template.clone();
            frame.hour += index as i32;
            if index != 0
                && let TableView::Complete(TableRows::UnitLeaders(rows)) =
                    &mut *frame.tables[TableId::UnitLeaders.index()]
            {
                rows[0].id = countries + index as u32;
                rows.sort_by_key(|row| row.id);
            }
            frame
        })
        .collect();
    let mut first = frames.last().unwrap().clone();
    first.hour += 1;
    frames.push(first.clone());
    for offset in 1..=hours {
        let mut next = first.clone();
        next.hour += offset as i32;
        if change == Some(TableId::UnitLeaders)
            && let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *next.tables[TableId::UnitLeaders.index()]
        {
            let row = rows.last_mut().unwrap();
            row.kind = 2;
            row.category = 2;
            row.owner = 100_001;
        }
        if change == Some(TableId::Countries)
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *next.tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = Some(2);
        }
        frames.push(next);
    }
    let replay = replay(&frames);
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut state = Prepared::new(&replay, 1);
    let mut tracker = FrameChangeTracker::new();
    for frame in &frames[..=closed] {
        let changes = tracker.observe(frame, replay.format);
        state.observe(frame, &replay, content, &changes)?;
    }
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, content, &[], &flag)?;
    writer.commit()?;
    state = resume(&fixture);
    tracker.invalidate();
    let changes = tracker.observe(&first, replay.format);
    state.observe(&first, &replay, content, &changes)?;
    let baseline = work(&state);
    let delta = if change.is_some() {
        let next = frames.last().unwrap();
        let changes = tracker.observe(next, replay.format);
        state.observe(next, &replay, content, &changes)?;
        let after = work(&state);
        Work {
            content_loads: after.content_loads - baseline.content_loads,
            row_visits: after.row_visits - baseline.row_visits,
            history_visits: after.history_visits - baseline.history_visits,
            emitted: after.emitted - baseline.emitted,
            interpreter_calls: after.interpreter_calls - baseline.interpreter_calls,
            dictionary_interpretations: after.dictionary_interpretations
                - baseline.dictionary_interpretations,
        }
    } else {
        observer_tests::unchanged_frames(
            baseline,
            &first,
            hours as i32,
            replay.format,
            |frame, changes| {
                state.observe(frame, &replay, content, changes)?;
                Ok(work(&state))
            },
        )?
    };
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, content, &[], &flag)?;
    writer.commit()?;
    state = resume(&fixture);
    assert_eq!(state.scan.leaders.len(), countries as usize + closed);
    let oracle = scan(&replay, &full_params("1"), content, &flag)?;
    let actual = query(&fixture.store, &fixture.key, &full_params("1"), None, &flag)?.unwrap();
    observer_tests::assert_reply_parity(&actual, &project(&oracle, &full_params("1"), None))?;
    Ok(delta)
}

#[test]
fn prepared_leaders_shared_driver_configured_content_success_and_failure_never_reload() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use dashboard_base::prepared::observer_tests;
    for refused in [false, true] {
        let (catalogue, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
            (
                "common/defines/configured.lua",
                "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
            ),
            (
                "common/unit_leader/configured.txt",
                "leader_skills={1={type=corps_commander cost=1}} leader_traits={}",
            ),
        ]);
        let catalogue = Arc::new(catalogue);
        let politics =
            Arc::new(politics_content::PoliticsContent::load(catalogue.clone()).unwrap());
        if refused {
            // Refuse the real inventoried database read at configuration.
            std::fs::remove_file(folder.0.join("common/unit_leader/configured.txt")).unwrap();
        }
        let before = super::super::content_loads();
        let content = Content::load(catalogue, politics);
        assert_eq!(content.is_err(), refused);
        assert_eq!(super::super::content_loads(), before + 1);
        let content = content.ok();
        let scaled = observer_tests::scaled_work(4, |hours| {
            admission_work(4, hours, 2, None, content.as_ref())
        })
        .unwrap();
        observer_tests::assert_flat(scaled);
        assert_eq!(scaled[0].content_loads, 0);
        assert_eq!(super::super::content_loads(), before + 1);
    }
}

#[test]
fn prepared_leaders_shared_driver_country_balance_changes_keep_resolved_joins_and_admissions() {
    use dashboard_base::prepared::observer_tests;
    let work = observer_tests::scaled_work(4, |history| {
        admission_work(history, 1, 2, Some(TableId::Countries), None)
    })
    .unwrap();
    observer_tests::assert_flat(work);
    assert_eq!(
        work[0].row_visits, 2,
        "only the existing country operand projection is compared"
    );
    assert_eq!(work[0].history_visits, 0);
    assert_eq!(work[0].emitted, 0);
    assert_eq!(work[0].content_loads, 0);
}

#[test]
fn prepared_leaders_deferred_carry_matches_hourly_oracle_after_append_gap_alias_and_failed_reads() {
    let mut base = frame(100, 2);
    set(
        &mut base,
        TableRows::OrdersGroups(vec![
            OrdersGroupRow {
                handle: 1,
                leader: reference(0),
                name: Recorded::Value(Some(b"Carried name".to_vec())),
                ..Default::default()
            },
            OrdersGroupRow {
                handle: 2,
                leader: reference(0),
                name: Recorded::Value(None),
                ..Default::default()
            },
        ]),
    );
    let prefix: Vec<_> = (100..=109)
        .map(|hour| {
            let mut frame = base.clone();
            frame.hour = hour;
            frame
        })
        .collect();
    let mut alias = base.clone();
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *alias.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"ALIAS".to_vec();
    }
    let mut failed = alias.clone();
    *failed.tables[TableId::OrdersGroups.index()] = TableView::Unavailable {
        reason: "Synthetic missing command read".into(),
    };
    let tail: Vec<_> = [110, 111, 112, 114, 115, 116, 120, 121, 122]
        .into_iter()
        .map(|hour| {
            let mut frame = if (115..=116).contains(&hour) {
                failed.clone()
            } else if hour >= 111 {
                alias.clone()
            } else {
                base.clone()
            };
            frame.hour = hour;
            frame
        })
        .collect();
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let initial = replay(&prefix);
    let mut state = Prepared::new(&initial, 1);
    observe(&mut state, &initial, &prefix);
    assert!(
        state.scan.materialized.unwrap() < state.scan.frontier.unwrap(),
        "equal frames must defer current endpoints"
    );
    // Reading open state seals through the same frontier as publication.
    let params = full_params("1");
    let mut open = Scan::new(&initial, 1).unwrap();
    let mut tracker = FrameChangeTracker::new();
    for frame in &prefix {
        open.observe(
            frame,
            &initial,
            &params,
            None,
            &tracker.observe(frame, initial.format),
            &mut |_, _, _| {},
        )
        .unwrap();
    }
    dashboard_base::prepared::observer_tests::assert_reply_parity(
        &open.finish(&params, None).unwrap(),
        &scan(&initial, &params, None, &flag).unwrap(),
    )
    .unwrap();
    publish(&mut state, &fixture, &flag);
    assert_parity(&fixture, &initial);
    state = resume(&fixture);
    // Bind accepted ranges to the grown input before observing its first frame.
    fixture.append_writer().commit().unwrap();
    let grown = initial
        .append(&[dashboard_core::test_fixtures::continuation_records(
            &initial,
            1,
            tail.clone(),
        )])
        .unwrap();
    observe(&mut state, &grown, &tail);
    publish(&mut state, &fixture, &flag);
    assert_parity(&fixture, &grown);
    let actual = query(&fixture.store, &fixture.key, &params, None, &flag)
        .unwrap()
        .unwrap();
    let leader = actual
        .leaders
        .iter()
        .find(|leader| leader.reference == reference(0).to_string())
        .unwrap();
    assert!(
        leader
            .commands
            .iter()
            .any(|command| command.first_hour == 100 && command.last_hour == 112)
    );
    assert!(
        leader
            .commands
            .iter()
            .any(|command| command.first_hour == 114 && command.last_hour == 114)
    );
    assert!(
        leader
            .commands
            .iter()
            .any(|command| command.first_hour == 120 && command.last_hour == 122)
    );
    assert!(
        leader
            .commands
            .iter()
            .all(
                |command| !(command.first_hour <= 113 && 113 <= command.last_hour
                    || command.first_hour <= 115 && 115 <= command.last_hour)
            )
    );
    state = resume(&fixture);
    assert_eq!(state.scan.materialized, state.scan.frontier);
}

#[test]
fn prepared_leaders_repeated_formation_bindings_keep_hourly_command_episodes() {
    use replay_data::rows::FleetRow;
    let mut base = frame(100, 1);
    if let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *base.tables[TableId::UnitLeaders.index()]
    {
        rows[0].kind = engine_native::cunit_leader::KIND_NAVY;
        rows[0].category = 2;
    }
    set(
        &mut base,
        TableRows::OrdersGroups(vec![OrdersGroupRow {
            handle: 1,
            leader: reference(0),
            name: Recorded::Value(None),
            ..Default::default()
        }]),
    );
    set(
        &mut base,
        TableRows::Fleets(vec![FleetRow {
            id: 1,
            owner: 100_000,
            leader: Recorded::Value(Some(reference(0))),
            name: Recorded::Value(None),
            ..Default::default()
        }]),
    );
    let frames: Vec<_> = (100..=104)
        .map(|hour| {
            let mut frame = base.clone();
            frame.hour = hour;
            frame
        })
        .collect();
    let replay = replay(&frames);
    let params = full_params("1");
    let mut scan_state = Scan::new(&replay, 1).unwrap();
    let mut tracker = FrameChangeTracker::new();
    for frame in &frames {
        let changes = tracker.observe(frame, replay.format);
        scan_state
            .observe(frame, &replay, &params, None, &changes, &mut |_, _, _| {})
            .unwrap();
        assert!(!scan_state.commands_continuous);
        assert_eq!(scan_state.materialized, Some(frame.hour));
    }
    let actual = scan_state.finish(&params, None).unwrap();
    assert_eq!(actual.leaders[0].commands.len(), 10);
    dashboard_base::prepared::observer_tests::assert_reply_parity(
        &actual,
        &scan(&replay, &params, None, &AtomicBool::new(false)).unwrap(),
    )
    .unwrap();
}

/// Observation and publication take no reader of the published series, yet
/// outcomes and naval sources published in one window still join and
/// conflict in the next.
#[test]
fn prepared_leaders_join_outcomes_published_in_earlier_windows() {
    use replay_data::{
        rows::NavalCombatDetailsRow, rows::NavalCombatResultDetailsRow,
        rows::NavalCombatSideDetails, rows::NavalResultRow, rows::NavalResultSideDetails,
        rows::NavalResultSourceRow,
    };
    let mut naval: Vec<_> = (100..=103).map(|hour| frame(hour, 2)).collect();
    for frame in &mut naval {
        if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *frame.tables[TableId::UnitLeaders.index()]
        {
            for row in rows {
                row.kind = 2;
                row.category = 2;
            }
        }
    }
    set(
        &mut naval[0],
        TableRows::NavalCombatDetails(vec![NavalCombatDetailsRow {
            combat: 91,
            custom_sea: Some(Vec::new()),
            sides: [
                NavalCombatSideDetails {
                    commander: reference(0),
                    ..Default::default()
                },
                Default::default(),
            ],
            ..Default::default()
        }]),
    );
    set(
        &mut naval[1],
        TableRows::NavalResultSources(vec![NavalResultSourceRow {
            id: 1,
            combat: 91,
            game_hour: 101,
            result: 71,
            ..Default::default()
        }]),
    );
    for (index, frame) in naval.iter_mut().enumerate().skip(2) {
        set(
            frame,
            TableRows::NavalResults(vec![NavalResultRow {
                id: 71,
                winning_side: if index == 3 { 2 } else { 1 },
                ..Default::default()
            }]),
        );
        set(
            frame,
            TableRows::NavalCombatResultDetails(vec![NavalCombatResultDetailsRow {
                result: 71,
                sides: [
                    NavalResultSideDetails {
                        leader: reference(1),
                        ..Default::default()
                    },
                    Default::default(),
                ],
            }]),
        );
    }
    for frames in [outcome_frames(false), outcome_frames(true), naval] {
        let fixture = Fixture::new();
        let cancelled = AtomicBool::new(false);
        let mut state = Prepared::new(&replay(&frames), 1);
        // One frame per window, so every later window reaches keys an
        // earlier one published.
        for end in 1..=frames.len() {
            let replay = replay(&frames[..end]);
            let changes = FrameChangeTracker::new().observe(&frames[end - 1], replay.format);
            state
                .observe(&frames[end - 1], &replay, None, &changes)
                .unwrap();
            let mut writer = fixture.writer();
            state.flush(&mut writer, 1, None, &[], &cancelled).unwrap();
            writer.commit().unwrap();
            assert_parity(&fixture, &replay);
            state = resume(&fixture);
        }
    }
}

#[test]
fn withheld_names_carry_unchanged_inputs_and_reread_changed_characters_and_tokens() {
    use replay_data::rows::LexicalTokenRow;
    use replay_frame::{frame::FrameDecoder, frame::FrameEncoder};
    let frames: Vec<_> = (100..=103)
        .map(|hour| {
            let mut input = frame(hour, 2);
            if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *input.tables[TableId::UnitLeaders.index()]
            {
                rows[1].owner = u32::MAX;
            }
            if let TableView::Complete(TableRows::Characters(rows)) =
                &mut *input.tables[TableId::Characters.index()]
            {
                rows[1].name_token_id = 7;
                if hour >= 102 {
                    rows[1].token = b"Renamed commander".to_vec();
                }
            }
            set(
                &mut input,
                TableRows::LexicalTokens(vec![
                    LexicalTokenRow {
                        token: 7,
                        literal: if hour < 103 {
                            b"first".to_vec()
                        } else {
                            b"second".to_vec()
                        },
                    },
                    LexicalTokenRow {
                        token: 8,
                        literal: hour.to_string().into_bytes(),
                    },
                ]),
            );
            input
        })
        .collect();
    let replay = replay(&frames);
    let mut state = Prepared::new(&replay, 1);
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut tracker = FrameChangeTracker::new();
    let mut saved = None;
    for (at, input) in frames.iter().enumerate() {
        let wire = encoder.encode(input.clone(), at == 0).unwrap();
        let decoded = decoder.advance(&wire).unwrap().clone();
        let changes = tracker.observe_updates(&decoded, Arc::new(wire), replay.format);
        state.observe(&decoded, &replay, None, &changes).unwrap();
        let index = state.scan.leader_indices[&reference(1)];
        let read = state.scan.leaders[index].withheld_name.as_ref().unwrap();
        assert_eq!(read.epoch, state.scan.identity_epoch);
        assert_eq!(
            read.saved.as_deref(),
            Some(if at < 2 {
                b"Synthetic commander 1".as_slice()
            } else {
                b"Renamed commander".as_slice()
            })
        );
        assert_eq!(
            read.token.as_deref(),
            Some(if at < 3 { "first" } else { "second" })
        );
        if at == 1 {
            assert_eq!(read.saved.as_ref().unwrap().as_ptr(), saved.unwrap());
        }
        saved = Some(read.saved.as_ref().unwrap().as_ptr());
    }
    let fixture = Fixture::new();
    publish(&mut state, &fixture, &AtomicBool::new(false));
    assert_parity(&fixture, &replay);
}

/// Changed identity tables re-read only the leaders whose own rows changed:
/// the others keep the identity they read the hour before, and careers still
/// match the per-hour oracle.
#[test]
fn prepared_leaders_reread_identity_only_for_leaders_whose_rows_changed() {
    use replay_data::rows::LexicalTokenRow;
    use replay_frame::{frame::FrameDecoder, frame::FrameEncoder};
    let frames: Vec<_> = (100..=103)
        .map(|hour| {
            let mut frame = frame(hour, 2);
            // Each hour adds a token no leader reads.
            set(
                &mut frame,
                TableRows::LexicalTokens(
                    (100..=hour)
                        .map(|added| LexicalTokenRow {
                            token: 5000 + added as u32,
                            literal: format!("synthetic_token_{added}").into_bytes(),
                        })
                        .collect(),
                ),
            );
            // The second leader's character changes once.
            if hour >= 102
                && let TableView::Complete(TableRows::Characters(rows)) =
                    &mut *frame.tables[TableId::Characters.index()]
            {
                rows[1].token = b"Renamed synthetic commander".to_vec();
            }
            frame
        })
        .collect();
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let replay = replay(&frames);
    let mut state = Prepared::new(&replay, 1);
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut tracker = FrameChangeTracker::new();
    let mut renewed = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        let wire = encoder.encode(frame.clone(), index == 0).unwrap();
        let decoded = decoder.advance(&wire).unwrap().clone();
        let changes = tracker.observe_updates(&decoded, std::sync::Arc::new(wire), replay.format);
        let before = state.scan.renewed;
        state.observe(&decoded, &replay, None, &changes).unwrap();
        renewed.push(state.scan.renewed - before);
    }
    // The keyframe has no delta to read. Then each leader keeps the person
    // it read, except the second at the hour its character changed.
    assert_eq!(renewed, [0, 2, 1, 2]);
    publish(&mut state, &fixture, &cancelled);
    assert_parity(&fixture, &replay);
}

/// Observe `frames` with the change facts of their recorded updates, as the
/// hourly walk observes them; each hour's leader rows read so far.
fn observe_recorded(state: &mut Prepared, replay: &Replay, frames: &[DecodedFrame]) -> Vec<usize> {
    use replay_frame::{frame::FrameDecoder, frame::FrameEncoder};
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut tracker = FrameChangeTracker::new();
    frames
        .iter()
        .enumerate()
        .map(|(index, frame)| {
            let wire = encoder.encode(frame.clone(), index == 0).unwrap();
            let decoded = decoder.advance(&wire).unwrap().clone();
            let changes =
                tracker.observe_updates(&decoded, std::sync::Arc::new(wire), replay.format);
            state.observe(&decoded, replay, None, &changes).unwrap();
            work(state).row_visits
        })
        .collect()
}

/// Of the combat details leaders read only each combat and its sides'
/// commanders, and nothing of task force positions, mission fuel or country
/// balances, all of which move every hour of the war: such an hour carries
/// every career on without reading a leader. A commander's change and a
/// combat's start or end (with its result) take the battle step alone; only
/// the changed command reads every leader again. Every reply is the per-hour
/// oracle's.
#[test]
fn prepared_leaders_read_every_leader_only_where_a_leader_input_changed() {
    let countries = 200;
    let hours: Vec<i32> = (100..=165).collect();
    let frames: Vec<_> = hours
        .iter()
        .map(|&hour| super::test_fixtures::war_frame(1, hour, countries))
        .collect();
    let replay = replay(&frames);
    let mut state = Prepared::new(&replay, 1);
    let visits = observe_recorded(&mut state, &replay, &frames);
    let read: BTreeMap<i32, usize> = hours[1..]
        .iter()
        .zip(visits.windows(2))
        .map(|(&hour, pair)| (hour, pair[1] - pair[0]))
        .collect();
    // An hour whose reads differ from the hour before's reads every leader.
    let masks: Vec<u32> = frames
        .iter()
        .map(|frame| super::super::read_mask(replay.format, &frame.metrics, frame.hour, true))
        .collect();
    let reads_changed: BTreeSet<i32> = hours[1..]
        .iter()
        .zip(masks.windows(2))
        .filter(|(_, pair)| pair[0] != pair[1])
        .map(|(&hour, _)| hour)
        .collect();
    // The commander swap, the second combat's start, each end with its
    // result, and the results read no more.
    let battles = [120, 130, 151, 152, 161, 162];
    for (&hour, &rows) in &read {
        if hour == 140 {
            assert!(
                rows >= countries as usize,
                "hour {hour}: the changed command reads every leader, not {rows} rows"
            );
        } else if reads_changed.contains(&hour) {
            continue;
        } else if battles.contains(&hour) {
            assert!(
                rows < countries as usize,
                "hour {hour}: {rows} rows read where only battles changed"
            );
        } else {
            assert_eq!(rows, 0, "hour {hour} carries every career on");
        }
    }
    for hour in [120, 130, 151, 161] {
        assert!(read[&hour] > 0, "hour {hour} reads its battles");
    }
    assert!(state.scan.materialized < state.scan.frontier);
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    publish(&mut state, &fixture, &cancelled);
    assert_parity(&fixture, &replay);
}

/// Published and resumed in the middle of both combats, the battle step goes
/// on from the restored careers exactly as the per-hour oracle reads them.
#[test]
fn prepared_leaders_battle_step_after_publication_and_resume_matches_the_oracle() {
    let frames: Vec<_> = (100..=170)
        .map(|hour| super::test_fixtures::war_frame(1, hour, 6))
        .collect();
    let split = frames.iter().position(|frame| frame.hour == 135).unwrap();
    let whole = replay(&frames);
    let prefix = replay(&frames[..split]);
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut state = Prepared::new(&whole, 1);
    observe_recorded(&mut state, &whole, &frames[..split]);
    publish(&mut state, &fixture, &cancelled);
    assert_parity(&fixture, &prefix);
    let mut state = resume(&fixture);
    observe_recorded(&mut state, &whole, &frames[split..]);
    publish(&mut state, &fixture, &cancelled);
    assert_parity(&fixture, &whole);
}

/// The continuation holds no count of the identity, owner and trait views
/// read: those count the hours read, which differ where hours are read
/// differently, while the careers do not.
#[test]
fn prepared_leaders_continuation_keeps_no_counts_of_views_read() {
    let frames: Vec<_> = (100..=103)
        .map(|hour| {
            let mut frame = frame(hour, 3);
            if hour >= 102
                && let TableView::Complete(TableRows::Characters(rows)) =
                    &mut *frame.tables[TableId::Characters.index()]
            {
                rows[0].token = b"Renamed synthetic commander".to_vec();
            }
            frame
        })
        .collect();
    let replay = replay(&frames);
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut state = Prepared::new(&replay, 1);
    observe(&mut state, &replay, &frames);
    publish(&mut state, &fixture, &cancelled);
    let saved: serde_json::Value = serde_json::from_slice(
        &zstd::stream::decode_all(
            fixture
                .store
                .read(&fixture.key, "unit-leaders-1", 0)
                .unwrap()
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let scan = saved["scan"].as_object().unwrap();
    for counted in ["identity_epoch", "owner_epoch", "trait_epoch"] {
        assert!(!scan.contains_key(counted), "{counted} is saved");
    }
    assert!(scan.contains_key("people"));
}

/// The battles read at an hour whose leaders, commands and reads are the
/// hour before's but whose combat details become unavailable, and again at
/// the hour they are read again: the missing battle reads, the coverage and
/// the careers made partial there are the per-hour oracle's, as only the
/// leader walk records them.
#[test]
fn prepared_leaders_battle_availability_changes_read_coverage_as_the_oracle() {
    let mut base = frame(100, 3);
    let combat = TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
        combat: 91,
        sides: [
            LandCombatSideDetails {
                commander: reference(0),
                ..Default::default()
            },
            LandCombatSideDetails {
                commander: reference(1),
                ..Default::default()
            },
        ],
        ..Default::default()
    }]);
    set(&mut base, combat.clone());
    let frames: Vec<_> = (100..=108)
        .map(|hour| {
            let mut frame = base.clone();
            frame.hour = hour;
            // A new view every hour with the same commanders, as a combat
            // whose operands move; unavailable from 103 to 105.
            *frame.tables[TableId::LandCombatDetails.index()] = if (103..=105).contains(&hour) {
                TableView::Unavailable {
                    reason: "Synthetic missing combat read".into(),
                }
            } else {
                TableView::Complete(combat.clone())
            };
            frame
        })
        .collect();
    let replay = replay(&frames);
    let mut state = Prepared::new(&replay, 1);
    observe(&mut state, &replay, &frames);
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    publish(&mut state, &fixture, &cancelled);
    assert_parity(&fixture, &replay);
    let actual = query(
        &fixture.store,
        &fixture.key,
        &full_params("1"),
        None,
        &cancelled,
    )
    .unwrap()
    .unwrap();
    assert!(
        actual
            .observed
            .iter()
            .any(|span| !span.land_battles && span.first_hour == 103 && span.last_hour == 105),
        "{:?}",
        actual.observed
    );
    assert!(
        actual
            .leaders
            .iter()
            .any(|leader| leader.partial_from == Some(103))
    );
}

#[test]
fn battle_only_hours_do_not_materialize_unchanged_leader_careers() {
    let frames: Vec<_> = (100..=170)
        .filter(|hour| *hour != 155)
        .map(|hour| {
            let mut frame = super::test_fixtures::war_frame(1, hour, 24);
            if hour == 143 {
                *frame.tables[TableId::LandCombatDetails.index()] = TableView::Unavailable {
                    reason: "synthetic battle refusal".into(),
                };
            }
            frame
        })
        .collect();
    let replay = replay(&frames);
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut states = [Prepared::new(&replay, 1), Prepared::new(&replay, 1)];
    states[1].scan.eager_battles = true;
    let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
    let flag = AtomicBool::new(false);
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    let mut decoder = replay_frame::frame::FrameDecoder::new();
    for (ordinal, input) in frames.iter().enumerate() {
        let wire = std::sync::Arc::new(
            encoder
                .encode(input.clone(), ordinal == 0 || input.hour == 148)
                .unwrap(),
        );
        let frame = decoder.advance(&wire).unwrap().clone();
        let mut visits = [0; 2];
        for (index, (state, tracker)) in states.iter_mut().zip(&mut trackers).enumerate() {
            let changes =
                tracker.observe_updates(&frame, std::sync::Arc::clone(&wire), replay.format);
            let before = state.scan.work.history_visits;
            state.observe(&frame, &replay, None, &changes).unwrap();
            visits[index] = state.scan.work.history_visits - before;
        }
        if frame.hour == 120 {
            assert_eq!(visits[0], 0, "battle step visits no career endpoint");
            assert!(
                visits[1] >= 24,
                "reference materializes all careers: {visits:?}"
            );
        }
        if [134, 170].contains(&frame.hour) {
            for (state, fixture) in states.iter_mut().zip(&fixtures) {
                publish(state, fixture, &flag);
            }
            dashboard_base::prepared::series::tests::assert_payloads_equal(
                &fixtures[0].payloads(),
                &fixtures[1].payloads(),
                "battle endpoint carry",
            );
            for (index, (state, fixture)) in states.iter_mut().zip(&fixtures).enumerate() {
                *state = resume(fixture);
                state.scan.eager_battles = index == 1;
            }
        }
    }
}

#[test]
fn a_leaderboard_at_an_hour_is_held_to_the_budget_only_once_projected() {
    // Skill changes each hour, so the whole history grows with every frame.
    let mut frames: Vec<_> = (100..=160).map(|hour| frame(hour, 2)).collect();
    for (index, frame) in frames.iter_mut().enumerate() {
        if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *frame.tables[TableId::UnitLeaders.index()]
        {
            for row in rows {
                row.components[0] = index as i32;
            }
        }
    }
    let replay = replay(&frames);
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut state = Prepared::new(&replay, 1);
    observe(&mut state, &replay, &frames);
    publish(&mut state, &fixture, &cancelled);
    let params = UnitLeadersParams {
        view: LeaderView::Leaderboard,
        ..full_params("1")
    };
    let whole = query(&fixture.store, &fixture.key, &params, None, &cancelled)
        .unwrap()
        .unwrap();
    let mut projected = whole.clone();
    leaderboard_at(&mut projected, 130);
    let [whole_bytes, projected_bytes] =
        [&whole, &projected].map(|reply| serde_json::to_vec(reply).unwrap().len());
    assert!(projected_bytes < whole_bytes);
    let budget = projected_bytes + (whole_bytes - projected_bytes) / 2;
    assert!(
        query_within(
            &fixture.store,
            &fixture.key,
            &params,
            None,
            None,
            budget,
            &cancelled
        )
        .is_err(),
        "the whole history exceeds the budget"
    );
    let reply = query_within(
        &fixture.store,
        &fixture.key,
        &params,
        None,
        Some(130),
        budget,
        &cancelled,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        serde_json::to_value(&reply).unwrap(),
        serde_json::to_value(&projected).unwrap()
    );
}

#[test]
fn leader_events_parse_from_their_rows_as_their_pages_return_them() {
    // Skill changes each hour, so every leader's career has an event per frame.
    let mut frames: Vec<_> = (100..=400).map(|hour| frame(hour, 2)).collect();
    for (index, frame) in frames.iter_mut().enumerate() {
        if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *frame.tables[TableId::UnitLeaders.index()]
        {
            for row in rows {
                row.components[0] = index as i32;
            }
        }
    }
    let replay = replay(&frames);
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut state = Prepared::new(&replay, 1);
    observe(&mut state, &replay, &frames);
    publish(&mut state, &fixture, &cancelled);
    let (keys, _) =
        series::read_catalogue::<Metadata>(&fixture.store, &fixture.key, "unit-leaders", 1)
            .unwrap()
            .unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, section(1), &cancelled).unwrap();
    let mut compared = 0;
    for &key in keys.values() {
        let paged = reader
            .events(&[key], i32::MIN, i32::MAX, 0, u32::MAX)
            .unwrap()
            .rows
            .into_iter()
            .map(|row| serde_json::from_slice::<serde_json::Value>(&row.row).unwrap())
            .collect::<Vec<_>>();
        let parsed = events(&mut reader, key, &cancelled)
            .unwrap()
            .iter()
            .map(|event| serde_json::to_value(event).unwrap())
            .collect::<Vec<_>>();
        compared += parsed.len();
        assert_eq!(parsed, paged, "key {key}");
    }
    assert!(compared > 256, "{compared} events cross a page");
}

/// A static table read again (the lexer's tokens timed at one hour) changes the hour's reads
/// and nothing a leader reads: the leaders are walked at that hour and the next with every
/// read they hold, none of them read anew.
#[test]
fn prepared_leaders_keep_their_reads_across_a_static_table_read() {
    let first = frame(100, 2);
    let frames: Vec<_> = (100..=104)
        .map(|hour| {
            let mut frame = first.clone();
            frame.hour = hour;
            if hour == 102 {
                frame.metrics.table_ns[TableId::LexicalTokens.index()] = 1;
            }
            frame
        })
        .collect();
    let replay = replay(&frames);
    let mut state = Prepared::new(&replay, 1);
    let mut tracker = FrameChangeTracker::new();
    let mut builds = Vec::new();
    let mut visits = Vec::new();
    for frame in &frames {
        let (built, visited) = (super::super::progress_builds(), work(&state).row_visits);
        let changes = tracker.observe(frame, replay.format);
        state.observe(frame, &replay, None, &changes).unwrap();
        builds.push(super::super::progress_builds() - built);
        visits.push(work(&state).row_visits - visited);
    }
    assert!(visits[2] > 0, "the hour its reads change walks the leaders");
    assert_eq!(builds, [2, 0, 0, 0, 0]);
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    publish(&mut state, &fixture, &cancelled);
    assert_parity(&fixture, &replay);
}

/// A leader renamed while the roster could not be read is read anew where the roster is read
/// again, though its character did not change from the hour before.
#[test]
fn prepared_leaders_read_anew_after_a_skipped_roster() {
    let first = frame(100, 1);
    let mut renamed = first.clone();
    renamed.hour = 101;
    if let TableView::Complete(TableRows::Characters(rows)) =
        &mut *renamed.tables[TableId::Characters.index()]
    {
        rows[0].token = b"Synthetic renamed while unread".to_vec();
    }
    let mut skipped = renamed.clone();
    skipped.tables[TableId::UnitLeaders.index()] = TableView::Unavailable {
        reason: "Synthetic skipped roster".into(),
    }
    .into();
    let mut frames = vec![first, skipped];
    frames.extend((102..=103).map(|hour| {
        let mut frame = renamed.clone();
        frame.hour = hour;
        frame
    }));
    let replay = replay(&frames);
    let mut state = Prepared::new(&replay, 1);
    observe_recorded(&mut state, &replay, &frames);
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    publish(&mut state, &fixture, &cancelled);
    assert_parity(&fixture, &replay);
    let result = query(
        &fixture.store,
        &fixture.key,
        &full_params("1"),
        None,
        &cancelled,
    )
    .unwrap()
    .unwrap();
    let leader = &result.leaders[0];
    assert_eq!(
        leader
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        vec![(100, 100), (102, 103)]
    );
    assert_ne!(leader.runs[0].value.person, leader.runs[1].value.person);
    assert_eq!(
        result.people[leader.runs[1].value.person as usize].portrait_hour,
        102
    );
    assert_eq!(leader.partial_from, Some(102));
}
