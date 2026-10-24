use super::*;
use crate::products::naval_battles::prepared as forward_query;
use dashboard_base::prepared::{frame_changes::FrameChangeTracker, series::tests::Fixture};
use serde_json::{Value, json};
use std::ops::RangeInclusive;

#[derive(Clone, Default)]
struct At {
    combats: Vec<u64>,
    results: Vec<(u64, i32, u32)>,
    sources: Vec<NavalResultSourceRow>,
    unread: bool,
}
fn link(id: u64, hour: i32, combat: u64, result: u64) -> NavalResultSourceRow {
    NavalResultSourceRow {
        id,
        game_hour: hour,
        result,
        combat,
        ..Default::default()
    }
}
fn hour_frame(hour: i32, at: At) -> DecodedFrame {
    let mut f = source(1, hour, Some(1));
    if at.unread {
        return f;
    }
    let complete = |rows: TableRows| TableView::Complete(rows);
    *f.tables[TableId::NavalCombats.index()] = complete(TableRows::NavalCombats(
        at.combats
            .iter()
            .map(|&id| NavalCombatRow {
                id,
                province: id as u32,
                ..Default::default()
            })
            .collect(),
    ));
    *f.tables[TableId::NavalCombatParticipants.index()] =
        complete(TableRows::NavalCombatParticipants(Vec::new()));
    *f.tables[TableId::NavalCombatDetails.index()] =
        complete(TableRows::NavalCombatDetails(Vec::new()));
    *f.tables[TableId::NavalResults.index()] = complete(TableRows::NavalResults(
        at.results
            .iter()
            .map(|&(id, result_hour, province)| NavalResultRow {
                id,
                province,
                result_hour,
                winning_side: 1,
                losing_side: 2,
                ..Default::default()
            })
            .collect(),
    ));
    *f.tables[TableId::NavalResultMembers.index()] =
        complete(TableRows::NavalResultMembers(Vec::new()));
    *f.tables[TableId::NavalResultCountries.index()] =
        complete(TableRows::NavalResultCountries(Vec::new()));
    *f.tables[TableId::NavalCombatResultDetails.index()] =
        complete(TableRows::NavalCombatResultDetails(Vec::new()));
    *f.tables[TableId::NavalResultSources.index()] =
        complete(TableRows::NavalResultSources(at.sources));
    f
}
fn block(hours: RangeInclusive<i32>, at: &dyn Fn(i32) -> At) -> Vec<DecodedFrame> {
    hours.map(|hour| hour_frame(hour, at(hour))).collect()
}
fn recording(blocks: Vec<Vec<DecodedFrame>>) -> Source {
    let count = blocks.len();
    let mut replay: Option<Replay> = None;
    for frames in blocks {
        replay = Some(match replay {
            None => Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap(),
            Some(replay) => replay
                .append(&[dashboard_base::test_fixtures::continuation_records(
                    &replay, 1, frames,
                )])
                .unwrap(),
        });
    }
    let replay = replay.unwrap();
    assert_eq!(
        replay.index.len(),
        count,
        "one replay block per fixture block"
    );
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "forward-naval".into(),
            revision: "1".into(),
        },
        "forward-naval.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn forward(source: &Source, fixture: &Fixture) {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let replay = source.replay_ref();
    let content = crate::products::ships_sunk::prepared_content(source, &context);
    let mut scan = Scan::default();
    for (ordinal, index) in replay
        .index
        .iter()
        .filter(|index| index.branch == 1)
        .enumerate()
    {
        let mut tracker = FrameChangeTracker::new();
        replay
            .read_block_alone(
                index,
                PREPARED_TABLES,
                |_, frame| {
                    scan.configure(source, 1, &content)?;
                    let changes = tracker.observe(frame, replay.format);
                    scan.observe_prepared(frame, &changes);
                    scan.observe_details(source, &context, frame, &flag, &changes)
                },
                || Ok(()),
            )
            .unwrap();
        let mut writer = fixture.writer();
        scan.publish_block(source, &context, &mut writer, 1, ordinal, &flag)
            .unwrap();
        writer.commit().unwrap();
    }
}

