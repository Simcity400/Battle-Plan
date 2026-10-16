use super::*;
use crate::prepared::frame_changes::FrameChangeTracker;
use crate::prepared::series::tests::Fixture;
use crate::test_fixtures::{CAMPAIGN, records, scale_frame};
use replay::{
    frame::TableView,
    rows::{
        LandCombatDetailsRow, LandCombatResultsRow, LandCombatSideDetails, LandResultSide,
        OrdersGroupRow, Recorded, TableRows,
    },
};

fn frame(hour: i32, countries: u32) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, countries);
    for rows in [
        TableRows::UnitLeaders(
            (0..countries)
                .map(|index| UnitLeaderRow {
                    id: index + 1,
                    reference_type: 4713,
                    owner: 100_000 + index,
                    kind: 1,
                    category: 1,
                    character_id: index + 1,
                    skill_status: 2,
                    skill_level: 3,
                    components: [1, 2, 3, 4],
                    ..Default::default()
                })
                .collect(),
        ),
        TableRows::Characters(
            (0..countries)
                .map(|index| CharacterRow {
                    id: index + 1,
                    token: format!("Synthetic commander {index}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ),
        TableRows::CharacterPortraits(
            (0..countries)
                .map(|index| CharacterPortraitRow {
                    character: index + 1,
                    portraits: Vec::new(),
                })
                .collect(),
        ),
        TableRows::UnitLeaderTraits(Vec::new()),
        TableRows::LandCombatDetails(Vec::new()),
        TableRows::NavalCombatDetails(Vec::new()),
        TableRows::LandCombatResults(Vec::new()),
        TableRows::NavalResultSources(Vec::new()),
        TableRows::NavalResults(Vec::new()),
        TableRows::NavalCombatResultDetails(Vec::new()),
        TableRows::OrdersGroups(Vec::new()),
        TableRows::Fleets(Vec::new()),
    ] {
        set(&mut frame, rows);
    }
    frame
}
fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let table = rows.table();
    *frame.tables[table.index()] = TableView::Complete(rows);
}
fn reference(index: u32) -> u64 {
    (u64::from(index + 1) << 32) | 4713
}
fn replay(frames: &[DecodedFrame]) -> Replay {
    Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap()
}
fn publish(state: &mut Prepared, fixture: &Fixture, cancelled: &AtomicBool) {
    let mut reader = Reader::new(&fixture.store, &fixture.key, section(1), cancelled).unwrap();
    let mut writer = fixture.writer();
    state
        .flush(&mut writer, 1, None, &mut reader, &[], cancelled)
        .unwrap();
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
fn observe(
    state: &mut Prepared,
    fixture: &Fixture,
    replay: &Replay,
    frames: &[DecodedFrame],
    cancelled: &AtomicBool,
) {
    let mut reader = Reader::new(&fixture.store, &fixture.key, section(1), cancelled).unwrap();
    let mut tracker = FrameChangeTracker::new();
    for frame in frames {
        let changes = tracker.observe(frame, replay.format);
        state
            .observe(frame, replay, None, &mut reader, &changes)
            .unwrap();
    }
}
fn assert_parity(fixture: &Fixture, replay: &Replay) {
    let cancelled = AtomicBool::new(false);
    let full = scan(replay, &full_params("1"), None, &cancelled).unwrap();
    let history =
        super::super::super::country_history::scan_histories(replay, 1, false, &[], &cancelled)
            .unwrap();
    for params in [
        UnitLeadersParams {
            view: LeaderView::Leaderboard,
            ..full_params("1")
        },
        UnitLeadersParams {
            view: LeaderView::Choices,
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
                tags: vec![String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap()],
            },
            view: LeaderView::Leaderboard,
            ..full_params("1")
        },
        UnitLeadersParams {
            selection: LeaderSelection::Countries {
                tags: vec![String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap()],
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
        let actual = query(
            &fixture.store,
            &fixture.key,
            &params,
            Some(&history),
            &cancelled,
        )
        .unwrap()
        .unwrap();
        crate::prepared::observer_tests::assert_reply_parity(
            &actual,
            &project(&full, &params, Some(&history)),
        )
        .unwrap();
    }
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
        observe(
            &mut state,
            &fixture,
            &replay,
            &frames[end - 2..end],
            &cancelled,
        );
        publish(&mut state, &fixture, &cancelled);
        assert_parity(&fixture, &replay);
        state = resume(&fixture);
        assert!(state.scan.battle_hours.is_empty());
        assert!(state.scan.stored_runs.len() <= state.scan.leaders.len() * 2);
    }
}

pub(crate) fn outcome_frames(conflict: bool) -> Vec<DecodedFrame> {
    let mut first = frame(100, 2);
    let mut changed = frame(101, 2);
    let mut ended = frame(102, 2);
    for (frame, commander) in [(&mut first, reference(0)), (&mut changed, reference(1))] {
        set(
            frame,
            TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
                combat: 91,
                sides: [
                    LandCombatSideDetails {
                        commander,
                        ..Default::default()
                    },
                    Default::default(),
                ],
                ..Default::default()
            }]),
        );
    }
    set(
        &mut ended,
        TableRows::LandCombatResults(vec![LandCombatResultsRow {
            id: 1,
            combat: 91,
            end_hour: 102,
            winners: 1,
            sides: [
                LandResultSide {
                    commander: reference(1),
                    ..Default::default()
                },
                Default::default(),
            ],
            ..Default::default()
        }]),
    );
    let mut repeated = ended.clone();
    repeated.hour = 103;
    if conflict
        && let TableView::Complete(TableRows::LandCombatResults(rows)) =
            &mut *repeated.tables[TableId::LandCombatResults.index()]
    {
        rows[0].winners = 2;
    }
    vec![first, changed, ended, repeated]
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
        observe(
            &mut state,
            &fixture,
            &replay,
            &frames[begin..end],
            &cancelled,
        );
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
        observe(
            &mut state,
            &fixture,
            &replay,
            &frames[begin..end],
            &cancelled,
        );
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
    use replay::rows::{
        NavalCombatDetailsRow, NavalCombatResultDetailsRow, NavalCombatSideDetails, NavalResultRow,
        NavalResultSideDetails, NavalResultSourceRow,
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
        observe(
            &mut state,
            &fixture,
            &replay,
            &frames[end - 1..end],
            &cancelled,
        );
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_leaders_five_thousand_countries_keep_continuation_bounded_by_current_facts() {
    let frames = [frame(100, 5000), frame(101, 5000), frame(103, 5000)];
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut state = Prepared::new(&replay(&frames), 1);
    for (index, frame) in frames.iter().enumerate() {
        let replay = replay(&frames[..=index]);
        observe(
            &mut state,
            &fixture,
            &replay,
            std::slice::from_ref(frame),
            &cancelled,
        );
        publish(&mut state, &fixture, &cancelled);
        assert_parity(&fixture, &replay);
        assert_eq!(state.scan.leaders.len(), 5000);
        assert!(state.scan.stored_runs.len() <= 10_000);
        eprintln!(
            "unit-leaders continuation: {} JSON / {} compressed bytes at 5000 countries, hour {}",
            serde_json::to_vec(&state).unwrap().len(),
            state.saved_bytes().unwrap().len(),
            frame.hour
        );
        state = resume(&fixture);
    }
}
#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_leader_long_career_moves_closed_commands_and_battles_out_of_continuation() {
    let first = frame(100, 2);
    let replay = replay(std::slice::from_ref(&first));
    let fixture = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut state = Prepared::new(&replay, 1);
    for batch in 0..4 {
        let mut reader = Reader::new(&fixture.store, &fixture.key, section(1), &cancelled).unwrap();
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
            state
                .observe(&next, &replay, None, &mut reader, &changes)
                .unwrap();
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
        observe(
            &mut state,
            &fixture,
            &replay,
            &frames[begin..end],
            &cancelled,
        );
        publish(&mut state, &fixture, &cancelled);
        state = resume(&fixture);
    }
    let full = scan(&replay, &full_params("1"), None, &cancelled).unwrap();
    let history =
        super::super::super::country_history::scan_histories(&replay, 1, false, &[], &cancelled)
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
            tags: vec![String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap()],
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
            let actual = query(
                &fixture.store,
                &fixture.key,
                &params,
                Some(&history),
                &cancelled,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(project(&full, &params, Some(&history))).unwrap()
            );
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

fn work(state: &Prepared) -> crate::prepared::observer_tests::Work {
    crate::prepared::observer_tests::Work {
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
) -> io::Result<crate::prepared::observer_tests::Work> {
    use crate::prepared::observer_tests::{self, Work};
    use replay::rows::{
        NavalCombatResultDetailsRow, NavalResultRow, NavalResultSideDetails, NavalResultSourceRow,
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
    let mut reader = Reader::new(&fixture.store, &fixture.key, section(1), &flag)?;
    for frame in &frames[..=closed] {
        let changes = tracker.observe(frame, replay.format);
        state.observe(frame, &replay, content, &mut reader, &changes)?;
    }
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, content, &mut reader, &[], &flag)?;
    writer.commit()?;
    state = resume(&fixture);
    reader = Reader::new(&fixture.store, &fixture.key, section(1), &flag)?;
    tracker.invalidate();
    let changes = tracker.observe(&first, replay.format);
    state.observe(&first, &replay, content, &mut reader, &changes)?;
    let baseline = work(&state);
    let buffered = state.buffered_bytes();
    let delta = if change.is_some() {
        let next = frames.last().unwrap();
        let changes = tracker.observe(next, replay.format);
        state.observe(next, &replay, content, &mut reader, &changes)?;
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
                state.observe(frame, &replay, content, &mut reader, changes)?;
                Ok(work(&state))
            },
        )?
    };
    if change.is_none() {
        assert_eq!(
            state.buffered_bytes(),
            buffered,
            "unchanged carry allocates no pending facts"
        );
    }
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, content, &mut reader, &[], &flag)?;
    writer.commit()?;
    state = resume(&fixture);
    assert_eq!(state.scan.leaders.len(), countries as usize + closed);
    let oracle = scan(&replay, &full_params("1"), content, &flag)?;
    let actual = query(&fixture.store, &fixture.key, &full_params("1"), None, &flag)?.unwrap();
    observer_tests::assert_reply_parity(&actual, &project(&oracle, &full_params("1"), None))?;
    Ok(delta)
}

