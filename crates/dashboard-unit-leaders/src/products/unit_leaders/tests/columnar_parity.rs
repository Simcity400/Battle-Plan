use super::*;
use crate::products::unit_leaders::prepared::{Prepared, test_fixtures::*};
use dashboard_base::prepared::{
    frame_changes::FrameChangeTracker,
    series::tests::{Fixture, assert_payloads_equal},
};
use dashboard_core::test_fixtures::{CAMPAIGN, continuation_records, records};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    container::Replay,
};
use replay_data::rows::{RowCodec, RowsVisit};
use std::sync::atomic::AtomicBool;

struct Write<'a> {
    walk: &'a mut Synthetic,
    table: TableId,
    missing: Option<u32>,
    read: bool,
}

impl RowsVisit for Write<'_> {
    type Output = ();

    fn rows<R: RowCodec>(self, rows: &[R]) {
        self.walk
            .table(
                self.table,
                Input::Snapshot {
                    rows,
                    partial: self.missing,
                    read: self.read,
                    sample: Sample::FromReadBit,
                },
            )
            .unwrap();
    }
}

fn write_hour(walk: &mut Synthetic, input: &DecodedFrame) {
    for &table in block::tables() {
        let read = input.metrics.table_ns[table.index()] != 0;
        match &*input.tables[table.index()] {
            TableView::Complete(rows) => rows.visit(Write {
                walk,
                table,
                missing: None,
                read,
            }),
            TableView::Truncated { rows, missing } => rows.visit(Write {
                walk,
                table,
                missing: Some(*missing),
                read,
            }),
            TableView::Unavailable { reason } => walk
                .table::<replay_data::rows::CountryRow>(
                    table,
                    Input::Unavailable {
                        reason,
                        read,
                        sample: Sample::FromReadBit,
                    },
                )
                .unwrap(),
            TableView::NotRecorded { .. } => walk
                .table::<replay_data::rows::CountryRow>(
                    table,
                    Input::NotRecorded {
                        read,
                        sample: Sample::FromReadBit,
                    },
                )
                .unwrap(),
        }
    }
}

fn flush(state: &mut Prepared, files: &Fixture, content: Option<&Content>) {
    let mut writer = files.writer();
    state
        .flush(&mut writer, 1, content, &[], &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
}

fn assert_publications_and_resume(files: &[Fixture; 2], label: &str) -> Prepared {
    let mut payloads = files.each_ref().map(Fixture::payloads);
    let continuation = ("unit-leaders-1".to_owned(), 0);
    let saved = payloads[1].remove(&continuation).unwrap();
    assert!(payloads[0].remove(&continuation).is_some());
    // Native continuations keep projection state; compare replies and resume the saved continuation.
    assert_payloads_equal(&payloads[1], &payloads[0], label);
    assert_eq!(
        serde_json::to_value(reply(&files[1])).unwrap(),
        serde_json::to_value(reply(&files[0])).unwrap(),
        "{label}",
    );
    let (keys, metadata) = dashboard_base::prepared::series::read_catalogue::<prepared::Metadata>(
        &files[1].store,
        &files[1].key,
        "unit-leaders",
        1,
    )
    .unwrap()
    .unwrap();
    Prepared::read(saved.as_slice(), keys, metadata).unwrap()
}

/// Block `at` of `replay` folded into `row` through production's block path, its change
/// tracking fresh as the block starts.
fn row_reference(replay: &Replay, at: usize, row: &mut Prepared, content: Option<&Content>) {
    let selection = block::bound(content.is_some());
    let mut extract = (selection.start)(replay.format);
    let mut tracker = FrameChangeTracker::new();
    tracker.invalidate();
    replay
        .read_block_alone(
            &replay.index[at],
            &selection.tables,
            |raw, frame| {
                let facts = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                extract.observe(replay, frame, &facts)
            },
            || Ok(()),
        )
        .unwrap();
    block::fold(
        row,
        extract.finish().unwrap(),
        replay,
        &replay.index[at],
        content,
        &mut |_, result| result.map(|()| true),
        &mut || Ok(()),
    )
    .unwrap();
}

/// The row path over `blocks`, block `at` of run `runs[at]`, and the projection over segments
/// that start at each block's first hour and also at each hour of `restarts`, both published
/// at each block's end, publish the same sections.
fn restarted(
    blocks: &[Vec<DecodedFrame>],
    runs: &[usize],
    restarts: &BTreeSet<i32>,
    content: Option<Arc<Content>>,
) {
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, blocks[0].clone())]).unwrap();
    for frames in &blocks[1..] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames.clone())])
            .unwrap();
    }
    assert_eq!(replay.index.len(), blocks.len());
    let files = [Fixture::new(), Fixture::new()];
    let mut row = Prepared::new(&replay, 1);
    let mut projected = Prepared::new(&replay, 1);
    let mut hours: Vec<Hour> = Vec::new();
    let mut walk = Synthetic::new(block::tables(), 1, blocks[0][0].hour).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    for (at, inputs) in blocks.iter().enumerate() {
        let run_starts = at == 0 || runs[at] != runs[at - 1];
        if run_starts && at > 0 {
            walk = Synthetic::new(block::tables(), 1, inputs[0].hour).unwrap();
        }
        let mut operator = None;
        for (ordinal, input) in inputs.iter().enumerate() {
            let opens = ordinal == 0 || restarts.contains(&input.hour);
            if opens {
                let mut segment = Projector::new(&replay, content.clone(), content.is_some());
                let start = if ordinal == 0 && run_starts {
                    Start::Run { continues: at > 0 }
                } else {
                    Start::Restart(walk.state())
                };
                assert!(segment.starts_at(start));
                segment.start(start, 1, input.hour).unwrap();
                operator = Some(segment);
            }
            let segment = operator.as_mut().expect("an open segment");
            write_hour(&mut walk, input);
            walk.end(|state, manifest, _, walk_starts| {
                let world = dashboard_world::World::at(
                    &mut world_held,
                    walk_starts,
                    &crate::products::unit_leaders::columnar::needs(false),
                    (
                        replay_data::format::FormatVersion::CURRENT,
                        replay_data::scope::CaptureMode::Full,
                    ),
                    state,
                    manifest,
                )
                .unwrap();
                segment
                    .hour(
                        &segments::Hour {
                            branch: 1,
                            hour: input.hour,
                            run: runs[at] as u32,
                            ordinal: ordinal as u32,
                            starts: opens,
                            opens,
                            state,
                            manifest,
                            world,
                        },
                        &mut hours,
                    )
                    .unwrap()
            })
            .unwrap();
            if inputs
                .get(ordinal + 1)
                .is_none_or(|next| restarts.contains(&next.hour))
            {
                segment.end(&mut hours).unwrap();
                assert_eq!(hours.len(), 1);
                for hour in std::mem::take(&mut hours) {
                    projected.observe_projected(hour).unwrap();
                }
            }
        }
        row_reference(&replay, at, &mut row, content.as_deref());
        flush(&mut row, &files[0], content.as_deref());
        flush(&mut projected, &files[1], content.as_deref());
        projected = assert_publications_and_resume(
            &files,
            &format!("leaders restarting at {restarts:?}, block {at}"),
        );
    }
}

/// [`restarted`] with no restart, a restart at every hour, and a restart at each single hour.
fn restarted_everywhere(
    blocks: &[Vec<DecodedFrame>],
    runs: &[usize],
    content: Option<Arc<Content>>,
) {
    let hours: Vec<i32> = blocks.iter().flatten().map(|frame| frame.hour).collect();
    restarted(blocks, runs, &BTreeSet::new(), content.clone());
    restarted(
        blocks,
        runs,
        &hours.iter().copied().collect(),
        content.clone(),
    );
    for &hour in &hours {
        restarted(blocks, runs, &[hour].into(), content.clone());
    }
}

