use super::forward::{Combat, hour_frame, recording};
use super::*;
use crate::products::land_battles::prepared::{self as forward_query, KEEP_ALL};
use dashboard_base::prepared::{
    frame_changes::FrameChangeTracker, observer_tests, series, series::tests::Fixture,
};
use replay_data::rows::{
    LandCombatResultsRow, LandEventCoverageRow, LandResultSide, LandSideLosses,
};
use serde_json::{Value, json};

/// Prepares `branch` block by block, as forward preparation does, restoring
/// the census from its saved state after every commit when `restore`. Returns
/// the battles held in memory after each block.
fn prepare(
    source: &Source,
    branch: u64,
    fixture: &Fixture,
    keep: bool,
    restore: bool,
) -> Vec<usize> {
    KEEP_ALL.set(keep);
    let flag = AtomicBool::new(false);
    let replay = source.replay_ref();
    let context = LoadContext { content: None };
    let mut census = Census::new(replay.format);
    let mut tracker = FrameChangeTracker::new();
    let mut held = Vec::new();
    for (ordinal, index) in replay
        .index
        .iter()
        .filter(|index| index.branch == branch)
        .enumerate()
    {
        replay
            .read_block_alone(
                index,
                &CENSUS_TABLES,
                |_, frame| {
                    census.configure(source, &context);
                    let changes = tracker.observe(frame, replay.format);
                    census.observe_prepared(frame, &changes)
                },
                || Ok(()),
            )
            .unwrap();
        let mut writer = fixture.writer();
        census
            .publish_block(&mut writer, replay, branch, ordinal, &flag)
            .unwrap();
        writer.commit().unwrap();
        assert!(census.pending.is_empty() && census.dirty.is_empty());
        held.push(census.episodes.len());
        if restore {
            census = serde_json::from_slice(&serde_json::to_vec(&census).unwrap()).unwrap();
            tracker.invalidate();
        }
    }
    KEEP_ALL.set(false);
    held
}

/// The ranking ranges [`served`] asks for start from `first` through `last`, `step` apart.
fn ranges(windows: &[BattleWindow]) -> (i32, i32, usize) {
    let first = windows[0].first_hour - 8;
    let last = windows.last().unwrap().last_hour + 2;
    (first, last, ((last - first) as usize / 20).max(5))
}

/// Every reply the prepared store serves for `branch`: the manifest, each
/// window, rankings over many ranges, and the course of each of `ids`.
fn served(source: &Source, branch: u64, fixture: &Fixture, ids: &[String]) -> Vec<Value> {
    let flag = AtomicBool::new(false);
    let query = |product: &str, params: Value| {
        forward_query::query(source, product, params, &fixture.store, &fixture.key, &flag)
            .unwrap_or_else(|error| panic!("{product}: {error}"))
    };
    let branch_id = branch.to_string();
    let mut replies = vec![query(
        "land-battles-manifest",
        json!({ "branchId": branch_id }),
    )];
    let windows = super::super::windows(source.replay_ref(), branch);
    for window in 0..windows.len() {
        replies.push(query(
            "land-battles",
            json!({ "branchId": branch_id, "window": window }),
        ));
    }
    let (first, last, step) = ranges(&windows);
    for measure in ["Casualties", "Peak size", "Length"] {
        for first_hour in (first..=last).step_by(step) {
            for last_hour in [first_hour, first_hour + 3, first_hour + 17, last] {
                for top_n in [3.0, 10_000.0] {
                    replies.push(query(
                        "land-battles-ranking",
                        serde_json::to_value(LandBattlesRankingParams {
                            branch_id: branch_id.clone(),
                            first_hour,
                            last_hour: last_hour.max(first_hour),
                            top_n,
                            measure: measure.into(),
                            countries: LandBattleCountryFilter {
                                tags: None,
                                membership: None,
                                unobserved: vec![],
                            },
                        })
                        .unwrap(),
                    ));
                }
            }
        }
    }
    for id in ids {
        replies.push(query(
            "land-battle-course",
            json!({ "branchId": branch_id, "id": id }),
        ));
    }
    replies
}

