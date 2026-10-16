//! The newest window prepared first, from the last block and a proven
//! look-back, publishes the page and manifest forward preparation publishes.
use super::*;
use crate::products::naval_battles::{newest as first, prepared as forward_query};
use dashboard_base::prepared::{frame_changes::FrameChangeTracker, series::tests::Fixture};
use serde_json::{Value, json};
use std::{io::Read, ops::RangeInclusive};

/// What one hour of the fixture holds.
#[derive(Clone, Default)]
struct At {
    /// The open combats, each in the province of its id.
    combats: Vec<u64>,
    /// The retained results, by id and result hour.
    results: Vec<(u64, i32)>,
    sources: Vec<NavalResultSourceRow>,
    /// The battle census is not recorded.
    unread: bool,
}

/// A persistent source `id` read at `hour`, linking `combat` to `result`.
fn link(id: u64, hour: i32, combat: u64, result: u64) -> NavalResultSourceRow {
    NavalResultSourceRow {
        id,
        game_hour: hour,
        result,
        combat,
        ..Default::default()
    }
}

/// An hour of branch 1 holding `at`, every other naval census table
/// complete and empty.
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
            .map(|&(id, result_hour)| NavalResultRow {
                id,
                province: id as u32,
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

/// One block: an hour frame for each hour of `hours`.
fn block(hours: RangeInclusive<i32>, at: &dyn Fn(i32) -> At) -> Vec<DecodedFrame> {
    hours.map(|hour| hour_frame(hour, at(hour))).collect()
}

/// A recording of `blocks` on branch 1, each one replay block.
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
            source_id: "newest-naval".into(),
            revision: "1".into(),
        },
        "newest-naval.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// Forward preparation of branch 1's naval family as it runs on its own:
/// every block in order, read alone and observed hour by hour with change
/// facts of its own, the details included, published and committed after
/// each block.
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

/// A range's stored bytes.
fn bytes(fixture: &Fixture, section: &str, ordinal: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    fixture
        .store
        .read(&fixture.key, section, ordinal)
        .unwrap()
        .unwrap_or_else(|| panic!("{section} {ordinal} is published"))
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

/// Prepares and publishes branch 1's newest window alone, checks its page
/// bytes, manifest and window reply against full forward preparation's, and
/// returns the newest window reply, the blocks its look-backs observed and
/// the earlier blocks read by their changes.
fn assert_newest_equals_forward(source: &Source) -> (Value, usize, usize) {
    let flag = AtomicBool::new(false);
    let forwarded = Fixture::new();
    forward(source, &forwarded);
    let newest = Fixture::new();
    let prepared = first::prepare(source, 1, &flag)
        .unwrap()
        .expect("a proven newest window");
    let mut writer = newest.writer();
    prepared.write(&mut writer).unwrap();
    writer.commit().unwrap();
    let last = windows(source.replay_ref(), 1).len() - 1;
    assert_eq!(
        bytes(&newest, "naval-battles-newest-1", 0),
        bytes(&forwarded, "naval-battles-1", last as u64),
        "the newest page is forward preparation's, byte for byte"
    );
    let context = LoadContext { content: None };
    let mut window = Value::Null;
    for (product, params) in [
        ("naval-battles-manifest", json!({ "branchId": "1" })),
        ("naval-battles", json!({ "branchId": "1", "window": last })),
    ] {
        let expected = forward_query::query(
            source,
            product,
            params.clone(),
            &context,
            &forwarded.store,
            &forwarded.key,
            &flag,
        )
        .unwrap();
        let actual = first::query(source, product, 1, &params, &newest.store, &newest.key).unwrap();
        assert_eq!(actual.as_ref(), Some(&expected), "{product}");
        window = expected;
    }
    // Only the newest window and the manifest answer from it.
    for (product, params) in [
        (
            "naval-battles",
            json!({ "branchId": "1", "window": last.wrapping_sub(1) }),
        ),
        (
            "naval-battles-ranking",
            json!({ "branchId": "1", "firstHour": 0, "lastHour": 0, "topN": 1,
                "measure": "Length", "countries": { "unobserved": [] } }),
        ),
    ] {
        assert_eq!(
            first::query(source, product, 1, &params, &newest.store, &newest.key).unwrap(),
            None,
            "{product} waits for forward preparation"
        );
    }
    (window, prepared.blocks_read, prepared.earlier_blocks)
}

/// The row of the battle `id` in a window reply.
fn row<'a>(window: &'a Value, id: &str) -> &'a Value {
    window["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("{id} is in the window: {window}"))
}

/// A combat open across the last block's first hour and a result retained
/// from an earlier block move the look-back to the earliest of their first
/// hours, where neither was open or read before.
#[test]
fn newest_window_reaches_open_combats_and_retained_results() {
    let at = |hour: i32| {
        let mut at = At::default();
        // Combat 9 ends at 106; its source at 107 creates result 10, retained
        // to the end.
        if (105..=106).contains(&hour) {
            at.combats.push(9);
        }
        if hour == 107 {
            at.sources.push(link(1, hour, 9, 10));
        }
        if hour >= 107 {
            at.results.push((10, 107));
        }
        // Combat 2 is open across the last block's first hour; its source
        // at 118 creates result 20.
        if (113..=117).contains(&hour) {
            at.combats.push(2);
        }
        if hour == 118 {
            at.sources.push(link(2, hour, 2, 20));
        }
        if hour >= 118 {
            at.results.push((20, 118));
        }
        at
    };
    let blocks = [100..=103, 104..=107, 108..=111, 112..=115, 116..=119]
        .into_iter()
        .map(|hours| block(hours, &at))
        .collect();
    let (window, read, earlier) = assert_newest_equals_forward(&recording(blocks));
    // The last block, then from result 10's first read.
    assert_eq!(read, 1 + 4);
    assert_eq!(
        earlier, 4,
        "the blocks before the last read once by changes"
    );
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

/// A source in the last block whose id is below an earlier source's is
/// refused by forward preparation; the look-back reaches that source, so the
/// window's warnings carry the refusal.
#[test]
fn newest_window_proves_its_sources_against_every_earlier_source() {
    let at = |hour: i32| {
        let mut at = At::default();
        if (101..=102).contains(&hour) {
            at.combats.push(3);
        }
        if hour == 102 {
            at.sources.push(link(5, hour, 3, 30));
        }
        if (102..=103).contains(&hour) {
            at.results.push((30, 102));
        }
        if (109..=110).contains(&hour) {
            at.combats.push(4);
        }
        if hour == 110 {
            at.sources.push(link(4, hour, 4, 31));
        }
        if hour >= 110 {
            at.results.push((31, 110));
        }
        at
    };
    let blocks = [96..=99, 100..=103, 104..=107, 108..=111]
        .into_iter()
        .map(|hours| block(hours, &at))
        .collect();
    let (window, read, _) = assert_newest_equals_forward(&recording(blocks));
    assert_eq!(
        read,
        1 + 3,
        "the last block, then from the earlier source's"
    );
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

/// A result first read in the last block whose date an earlier block
/// recorded is named by that date, as forward preparation names it, though
/// the look-back reads the last block alone.
#[test]
fn newest_window_places_results_by_every_recorded_hour() {
    let at = |hour: i32| At {
        results: if hour >= 106 {
            vec![(40, 101)]
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
    let (window, read, _) = assert_newest_equals_forward(&recording(blocks));
    assert_eq!(read, 1);
    let place = &row(&window, "result:40")["runs"][0]["row"]["place"];
    assert_eq!(place["hour"], 101);
    assert!(place["namedAt"].is_null(), "hour 101 was recorded: {place}");
}

/// The manifest's availability is the branch's: an available hour before
/// the look-back makes it available, and a branch never available publishes
/// its manifest unavailable.
#[test]
fn newest_window_manifest_reads_every_hours_availability() {
    let earlier = |hour: i32| At {
        unread: hour >= 104,
        ..At::default()
    };
    let blocks = [100..=103, 104..=107]
        .into_iter()
        .map(|hours| block(hours, &earlier))
        .collect();
    let source = recording(blocks);
    let (_, read, _) = assert_newest_equals_forward(&source);
    assert_eq!(read, 1);
    let flag = AtomicBool::new(false);
    let prepared = first::prepare(&source, 1, &flag).unwrap().unwrap();
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    prepared.write(&mut writer).unwrap();
    writer.commit().unwrap();
    let manifest = first::query(
        &source,
        "naval-battles-manifest",
        1,
        &json!({ "branchId": "1" }),
        &fixture.store,
        &fixture.key,
    )
    .unwrap()
    .unwrap();
    assert_eq!(manifest["available"], true);

    let never = |_: i32| At {
        unread: true,
        ..At::default()
    };
    let blocks = [100..=103, 104..=107]
        .into_iter()
        .map(|hours| block(hours, &never))
        .collect();
    let source = recording(blocks);
    assert_newest_equals_forward(&source);
    let prepared = first::prepare(&source, 1, &flag).unwrap().unwrap();
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    prepared.write(&mut writer).unwrap();
    writer.commit().unwrap();
    let manifest = first::query(
        &source,
        "naval-battles-manifest",
        1,
        &json!({ "branchId": "1" }),
        &fixture.store,
        &fixture.key,
    )
    .unwrap()
    .unwrap();
    assert_eq!(manifest["available"], false);
    assert_eq!(manifest["unavailable"], "Not in this recording");
}

/// After a recorded gap forward preparation closes every combat, so a combat
/// open before and after the gap moves no look-back.
#[test]
fn newest_window_after_a_gap_reads_the_last_block_alone() {
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
    let (window, read, _) = assert_newest_equals_forward(&recording(blocks));
    assert_eq!(read, 1);
    row(&window, "combat:5:108");
}

/// The newest window observes its last block and the look-back reaching the
/// open combat's first hour, however many blocks precede it; the earlier
/// blocks are read only by their changes, once.
#[test]
fn newest_window_observes_only_the_last_block_and_its_proven_look_back() {
    let counted = |quiet: i32| {
        let opened = 100 + 4 * quiet + 1;
        let at = move |hour: i32| At {
            combats: if hour >= opened { vec![1] } else { Vec::new() },
            ..At::default()
        };
        let blocks = (0..quiet + 2)
            .map(|block_at| block(100 + 4 * block_at..=103 + 4 * block_at, &at))
            .collect();
        let (_, read, earlier) = assert_newest_equals_forward(&recording(blocks));
        (read, earlier)
    };
    // The last block, then the two from the combat's first block.
    assert_eq!(counted(2), (1 + 2, 3));
    assert_eq!(counted(12), (1 + 2, 13));
}

/// A look-back starting at the branch's first block would be forward
/// preparation's walk of the branch: a single block's, or one whose proof
/// moves it there. No look-back is tried and the window is left to forward
/// preparation.
#[test]
fn newest_window_look_back_stops_at_the_branch_start() {
    let at = |hour: i32| At {
        combats: vec![7],
        results: if hour >= 101 {
            vec![(70, 101)]
        } else {
            Vec::new()
        },
        sources: if hour == 101 {
            vec![link(1, hour, 7, 70)]
        } else {
            Vec::new()
        },
        ..At::default()
    };
    let flag = AtomicBool::new(false);
    let single = recording(vec![block(100..=103, &at)]);
    assert!(first::prepare(&single, 1, &flag).unwrap().is_none());
    // Combat 7 is open from the branch's first hour.
    let open = recording(vec![block(100..=103, &at), block(104..=107, &at)]);
    assert!(first::prepare(&open, 1, &flag).unwrap().is_none());
}

/// The earlier blocks' facts read on parallel workers, each block alone and
/// joined in block order, are those of reading them in order: a combat open
/// across block boundaries keeps its first hour, and after a gap opens again.
#[test]
fn newest_window_earlier_facts_read_in_parallel_equal_reading_them_in_order() {
    let at = |hour: i32| {
        let mut at = At::default();
        // Combat 1 is open throughout; combat 2 twice in the second block.
        at.combats.push(1);
        if (104..=105).contains(&hour) || hour == 107 {
            at.combats.push(2);
        }
        if hour >= 102 {
            at.results.push((10, 102));
        }
        if hour >= 117 {
            at.results.push((11, 117));
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
    let replay = source.replay_ref();
    let positions: Vec<usize> = (0..replay.index.len()).collect();
    let flag = AtomicBool::new(false);
    let in_order = first::read_earlier_in_order(replay, &positions, &flag).unwrap();
    assert_eq!(
        in_order.open[2][&1], 100,
        "open since the branch's first hour"
    );
    assert_eq!(in_order.open[3][&1], 116, "opened again after the gap");
    assert_eq!(
        in_order.open[1][&2], 107,
        "combat 2 opened again in its block"
    );
    for workers in [1, 3] {
        assert_eq!(
            first::read_earlier(replay, &positions, workers, &flag).unwrap(),
            in_order,
            "{workers} workers"
        );
    }
}

/// A gap inside an earlier block closes every combat; at the hour after it,
/// where only a result's keys changed, forward preparation opens again the
/// combat its census holds. The look-back reaches that hour, so the page
/// holds the combat's episode from there.
#[test]
fn newest_window_reopens_combats_after_a_gap_inside_an_earlier_block() {
    let at = |hour: i32| At {
        combats: vec![1],
        results: if hour == 104 {
            vec![(9, 104)]
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
    let (window, read, _) = assert_newest_equals_forward(&source);
    row(&window, "combat:1:104");
    assert_eq!(read, 1 + 2, "the last block, then from the gap's block");
    let replay = source.replay_ref();
    let flag = AtomicBool::new(false);
    let facts = first::read_earlier(replay, &[1], 2, &flag).unwrap();
    assert_eq!(
        facts.open[0][&1], 104,
        "open again from the hour after the gap"
    );
}