/// One segment's projection of `frames`, handed on.
fn segment(frames: &[DecodedFrame], content: Option<Arc<Content>>) -> Hour {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let mut operator = Projector::new(&replay, content.clone(), content.is_some());
    let mut hours: Vec<Hour> = Vec::new();
    let mut walk = Synthetic::new(block::tables(), 1, frames[0].hour).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    for (ordinal, input) in frames.iter().enumerate() {
        write_hour(&mut walk, input);
        walk.end(|state, manifest, _, walk_starts| {
            let world = dashboard_world::World::at(
                &mut world_held,
                walk_starts,
                &crate::products::unit_leaders::columnar::needs(false),
                (
                    replay_data::format::FormatVersion::CURRENT,
                    replay_data::scope::CaptureMode::Full,
                ),
                state,
                manifest,
            )
            .unwrap();
            operator
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: input.hour,
                        run: 0,
                        ordinal: ordinal as u32,
                        starts: ordinal == 0,
                        opens: ordinal == 0,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap()
        })
        .unwrap();
    }
    operator.end(&mut hours).unwrap();
    hours.pop().unwrap()
}

/// A leader's runs as their first and last hours.
fn spans(leader: &LeaderCareer) -> Vec<(i32, i32)> {
    leader
        .runs
        .iter()
        .map(|run| (run.first_hour, run.last_hour))
        .collect()
}

/// A leader's career in `result`.
fn career(result: &UnitLeadersReply, index: u32) -> &LeaderCareer {
    result
        .leaders
        .iter()
        .find(|leader| leader.reference == reference(index).to_string())
        .unwrap()
}

fn parity(runs: &[Vec<DecodedFrame>]) {
    parity_with_content(runs, None);
}

fn parity_with_content(runs: &[Vec<DecodedFrame>], content: Option<Arc<Content>>) {
    for split in 0..runs[0].len() {
        let mut blocks = Vec::new();
        let mut run_of = Vec::new();
        for (run, frames) in runs.iter().enumerate() {
            if run == 0 && split > 0 {
                blocks.push(frames[..split].to_vec());
                run_of.push(run);
                blocks.push(frames[split..].to_vec());
                run_of.push(run);
            } else {
                blocks.push(frames.clone());
                run_of.push(run);
            }
        }
        let mut replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, blocks[0].clone())]).unwrap();
        for frames in &blocks[1..] {
            replay = replay
                .append(&[continuation_records(&replay, 1, frames.clone())])
                .unwrap();
        }
        assert_eq!(replay.index.len(), blocks.len());
        let owned = content.clone();
        let files = [Fixture::new(), Fixture::new()];
        let mut row = Prepared::new(&replay, 1);
        let mut projected = Prepared::new(&replay, 1);
        let mut walk = Synthetic::new(block::tables(), 1, blocks[0][0].hour).unwrap();
        let mut world_held: Option<dashboard_world::World> = None;
        for (at, inputs) in blocks.iter().enumerate() {
            let later_run = at == 0 || run_of[at] != run_of[at - 1];
            if later_run && at > 0 {
                walk = Synthetic::new(block::tables(), 1, inputs[0].hour).unwrap();
            }
            let mut operator = Projector::new(&replay, owned.clone(), owned.is_some());
            let mut hours: Vec<Hour> = Vec::new();
            let start = if later_run {
                Start::Run { continues: at > 0 }
            } else {
                Start::Restart(walk.state())
            };
            assert!(operator.starts_at(start));
            operator.start(start, 1, inputs[0].hour).unwrap();
            for (ordinal, input) in inputs.iter().enumerate() {
                write_hour(&mut walk, input);
                walk.end(|state, manifest, _, walk_starts| {
                    let world = dashboard_world::World::at(
                        &mut world_held,
                        walk_starts,
                        &crate::products::unit_leaders::columnar::needs(false),
                        (
                            replay_data::format::FormatVersion::CURRENT,
                            replay_data::scope::CaptureMode::Full,
                        ),
                        state,
                        manifest,
                    )
                    .unwrap();
                    operator
                        .hour(
                            &segments::Hour {
                                branch: 1,
                                hour: input.hour,
                                run: run_of[at] as u32,
                                ordinal: ordinal as u32,
                                starts: ordinal == 0,
                                opens: ordinal == 0,
                                state,
                                manifest,
                                world,
                            },
                            &mut hours,
                        )
                        .unwrap();
                })
                .unwrap();
            }
            operator.end(&mut hours).unwrap();
            assert_eq!(hours.len(), 1);
            for hour in hours {
                projected.observe_projected(hour).unwrap();
            }
            let selection = block::bound(content.is_some());
            let mut extract = (selection.start)(replay.format);
            let mut tracker = FrameChangeTracker::new();
            tracker.invalidate();
            replay
                .read_block_alone(
                    &replay.index[at],
                    &selection.tables,
                    |raw, frame| {
                        let facts = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                        extract.observe(&replay, frame, &facts)
                    },
                    || Ok(()),
                )
                .unwrap();
            block::fold(
                &mut row,
                extract.finish().unwrap(),
                &replay,
                &replay.index[at],
                content.as_deref(),
                &mut |_, result| result.map(|()| true),
                &mut || Ok(()),
            )
            .unwrap();
            flush(&mut row, &files[0], content.as_deref());
            flush(&mut projected, &files[1], content.as_deref());
            projected = assert_publications_and_resume(
                &files,
                &format!("leaders split {split}, block {at}"),
            );
        }
        let payloads = files[1].payloads();
        let names: BTreeSet<_> = payloads.keys().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "unit-leaders-1",
                "unit-leaders-1-catalogue",
                "unit-leaders-events-1"
            ]
            .into()
        );
    }
}

fn refused(mut input: DecodedFrame, table: TableId, reason: &str) -> DecodedFrame {
    input.tables[table.index()] = TableView::Unavailable {
        reason: reason.into(),
    }
    .into();
    input
}

fn partial(mut input: DecodedFrame, missing: u32) -> DecodedFrame {
    let TableView::Complete(rows) = &*input.tables[TableId::UnitLeaders.index()] else {
        unreachable!();
    };
    let mut rows = rows.clone();
    rows.truncate(1);
    input.tables[TableId::UnitLeaders.index()] = TableView::Truncated { rows, missing }.into();
    input
}

#[test]
fn every_leaders_section_matches_at_restarts_refusals_identity_changes_and_later_runs() {
    let mut frames: Vec<_> = (100..114).map(|hour| war_frame(1, hour, 7)).collect();
    for input in &mut frames {
        if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *input.tables[TableId::UnitLeaders.index()]
        {
            rows.pop();
            if input.hour >= 103 {
                rows[0].experience = 123456;
                rows[0].components[0] = 9;
            }
            if input.hour == 109 {
                rows[0].captured = 1;
            }
            if input.hour >= 110 {
                rows[0].owner = 100_001;
            }
        }
        let token = if input.hour >= 108 {
            b"synthetic_second".to_vec()
        } else {
            b"synthetic_first".to_vec()
        };
        set(
            input,
            TableRows::UnitLeaderTraits(vec![UnitLeaderTraitRow {
                leader_reference: reference(0),
                ordinal: 0,
                token,
                ..Default::default()
            }]),
        );
        if input.hour >= 110 {
            if let TableView::Complete(TableRows::Characters(rows)) =
                &mut *input.tables[TableId::Characters.index()]
            {
                rows[0].token = b"Synthetic renamed commander".to_vec();
            }
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *input.tables[TableId::Countries.index()]
            {
                rows[0].tag = b"RNM".to_vec();
            }
        }
    }
    frames[2] = refused(
        frames[2].clone(),
        TableId::UnitLeaderTraits,
        "traits unavailable",
    );
    frames[4] = partial(frames[4].clone(), 1);
    frames[5] = partial(frames[5].clone(), 2);
    frames[6] = refused(
        frames[6].clone(),
        TableId::CountryLinks,
        "first link refusal",
    );
    frames[7] = refused(
        frames[7].clone(),
        TableId::CountryLinks,
        "second link refusal",
    );
    for next in [114, 120] {
        parity(&[
            frames.clone(),
            (next..next + 4).map(|hour| war_frame(1, hour, 7)).collect(),
        ]);
    }
    parity(&[outcome_frames(false)]);
    parity(&[outcome_frames(true)]);
}

