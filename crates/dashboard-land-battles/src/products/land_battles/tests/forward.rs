use super::*;
use crate::products::land_battles::prepared as forward_query;
use dashboard_base::prepared::{frame_changes::FrameChangeTracker, series::tests::Fixture};
use replay_data::rows::{
    LandCombatResultsRow, LandEventCoverageRow, LandResultSide, LandSideLosses, LexicalTokenRow,
    VariantDefinitionRow,
};
use serde_json::{Value, json};
use std::ops::RangeInclusive;

#[derive(Clone, Copy)]
pub(super) struct Combat {
    pub(super) id: u64,
    pub(super) province: u32,
    pub(super) zero: i32,
    pub(super) attackers: u64,
    pub(super) lost: Option<i32>,
}

impl Combat {
    pub(super) fn new(id: u64, zero: i32) -> Self {
        Self {
            id,
            province: id as u32,
            zero,
            attackers: 2,
            lost: None,
        }
    }
}
pub(super) fn hour_frame(branch: u64, hour: i32, mut present: Vec<Combat>) -> DecodedFrame {
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
fn block(
    branch: u64,
    hours: RangeInclusive<i32>,
    present: &dyn Fn(i32) -> Vec<Combat>,
) -> Vec<DecodedFrame> {
    hours
        .map(|hour| hour_frame(branch, hour, present(hour)))
        .collect()
}
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
pub(super) fn recording(blocks: Vec<Vec<DecodedFrame>>) -> Source {
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
            source_id: "forward-land".into(),
            revision: "1".into(),
        },
        "forward-land.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn forward(source: &Source, branch: u64, fixture: &Fixture) -> Census {
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
    census
}