/// What the oracle, which scans the whole branch into memory, answers for
/// the same requests as [`served`].
fn oracle_served(source: &Source, branch: u64, ids: &[String]) -> Vec<Value> {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let census = census(source, branch, &flag, &context).unwrap();
    let windows = super::super::windows(source.replay_ref(), branch);
    let mut replies = vec![
        serde_json::to_value(
            LandBattlesManifest::load(
                source,
                &LandBattlesManifestParams {
                    branch_id: branch.to_string(),
                },
                &context,
                &flag,
            )
            .unwrap(),
        )
        .unwrap(),
    ];
    for window in 0..windows.len() {
        let params = LandBattlesParams {
            branch_id: branch.to_string(),
            window,
        };
        replies.push(serde_json::to_value(census.reply(&params, &windows).unwrap()).unwrap());
    }
    let (first, last, step) = ranges(&windows);
    for measure in ["Casualties", "Peak size", "Length"] {
        for first_hour in (first..=last).step_by(step) {
            for last_hour in [first_hour, first_hour + 3, first_hour + 17, last] {
                for top_n in [3.0, 10_000.0] {
                    let params = LandBattlesRankingParams {
                        branch_id: branch.to_string(),
                        first_hour,
                        last_hour: last_hour.max(first_hour),
                        top_n,
                        measure: measure.into(),
                        countries: LandBattleCountryFilter {
                            tags: None,
                            membership: None,
                            unobserved: vec![],
                        },
                    };
                    replies.push(
                        serde_json::to_value(census.ranked(&params, &flag).unwrap()).unwrap(),
                    );
                }
            }
        }
    }
    for id in ids {
        replies.push(serde_json::to_value(census.course(id).unwrap()).unwrap());
    }
    replies
}

fn finalizer(
    id: u64,
    combat: u64,
    province: u32,
    end_hour: i32,
    countries: [u32; 2],
) -> LandCombatResultsRow {
    LandCombatResultsRow {
        id,
        combat,
        province,
        end_hour,
        duration: 9,
        winners: 2,
        sides: std::array::from_fn(|index| LandResultSide {
            countries: vec![countries[index]],
            ..Default::default()
        }),
        losses: std::array::from_fn(|index| LandSideLosses {
            manpower: 30 + index as i32,
            equipment: vec![LandEquipmentLoss {
                variant: 8,
                amount_raw: -70_000,
            }],
        }),
        ..Default::default()
    }
}

/// Finalizers read at `sample`'s hour, covering the hours after `from`.
fn finalize(sample: &mut DecodedFrame, from: i32, rows: Vec<LandCombatResultsRow>) {
    *sample.tables[TableId::LandCombatResults.index()] =
        TableView::Complete(TableRows::LandCombatResults(rows));
    *sample.tables[TableId::LandEventCoverage.index()] =
        TableView::Complete(TableRows::LandEventCoverage(vec![LandEventCoverageRow {
            from_hour: from,
            to_hour: sample.hour,
            complete: true,
            ..Default::default()
        }]));
}

fn at_hour(blocks: &mut [Vec<DecodedFrame>], hour: i32) -> &mut DecodedFrame {
    blocks
        .iter_mut()
        .flatten()
        .find(|sample| sample.hour == hour)
        .unwrap()
}