#[test]
fn naval_sources_late_results_conflicts_and_repeated_commands_match_at_restarts() {
    use replay_data::rows::{
        FleetRow, NavalCombatDetailsRow, NavalCombatResultDetailsRow, NavalCombatSideDetails,
        NavalResultRow, NavalResultSideDetails, NavalResultSourceRow, OrdersGroupRow, Recorded,
    };
    let mut frames: Vec<_> = (100..106).map(|hour| frame(hour, 2)).collect();
    for input in &mut frames {
        if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *input.tables[TableId::UnitLeaders.index()]
        {
            for row in rows {
                row.kind = 2;
                row.category = 2;
            }
        }
        set(
            input,
            TableRows::OrdersGroups(vec![OrdersGroupRow {
                handle: 1,
                leader: reference(0),
                name: Recorded::Value(None),
                ..Default::default()
            }]),
        );
        set(
            input,
            TableRows::Fleets(vec![FleetRow {
                id: 1,
                owner: 100_000,
                leader: Recorded::Value(Some(reference(0))),
                name: Recorded::Value(None),
                ..Default::default()
            }]),
        );
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
    for (index, input) in frames.iter_mut().enumerate().skip(2) {
        set(
            input,
            TableRows::NavalResults(vec![NavalResultRow {
                id: 71,
                winning_side: if index == 3 { 2 } else { 1 },
                ..Default::default()
            }]),
        );
        set(
            input,
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
    parity(&[frames.clone()]);
    let (_, files) = projected(&[frames[..2].to_vec(), frames[2..].to_vec()]);
    let result = reply(&files);
    let late = result
        .leaders
        .iter()
        .find(|leader| leader.reference == reference(1).to_string())
        .unwrap();
    assert_eq!(late.battle_starts.as_deref(), Some(&[101][..]));
    assert_eq!(late.battle_start_read_hours.as_deref(), Some(&[102][..]));
    assert_eq!(late.battle_won.as_deref(), Some(&[None][..]));
}

#[test]
fn content_visibility_training_and_context_changes_match_at_restarts() {
    use replay_data::rows::{LexicalTokenRow, ScriptFlagRow, UnitLeaderTraitExperienceRow};
    use std::sync::Arc;
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/synthetic.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/unit_leader/synthetic.txt",
            "leader_skills={1={type=corps_commander cost=1}} leader_traits={synthetic_training={type=corps_commander trait_type=basic experience=10}}",
        ),
        (
            "common/characters/synthetic.txt",
            "characters={synthetic_commander={corps_commander={visible={has_country_flag=synthetic_visible}}}}",
        ),
    ]);
    let templates =
        installed_content_core::characters::leader_templates(&catalogue.content.layers).unwrap();
    let template = &templates[0];
    let mut frames: Vec<_> = (100..108).map(|hour| frame(hour, 3)).collect();
    for input in &mut frames {
        let hour = input.hour;
        for table in [TableId::Provinces, TableId::States] {
            set(input, TableRows::empty(table));
        }
        set(
            input,
            TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 900,
                    literal: b"synthetic_visible".to_vec(),
                },
                LexicalTokenRow {
                    token: 901,
                    literal: template.tag.as_bytes().to_vec(),
                },
                LexicalTokenRow {
                    token: 902,
                    literal: b"synthetic_training".to_vec(),
                },
            ]),
        );
        set(
            input,
            TableRows::ScriptFlags(if hour < 103 || hour >= 106 {
                vec![ScriptFlagRow {
                    scope_kind: 1,
                    scope_id: 100_000,
                    token: 900,
                    value: 1,
                    ..Default::default()
                }]
            } else {
                Vec::new()
            }),
        );
        set(
            input,
            TableRows::UnitLeaderTraitExperience(vec![UnitLeaderTraitExperienceRow {
                leader_reference: reference(0),
                list: 0,
                ordinal: 0,
                token_id: 902,
                definition_ordinal: 0,
                experience: i64::from(hour) * 100_000,
            }]),
        );
        if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *input.tables[TableId::UnitLeaders.index()]
        {
            rows.pop();
            rows[0].visibility_choice = 1;
        }
        if let TableView::Complete(TableRows::Characters(rows)) =
            &mut *input.tables[TableId::Characters.index()]
        {
            rows[0].template_present = 1;
            rows[0].template_tag = 901;
            rows[0].template_file = template.path.as_bytes().to_vec();
            rows[0].template_line = template.line.unwrap() as i32;
            if hour >= 102 {
                rows[1].token = b"Synthetic unrelated identity change".to_vec();
            }
        }
    }
    frames[4] = refused(
        frames[4].clone(),
        TableId::UnitLeaderTraitExperience,
        "training unavailable",
    );
    let catalogue = Arc::new(catalogue);
    let content = Content::load(
        catalogue.clone(),
        Arc::new(
            dashboard_base::products::politics_content::PoliticsContent::load(catalogue).unwrap(),
        ),
    )
    .unwrap();
    let content = Arc::new(content);
    dashboard_text::triggers::inputs::reusing(|| {
        parity_with_content(&[frames.clone()], Some(content.clone()));
        let (_, files) =
            projected_with_content(&[frames[..4].to_vec(), frames[4..].to_vec()], Some(content));
        let result = reply(&files);
        let leader = result
            .leaders
            .iter()
            .find(|leader| leader.reference == reference(0).to_string())
            .unwrap();
        assert_eq!(
            leader
                .runs
                .iter()
                .map(|run| run.first_hour)
                .collect::<Vec<_>>(),
            vec![100, 101, 102, 106, 107]
        );
        assert_eq!(leader.partial_from, Some(106));
        assert!(
            leader
                .runs
                .iter()
                .all(|run| run.value.progress.as_ref().unwrap().training.is_some())
        );
    });
}

fn projected(blocks: &[Vec<DecodedFrame>]) -> (Prepared, Fixture) {
    projected_with_content(blocks, None)
}

fn projected_with_content(
    blocks: &[Vec<DecodedFrame>],
    content: Option<Arc<Content>>,
) -> (Prepared, Fixture) {
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, blocks[0].clone())]).unwrap();
    for block in &blocks[1..] {
        replay = replay
            .append(&[continuation_records(&replay, 1, block.clone())])
            .unwrap();
    }
    let mut prepared = Prepared::new(&replay, 1);
    let files = Fixture::new();
    for block in blocks {
        let mut walk = Synthetic::new(block::tables(), 1, block[0].hour).unwrap();
        let mut world_held: Option<dashboard_world::World> = None;
        let mut operator = Projector::new(&replay, content.clone(), content.is_some());
        let mut hours: Vec<Hour> = Vec::new();
        operator
            .start(Start::Run { continues: true }, 1, block[0].hour)
            .unwrap();
        for (ordinal, input) in block.iter().enumerate() {
            write_hour(&mut walk, input);
            walk.end(|state, manifest, _, walk_starts| {
                let world = dashboard_world::World::at(
                    &mut world_held,
                    walk_starts,
                    &crate::products::unit_leaders::columnar::needs(false),
                    (
                        replay_data::format::FormatVersion::CURRENT,
                        replay_data::scope::CaptureMode::Full,
                    ),
                    state,
                    manifest,
                )
                .unwrap();
                operator
                    .hour(
                        &segments::Hour {
                            branch: 1,
                            hour: input.hour,
                            run: 0,
                            ordinal: ordinal as u32,
                            starts: ordinal == 0,
                            opens: ordinal == 0,
                            state,
                            manifest,
                            world,
                        },
                        &mut hours,
                    )
                    .unwrap()
            })
            .unwrap();
        }
        operator.end(&mut hours).unwrap();
        for hour in hours {
            prepared.observe_projected(hour).unwrap();
        }
        flush(&mut prepared, &files, content.as_deref());
    }
    (prepared, files)
}