#[test]
fn prepared_leaders_shared_driver_unchanged_admission_work_is_flat_in_hours_and_history() {
    use crate::prepared::observer_tests::{self, Work};
    let histories = observer_tests::scaled_work(4, |history| {
        let hours =
            observer_tests::scaled_work(4, |hours| admission_work(history, hours, 2, None, None))?;
        observer_tests::assert_flat(hours);
        assert_eq!(
            hours,
            [Work::default(); 2],
            "equal reads must not revisit native indexes or admission histories"
        );
        Ok(hours[0])
    })
    .unwrap();
    observer_tests::assert_flat(histories);
}

#[test]
fn prepared_leaders_shared_driver_one_changed_admission_is_bounded_after_ten_times_history() {
    use crate::prepared::observer_tests;
    let histories = observer_tests::scaled_work(4, |history| {
        admission_work(history, 1, 2, Some(TableId::UnitLeaders), None)
    })
    .unwrap();
    observer_tests::assert_flat(histories);
    assert_eq!(histories[0].history_visits, 1);
    assert_eq!(
        histories[0].row_visits, 13,
        "changed native roster plus supplying result/command rows; no country join or retained roster"
    );
    assert_eq!(
        histories[0].emitted, 3,
        "new kind, country and native-holder admission"
    );
    assert_eq!(histories[0].content_loads, 0);
}

