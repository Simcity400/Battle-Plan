//! The newest window prepared first, from the last block and a proven
//! look-back, publishes the page and manifest forward preparation publishes.
use super::*;
use crate::products::land_battles::{newest as first, prepared as forward_query};
use dashboard_base::prepared::{frame_changes::FrameChangeTracker, series::tests::Fixture};
use replay::rows::{
    LandCombatResultsRow, LandEventCoverageRow, LandResultSide, LandSideLosses, LexicalTokenRow,
    VariantDefinitionRow,
};
use serde_json::{Value, json};
use std::{io::Read, ops::RangeInclusive};

/// One land combat present at an hour.
#[derive(Clone, Copy)]
struct Combat {
    id: u64,
    province: u32,
    /// The hour its native duration counter read zero.
    zero: i32,
    attackers: u64,
    /// Attacker manpower lost so far, when the hour's progress names the
    /// combat.
    lost: Option<i32>,
}

impl Combat {
    fn new(id: u64, zero: i32) -> Self {
        Self {
            id,
            province: id as u32,
            zero,
            attackers: 2,
            lost: None,
        }
    }
}

/// An hour of `branch` whose land combats are `present`. Combat 3's defender
/// names country reference 99, which no link resolves.
fn hour_frame(branch: u64, hour: i32, mut present: Vec<Combat>) -> DecodedFrame {
    present.sort_unstable_by_key(|combat| combat.id);
    let mut sample = frame(hour, 8, 0);
    sample.branch = branch;
    *sample.tables[TableId::LandCombats.index()] = TableView::Complete(TableRows::LandCombats(
        present
            .iter()
            .map(|combat| LandCombatRow {
                id: combat.id,
                province: combat.province,
                day: 0,
                duration: hour - combat.zero,
                attackers: (0..combat.attackers)
                    .map(|at| combat.id << 32 | at)
                    .collect(),
                defenders: vec![combat.id << 32 | 99],
                ..Default::default()
            })
            .collect(),
    ));
    *sample.tables[TableId::LandCombatSides.index()] =
        TableView::Complete(TableRows::LandCombatSides(
            present
                .iter()
                .map(|combat| LandCombatSideRow {
                    combat: combat.id,
                    attacker_primary: vec![1],
                    defender_primary: vec![if combat.id == 3 { 99 } else { 2 }],
                    ..Default::default()
                })
                .collect(),
        ));
    set_progress(
        &mut sample,
        present
            .iter()
            .filter_map(|combat| {
                combat.lost.map(|lost| LandCombatProgressRow {
                    combat: combat.id,
                    attacker_manpower_loss: lost.into(),
                    defender_manpower_loss: (lost / 2).into(),
                    attacker_equipment_losses: vec![LandEquipmentLoss {
                        variant: 7,
                        amount_raw: -10_000 * i64::from(lost),
                    }]
                    .into(),
                    ..Default::default()
                })
            })
            .collect(),
    );
    sample
}

/// One block of `branch`: an hour frame for each hour of `hours`.
fn block(
    branch: u64,
    hours: RangeInclusive<i32>,
    present: &dyn Fn(i32) -> Vec<Combat>,
) -> Vec<DecodedFrame> {
    hours
        .map(|hour| hour_frame(branch, hour, present(hour)))
        .collect()
}

/// A finalizer result for `combat` read at `hour`, ending it at `end` and
/// covering the hours after `from`.
fn add_result(sample: &mut DecodedFrame, combat: u64, end: i32, from: i32) {
    *sample.tables[TableId::LandCombatResults.index()] =
        TableView::Complete(TableRows::LandCombatResults(vec![LandCombatResultsRow {
            id: 1,
            combat,
            province: combat as u32,
            end_hour: end,
            duration: 6,
            winners: 1,
            sides: std::array::from_fn(|index| LandResultSide {
                countries: vec![index as u32 + 1],
                ..Default::default()
            }),
            losses: std::array::from_fn(|index| LandSideLosses {
                manpower: 5 + index as i32,
                equipment: Vec::new(),
            }),
            ..Default::default()
        }]));
    *sample.tables[TableId::LandEventCoverage.index()] =
        TableView::Complete(TableRows::LandEventCoverage(vec![LandEventCoverageRow {
            from_hour: from,
            to_hour: sample.hour,
            complete: true,
            ..Default::default()
        }]));
}