fn reply(files: &Fixture) -> UnitLeadersReply {
    prepared::query(
        &files.store,
        &files.key,
        &prepared::full_params("1"),
        None,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap()
}

#[test]
fn individual_refusals_keep_recorded_coverage_and_match_scan_projection_and_restarts() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/synthetic.lua",
            "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/unit_leader/synthetic.txt",
            "leader_skills={1={type=corps_commander cost=1}} leader_traits={}",
        ),
    ]);
    let catalogue = Arc::new(catalogue);
    let content = Arc::new(
        Content::load(
            catalogue.clone(),
            Arc::new(politics_content::PoliticsContent::load(catalogue).unwrap()),
        )
        .unwrap(),
    );
    let mut frames: Vec<_> = (100..109).map(|hour| frame(hour, 3)).collect();
    let owner = frames[0].complete::<tables::Countries>().unwrap()[0]
        .tag
        .clone();
    for input in &mut frames {
        for table in [TableId::Provinces, TableId::States] {
            set(input, TableRows::empty(table));
        }
        if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *input.tables[TableId::UnitLeaders.index()]
        {
            rows[0].visibility_choice = 1;
            rows[1].owner = u32::MAX;
            if input.hour >= 102 {
                rows[2].skill_level = 4;
            }
        }
    }
    frames[4].metrics.table_ns[TableId::UnitLeaders.index()] = 0;
    let rows = frames[5]
        .complete::<tables::UnitLeaders>()
        .unwrap()
        .to_vec();
    *frames[5].tables[TableId::UnitLeaders.index()] = TableView::Truncated {
        rows: TableRows::UnitLeaders(rows),
        missing: 1,
    };
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let params = prepared::full_params("1");
    let scanned = scan(&replay, &params, Some(&content), &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.leaders.len(), 1);
    assert_eq!(scanned.leaders[0].reference, reference(2).to_string());
    for hour in 100..109 {
        assert_eq!(
            scanned
                .observed
                .iter()
                .find(|span| span.first_hour <= hour && hour <= span.last_hour)
                .unwrap()
                .leaders,
            ![104, 105].contains(&hour)
        );
    }
    assert_eq!(scanned.withheld.len(), 2);
    for (index, withheld) in scanned.withheld.iter().enumerate() {
        assert_eq!(withheld.reference, reference(index as u32).to_string());
        assert_eq!(withheld.kind, 1);
        assert_eq!(
            withheld.detail.as_deref(),
            (index == 1).then_some("raw owner 4294967295 has no CountryLinks row")
        );
        assert_eq!(
            withheld.owner,
            (index == 0).then(|| String::from_utf8(owner.clone()).unwrap())
        );
        assert_eq!(
            withheld.name.as_deref(),
            Some(format!("Synthetic commander {index}").as_str())
        );
        assert_eq!(
            withheld.spans,
            vec![
                HourSpan {
                    first_hour: 100,
                    last_hour: 103
                },
                HourSpan {
                    first_hour: 106,
                    last_hour: 108
                }
            ]
        );
        assert!(
            scanned
                .warnings
                .iter()
                .any(|warning| warning.contains(&withheld.reason)
                    && warning.contains(&withheld.reference))
        );
    }
    assert!(scanned.withheld[0].reason.contains("template"));
    assert_eq!(
        scanned.withheld[1].reason,
        "Leader owner has no resolved recorded country tag"
    );
    dashboard_text::triggers::inputs::reusing(|| {
        let (_, files) = projected_with_content(
            &[frames[..3].to_vec(), frames[3..].to_vec()],
            Some(content.clone()),
        );
        let projected = reply(&files);
        assert_eq!(projected.withheld, scanned.withheld);
        assert_eq!(projected.observed, scanned.observed);
        assert_eq!(
            projected
                .leaders
                .iter()
                .map(|leader| &leader.reference)
                .collect::<Vec<_>>(),
            scanned
                .leaders
                .iter()
                .map(|leader| &leader.reference)
                .collect::<Vec<_>>()
        );
        for hour in 100..109 {
            let from = project_at(
                &projected,
                &UnitLeadersParams {
                    view: LeaderView::Leaderboard,
                    ..params.clone()
                },
                None,
                Some(hour),
                None,
            );
            let counted = project(
                &projected,
                &UnitLeadersParams {
                    view: LeaderView::Leaderboard,
                    ..params.clone()
                },
                None,
            );
            assert_eq!(from.withheld, leaderboard_from(&counted, hour).withheld);
            assert_eq!(
                from.withheld.len(),
                if [104, 105].contains(&hour) { 0 } else { 2 }
            );
        }
        restarted_everywhere(&[frames], &[0], Some(content));
    });
}