#[test]
#[cfg(feature = "slow-tests")]
fn prepared_leaders_shared_driver_five_thousand_current_rows_keep_admission_work_bounded() {
    use crate::prepared::observer_tests;
    let unchanged =
        observer_tests::scaled_work(2, |history| admission_work(history, 10, 5000, None, None))
            .unwrap();
    observer_tests::assert_flat(unchanged);
    assert_eq!(unchanged[0], Default::default());
    let changed = observer_tests::scaled_work(2, |history| {
        admission_work(history, 1, 5000, Some(TableId::UnitLeaders), None)
    })
    .unwrap();
    observer_tests::assert_flat(changed);
    assert_eq!(changed[0].history_visits, 1);
    assert_eq!(changed[0].row_visits, 10_009);
    assert_eq!(changed[0].emitted, 3);
}

#[test]
#[cfg(feature = "slow-tests")]
fn prepared_leaders_shared_driver_configured_content_success_and_failure_never_reload() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::prepared::observer_tests;
    for refused in [false, true] {
        let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
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
    use crate::prepared::observer_tests;
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
    observe(&mut state, &fixture, &initial, &prefix, &flag);
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
    crate::prepared::observer_tests::assert_reply_parity(
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
        .append(&[crate::test_fixtures::continuation_records(
            &initial,
            1,
            tail.clone(),
        )])
        .unwrap();
    observe(&mut state, &fixture, &grown, &tail, &flag);
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
    use replay::rows::FleetRow;
    let mut base = frame(100, 1);
    if let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *base.tables[TableId::UnitLeaders.index()]
    {
        rows[0].kind = engine::cunit_leader::KIND_NAVY;
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
    crate::prepared::observer_tests::assert_reply_parity(
        &actual,
        &scan(&replay, &params, None, &AtomicBool::new(false)).unwrap(),
    )
    .unwrap();
}