/// A recording of `blocks`, each one replay block of its frames' branch.
fn recording(blocks: Vec<Vec<DecodedFrame>>) -> Source {
    let count = blocks.len();
    let mut replay: Option<Replay> = None;
    for frames in blocks {
        let branch = frames[0].branch;
        replay = Some(match replay {
            None => Replay::from_records(
                dashboard_base::test_fixtures::CAMPAIGN.into(),
                &[dashboard_base::test_fixtures::records(branch, frames)],
            )
            .unwrap(),
            Some(replay) if replay.branches.iter().any(|start| start.id == branch) => replay
                .append(&[dashboard_base::test_fixtures::continuation_records(
                    &replay, branch, frames,
                )])
                .unwrap(),
            Some(replay) => replay
                .append(&[dashboard_base::test_fixtures::records(branch, frames)])
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
        dashboard_base::SourceKey {
            source_id: "newest-land".into(),
            revision: "1".into(),
        },
        "newest-land.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// Forward preparation of a branch's land battles as the hourly walk runs
/// it: every block in order, each hour observed with the change facts of a
/// continuing tracker, published and committed after each block.
fn forward(source: &Source, branch: u64, fixture: &Fixture) {
    let flag = AtomicBool::new(false);
    let replay = source.replay_ref();
    let context = LoadContext { content: None };
    let mut census = Census::new(replay.format);
    let mut tracker = FrameChangeTracker::new();
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

/// Prepares and publishes the branch's newest window alone, checks its page
/// bytes, manifest and window reply against full forward preparation's, and
/// returns its newest window reply and the blocks it read.
fn assert_newest_equals_forward(source: &Source, branch: u64) -> (Value, usize) {
    let flag = AtomicBool::new(false);
    let forwarded = Fixture::new();
    forward(source, branch, &forwarded);
    let newest = Fixture::new();
    let prepared = first::prepare(source, branch, &LoadContext { content: None }, &flag)
        .unwrap()
        .expect("a proven newest window");
    let mut writer = newest.writer();
    prepared.write(&mut writer).unwrap();
    writer.commit().unwrap();
    let last = super::super::windows(source.replay_ref(), branch).len() - 1;
    assert_eq!(
        bytes(&newest, &format!("land-battles-newest-{branch}"), 0),
        bytes(&forwarded, &format!("land-battles-{branch}"), last as u64),
        "branch {branch}: the newest page is forward preparation's, byte for byte"
    );
    let id = branch.to_string();
    let mut window = Value::Null;
    for (product, params) in [
        ("land-battles-manifest", json!({ "branchId": id })),
        ("land-battles", json!({ "branchId": id, "window": last })),
    ] {
        let expected = forward_query::query(
            source,
            product,
            params.clone(),
            &forwarded.store,
            &forwarded.key,
            &flag,
        )
        .unwrap();
        let actual =
            first::query(source, product, branch, &params, &newest.store, &newest.key).unwrap();
        assert_eq!(
            actual.as_ref(),
            Some(&expected),
            "branch {branch}: {product}"
        );
        window = expected;
    }
    // Only the newest window answers from it.
    if last > 0 {
        assert_eq!(
            first::query(
                source,
                "land-battles",
                branch,
                &json!({ "branchId": id, "window": last - 1 }),
                &newest.store,
                &newest.key,
            )
            .unwrap(),
            None
        );
    }
    (window, prepared.blocks_read)
}

/// The row of the battle `id` in a window reply.
fn row<'a>(window: &'a Value, id: &str) -> &'a Value {
    window["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("{id} is in the window"))
}

/// Branch 1: a battle from just after a recorded gap to the end, one starting
/// in the last block, one whose participants are unresolved, one ending at
/// the last block's first hour with its result read after it, and an early
/// battle around an unread hour. Branch 2: two blocks, a battle open across
/// their boundary.
#[test]
fn newest_window_page_and_manifest_equal_forward_preparation() {
    let first_branch = |hour: i32| {
        let mut present = Vec::new();
        if (112..=131).contains(&hour) {
            present.push(Combat {
                attackers: match hour {
                    ..=117 => 3,
                    130.. => 4,
                    _ => 2,
                },
                lost: Some(match hour {
                    ..=119 => 10,
                    120..=129 => 20,
                    _ => 30,
                }),
                ..Combat::new(1, 112)
            });
        }
        if (129..=131).contains(&hour) {
            present.push(Combat::new(2, 129));
        }
        if (125..=130).contains(&hour) {
            present.push(Combat::new(3, 125));
        }
        if (122..=127).contains(&hour) {
            present.push(Combat::new(4, 122));
        }
        if (100..=106).contains(&hour) {
            present.push(Combat::new(9, 100));
        }
        present
    };
    let mut blocks: Vec<_> = [
        100..=103,
        104..=107,
        // Hours 108 to 111 are not recorded.
        112..=115,
        116..=119,
        120..=123,
        124..=127,
        128..=131,
    ]
    .into_iter()
    .map(|hours| block(1, hours, &first_branch))
    .collect();
    *blocks[0][2].tables[TableId::LandCombats.index()] = TableView::Unavailable {
        reason: "fixture unread hour".into(),
    };
    add_result(&mut blocks[6][1], 4, 128, 127);
    let second_branch = |hour: i32| {
        let mut present = Vec::new();
        if (201..=206).contains(&hour) {
            present.push(Combat {
                lost: Some(5),
                ..Combat::new(7, 201)
            });
        }
        if (206..=207).contains(&hour) {
            present.push(Combat::new(8, 206));
        }
        present
    };
    blocks.push(block(2, 200..=203, &second_branch));
    blocks.push(block(2, 204..=207, &second_branch));
    let source = recording(blocks);

    let (window, read) = assert_newest_equals_forward(&source, 1);
    // The last block and the one before it, then the blocks from the gap on:
    // the long battle's counter moved the look-back there.
    assert_eq!(read, 2 + 5);
    let long = row(&window, "1:1:112");
    assert_eq!(long["start"], json!({ "hour": 112, "bound": "by" }));
    assert_eq!(long["counterBase"], 0);
    assert_eq!(long["priorPeak"], json!({ "value": 4, "hour": 112 }));
    assert_eq!(
        long["losses"][0]["hour"], 120,
        "the read carried into the window"
    );
    assert_eq!(row(&window, "1:2:129")["start"]["bound"], "exact");
    assert_eq!(row(&window, "1:3:125")["start"]["bound"], "exact");
    let ended = row(&window, "1:4:122");
    assert_eq!(ended["result"]["hour"], 128);
    assert!(
        window["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "Participant country reference 99 unresolved")
    );

    let (_, read) = assert_newest_equals_forward(&source, 2);
    assert_eq!(
        read, 2,
        "from the branch's first block, the look-back is its walk"
    );
}

/// The newest window reads its last block and a look-back reaching the open
/// battle's first hour, however many blocks precede it.
#[test]
fn newest_window_reads_only_the_last_block_and_its_proven_look_back() {
    let counted = |quiet: i32| {
        let started = 100 + 4 * quiet + 1;
        let present = move |hour: i32| {
            if hour >= started {
                vec![Combat::new(1, started)]
            } else {
                Vec::new()
            }
        };
        let blocks = (0..quiet + 4)
            .map(|at| block(1, 100 + 4 * at..=103 + 4 * at, &present))
            .collect();
        assert_newest_equals_forward(&recording(blocks), 1).1
    };
    // The last two blocks, then the four from the battle's first block.
    assert_eq!(counted(2), 2 + 4);
    assert_eq!(counted(12), 2 + 4);
}

/// The last block follows a recorded gap, so a look-back of it alone sees
/// every battle forward preparation still holds open; but a result read in it
/// closes a battle that ended before the gap. Its coverage moves the
/// look-back before the gap, and then to that battle's first hour.
#[test]
fn newest_window_reaches_the_battles_its_results_close() {
    let present = |hour: i32| {
        if (101..=105).contains(&hour) {
            vec![Combat::new(6, 101)]
        } else {
            Vec::new()
        }
    };
    let mut blocks: Vec<_> = [
        100..=103,
        104..=107,
        // Hours 108 to 111 are not recorded.
        112..=115,
    ]
    .into_iter()
    .map(|hours| block(1, hours, &present))
    .collect();
    add_result(&mut blocks[2][1], 6, 106, 105);
    let source = recording(blocks);
    let (window, read) = assert_newest_equals_forward(&source, 1);
    // The last block, then from the block its result covers, then from the
    // battle's first block.
    assert_eq!(read, 1 + 2 + 3);
    assert_eq!(row(&window, "1:6:101")["result"]["hour"], 106);
}

/// A loss read in the newest window naming a variant whose naming the
/// look-back neither read nor forgot moves the look-back back until it is
/// proven.
#[test]
fn newest_window_proves_the_equipment_names_its_loss_reads_use() {
    let present = |hour: i32| {
        if hour >= 113 {
            vec![Combat {
                lost: Some(10),
                ..Combat::new(1, 113)
            }]
        } else {
            Vec::new()
        }
    };
    let blocks: Vec<_> = [100..=103, 104..=107, 108..=111, 112..=115]
        .into_iter()
        .map(|hours| {
            let mut frames = block(1, hours, &present);
            for sample in &mut frames {
                *sample.tables[TableId::VariantDefinitions.index()] =
                    TableView::Complete(TableRows::VariantDefinitions(Vec::new()));
            }
            frames
        })
        .collect();
    let (_, read) = assert_newest_equals_forward(&recording(blocks), 1);
    assert_eq!(read, 2 + 3 + 4);
}

/// Without an available hour in the look-back, availability is not proven and
/// nothing is published.
#[test]
fn newest_window_publishes_nothing_without_proven_availability() {
    let blocks: Vec<Vec<DecodedFrame>> = [100..=101, 102..=103]
        .into_iter()
        .map(|hours| {
            hours
                .map(|hour| dashboard_base::test_fixtures::source(1, hour, Some(0)))
                .collect::<Vec<_>>()
        })
        .collect();
    let source = recording(blocks);
    assert!(
        first::prepare(
            &source,
            1,
            &LoadContext { content: None },
            &AtomicBool::new(false)
        )
        .unwrap()
        .is_none()
    );
}

/// An early block's variant read refuses a naming operand. The last two
/// blocks hold complete, empty combat and variant tables, so the first
/// look-back, over them, proves the window without reaching that read; the
/// window's reply still carries forward preparation's diagnostics, which
/// every variant read on the branch decides.
#[test]
fn newest_window_diagnostics_cover_every_variant_read_on_the_branch() {
    let quiet = |_: i32| -> Vec<Combat> { Vec::new() };
    // The first block is a whole day, so the variant table's daily read
    // lands in it; none lands in the last two.
    let mut blocks: Vec<_> = [100..=123, 124..=127, 128..=131]
        .into_iter()
        .map(|hours| block(1, hours, &quiet))
        .collect();
    for (at, frames) in blocks.iter_mut().enumerate() {
        for sample in frames {
            let definitions = if at == 0 {
                vec![VariantDefinitionRow {
                    id: 7,
                    equipment_token: 5,
                    creator_link: 1,
                    display_name: vec![0xff],
                    ..Default::default()
                }]
            } else {
                Vec::new()
            };
            *sample.tables[TableId::VariantDefinitions.index()] =
                TableView::Complete(TableRows::VariantDefinitions(definitions));
            *sample.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 5,
                    literal: b"mod_rifle_1".to_vec(),
                }]));
        }
    }
    let source = recording(blocks);
    let (window, read) = assert_newest_equals_forward(&source, 1);
    assert_eq!(read, 2, "the look-back over the last two blocks is proven");
    assert!(
        window["contentWarnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .any(|warning| warning.contains("equipment display name is not UTF-8")),
        "forward preparation reports the early read's refusal: {window}"
    );
    let prepared = first::prepare(
        &source,
        1,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap()
    .expect("a proven newest window");
    assert_eq!(
        prepared.naming_blocks, 1,
        "the diagnostics read the block before the look-back's by its changes"
    );
}