#[test]
fn deleted_refusals_end_before_quiet_or_battle_hours_and_reappear_with_the_recorded_kind() {
    for battles in [false, true] {
        let frames: Vec<_> = (100..=108)
            .map(|hour| {
                let mut input = frame(hour, 2);
                if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                    &mut *input.tables[TableId::UnitLeaders.index()]
                {
                    rows[1].owner = u32::MAX;
                    if hour == 107 {
                        rows[1].kind = 2;
                    }
                    if (101..105).contains(&hour) || hour == 108 {
                        rows.pop();
                    }
                }
                if battles && hour >= 101 {
                    set(
                        &mut input,
                        TableRows::LandCombatDetails(vec![
                            replay_data::rows::LandCombatDetailsRow {
                                combat: 91,
                                sides: [
                                    replay_data::rows::LandCombatSideDetails {
                                        commander: reference(0),
                                        ..Default::default()
                                    },
                                    Default::default(),
                                ],
                                ..Default::default()
                            },
                        ]),
                    );
                }
                input
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
        let scanned = scan(
            &replay,
            &prepared::full_params("1"),
            None,
            &AtomicBool::new(false),
        )
        .unwrap();
        let (_, files) = projected(&[frames.clone()]);
        let projected = reply(&files);
        assert_eq!(projected.withheld, scanned.withheld);
        assert_eq!(projected.withheld.len(), 2);
        assert_eq!(projected.withheld[0].kind, 1);
        assert_eq!(
            projected.withheld[0].spans,
            vec![
                HourSpan {
                    first_hour: 100,
                    last_hour: 100
                },
                HourSpan {
                    first_hour: 105,
                    last_hour: 106
                },
            ]
        );
        assert_eq!(projected.withheld[1].kind, 2);
        assert_eq!(
            projected.withheld[1].spans,
            vec![HourSpan {
                first_hour: 107,
                last_hour: 107
            }]
        );
        assert!(projected.observed.iter().all(|span| span.leaders));
        restarted_everywhere(&[frames[..4].to_vec(), frames[4..].to_vec()], &[0, 0], None);
    }
}

#[test]
fn withheld_names_renamed_while_visible_match_fresh_reads_at_restarts_and_resume() {
    let frames: Vec<_> = (100..=105)
        .map(|hour| {
            let mut input = frame(hour, 2);
            if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *input.tables[TableId::UnitLeaders.index()]
            {
                if hour != 101 {
                    rows[1].owner = u32::MAX;
                }
            }
            if let TableView::Complete(TableRows::Characters(rows)) =
                &mut *input.tables[TableId::Characters.index()]
            {
                rows[1].token = if hour == 100 {
                    b"Before".to_vec()
                } else {
                    b"After".to_vec()
                };
            }
            input
        })
        .collect();
    assert_withheld_rename_parity(&frames);
}

#[test]
fn withheld_names_renamed_while_deleted_match_fresh_reads_at_restarts_and_resume() {
    for full_deletion in [false, true] {
        let frames: Vec<_> = (100..=106)
            .map(|hour| {
                let mut input = frame(hour, 2);
                if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                    &mut *input.tables[TableId::UnitLeaders.index()]
                {
                    rows[1].owner = u32::MAX;
                    if (101..104).contains(&hour) {
                        rows.pop();
                    }
                }
                if let TableView::Complete(TableRows::Characters(rows)) =
                    &mut *input.tables[TableId::Characters.index()]
                {
                    rows[1].token = if hour < 102 {
                        b"Before".to_vec()
                    } else {
                        b"After".to_vec()
                    };
                }
                if full_deletion && hour >= 101 {
                    if let TableView::Complete(TableRows::Countries(rows)) =
                        &mut *input.tables[TableId::Countries.index()]
                    {
                        rows[0].tag = b"REN".to_vec();
                    }
                }
                input
            })
            .collect();
        assert_withheld_rename_parity(&frames);
    }
}

fn assert_withheld_rename_parity(frames: &[DecodedFrame]) {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/synthetic.lua",
            "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/unit_leader/synthetic.txt",
            "leader_skills={1={type=corps_commander cost=1}} leader_traits={}",
        ),
    ]);
    let catalogue = Arc::new(catalogue);
    let content = Arc::new(
        Content::load(
            catalogue.clone(),
            Arc::new(politics_content::PoliticsContent::load(catalogue).unwrap()),
        )
        .unwrap(),
    );
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let params = prepared::full_params("1");
    let cancelled = AtomicBool::new(false);
    let fresh: Vec<_> = frames
        .iter()
        .map(|input| {
            let replay =
                Replay::from_records(CAMPAIGN.into(), &[records(1, vec![input.clone()])]).unwrap();
            let reply = scan(&replay, &params, Some(&content), &cancelled).unwrap();
            assert_eq!(
                reply.withheld.len(),
                input
                    .complete::<tables::UnitLeaders>()
                    .unwrap()
                    .iter()
                    .filter(|row| row.owner == u32::MAX)
                    .count(),
            );
            for withheld in &reply.withheld {
                assert_eq!(withheld.reference, reference(1).to_string());
                assert_eq!(
                    withheld.name.as_deref(),
                    Some(if input.hour == 100 { "Before" } else { "After" }),
                );
            }
            reply
        })
        .collect();
    let check = |reply: &UnitLeadersReply| {
        for (input, fresh) in frames.iter().zip(&fresh) {
            let names = |reply: &UnitLeadersReply| {
                withheld_at(&reply.withheld, Some(input.hour), |_| true)
                    .into_iter()
                    .map(|row| (row.reference, row.name))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                names(reply),
                names(fresh),
                "withheld names at {}",
                input.hour
            );
        }
    };
    dashboard_text::triggers::inputs::reusing(|| {
        let scanned = scan(&replay, &params, Some(&content), &cancelled).unwrap();
        check(&scanned);
        let (_, files) = projected_with_content(&[frames.to_vec()], Some(content.clone()));
        let projected = reply(&files);
        check(&projected);
        assert_eq!(projected.withheld, scanned.withheld);
        restarted_everywhere(&[frames.to_vec()], &[0], Some(content.clone()));
        for split in 1..frames.len() {
            for projected in [false, true] {
                let files = Fixture::new();
                let mut state = Prepared::new(&replay, 1);
                let mut tracker = FrameChangeTracker::new();
                for (at, inputs) in [&frames[..split], &frames[split..]].into_iter().enumerate() {
                    if projected {
                        state
                            .observe_projected(segment(inputs, Some(content.clone())))
                            .unwrap();
                    } else {
                        for input in inputs {
                            let changes = tracker.observe(input, replay.format);
                            state
                                .observe(input, &replay, Some(&content), &changes)
                                .unwrap();
                        }
                    }
                    flush(&mut state, &files, Some(&content));
                    if at == 0 {
                        state = Prepared::resumed(
                            state.saved_bytes().unwrap().as_slice(),
                            &state.saved_catalogue().unwrap(),
                        )
                        .unwrap();
                    }
                }
                let resumed = reply(&files);
                check(&resumed);
                assert_eq!(resumed.withheld, scanned.withheld);
            }
        }
    });
}

#[test]
fn owner_refusal_steps_and_raw_values_match_scan_projection_and_restarts() {
    let frames: Vec<_> = (100..=105)
        .map(|hour| {
            let mut input = frame(hour, 2);
            if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *input.tables[TableId::UnitLeaders.index()]
            {
                rows[1].owner = if hour == 100 { 0 } else { 100_001 };
            }
            if hour == 101 {
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *input.tables[TableId::CountryLinks.index()]
                {
                    rows.retain(|row| row.id != 100_001);
                }
            }
            if hour == 102 {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *input.tables[TableId::Countries.index()]
                {
                    rows.pop();
                }
            }
            if hour >= 103 {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *input.tables[TableId::Countries.index()]
                {
                    rows[1].tag = if hour == 103 { Vec::new() } else { vec![0xff] };
                }
            }
            input
        })
        .collect();
    let slot = frames[0]
        .complete::<tables::CountryLinks>()
        .unwrap()
        .iter()
        .find(|row| row.id == 100_001)
        .unwrap()
        .canonical_slot;
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let scanned = scan(
        &replay,
        &prepared::full_params("1"),
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    let (_, files) = projected(&[frames.clone()]);
    let projected = reply(&files);
    assert_eq!(projected.withheld, scanned.withheld);
    // Raw owner 0 (hour 100) is no country: a retired character's leader, in no roster.
    assert_eq!(projected.withheld.len(), 4);
    let details = [
        "raw owner 100001 has no CountryLinks row".to_owned(),
        format!("raw owner 100001 links to slot {slot}, which has no Countries row"),
        format!("raw owner 100001 links to slot {slot}, whose Countries row has no tag"),
        format!(
            "raw owner 100001 links to slot {slot}, whose Countries row has a tag that isn't UTF-8"
        ),
    ];
    for (at, detail) in details.iter().enumerate() {
        let row = projected
            .withheld
            .iter()
            .find(|row| row.detail.as_ref() == Some(detail))
            .unwrap();
        assert_eq!(row.owner, None);
        assert_eq!(
            row.reason,
            "Leader owner has no resolved recorded country tag"
        );
        assert_eq!(
            row.spans,
            vec![HourSpan {
                first_hour: 101 + at as i32,
                last_hour: if at == 3 { 105 } else { 101 + at as i32 },
            }]
        );
    }
    restarted_everywhere(&[frames], &[0], None);
}

#[test]
fn a_1500_country_hour_resolves_only_the_changed_leader_and_extends_other_runs() {
    let first = frame(100, 1500);
    let mut next = frame(101, 1500);
    if let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *next.tables[TableId::UnitLeaders.index()]
    {
        rows[1499].experience = 123456;
    }
    for recorded_reads in [false, true] {
        let mut frames = vec![first.clone(), next.clone()];
        if recorded_reads {
            for input in &mut frames {
                for &table in block::tables() {
                    input.metrics.table_ns[table.index()] = 1;
                }
            }
        }
        let hour = segment(&frames, None);
        assert_eq!(hour.counts.walks, [1, 1]);
        assert_eq!(hour.work.resolved, 1501);
        assert_eq!(hour.scan.stored_runs.len(), 1501);
        assert_eq!(hour.scan.stored_runs[0].run.last_hour, 101);
        assert_eq!(hour.scan.leaders[0].visible_hour, Some(101));
        assert_eq!(
            hour.scan
                .stored_runs
                .last()
                .unwrap()
                .run
                .value
                .progress
                .as_ref()
                .unwrap()
                .experience_raw,
            "123456"
        );
    }
}

#[test]
fn a_skipped_roster_breaks_runs_and_cannot_hide_an_identity_or_trait_change() {
    let mut frames: Vec<_> = (100..105).map(|hour| frame(hour, 2)).collect();
    for input in &mut frames {
        let hour = input.hour;
        if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *input.tables[TableId::UnitLeaders.index()]
        {
            rows.pop();
        }
        if hour >= 101 {
            if let TableView::Complete(TableRows::Characters(rows)) =
                &mut *input.tables[TableId::Characters.index()]
            {
                rows[0].token = b"Synthetic renamed across skipped roster".to_vec();
            }
        }
        set(
            input,
            TableRows::UnitLeaderTraits(vec![UnitLeaderTraitRow {
                leader_reference: reference(0),
                ordinal: 0,
                token: if hour < 101 {
                    b"synthetic_before".to_vec()
                } else {
                    b"synthetic_after".to_vec()
                },
                ..Default::default()
            }]),
        );
    }
    frames[1] = refused(
        frames[1].clone(),
        TableId::UnitLeaders,
        "synthetic skipped roster",
    );
    parity(&[frames.clone()]);
    let (_, files) = projected(&[frames]);
    let result = reply(&files);
    assert_eq!(result.leaders.len(), 1);
    let leader = &result.leaders[0];
    assert_eq!(leader.partial_from, Some(102));
    assert_eq!(leader.runs.len(), 2);
    assert_eq!(
        (leader.runs[0].first_hour, leader.runs[0].last_hour),
        (100, 100)
    );
    assert_eq!(
        (leader.runs[1].first_hour, leader.runs[1].last_hour),
        (102, 104)
    );
    assert_eq!(
        result.people[leader.runs[1].value.person as usize].portrait_hour,
        102
    );
    let gain = leader
        .trait_gains
        .iter()
        .find(|gain| result.traits[gain.token as usize].token == "synthetic_after")
        .unwrap();
    assert_eq!(gain.hour, 102);
    assert!(matches!(gain.date, LeaderChangeDate::FirstObserved));
    assert!(
        result
            .observed
            .iter()
            .any(|span| span.first_hour == 101 && !span.leaders)
    );
}

#[test]
fn unknown_command_names_stitch_before_later_creation_in_captured_order() {
    use replay_data::rows::{OrdersGroupRow, Recorded};
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/synthetic.lua",
            "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/unit_leader/synthetic.txt",
            "leader_skills={1={type=corps_commander cost=1}} leader_traits={}",
        ),
    ]);
    let catalogue = Arc::new(catalogue);
    let content = Arc::new(
        Content::load(
            catalogue.clone(),
            Arc::new(
                dashboard_base::products::politics_content::PoliticsContent::load(catalogue)
                    .unwrap(),
            ),
        )
        .unwrap(),
    );
    let mut frames: Vec<_> = (100..105).map(|hour| frame(hour, 1)).collect();
    for input in &mut frames {
        let at = input.hour;
        let mut groups = Vec::new();
        if at >= 102 {
            groups.push(OrdersGroupRow {
                handle: 1,
                leader: reference(0),
                name: Recorded::Value(Some(b"Synthetic other".to_vec())),
                ..Default::default()
            });
        }
        groups.push(OrdersGroupRow {
            handle: 2,
            leader: reference(0),
            name: Recorded::Value(if at == 100 {
                Some(b"Synthetic old".to_vec())
            } else if at >= 103 {
                Some(b"Synthetic new".to_vec())
            } else {
                None
            }),
            ..Default::default()
        });
        set(input, TableRows::OrdersGroups(groups));
    }
    parity_with_content(&[frames.clone()], Some(content.clone()));
    let (_, files) =
        projected_with_content(&[frames[..1].to_vec(), frames[1..].to_vec()], Some(content));
    let result = reply(&files);
    let commands = &result.leaders[0].commands;
    assert_eq!(commands.len(), 3);
    assert_eq!(
        commands
            .iter()
            .map(|command| command.first_hour)
            .collect::<Vec<_>>(),
        vec![100, 102, 103]
    );
    assert_eq!(commands[0].last_hour, 102);
    assert_eq!(commands[0].name_unknown.len(), 1);
    assert_eq!(
        (
            commands[0].name_unknown[0].first_hour,
            commands[0].name_unknown[0].last_hour
        ),
        (101, 102)
    );
    assert_eq!(commands[2].name_read_hour, Some(103));
}

