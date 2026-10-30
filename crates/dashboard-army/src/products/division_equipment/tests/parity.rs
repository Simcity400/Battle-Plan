//! Each hour a prepared history holds, against the reached product's served reply, field by
//! field: through the reads, renames and refusals that change what the history publishes.
use super::*;

/// The first field two replies differ in, by its path; none where they agree.
fn difference(held: &serde_json::Value, served: &serde_json::Value, path: &str) -> Option<String> {
    use serde_json::Value;
    match (held, served) {
        (Value::Object(held), Value::Object(served)) => held
            .keys()
            .chain(served.keys())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .find_map(|key| match (held.get(key), served.get(key)) {
                (Some(held), Some(served)) => difference(held, served, &format!("{path}.{key}")),
                _ => Some(format!("{path}.{key} is in one reply only")),
            }),
        (Value::Array(held), Value::Array(served)) if held.len() == served.len() => held
            .iter()
            .zip(served)
            .enumerate()
            .find_map(|(at, (held, served))| difference(held, served, &format!("{path}[{at}]"))),
        _ => (held != served).then(|| format!("{path}: {held} held, {served} served")),
    }
}

/// `history`'s reply at `hour` is the reached product's served reply, field by field.
fn assert_served(
    source: &Source,
    context: &LoadContext,
    history: &history::DivisionEquipmentHistoryReply,
    hour: i32,
) {
    let (tag, count_for) = (&history.tag, history.count_for);
    let held = history::reply_at(history, hour)
        .unwrap_or_else(|| panic!("{tag} {count_for:?} at {hour} is not in the history"));
    let mut p = params();
    (p.tag, p.count_for, p.hour) = (tag.clone(), count_for, hour);
    let reached = served(source, &p, context);
    if let Some(field) = difference(&serde_json::to_value(&held).unwrap(), &reached, "reply") {
        panic!("{tag} {count_for:?} at {hour}: {field}");
    }
}

/// The value `series` of `history` holds at `hour`; none where it holds none.
fn value_at(
    history: &history::DivisionEquipmentHistoryReply,
    series: &str,
    hour: i32,
) -> Option<String> {
    history
        .window
        .series
        .iter()
        .find(|held| held.name == series)?
        .runs
        .iter()
        .find(|run| run.0 <= hour && hour <= run.1)
        .map(|run| run.2.clone())
}

const FIRST_HOUR: i32 = 60_000_002;
const LAST_HOUR: i32 = 60_000_110;
/// The recording's gap: its frames are kept, its hours are not known.
const GAP: (i32, i32) = (60_000_084, 60_000_086);