/// Short battles closing every few hours, a long one, a recorded gap, and
/// finalizers: some hourly, and some whose coverage reaches back to battles
/// several windows old, which by then are paged out.
fn campaign() -> Source {
    let present = |hour: i32| {
        let mut present = Vec::new();
        if hour <= 190 {
            present.push(Combat {
                lost: Some(hour - 99),
                ..Combat::new(1, 100)
            });
        }
        if (110..=120).contains(&hour) {
            present.push(Combat::new(3, 110));
        }
        for i in 0..28 {
            let start = 100 + 3 * i;
            if (start..=start + 4).contains(&hour) {
                present.push(Combat {
                    attackers: 1 + (hour - start) as u64 % 3,
                    lost: Some(2 * (hour - start) + i),
                    ..Combat::new(100 + i as u64, start)
                });
            }
        }
        // A combat handle reused after its old battle was paged.
        if (168..=170).contains(&hour) {
            present.push(Combat::new(110, 168));
        }
        present
    };
    let hours = [
        100..=105,
        106..=111,
        112..=117,
        118..=123,
        124..=129,
        130..=135,
        136..=141,
        142..=147,
        148..=153,
        // Hours 154 to 159 are not recorded.
        160..=165,
        166..=171,
        172..=177,
        178..=183,
    ];
    let mut blocks: Vec<Vec<DecodedFrame>> = hours
        .into_iter()
        .map(|hours| {
            hours
                .map(|hour| hour_frame(1, hour, present(hour)))
                .collect()
        })
        .collect();
    // Hourly: battle 105 (hours 115 to 119) ends as its row goes.
    finalize(
        at_hour(&mut blocks, 120),
        119,
        vec![finalizer(1, 105, 105, 120, [1, 2])],
    );
    // Battle 102 (hours 106 to 110) ends in block 1; its finalizer, with an
    // unresolved country, is read in block 6.
    finalize(
        at_hour(&mut blocks, 140),
        109,
        vec![finalizer(2, 102, 102, 111, [1, 99])],
    );
    // A finalizer of 104 (hours 112 to 116) in another province keeps its own battle.
    finalize(
        at_hour(&mut blocks, 150),
        113,
        vec![finalizer(3, 104, 9_999, 116, [1, 2])],
    );
    // One dated before the recording's first window opens its first interval earlier.
    finalize(
        at_hour(&mut blocks, 151),
        90,
        vec![finalizer(4, 7_777, 7_777, 95, [99, 2])],
    );
    // Two of one combat: the first ends paged battle 108, the second keeps its own.
    finalize(
        at_hour(&mut blocks, 162),
        125,
        vec![
            finalizer(5, 108, 108, 129, [1, 2]),
            finalizer(6, 108, 108, 129, [2, 1]),
        ],
    );
    // Paged battle 110 (hours 130 to 134) ends just before its handle's new battle is read.
    finalize(
        at_hour(&mut blocks, 167),
        131,
        vec![finalizer(7, 110, 110, 135, [1, 2])],
    );
    finalize(
        at_hour(&mut blocks, 171),
        170,
        vec![finalizer(8, 110, 110, 171, [1, 2])],
    );
    recording(blocks)
}

fn battle_ids(source: &Source) -> Vec<String> {
    let flag = AtomicBool::new(false);
    let census = census(source, 1, &flag, &LoadContext { content: None }).unwrap();
    census
        .episodes
        .values()
        .map(|episode| episode.id.clone())
        .collect()
}