#[test]
fn a_combat_first_seen_at_a_segment_boundary_uses_the_previous_census() {
    use replay_data::rows::{LandCombatDetailsRow, LandCombatSideDetails};
    let first = frame(100, 1);
    let mut next = frame(101, 1);
    set(
        &mut next,
        TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
            combat: 91,
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
    let (_, files) = projected(&[vec![first], vec![next]]);
    let result = reply(&files);
    assert_eq!(result.leaders[0].battle_starts.as_deref(), Some(&[101][..]));
    assert_eq!(
        result.leaders[0].battle_start_read_hours.as_deref(),
        Some(&[101][..])
    );
    assert_eq!(
        result.leaders[0].unknown_battle_starts.as_deref(),
        Some(&[][..])
    );
}

#[test]
fn quiet_hours_before_a_restricted_read_keep_coverage_and_untouched_endpoints() {
    let mut frames: Vec<_> = (100..105).map(|hour| frame(hour, 2)).collect();
    if let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *frames[3].tables[TableId::UnitLeaders.index()]
    {
        rows[1].experience = 123456;
    }
    frames[4] = frames[3].clone();
    frames[4].hour = 104;
    parity(&[frames.clone()]);
    let (_, files) = projected(&[frames]);
    let result = reply(&files);
    assert_eq!(result.observed.len(), 1);
    assert_eq!(
        (result.observed[0].first_hour, result.observed[0].last_hour),
        (100, 104)
    );
    let untouched = result
        .leaders
        .iter()
        .find(|leader| leader.reference == reference(0).to_string())
        .unwrap();
    assert_eq!(untouched.runs.len(), 1);
    assert_eq!(
        (untouched.runs[0].first_hour, untouched.runs[0].last_hour),
        (100, 104)
    );
}

/// Each table of `after` differing from `before`'s changed anywhere, and the leaders an hour
/// inserted, as their decoded delta tells them.
struct Inserted<'a> {
    before: Option<&'a DecodedFrame>,
    after: &'a DecodedFrame,
    leaders: &'a [DeltaRow],
}

impl TableChanges for Inserted<'_> {
    fn table(&self, table: TableId) -> TableChange<'_> {
        match self.before {
            Some(_) if table == TableId::UnitLeaders && !self.leaders.is_empty() => {
                TableChange::Rows(self.leaders, &[])
            }
            Some(before) if before.tables[table.index()] == self.after.tables[table.index()] => {
                TableChange::Same
            }
            _ => TableChange::Any,
        }
    }
}

/// A leader gone from the roster while its formation stays bound to it commands nothing from
/// then on, though no command table changed.
#[test]
fn a_command_bound_to_a_leader_gone_from_the_roster_ends_with_its_leader() {
    use replay_data::rows::{OrdersGroupRow, Recorded};
    let mut frames: Vec<_> = (100..105).map(|hour| frame(hour, 2)).collect();
    for input in &mut frames {
        if input.hour >= 102
            && let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *input.tables[TableId::UnitLeaders.index()]
        {
            rows.pop();
        }
        set(
            input,
            TableRows::OrdersGroups(vec![OrdersGroupRow {
                handle: 1,
                leader: reference(1),
                name: Recorded::Value(Some(b"Synthetic army".to_vec())),
                ..Default::default()
            }]),
        );
    }
    parity(&[frames.clone()]);
    let (_, files) = projected(&[frames]);
    let result = reply(&files);
    let gone = result
        .leaders
        .iter()
        .find(|leader| leader.reference == reference(1).to_string())
        .unwrap();
    assert_eq!(
        gone.commands
            .iter()
            .map(|command| (command.first_hour, command.last_hour))
            .collect::<Vec<_>>(),
        vec![(100, 101)]
    );
}