/// GER and ITA over five days. Designs are read at 2 and each 22, the pool each 2, stockpiles
/// at 2, each 8, and at 15 and 63 between. Division 1 changes template from 30 to 70; design 70
/// is renamed at 40 for the read at 46; the stockpile changes at 50, a pool at 60. Stockpiles
/// are unavailable from 12 to 14, statuses truncated at 33 and 34, template icons unavailable
/// from 64 to 66, the pool truncated at 75 and designs at 96; 84 to 86 are a recorded gap.
fn branch_frames() -> Vec<DecodedFrame> {
    (2..=110)
        .map(|offset| {
            let mut f = frame();
            f.hour = 60_000_000 + offset;
            if offset == 2 {
                f.metrics.table_ns[TableId::VariantModules.index()] = 1;
            }
            if [2, 15, 63].contains(&offset) {
                f.metrics.table_ns[TableId::Stockpiles.index()] = 1;
            }
            if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
                &mut *f.tables[TableId::DivisionTemplateIcons.index()]
            {
                let mut second = rows[0].clone();
                second.template = 6;
                rows.push(second);
                for (row, name) in rows
                    .iter_mut()
                    .zip([&b"Line division"[..], &b"Assault division"[..]])
                {
                    row.display_name = Recorded::Value(Some(TemplateDisplayName {
                        sample_hour: PacketHour::Host,
                        value: TemplateNameValue::Changed {
                            literal: name.to_vec(),
                            fallback_key: None,
                        },
                    }));
                }
            }
            if (64..=66).contains(&offset) {
                *f.tables[TableId::DivisionTemplateIcons.index()] = TableView::Unavailable {
                    reason: "template icon walk refused".into(),
                };
            }
            *f.tables[TableId::DivisionTemplates.index()] =
                TableView::Complete(TableRows::DivisionTemplates(
                    [5, 6]
                        .into_iter()
                        .map(|template| DivisionTemplateRow {
                            template,
                            owner: 11,
                            ..Default::default()
                        })
                        .collect(),
                ));
            // Division 4 is GER's, hosted by ITA.
            let mut divisions = vec![
                division(1, 11),
                division(2, 9001),
                division(3, 21),
                division(4, 11),
            ];
            divisions[3].expeditionary_owner = 21;
            if (30..70).contains(&offset) {
                divisions[0].template = 6;
            }
            *f.tables[TableId::Divisions.index()] =
                TableView::Complete(TableRows::Divisions(divisions));
            let statuses: Vec<_> = (1..=4).map(status).collect();
            *f.tables[TableId::DivisionStatus.index()] = if (33..=34).contains(&offset) {
                TableView::Truncated {
                    rows: TableRows::DivisionStatus(statuses[..2].to_vec()),
                    missing: 2,
                }
            } else {
                TableView::Complete(TableRows::DivisionStatus(statuses))
            };
            if let TableView::Complete(TableRows::DivisionLandInputs(rows)) =
                &mut *f.tables[TableId::DivisionLandInputs.index()]
            {
                for division in [3, 4] {
                    rows.push(DivisionLandInputsRow {
                        division,
                        manpower: vec![DivisionManpowerContributor {
                            country: 21,
                            amount: 500,
                        }],
                        ..Default::default()
                    });
                }
            }
            let pools = vec![
                equipment(1, 0, 70, 15_000_000),
                equipment(2, 0, 70, 5_000_000),
                equipment(2, 1, 71, 1_000_000),
                equipment(3, 0, 71, if offset >= 60 { 4_000_000 } else { 2_000_000 }),
                equipment(4, 0, 70, 3_000_000),
            ];
            *f.tables[TableId::DivisionEquipment.index()] = if offset == 75 {
                TableView::Truncated {
                    rows: TableRows::DivisionEquipment(pools[..2].to_vec()),
                    missing: 3,
                }
            } else {
                TableView::Complete(TableRows::DivisionEquipment(pools))
            };
            let stock = vec![
                StockpileRow {
                    country: 21,
                    ordinal: 0,
                    variant_handle: 71,
                    amount_raw: 1_000,
                },
                StockpileRow {
                    country: 9001,
                    ordinal: 0,
                    variant_handle: 70,
                    amount_raw: 900_000_000,
                },
                StockpileRow {
                    country: 9001,
                    ordinal: 1,
                    variant_handle: 71,
                    amount_raw: if offset >= 50 { 75 } else { 50 },
                },
            ];
            *f.tables[TableId::Stockpiles.index()] = if (12..=14).contains(&offset) {
                TableView::Unavailable {
                    reason: "stockpile walk refused".into(),
                }
            } else {
                TableView::Complete(TableRows::Stockpiles(stock))
            };
            let name: &[u8] = if offset >= 40 {
                b"Renamed model"
            } else {
                b"Synthetic model"
            };
            let designs = vec![
                VariantDefinitionRow {
                    id: 70,
                    equipment_token: 3,
                    creator_link: 11,
                    display_name: name.to_vec(),
                    ..Default::default()
                },
                VariantDefinitionRow {
                    id: 71,
                    equipment_token: 3,
                    creator_link: 21,
                    display_name: b"Second model".to_vec(),
                    ..Default::default()
                },
            ];
            *f.tables[TableId::VariantDefinitions.index()] = if offset == 96 {
                TableView::Truncated {
                    rows: TableRows::VariantDefinitions(designs[..1].to_vec()),
                    missing: 1,
                }
            } else {
                TableView::Complete(TableRows::VariantDefinitions(designs))
            };
            f
        })
        .collect()
}