fn assert_forward(source: &Source) -> (Value, Value) {
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    forward(source, &fixture);
    let context = LoadContext { content: None };
    let manifest_params = NavalBattlesManifestParams {
        branch_id: "1".into(),
    };
    let manifest = forward_query::query(
        source,
        "naval-battles-manifest",
        serde_json::to_value(&manifest_params).unwrap(),
        &context,
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .unwrap();
    assert_eq!(
        manifest,
        serde_json::to_value(
            NavalBattlesManifest::load(source, &manifest_params, &context, &flag).unwrap()
        )
        .unwrap()
    );
    let mut last = Value::Null;
    for window in 0..windows(source.replay_ref(), 1).len() {
        let params = NavalBattlesParams {
            branch_id: "1".into(),
            window,
        };
        last = forward_query::query(
            source,
            "naval-battles",
            serde_json::to_value(&params).unwrap(),
            &context,
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        assert_eq!(
            last,
            serde_json::to_value(NavalBattles::load(source, &params, &context, &flag).unwrap())
                .unwrap(),
            "window {window}"
        );
    }
    (manifest, last)
}

fn row<'a>(window: &'a Value, id: &str) -> &'a Value {
    window["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("{id} is in the window: {window}"))
}

#[test]
fn forward_windows_keep_open_combats_and_retained_results() {
    let at = |hour: i32| {
        let mut at = At::default();
        if (105..=106).contains(&hour) {
            at.combats.push(9);
        }
        if hour == 107 {
            at.sources.push(link(1, hour, 9, 10));
        }
        if hour >= 107 {
            at.results.push((10, 107, 9));
        }
        if (113..=117).contains(&hour) {
            at.combats.push(2);
        }
        if hour == 118 {
            at.sources.push(link(2, hour, 2, 20));
        }
        if hour >= 118 {
            at.results.push((20, 118, 2));
        }
        at
    };
    let blocks = [100..=103, 104..=107, 108..=111, 112..=115, 116..=119]
        .into_iter()
        .map(|hours| block(hours, &at))
        .collect();
    let source = recording(blocks);
    let (_, window) = assert_forward(&source);
    assert_eq!(row(&window, "combat:2:113")["runs"][0]["firstHour"], 116);
    let linked = row(&window, "result:20");
    assert_eq!(
        linked["runs"][0]["row"]["start"],
        json!({ "hour": 113, "bound": "by" }),
        "the source joins the combat's whole episode"
    );
    assert_eq!(
        row(&window, "result:10")["runs"][0]["row"]["start"]["hour"],
        105
    );
}

#[test]
fn forward_sources_refuse_nonmonotonic_persistent_identities() {
    let at = |hour: i32| {
        let mut at = At::default();
        if (101..=102).contains(&hour) {
            at.combats.push(3);
        }
        if hour == 102 {
            at.sources.push(link(5, hour, 3, 30));
        }
        if (102..=103).contains(&hour) {
            at.results.push((30, 102, 30));
        }
        if (109..=110).contains(&hour) {
            at.combats.push(4);
        }
        if hour == 110 {
            at.sources.push(link(4, hour, 4, 31));
        }
        if hour >= 110 {
            at.results.push((31, 110, 31));
        }
        at
    };
    let blocks = [96..=99, 100..=103, 104..=107, 108..=111]
        .into_iter()
        .map(|hours| block(hours, &at))
        .collect();
    let (_, window) = assert_forward(&recording(blocks));
    assert!(
        window["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| {
                warning["hour"] == 110
                    && warning["reason"]
                        == "Invalid naval result source identity, hour or persistent merge"
            }),
        "{window}"
    );
}