/// War hours move every country's balances, task force positions, mission fuel and combat
/// sides: the projection reads again only each changed combat's sides, walks every leader only
/// at the segment's first hour, and carries every other hour unread.
#[test]
fn war_hours_read_only_the_fields_leaders_hold() {
    let frames: Vec<_> = (100..=110).map(|hour| war_frame(1, hour, 6)).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let mut operator = Projector::new(&replay, None, false);
    let mut hours: Vec<Hour> = Vec::new();
    let mut walk = Synthetic::new(block::tables(), 1, 100).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    let watched = asks(&holds(false));
    for (ordinal, input) in frames.iter().enumerate() {
        write_hour(&mut walk, input);
        walk.end(|state, manifest, _, walk_starts| {
            let world = dashboard_world::World::at(
                &mut world_held,
                walk_starts,
                &crate::products::unit_leaders::columnar::needs(false),
                (
                    replay_data::format::FormatVersion::CURRENT,
                    replay_data::scope::CaptureMode::Full,
                ),
                state,
                manifest,
            )
            .unwrap();
            operator
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: input.hour,
                        run: 0,
                        ordinal: ordinal as u32,
                        starts: ordinal == 0,
                        opens: ordinal == 0,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap();
            let held = world.frame_held(state, &watched).unwrap();
            assert_eq!(
                held.complete::<tables::Countries>().unwrap()[0].political_power_raw,
                Some(100_000)
            );
            assert_eq!(
                held.complete::<tables::TaskForces>().unwrap()[0].province,
                100
            );
            assert_eq!(
                held.complete::<tables::NavalMissionTaskForces>().unwrap()[0].fuel,
                100
            );
            assert_eq!(
                held.complete::<tables::LandCombatDetails>().unwrap()[0].sides[0].committed,
                vec![input.hour as u64 * 3]
            );
        })
        .unwrap();
    }
    operator.end(&mut hours).unwrap();
    let hour = hours.pop().unwrap();
    let local_decoded = |table: TableId| hour.counts.decoded[table.index()];
    assert_eq!(hour.counts.hours, 11);
    assert_eq!(hour.counts.walks, [1, 0]);
    assert_eq!(hour.counts.resolved, 6);
    assert_eq!(hour.counts.quiet, 10);
    assert_eq!(hour.counts.fallbacks, 0);
    assert_eq!(local_decoded(TableId::Characters), 6);
    assert_eq!(local_decoded(TableId::CharacterPortraits), 6);
    for &table in block::tables() {
        if ![TableId::Characters, TableId::CharacterPortraits].contains(&table) {
            assert_eq!(local_decoded(table), 0, "{table:?}");
        }
    }
}

/// Two leaders whose template visibility reads a flag: the flag cleared at 103 and set again
/// at 106 hides both meanwhile; an unrelated flag set at 101 reads the same verdicts. Every
/// segment starting after a change reads the changed input cold.
#[test]
fn flag_visibility_and_inputs_changed_in_earlier_segments_match_at_every_restart() {
    use replay_data::rows::{LexicalTokenRow, ScriptFlagRow, UnitLeaderTraitExperienceRow};
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/synthetic.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/unit_leader/synthetic.txt",
            "leader_skills={1={type=corps_commander cost=1}} leader_traits={synthetic_training={type=corps_commander trait_type=basic experience=10}}",
        ),
        (
            "common/characters/synthetic.txt",
            "characters={synthetic_commander={corps_commander={visible={has_country_flag=synthetic_visible}}}}",
        ),
    ]);
    let templates =
        installed_content_core::characters::leader_templates(&catalogue.content.layers).unwrap();
    let template = &templates[0];
    let frames: Vec<_> = (100..110)
        .map(|hour| {
            let mut input = frame(hour, 3);
            for table in [TableId::Provinces, TableId::States] {
                set(&mut input, TableRows::empty(table));
            }
            set(
                &mut input,
                TableRows::LexicalTokens(vec![
                    LexicalTokenRow {
                        token: 900,
                        literal: b"synthetic_visible".to_vec(),
                    },
                    LexicalTokenRow {
                        token: 901,
                        literal: template.tag.as_bytes().to_vec(),
                    },
                    LexicalTokenRow {
                        token: 902,
                        literal: b"synthetic_training".to_vec(),
                    },
                    LexicalTokenRow {
                        token: 903,
                        literal: b"synthetic_unrelated".to_vec(),
                    },
                ]),
            );
            let mut flags = Vec::new();
            if !(103..106).contains(&hour) {
                flags.push(ScriptFlagRow {
                    scope_kind: 1,
                    scope_id: 100_000,
                    token: 900,
                    value: 1,
                    ..Default::default()
                });
            }
            if hour >= 101 {
                flags.push(ScriptFlagRow {
                    scope_kind: 1,
                    scope_id: 100_000,
                    token: 903,
                    value: 1,
                    ..Default::default()
                });
            }
            set(&mut input, TableRows::ScriptFlags(flags));
            set(
                &mut input,
                TableRows::UnitLeaderTraitExperience(vec![UnitLeaderTraitExperienceRow {
                    leader_reference: reference(0),
                    list: 0,
                    ordinal: 0,
                    token_id: 902,
                    definition_ordinal: 0,
                    experience: i64::from(hour) * 100_000,
                }]),
            );
            if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *input.tables[TableId::UnitLeaders.index()]
            {
                rows.pop();
                for row in rows.iter_mut() {
                    row.visibility_choice = 1;
                    // Both leaders are owned by the flag's country.
                    row.owner = 100_000;
                }
            }
            if let TableView::Complete(TableRows::Characters(rows)) =
                &mut *input.tables[TableId::Characters.index()]
            {
                for row in rows.iter_mut().take(2) {
                    row.template_present = 1;
                    row.template_tag = 901;
                    row.template_file = template.path.as_bytes().to_vec();
                    row.template_line = template.line.unwrap() as i32;
                }
            }
            input
        })
        .collect();
    let catalogue = Arc::new(catalogue);
    let content = Arc::new(
        Content::load(
            catalogue.clone(),
            Arc::new(
                dashboard_base::products::politics_content::PoliticsContent::load(catalogue)
                    .unwrap(),
            ),
        )
        .unwrap(),
    );
    dashboard_text::triggers::inputs::reusing(|| {
        restarted_everywhere(
            &[frames[..5].to_vec(), frames[5..].to_vec()],
            &[0, 0],
            Some(content.clone()),
        );
        let (_, files) = projected_with_content(&[frames.clone()], Some(content.clone()));
        let result = reply(&files);
        for index in [0, 1] {
            assert_eq!(
                spans(career(&result, index))
                    .iter()
                    .map(|span| span.0)
                    .filter(|&first| (103..106).contains(&first))
                    .count(),
                0,
                "a hidden leader has no run"
            );
        }
        assert_eq!(spans(career(&result, 1)), vec![(100, 102), (106, 109)]);
        assert_eq!(career(&result, 1).partial_from, Some(106));
        // One segment reads both leaders at its first hour, then only the trained leader and
        // the leaders whose verdict a flag changed.
        let hour = segment(&frames, Some(content.clone()));
        assert_eq!(hour.counts.walks[0], 1);
        assert_eq!(hour.counts.walks[1], 9);
        assert!(hour.counts.fallbacks <= 2);
        assert!(hour.counts.suspects >= 4);
    });
}

/// Leader 0 gains experience and training once a day (at 108 and 132), leader 2's skill rises
/// at 140, leader 1 never changes: only those hours read a leader, and only the changed one.
#[test]
fn daily_experience_reads_only_its_changed_leader_at_every_restart() {
    use replay_data::rows::{LexicalTokenRow, UnitLeaderTraitExperienceRow};
    let frames: Vec<_> = (100..=150)
        .map(|hour| {
            let ticks = i64::from(hour >= 108) + i64::from(hour >= 132);
            let mut input = frame(hour, 3);
            if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *input.tables[TableId::UnitLeaders.index()]
            {
                rows[0].experience = ticks * 1_000;
                if hour >= 140 {
                    rows[2].skill_level = 4;
                }
            }
            set(
                &mut input,
                TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 902,
                    literal: b"synthetic_training".to_vec(),
                }]),
            );
            set(
                &mut input,
                TableRows::UnitLeaderTraitExperience(vec![UnitLeaderTraitExperienceRow {
                    leader_reference: reference(0),
                    list: 0,
                    ordinal: 0,
                    token_id: 902,
                    definition_ordinal: 0,
                    experience: ticks * 100_000,
                }]),
            );
            input
        })
        .collect();
    restarted_everywhere(
        &[frames[..26].to_vec(), frames[26..].to_vec()],
        &[0, 0],
        None,
    );
    let (_, files) = projected(&[frames.clone()]);
    let result = reply(&files);
    assert_eq!(
        spans(career(&result, 0)),
        vec![(100, 107), (108, 131), (132, 150)]
    );
    assert_eq!(spans(career(&result, 1)), vec![(100, 150)]);
    assert_eq!(spans(career(&result, 2)), vec![(100, 139), (140, 150)]);
    let hour = segment(&frames, None);
    assert_eq!(hour.counts.walks, [1, 3]);
    assert_eq!(hour.counts.quiet, 47);
    assert_eq!(hour.counts.resolved, 6);
    assert_eq!(hour.counts.runs, 6);
}