fn assert_forward(source: &Source, branch: u64) -> (Value, Value) {
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    forward(source, branch, &fixture);
    let context = LoadContext { content: None };
    let branch_id = branch.to_string();
    let manifest_params = LandBattlesManifestParams {
        branch_id: branch_id.clone(),
    };
    let manifest = forward_query::query(
        source,
        "land-battles-manifest",
        serde_json::to_value(&manifest_params).unwrap(),
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .unwrap();
    assert_eq!(
        manifest,
        serde_json::to_value(
            LandBattlesManifest::load(source, &manifest_params, &context, &flag).unwrap()
        )
        .unwrap()
    );
    let mut last = Value::Null;
    for window in 0..super::super::windows(source.replay_ref(), branch).len() {
        let params = LandBattlesParams {
            branch_id: branch_id.clone(),
            window,
        };
        last = forward_query::query(
            source,
            "land-battles",
            serde_json::to_value(&params).unwrap(),
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        assert_eq!(
            last,
            serde_json::to_value(LandBattles::load(source, &params, &context, &flag).unwrap())
                .unwrap(),
            "branch {branch}, window {window}"
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
        .unwrap_or_else(|| panic!("{id} is in the window"))
}

fn page_and_manifest_recording() -> Source {
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
    recording(blocks)
}

#[test]
fn forward_windows_keep_battles_results_losses_and_participant_refusals() {
    let source = page_and_manifest_recording();
    let (_, window) = assert_forward(&source, 1);
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

    let (_, second) = assert_forward(&source, 2);
    assert_eq!(row(&second, "2:7:201")["start"]["hour"], 201);
}

fn results_close_recording(earlier: &[RangeInclusive<i32>]) -> Source {
    let present = |hour: i32| {
        if (101..=105).contains(&hour) {
            vec![Combat::new(6, 101)]
        } else {
            Vec::new()
        }
    };
    let mut blocks: Vec<_> = earlier
        .iter()
        .cloned()
        .chain([
            100..=103,
            104..=107,
            // Hours 108 to 111 are not recorded.
            112..=115,
        ])
        .map(|hours| block(1, hours, &present))
        .collect();
    let last = blocks.len() - 1;
    add_result(&mut blocks[last][1], 6, 106, 105);
    recording(blocks)
}

#[test]
fn forward_results_close_battles_before_a_recorded_gap() {
    for earlier in [vec![], vec![88..=91, 92..=95, 96..=99]] {
        let (_, window) = assert_forward(&results_close_recording(&earlier), 1);
        assert_eq!(row(&window, "1:6:101")["result"]["hour"], 106);
    }
}

fn equipment_names_recording(unrecorded: &[RangeInclusive<i32>]) -> Source {
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
    let blocks: Vec<_> = unrecorded
        .iter()
        .cloned()
        .map(|hours| (false, hours))
        .chain(
            [100..=103, 104..=107, 108..=111, 112..=115]
                .into_iter()
                .map(|hours| (true, hours)),
        )
        .map(|(recorded, hours)| {
            let mut frames = block(1, hours, &present);
            for sample in frames.iter_mut().filter(|_| recorded) {
                *sample.tables[TableId::VariantDefinitions.index()] =
                    TableView::Complete(TableRows::VariantDefinitions(Vec::new()));
            }
            frames
        })
        .collect();
    recording(blocks)
}

#[test]
fn forward_loss_reads_keep_unknown_equipment_names() {
    for earlier in [vec![], vec![88..=91, 92..=95, 96..=99]] {
        let source = equipment_names_recording(&earlier);
        let (_, window) = assert_forward(&source, 1);
        let loss = &row(&window, "1:1:113")["losses"][0];
        assert!(loss.is_object());
        let fixture = Fixture::new();
        let census = forward(&source, 1, &fixture);
        assert!(
            census
                .designs
                .values
                .iter()
                .any(|design| design.variant == "7" && design.name.is_none())
        );
    }
}

fn unavailable_recording(hours: &[RangeInclusive<i32>]) -> Source {
    let blocks: Vec<Vec<DecodedFrame>> = hours
        .iter()
        .cloned()
        .map(|hours| {
            hours
                .map(|hour| dashboard_base::test_fixtures::source(1, hour, Some(0)))
                .collect::<Vec<_>>()
        })
        .collect();
    recording(blocks)
}

#[test]
fn forward_manifest_keeps_missing_battle_capture_unavailable() {
    let (manifest, window) =
        assert_forward(&unavailable_recording(&[98..=99, 100..=101, 102..=103]), 1);
    assert_eq!(manifest["available"], false);
    assert_eq!(manifest["unavailable"], "Not in this recording");
    assert_eq!(window["observed"], json!([]));
}

fn diagnostics_recording() -> Source {
    let quiet = |_: i32| -> Vec<Combat> { Vec::new() };
    // Only the first block holds a daily variant read.
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
    recording(blocks)
}

#[test]
fn forward_diagnostics_keep_early_variant_read_refusals() {
    let source = diagnostics_recording();
    let (_, window) = assert_forward(&source, 1);
    assert!(
        window["contentWarnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .any(|warning| warning.contains("equipment display name is not UTF-8")),
        "forward preparation reports the early read's refusal: {window}"
    );
}

#[test]
fn forward_variant_reads_keep_changed_operands_and_refusals() {
    use crate::products::land_battles::prepared::KEEP_ALL;
    use dashboard_base::products::equipment::VariantCatalogue;
    use std::collections::BTreeSet;
    let quiet = |_: i32| -> Vec<Combat> { Vec::new() };
    // Each block must hold a daily variant read; the refusing name is read first.
    let names: [&[u8]; 4] = [&[0xff], b"Second", b"Second", b"First"];
    let blocks: Vec<_> = (0..4)
        .map(|day| {
            let mut frames = block(1, 100 + 24 * day..=123 + 24 * day, &quiet);
            for sample in &mut frames {
                *sample.tables[TableId::VariantDefinitions.index()] =
                    TableView::Complete(TableRows::VariantDefinitions(
                        [(7, names[day as usize]), (8, b"Constant".as_slice())]
                            .into_iter()
                            .map(|(id, name)| VariantDefinitionRow {
                                id,
                                equipment_token: 5,
                                creator_link: 1,
                                display_name: name.to_vec(),
                                ..Default::default()
                            })
                            .collect(),
                    ));
                *sample.tables[TableId::LexicalTokens.index()] =
                    TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                        token: 5,
                        literal: b"mod_rifle_1".to_vec(),
                    }]));
            }
            frames
        })
        .collect();
    let source = recording(blocks);
    // Every variant's operands read, whatever hour first read them.
    let operands = |variants: &VariantCatalogue| -> BTreeSet<(u64, String)> {
        variants
            .versions
            .iter()
            .flat_map(|(id, versions)| {
                versions.iter().filter_map(move |version| {
                    let mut raw = version.raw.clone()?;
                    raw.first_hour = 0;
                    Some((*id, format!("{raw:?}")))
                })
            })
            .collect()
    };
    let names = |variants: &VariantCatalogue| {
        variants
            .versions
            .iter()
            .map(|(&id, versions)| {
                (
                    id,
                    versions
                        .iter()
                        .map(|version| version.raw.as_ref().map(|raw| raw.display_name.clone()))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    KEEP_ALL.set(true);
    let kept = forward(&source, 1, &Fixture::new()).variants;
    KEEP_ALL.set(false);
    assert_eq!(
        names(&kept),
        [
            (
                7,
                vec![
                    Some(String::new()),
                    Some("Second".into()),
                    Some("First".into())
                ]
            ),
            (8, vec![Some("Constant".into())]),
        ],
        "the repeated Second read and unchanged variant 8 add no versions"
    );
    assert_eq!(
        operands(&kept).len(),
        4,
        "three names of variant 7 and one of variant 8"
    );
    assert!(
        kept.warnings()
            .iter()
            .any(|warning| warning.contains("not UTF-8")),
        "{:?}",
        kept.warnings()
    );
    // Paging keeps each variant's current operands and every refusal read before them.
    let paged = forward(&source, 1, &Fixture::new()).variants;
    assert_eq!(
        names(&paged),
        [
            (7, vec![Some("First".into())]),
            (8, vec![Some("Constant".into())]),
        ]
    );
    assert_eq!(paged.warnings(), kept.warnings());
}

#[test]
fn loss_reads_name_designs_renamed_or_created_between_variant_reads() {
    use dashboard_base::products::equipment::VariantCatalogue;
    use replay_data::rows::VariantChangeRow;
    assert!(
        VariantCatalogue::TABLES
            .iter()
            .all(|table| CENSUS_TABLES.contains(table)),
        "the census selects every table its variant catalogue reads"
    );
    // Variants are read at hour 118; at 120 the writer renames design 7 and creates design 8.
    let frames: Vec<DecodedFrame> = (118..=123)
        .map(|hour| {
            let mut sample = hour_frame(
                1,
                hour,
                vec![Combat {
                    lost: Some(hour - 99),
                    ..Combat::new(1, 118)
                }],
            );
            let changed = hour >= 120;
            let definitions = [(7, if changed { "Renamed" } else { "First" })]
                .into_iter()
                .chain(changed.then_some((8, "Created")))
                .map(|(id, name)| VariantDefinitionRow {
                    id,
                    equipment_token: 5,
                    creator_link: 1,
                    display_name: name.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect();
            *sample.tables[TableId::VariantDefinitions.index()] =
                TableView::Complete(TableRows::VariantDefinitions(definitions));
            *sample.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 5,
                    literal: b"mod_rifle_1".to_vec(),
                }]));
            let events = if hour == 120 { vec![7, 8] } else { Vec::new() };
            *sample.tables[TableId::VariantChanges.index()] =
                TableView::Complete(TableRows::VariantChanges(
                    (1..)
                        .zip(events)
                        .map(|(sequence, variant)| VariantChangeRow {
                            sequence,
                            hour,
                            variant,
                            flags: Some([0; 3]),
                            ..Default::default()
                        })
                        .collect(),
                ));
            if changed
                && let TableView::Complete(TableRows::LandCombatProgress(rows)) =
                    &mut *sample.tables[TableId::LandCombatProgress.index()]
            {
                rows[0].defender_equipment_losses = vec![LandEquipmentLoss {
                    variant: 8,
                    amount_raw: -1_000 * i64::from(hour),
                }]
                .into();
            }
            sample
        })
        .collect();
    let source = recording(vec![frames]);
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[]);
    let catalogue = Arc::new(catalogue);
    let expected = json!([
        { "name": "First", "variant": "7" },
        { "name": "Renamed", "variant": "7" },
        { "name": "Created", "variant": "8" },
    ]);
    let flag = AtomicBool::new(false);
    let oracle = scan(source.replay_ref(), 1, &flag, Some(catalogue.clone())).unwrap();
    assert_eq!(
        serde_json::to_value(oracle.course("1:1:118").unwrap().designs).unwrap(),
        expected
    );
    let replay = source.replay_ref();
    let mut census = Census::new(replay.format);
    census.configure(&source, &LoadContext { content: None });
    census.catalogue = Some(catalogue);
    let fixture = Fixture::new();
    let mut tracker = FrameChangeTracker::new();
    for (ordinal, index) in replay.index.iter().enumerate() {
        replay
            .read_block_alone(
                index,
                &CENSUS_TABLES,
                |_, frame| {
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
    }
    let course = forward_query::query(
        &source,
        "land-battle-course",
        json!({ "branchId": "1", "id": "1:1:118" }),
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .unwrap();
    assert_eq!(course["designs"], expected);
}

#[test]
fn forward_pages_survive_an_unused_refusing_variant_name() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation");
    let (_, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "localisation/mod_l_english.yml",
        "\u{feff}l_english:\n mod_rifle_1:0 \"a\u{a4}b\"\n",
    )]);
    let quiet = |_: i32| -> Vec<Combat> { Vec::new() };
    let mut earlier = block(1, 100..=123, &quiet);
    for frame in &mut earlier {
        *frame.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
                id: 7,
                equipment_token: 5,
                creator_link: 1,
                ..Default::default()
            }]));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 5,
                literal: b"mod_rifle_1".to_vec(),
            }]));
    }
    let (first, context) = dashboard_base::test_fixtures::source_with_content(earlier, &folder.0);
    let mut grown: Option<Replay> = None;
    for hours in [124..=127, 128..=131] {
        let replay = grown.as_ref().unwrap_or_else(|| first.replay_ref());
        grown = Some(
            replay
                .append(&[dashboard_base::test_fixtures::continuation_records(
                    replay,
                    1,
                    block(1, hours, &quiet),
                )])
                .unwrap(),
        );
    }
    let replay = grown.unwrap();
    let flag = AtomicBool::new(false);
    let source = Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "forward-refusing-name".into(),
            revision: "1".into(),
        },
        "forward-refusing-name.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    let fixture = Fixture::new();
    let mut census = Census::new(source.replay_ref().format);
    let mut tracker = FrameChangeTracker::new();
    for (ordinal, index) in source.replay_ref().index.iter().enumerate() {
        source
            .replay_ref()
            .read_block_alone(
                index,
                &CENSUS_TABLES,
                |_, frame| {
                    census.configure(&source, &context);
                    let changes = tracker.observe(frame, source.replay_ref().format);
                    census.observe_prepared(frame, &changes)
                },
                || Ok(()),
            )
            .unwrap();
        let mut writer = fixture.writer();
        census
            .publish_block(&mut writer, source.replay_ref(), 1, ordinal, &flag)
            .unwrap();
        writer.commit().unwrap();
    }
    let window = forward_query::query(
        &source,
        "land-battles",
        json!({"branchId":"1", "window":2}),
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .unwrap();
    assert_eq!(window["rows"], json!([]));
    assert_eq!(window["observed"], json!([[128, 131]]));
    assert!(census.refusals == dashboard_base::catalogue::OwnRefusals::default());
}