#[test]
fn forward_results_keep_their_recorded_date() {
    let at = |hour: i32| At {
        results: if hour >= 106 {
            vec![(40, 101, 40)]
        } else {
            Vec::new()
        },
        ..At::default()
    };
    let mut blocks: Vec<_> = [100..=103, 104..=107]
        .into_iter()
        .map(|hours| block(hours, &at))
        .collect();
    // The results census is not recorded before hour 106.
    for frames in &mut blocks {
        for sample in frames.iter_mut().filter(|sample| sample.hour < 106) {
            *sample.tables[TableId::NavalResults.index()] = TableView::Unavailable {
                reason: "fixture unread results".into(),
            };
        }
    }
    let (_, window) = assert_forward(&recording(blocks));
    let place = &row(&window, "result:40")["runs"][0]["row"]["place"];
    assert_eq!(place["hour"], 101);
    assert!(place["namedAt"].is_null(), "hour 101 was recorded: {place}");
}

#[test]
fn forward_combats_reopen_after_a_recorded_gap() {
    let at = |hour: i32| At {
        combats: if (102..=110).contains(&hour) {
            vec![5]
        } else {
            Vec::new()
        },
        ..At::default()
    };
    // Hours 104 to 107 are not recorded.
    let blocks = [100..=103, 108..=111]
        .into_iter()
        .map(|hours| block(hours, &at))
        .collect();
    let (_, window) = assert_forward(&recording(blocks));
    row(&window, "combat:5:108");
}

#[test]
fn forward_manifest_keeps_branch_availability_across_unread_hours() {
    for (never, available) in [(false, true), (true, false)] {
        let at = |hour| At {
            unread: never || hour >= 104,
            ..At::default()
        };
        let source = recording(
            [100..=103, 104..=107]
                .into_iter()
                .map(|hours| block(hours, &at))
                .collect(),
        );
        let (manifest, window) = assert_forward(&source);
        assert_eq!(manifest["available"], available);
        assert_eq!(window["observed"], json!([]));
        if !available {
            assert_eq!(manifest["unavailable"], "Not in this recording");
        }
    }
}

#[test]
fn forward_windows_keep_episode_starts_and_reappearances() {
    let at = |hour: i32| {
        let mut at = At::default();
        // Combat 1 is open throughout; combat 2 twice in the second block.
        at.combats.push(1);
        if (104..=105).contains(&hour) || hour == 107 {
            at.combats.push(2);
        }
        if hour >= 102 {
            at.results.push((10, 102, 10));
        }
        if hour >= 117 {
            at.results.push((11, 117, 11));
        }
        if hour == 102 {
            at.sources.push(link(1, hour, 3, 10));
        }
        if hour == 117 {
            at.sources.push(link(2, hour, 4, 11));
        }
        at
    };
    // Hours 112 to 115 are not recorded.
    let blocks = [100..=103, 104..=107, 108..=111, 116..=119, 120..=123]
        .into_iter()
        .map(|hours| block(hours, &at))
        .collect();
    let source = recording(blocks);
    let fixture = Fixture::new();
    forward(&source, &fixture);
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    for (window, id, start) in [
        (2, "combat:1:100", 100),
        (3, "combat:1:116", 116),
        (1, "combat:2:107", 107),
    ] {
        let reply = forward_query::query(
            &source,
            "naval-battles",
            json!({"branchId":"1", "window":window}),
            &context,
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        assert_eq!(row(&reply, id)["runs"][0]["row"]["start"]["hour"], start);
    }
    assert_forward(&source);
}

#[test]
fn forward_combats_reopen_after_a_gap_inside_a_block() {
    let at = |hour: i32| At {
        combats: vec![1],
        results: if hour == 104 {
            vec![(9, 104, 9)]
        } else {
            Vec::new()
        },
        ..At::default()
    };
    // Hours 96 to 99, 102 and 103 are not recorded.
    let blocks = vec![
        block(92..=95, &at),
        [100, 101, 104, 105]
            .into_iter()
            .map(|hour| hour_frame(hour, at(hour)))
            .collect(),
        block(106..=109, &at),
    ];
    let source = recording(blocks);
    let (_, window) = assert_forward(&source);
    row(&window, "combat:1:104");
}