/// Leader 1 leaves the roster at 104 and joins it again at 107, its army bound to it all along:
/// its career and its command break there and resume, the career partial from 107.
#[test]
fn a_leader_deleted_and_added_again_matches_at_every_restart() {
    use replay_data::rows::{OrdersGroupRow, Recorded};
    let frames: Vec<_> = (100..=110)
        .map(|hour| {
            let mut input = frame(hour, 2);
            if (104..107).contains(&hour)
                && let TableView::Complete(TableRows::UnitLeaders(rows)) =
                    &mut *input.tables[TableId::UnitLeaders.index()]
            {
                rows.pop();
            }
            set(
                &mut input,
                TableRows::OrdersGroups(vec![OrdersGroupRow {
                    handle: 1,
                    leader: reference(1),
                    name: Recorded::Value(Some(b"Synthetic army".to_vec())),
                    ..Default::default()
                }]),
            );
            input
        })
        .collect();
    restarted_everywhere(&[frames[..6].to_vec(), frames[6..].to_vec()], &[0, 0], None);
    let (_, files) = projected(&[frames.clone()]);
    let result = reply(&files);
    let leader = career(&result, 1);
    assert_eq!(spans(leader), vec![(100, 103), (107, 110)]);
    assert_eq!(leader.partial_from, Some(107));
    assert_eq!(
        leader
            .commands
            .iter()
            .map(|command| (command.first_hour, command.last_hour))
            .collect::<Vec<_>>(),
        vec![(100, 103), (107, 110)]
    );
    assert_eq!(spans(career(&result, 0)), vec![(100, 110)]);
}

/// A recording from before fleets recorded their leader joins its naval commands through its
/// missions: a leader joining the roster, a mission added and a mission conflicting with
/// another for one hour bind as the row path binds them every hour.
#[test]
fn fleet_missions_before_fleets_recorded_their_leader_bind_as_the_row_path() {
    use replay_data::{
        format::FormatVersion,
        rows::{FleetRow, NavalMissionTaskForceRow, Recorded, TaskForceRow, compatibility},
    };
    let format = (compatibility::PROVEN_FROM..compatibility::WIDGET_CAPTURE_FORMAT)
        .rev()
        .find_map(|version| FormatVersion::read(version).ok())
        .expect("a proven format before fleets recorded their leader");
    assert!(format.field_gaps(TableId::Fleets).next().is_some());
    let frames: Vec<_> = (100..=106)
        .map(|hour| {
            let mut input = frame(hour, 2);
            for &table in LEADER_TABLES.iter().chain(COMMAND_TABLES) {
                input.metrics.table_ns[table.index()] = 1;
            }
            if let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *input.tables[TableId::UnitLeaders.index()]
            {
                for row in rows.iter_mut() {
                    row.kind = 2;
                    row.category = 2;
                }
                if hour < 102 {
                    rows.pop();
                }
            }
            set(
                &mut input,
                TableRows::Fleets(
                    [200, 201]
                        .map(|id| FleetRow {
                            id,
                            owner: 100_000,
                            leader: Recorded::NotRecorded,
                            name: Recorded::NotRecorded,
                            ..Default::default()
                        })
                        .to_vec(),
                ),
            );
            set(
                &mut input,
                TableRows::TaskForces(
                    [(7, 200), (8, 201), (9, 200)]
                        .map(|(id, fleet)| TaskForceRow {
                            id,
                            fleet,
                            ..Default::default()
                        })
                        .to_vec(),
                ),
            );
            let mut missions = vec![NavalMissionTaskForceRow {
                task_force: 7,
                leader: reference(0),
                ..Default::default()
            }];
            if hour >= 103 {
                missions.push(NavalMissionTaskForceRow {
                    task_force: 8,
                    leader: reference(1),
                    ..Default::default()
                });
            }
            if hour == 105 {
                missions.push(NavalMissionTaskForceRow {
                    task_force: 9,
                    leader: reference(1),
                    ..Default::default()
                });
            }
            set(&mut input, TableRows::NavalMissionTaskForces(missions));
            input
        })
        .collect();
    let fields = TableId::UnitLeaders.schema().len();
    let joined = [DeltaRow {
        format: format.version(),
        key: u128::from(reference(1)),
        mask: full_mask(fields),
        recorded_fields: fields,
        values: Vec::new(),
    }];
    let params = prepared::full_params("1");
    let mut oracle = Scan::at(format, 1, 100);
    let mut scan = Scan::at(format, 1, 100);
    scan.reuse_evaluations = true;
    scan.projection = Some(Work::default());
    for (at, input) in frames.iter().enumerate() {
        oracle
            .observe_rows(
                input,
                format,
                &params,
                None,
                None,
                None,
                None,
                None,
                &mut |_, _, _| {},
            )
            .unwrap();
        let tables = Inserted {
            before: at.checked_sub(1).map(|before| &frames[before]),
            after: input,
            leaders: if input.hour == 102 { &joined } else { &[] },
        };
        let facts = FrameChanges::rebuilt(input, format, at > 0, |table| {
            !matches!(tables.table(table), TableChange::Same)
        });
        match plan(&mut scan, input, format, None, &facts, &tables, None).unwrap() {
            Step::Quiet => scan.advance(input.hour),
            Step::Battles => {
                let (land, navy) = battle_tables(format, input);
                scan.observe_battles(input, format, land, navy, &tables);
            }
            Step::Read {
                rows,
                leaders,
                gone,
            } => {
                prepare(&mut scan, rows.is_none(), &leaders, &gone);
                scan.frontier = Some(input.hour);
                scan.materialized = Some(input.hour);
                let since = Since::hour_before(input, &tables);
                assert!(
                    scan.observe_rows(
                        input,
                        format,
                        &params,
                        None,
                        Some(&tables),
                        since.as_ref(),
                        None,
                        rows.as_deref(),
                        &mut |_, _, _| {},
                    )
                    .unwrap()
                );
                index(&mut scan, input, rows.is_none());
            }
        }
    }
    let hour = hand(scan, 100, Counts::default(), Default::default(), Vec::new()).unwrap();
    assert_eq!(
        hour.counts.walks[1], 4,
        "only the hours missions change read"
    );
    let mut stitched = Scan::at(format, 1, 100);
    hour.stitch(&mut stitched, &BTreeMap::new(), &BTreeMap::new())
        .unwrap();
    assert_eq!(stitched.reply.observed, oracle.reply.observed);
    assert_eq!(
        stitched.refusals.lines("leaders"),
        oracle.refusals.lines("leaders")
    );
    for leader in &oracle.leaders {
        let at = stitched.leader_indices[&leader.reference];
        assert_eq!(
            stitched.leaders[at].career.commands, leader.career.commands,
            "leader {}",
            leader.reference
        );
    }
    let commands = |index: u32| {
        let at = stitched.leader_indices[&reference(index)];
        stitched.leaders[at]
            .career
            .commands
            .iter()
            .map(|command| {
                (
                    command.formation.clone(),
                    command.first_hour,
                    command.last_hour,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        commands(0),
        vec![("200".to_owned(), 100, 104), ("200".to_owned(), 106, 106)]
    );
    assert_eq!(commands(1), vec![("201".to_owned(), 103, 106)]);
    assert!(
        stitched
            .reply
            .observed
            .iter()
            .any(|read| read.first_hour == 105 && !read.naval_commands)
    );
}