#[test]
fn every_hour_through_renames_retemplates_reads_refusals_and_a_gap_is_the_served_reply() {
    let (source, context, _folder) = recorded_blocks(
        branch_frames(),
        &[60_000_040, 60_000_090],
        &[GAP],
        "parity",
        DOCUMENT,
    );
    let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
    prepare_history(&source, &context, &mut fixture, None);
    let in_gap = |hour: i32| GAP.0 <= hour && hour <= GAP.1;
    for tag in ["GER", "ITA", "SOV"] {
        for count_for in [ArmyAttribution::Owner, ArmyAttribution::Host] {
            let history = history_of(&source, &context, &fixture, tag, count_for, Some(LAST_HOUR));
            for hour in FIRST_HOUR..=LAST_HOUR {
                if in_gap(hour) {
                    assert!(history::reply_at(&history, hour).is_none(), "{hour}");
                } else {
                    assert_served(&source, &context, &history, hour);
                }
            }
        }
    }
    let history = history_of(
        &source,
        &context,
        &fixture,
        "GER",
        ArmyAttribution::Owner,
        Some(LAST_HOUR),
    );
    // The stockpile's read is dated once for every reply, never beside each country's stock.
    assert!(
        history
            .window
            .series
            .iter()
            .all(|series| !series.name.ends_with("stockHour")),
        "{:?}",
        history.window.series
    );
    for (offset, read) in [
        (5, Some(2)),
        (10, Some(8)),
        (13, None),
        (20, Some(15)),
        (40, Some(32)),
        (60, Some(56)),
        (70, Some(63)),
        (82, Some(80)),
        (90, None),
        (105, Some(104)),
    ] {
        assert_eq!(
            value_at(&history, "*|stock", 60_000_000 + offset),
            read.map(|read: i32| (60_000_000 + read).to_string()),
            "+{offset}"
        );
    }
    // A design keeps its number across the designs reads at 22, 46 and 70 until it is renamed,
    // and is numbered alike wherever it is held, in a pool or in stock.
    let unrenamed = history
        .window
        .series
        .iter()
        .find(|series| series.name.ends_with("|design|71"))
        .unwrap();
    assert_eq!(unrenamed.runs.len(), 1, "{unrenamed:?}");
    assert_eq!(
        (unrenamed.runs[0].0, unrenamed.runs[0].1),
        (FIRST_HOUR, 60_000_074)
    );
    let renamed = series_ending(&history, "|design|70");
    let renamed = renamed.as_str();
    let at = |series: &str, offset: i32| value_at(&history, series, 60_000_000 + offset).unwrap();
    assert_eq!(at(renamed, 2), at(renamed, 45));
    assert_ne!(at(renamed, 45), at(renamed, 46));
    assert_eq!(at(renamed, 46), at(renamed, 74));
    for offset in [2, 11, 15, 55] {
        assert_eq!(at("stockDesign|70", offset), at(renamed, 45), "+{offset}");
    }
    for offset in [56, 83] {
        assert_eq!(at("stockDesign|70", offset), at(renamed, 46), "+{offset}");
    }
    assert_eq!(history.designs.len(), 3, "{:?}", history.designs);
    let names: BTreeSet<_> = history.designs.values().flatten().cloned().collect();
    assert_eq!(
        names,
        BTreeSet::from(["Renamed model", "Second model", "Synthetic model"].map(String::from))
    );
}

#[test]
fn a_history_resumed_mid_block_answers_as_served_once_its_reads_are_proved_again() {
    let (source, context, _folder) = recorded_blocks(
        branch_frames(),
        &[60_000_040, 60_000_090],
        &[GAP],
        "parity-resume",
        DOCUMENT,
    );
    let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let resume = 60_000_050;
    prepare_history(&source, &context, &mut fixture, Some(resume));
    for count_for in [ArmyAttribution::Owner, ArmyAttribution::Host] {
        let history = history_of(
            &source,
            &context,
            &fixture,
            "GER",
            count_for,
            Some(LAST_HOUR),
        );
        for hour in FIRST_HOUR..=LAST_HOUR {
            let proved = hour <= resume || hour > GAP.1;
            if history::reply_at(&history, hour).is_some() {
                assert_served(&source, &context, &history, hour);
            } else {
                assert!(!proved, "{count_for:?} at {hour}");
            }
        }
    }
}