#[test]
fn paged_preparation_serves_what_kept_preparation_and_the_oracle_serve() {
    let source = campaign();
    let ids = battle_ids(&source);
    assert!(
        [
            "1:102:106",
            "1:104:result:3",
            "1:7777:result:4",
            "1:108:result:6",
            "1:110:168"
        ]
        .iter()
        .all(|id| ids.iter().any(|known| known == id)),
        "{ids:?}"
    );
    let kept = Fixture::new();
    let kept_held = prepare(&source, 1, &kept, true, false);
    let paged = Fixture::new();
    let paged_held = prepare(&source, 1, &paged, false, false);
    let restored = Fixture::new();
    let restored_held = prepare(&source, 1, &restored, false, true);
    assert_eq!(*kept_held.last().unwrap(), ids.len());
    assert!(
        paged_held.iter().all(|held| *held < ids.len() / 2),
        "{paged_held:?}"
    );
    assert_eq!(paged_held, restored_held);
    let expected = oracle_served(&source, 1, &ids);
    for (name, fixture) in [("kept", &kept), ("paged", &paged), ("restored", &restored)] {
        let actual = served(&source, 1, fixture, &ids);
        assert_eq!(actual.len(), expected.len());
        for (at, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(actual, expected, "{name} reply {at}");
        }
    }
    let late = |fixture: &Fixture| {
        forward_query::query(
            &source,
            "land-battles",
            json!({ "branchId": "1", "window": 1 }),
            &fixture.store,
            &fixture.key,
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let window = late(&paged);
    let ended = window["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "1:102:106")
        .unwrap();
    assert_eq!(
        ended["result"]["hour"], 111,
        "a paged battle's late finalizer: {ended}"
    );
    assert!(
        window["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "Participant country reference 99 unresolved")
    );
}

#[test]
fn paged_preparation_restores_to_the_same_pages_and_state() {
    let source = campaign();
    let replay = source.replay_ref();
    let mut blocks = Vec::new();
    for index in &replay.index {
        let mut frames = Vec::new();
        replay
            .read_block_alone(
                index,
                &CENSUS_TABLES,
                |_, frame| {
                    frames.push(frame.clone());
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        blocks.push(frames);
    }
    let blocks: Vec<&[DecodedFrame]> = blocks.iter().map(Vec::as_slice).collect();
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    observer_tests::assert_restore_continues(
        replay.format,
        &blocks,
        || Census::new(replay.format),
        |census, frame, changes, _| {
            census.configure(&source, &context);
            census.observe_prepared(frame, changes).unwrap();
        },
        |census, writer, ordinal| {
            census.configure(&source, &context);
            census
                .publish_block(writer, replay, 1, ordinal, &flag)
                .unwrap();
            series::write_reply(writer, "land-battle-state-1", 0, &*census).unwrap();
        },
        |fixture| {
            series::read_reply(&fixture.store, &fixture.key, "land-battle-state-1", 0)
                .unwrap()
                .unwrap()
        },
    );
}

#[test]
fn paged_census_holds_open_battles_and_those_its_window_ended() {
    // Battle i is read at hours 100 + i to 102 + i, so three are open at any
    // hour and ten end in each ten-hour block.
    let battles = 400;
    let present = |hour: i32| -> Vec<Combat> {
        (hour - 102..=hour - 100)
            .filter(|i| (0..battles).contains(i))
            .map(|i| Combat {
                lost: Some(hour - 100 - i + 1),
                ..Combat::new(1_000 + i as u64, 100 + i)
            })
            .collect()
    };
    let blocks: Vec<Vec<DecodedFrame>> = (0..40)
        .map(|block| {
            (100 + 10 * block..=109 + 10 * block)
                .map(|hour| hour_frame(1, hour, present(hour)))
                .collect()
        })
        .collect();
    let source = recording(blocks);
    let flag = AtomicBool::new(false);
    let replay = source.replay_ref();
    let context = LoadContext { content: None };
    let fixture = Fixture::new();
    let mut prepared = Census::new(replay.format);
    let mut tracker = FrameChangeTracker::new();
    for (ordinal, index) in replay.index.iter().enumerate() {
        replay
            .read_block_alone(
                index,
                &CENSUS_TABLES,
                |_, frame| {
                    prepared.configure(&source, &context);
                    let changes = tracker.observe(frame, replay.format);
                    prepared.observe_prepared(frame, &changes)
                },
                || Ok(()),
            )
            .unwrap();
        let mut writer = fixture.writer();
        prepared
            .publish_block(&mut writer, replay, 1, ordinal, &flag)
            .unwrap();
        writer.commit().unwrap();
        let first = 100 + 10 * ordinal as i32;
        assert_eq!(prepared.created, (10 * ordinal + 10).min(battles as usize));
        assert_eq!(prepared.open.len(), 3, "block {ordinal}");
        // Ten battles end in each block after the first, which ended seven.
        assert_eq!(
            prepared.episodes.len(),
            if ordinal == 0 { 10 } else { 13 },
            "block {ordinal}"
        );
        assert!(prepared.latest.len() <= prepared.episodes.len());
        assert_eq!(prepared.observed, [(100, first + 9)]);
        // Every hour of the block warns of its unnamed equipment design, and no earlier one is held.
        assert_eq!(
            prepared
                .warnings
                .hourly()
                .into_iter()
                .map(|(hour, _, _)| hour)
                .collect::<Vec<_>>(),
            (first..first + 10).collect::<Vec<_>>(),
            "block {ordinal}"
        );
        // Three battles read losses each hour from the third on: the reason is held once, with one run.
        assert_eq!(
            prepared.warnings.len(),
            if ordinal == 0 { 3 } else { 1 },
            "block {ordinal}"
        );
    }
    let ids: Vec<String> = census(&source, 1, &flag, &context)
        .unwrap()
        .episodes
        .values()
        .step_by(37)
        .map(|episode| episode.id.clone())
        .collect();
    assert_eq!(
        served(&source, 1, &fixture, &ids),
        oracle_served(&source, 1, &ids)
    );
}

#[test]
fn paged_histories_take_the_same_pages_whatever_the_hash_order() {
    // Six battles are read every hour of six ten-hour blocks, so every block pages each
    // battle's earlier runs; a census's open map has a fresh hash order each time.
    let present = |hour: i32| {
        (0..6)
            .map(|i| Combat {
                attackers: 1 + ((hour - 100) / 15) as u64 % 3,
                lost: Some(hour - 99),
                ..Combat::new(1 + i, 100)
            })
            .collect::<Vec<_>>()
    };
    let blocks: Vec<Vec<DecodedFrame>> = (0..6)
        .map(|block| {
            (100 + 10 * block..=109 + 10 * block)
                .map(|hour| hour_frame(1, hour, present(hour)))
                .collect()
        })
        .collect();
    let source = recording(blocks);
    let pages = || {
        let flag = AtomicBool::new(false);
        let replay = source.replay_ref();
        let context = LoadContext { content: None };
        let fixture = Fixture::new();
        let mut census = Census::new(replay.format);
        let mut tracker = FrameChangeTracker::new();
        let mut pages = Vec::new();
        for (ordinal, index) in replay.index.iter().enumerate() {
            replay
                .read_block_alone(
                    index,
                    &CENSUS_TABLES,
                    |_, frame| {
                        census.configure(&source, &context);
                        let changes = tracker.observe(frame, replay.format);
                        census.observe_prepared(frame, &changes)
                    },
                    || Ok(()),
                )
                .unwrap();
            let mut writer = fixture.writer();
            census
                .publish_block(&mut writer, replay, 1, ordinal, &flag)
                .unwrap();
            writer.commit().unwrap();
            pages.push(
                census
                    .episodes
                    .iter()
                    .map(|(&at, episode)| (at, episode.history))
                    .collect::<Vec<_>>(),
            );
        }
        pages
    };
    let first = pages();
    let last = first.last().unwrap();
    assert_eq!(last.len(), 6);
    assert!(last.iter().all(|(_, history)| history.is_some()));
    for _ in 0..4 {
        assert_eq!(pages(), first);
    }
}

#[test]
fn paged_census_holds_only_a_long_battles_newest_runs_and_reads() {
    // One battle is read every hour of 40 ten-hour blocks: its roster changes
    // every 15 hours and its losses every hour.
    let present = |hour: i32| {
        vec![Combat {
            attackers: 1 + ((hour - 100) / 15) as u64 % 3,
            lost: Some(hour - 99),
            ..Combat::new(1, 100)
        }]
    };
    let blocks: Vec<Vec<DecodedFrame>> = (0..40)
        .map(|block| {
            (100 + 10 * block..=109 + 10 * block)
                .map(|hour| hour_frame(1, hour, present(hour)))
                .collect()
        })
        .collect();
    let source = recording(blocks);
    let flag = AtomicBool::new(false);
    let replay = source.replay_ref();
    let context = LoadContext { content: None };
    let fixture = Fixture::new();
    let mut prepared = Census::new(replay.format);
    let mut tracker = FrameChangeTracker::new();
    for (ordinal, index) in replay.index.iter().enumerate() {
        replay
            .read_block_alone(
                index,
                &CENSUS_TABLES,
                |_, frame| {
                    prepared.configure(&source, &context);
                    let changes = tracker.observe(frame, replay.format);
                    prepared.observe_prepared(frame, &changes)
                },
                || Ok(()),
            )
            .unwrap();
        let mut writer = fixture.writer();
        prepared
            .publish_block(&mut writer, replay, 1, ordinal, &flag)
            .unwrap();
        writer.commit().unwrap();
        let battle = &prepared.episodes[&0];
        if ordinal == 0 {
            assert_eq!(battle.losses.len(), 10);
            assert!(battle.history.is_none());
            continue;
        }
        let first = 100 + 10 * ordinal as i32;
        // The read at the block's first hour carries into it; no earlier one is held.
        assert_eq!(
            battle
                .losses
                .iter()
                .map(|read| read.hour)
                .collect::<Vec<_>>(),
            (first..first + 10).collect::<Vec<_>>(),
            "block {ordinal}"
        );
        // Runs of 15 hours: at most two reach a ten-hour block.
        assert!(
            (1..=2).contains(&battle.runs.len()) && battle.runs[0].last_hour >= first,
            "block {ordinal}: {:?}",
            battle.runs
        );
        assert!(battle.history.is_some());
    }
    assert_eq!(prepared.episodes.len(), 1);
    let ids = vec!["1:1:100".to_string()];
    assert_eq!(
        served(&source, 1, &fixture, &ids),
        oracle_served(&source, 1, &ids)
    );
}

#[test]
fn redraws_keep_a_read_appended_before_a_backdated_finalizer() {
    // A long battle is last read at hour 190; the census is not read at 191,
    // which leaves an unknown loss read; two blocks later a finalizer dated 190
    // ends it, appending its terminal read after the unknown one.
    let present = |hour: i32| {
        if hour <= 190 {
            vec![Combat {
                lost: Some(hour - 99),
                ..Combat::new(1, 100)
            }]
        } else {
            Vec::new()
        }
    };
    let mut blocks: Vec<Vec<DecodedFrame>> = (0..12)
        .map(|block| {
            (100 + 10 * block..=109 + 10 * block)
                .map(|hour| hour_frame(1, hour, present(hour)))
                .collect()
        })
        .collect();
    *at_hour(&mut blocks, 191).tables[TableId::LandCombats.index()] = TableView::Unavailable {
        reason: "fixture unread hour".into(),
    };
    finalize(
        at_hour(&mut blocks, 215),
        185,
        vec![finalizer(1, 1, 1, 190, [1, 2])],
    );
    let source = recording(blocks);
    let ids = battle_ids(&source);
    assert_eq!(ids, ["1:1:100"]);
    let expected = oracle_served(&source, 1, &ids);
    for keep in [true, false] {
        let fixture = Fixture::new();
        prepare(&source, 1, &fixture, keep, false);
        // Window 9's redrawn page and window 11's page, where the finalizer was read.
        for (first_hour, last_hour) in [(190, 199), (200, 219)] {
            let ranking = forward_query::query(
                &source,
                "land-battles-ranking",
                serde_json::to_value(LandBattlesRankingParams {
                    branch_id: "1".into(),
                    first_hour,
                    last_hour,
                    top_n: 3.0,
                    measure: "Casualties".into(),
                    countries: LandBattleCountryFilter {
                        tags: None,
                        membership: None,
                        unobserved: vec![],
                    },
                })
                .unwrap(),
                &fixture.store,
                &fixture.key,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(
                ranking["effectiveMeasure"], "Casualties",
                "kept: {keep}, from {first_hour}: the terminal read appended last ranks casualties"
            );
        }
        assert_eq!(served(&source, 1, &fixture, &ids), expected, "kept: {keep}");
        let window = forward_query::query(
            &source,
            "land-battles",
            json!({ "branchId": "1", "window": 9 }),
            &fixture.store,
            &fixture.key,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            window["rows"][0]["losses"]
                .as_array()
                .unwrap()
                .iter()
                .map(|read| (read["hour"].clone(), read["terminal"].clone()))
                .collect::<Vec<_>>(),
            [(json!(190), json!(true)), (json!(191), json!(false))],
            "the terminal read carries into the window, and the unknown read after it stays"
        );
    }
}

#[test]
fn repeated_finalizer_ids_answer_with_the_last_published_course() {
    // One combat's finalizers in two sessions share their hook sequence.
    let quiet = |_: i32| Vec::new();
    let mut blocks: Vec<Vec<DecodedFrame>> = [100..=103, 104..=107]
        .into_iter()
        .map(|hours| hours.map(|hour| hour_frame(1, hour, quiet(hour))).collect())
        .collect();
    finalize(
        &mut blocks[0][2],
        101,
        vec![finalizer(9, 5, 5, 102, [1, 2])],
    );
    finalize(
        &mut blocks[1][2],
        105,
        vec![finalizer(9, 5, 5, 106, [2, 1])],
    );
    let source = recording(blocks);
    let fixture = Fixture::new();
    prepare(&source, 1, &fixture, false, false);
    let flag = AtomicBool::new(false);
    let census = census(&source, 1, &flag, &LoadContext { content: None }).unwrap();
    let repeated: Vec<_> = census
        .episodes
        .values()
        .filter(|episode| episode.id == "1:5:result:9")
        .collect();
    assert_eq!(repeated.len(), 2);
    let course = forward_query::query(
        &source,
        "land-battle-course",
        json!({ "branchId": "1", "id": "1:5:result:9" }),
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .unwrap();
    assert_eq!(course["battle"]["result"]["hour"], 106);
    assert_eq!(
        course["battle"]["result"]["countries"],
        json!([["mod country 2"], ["mod country 1"]])
    );
}