/// `count` countries each holding one of designs 70 to 72 in its division's pool and another in
/// stock, all read at 2 and then daily; nothing the game holds changes.
fn quiet_frames(count: u32) -> Vec<DecodedFrame> {
    (2..=58)
        .map(|offset| {
            let mut f = scale_frame(count, offset);
            if offset == 2 {
                f.metrics.table_ns[TableId::VariantModules.index()] = 1;
                f.metrics.table_ns[TableId::Stockpiles.index()] = 1;
            }
            let countries = 0..u64::from(count);
            *f.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
                countries
                    .clone()
                    .map(|at| division(1 + at, 100 + at as u32))
                    .collect(),
            ));
            *f.tables[TableId::DivisionStatus.index()] = TableView::Complete(
                TableRows::DivisionStatus(countries.clone().map(|at| status(1 + at)).collect()),
            );
            *f.tables[TableId::DivisionLandInputs.index()] =
                TableView::Complete(TableRows::DivisionLandInputs(
                    countries
                        .clone()
                        .map(|at| DivisionLandInputsRow {
                            division: 1 + at,
                            manpower: vec![DivisionManpowerContributor {
                                country: 100 + at as u32,
                                amount: 1_000,
                            }],
                            ..Default::default()
                        })
                        .collect(),
                ));
            *f.tables[TableId::DivisionEquipment.index()] =
                TableView::Complete(TableRows::DivisionEquipment(
                    countries
                        .clone()
                        .map(|at| equipment(1 + at, 0, 70 + at % 3, 1_000_000))
                        .collect(),
                ));
            *f.tables[TableId::Stockpiles.index()] = TableView::Complete(TableRows::Stockpiles(
                countries
                    .map(|at| StockpileRow {
                        country: 100 + at as u32,
                        ordinal: 0,
                        variant_handle: 70 + (at + 1) % 3,
                        amount_raw: 500,
                    })
                    .collect(),
            ));
            *f.tables[TableId::VariantDefinitions.index()] =
                TableView::Complete(TableRows::VariantDefinitions(
                    (70..=72)
                        .map(|id| VariantDefinitionRow {
                            id,
                            equipment_token: 3,
                            creator_link: 100,
                            display_name: format!("Model {id}").into_bytes(),
                            ..Default::default()
                        })
                        .collect(),
                ));
            f
        })
        .collect()
}

#[test]
fn daily_reads_alike_publish_only_their_hours_among_fifteen_hundred_countries() {
    let count = 1_500;
    let (source, context, _folder) = content_blocks(quiet_frames(count), &[], "parity-quiet");
    let replay = source.replay_ref();
    let mut prepared = history::Prepared::new();
    prepared.configure(&source, &context);
    let mut hours = Vec::new();
    for index in replay.index.iter().filter(|index| index.branch == 1) {
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        replay
            .read_block_recorded_alone(
                index,
                history::tables(),
                |raw, frame| {
                    let changes = tracker.observe_updates(frame, raw, replay.format);
                    let before = prepared.observations();
                    prepared.observe(frame, &changes, replay)?;
                    hours.push((frame.hour, prepared.observations() - before));
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    // The first hour publishes every country. After it, a pool or stockpile read publishes only
    // its own hour, once for all countries, and a designs read nothing: each design keeps its
    // number, the three of them numbered once.
    assert!(hours[0].1 > 2 * count as usize, "{:?}", hours[0]);
    let expected: Vec<(i32, usize)> = (3..=58)
        .map(|offset| {
            let read = [8, 26, 32, 50, 56].contains(&offset);
            (60_000_000 + offset, usize::from(read))
        })
        .collect();
    assert_eq!(&hours[1..], expected.as_slice());
    assert_eq!(prepared.designs().len(), 3);
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut writer = fixture.writer();
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
    for (tag, attributions) in [
        (
            "C0000",
            &[ArmyAttribution::Owner, ArmyAttribution::Host][..],
        ),
        ("C0749", &[ArmyAttribution::Owner][..]),
        (
            "C1499",
            &[ArmyAttribution::Owner, ArmyAttribution::Host][..],
        ),
    ] {
        for &count_for in attributions {
            let history = history_of(
                &source,
                &context,
                &fixture,
                tag,
                count_for,
                Some(60_000_058),
            );
            for hour in 60_000_002..=60_000_058 {
                assert_served(&source, &context, &history, hour);
            }
        }
    }
}
