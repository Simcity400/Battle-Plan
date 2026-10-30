use super::*;
use crate::products::equipment_stockpile::columnar::{self, TABLES};
use dashboard_base::prepared::{
    frame_changes::FrameChangeTracker,
    series::tests::{Fixture, assert_payloads_equal},
};
use engine_native::cequipment_production_line::{KIND_MILITARY, KIND_NAVAL};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator, Start},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryLinkRow, CountryRow, LexicalTokenRow, LogisticsEquipmentRow, ProductionLineRow,
        Recorded, StockpileRow, TableRows, TableSchema, VariantDefinitionRow,
    },
};
use replay_frame::frame::TableView;

fn logistics(
    country: u32,
    archetype: u32,
    token: u32,
    balance_raw: i64,
    stock_raw: i64,
) -> LogisticsEquipmentRow {
    LogisticsEquipmentRow {
        country,
        archetype,
        token,
        balance_raw,
        stock_raw,
        available_raw: Recorded::Value(Some(stock_raw.max(0))),
    }
}

fn line(
    handle: u64,
    country: u32,
    variant: u64,
    speed_raw: i64,
    cost_raw: i64,
    kind: u8,
) -> ProductionLineRow {
    ProductionLineRow {
        handle,
        country,
        kind,
        variant,
        speed_raw,
        cost_raw,
        ..Default::default()
    }
}

/// Germany (canonical slot 7) through links 11 and 9001, Italy (21, slot 8), France (31,
/// slot 9) and Sweden (41, slot 10), which holds nothing.
fn frame(hour: i32) -> DecodedFrame {
    let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(1));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        [
            (21, 8, "ITA"),
            (31, 9, "FRA"),
            (41, 10, "SWE"),
            (9001, 7, "GER"),
        ]
        .into_iter()
        .map(|(id, canonical_slot, tag)| CountryRow {
            id,
            canonical_slot,
            tag: tag.as_bytes().to_vec(),
            ..Default::default()
        })
        .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [(11, 7), (21, 8), (31, 9), (41, 10), (9001, 7)]
            .into_iter()
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect(),
    ));
    let stock = |country, ordinal, variant_handle, amount_raw| StockpileRow {
        country,
        ordinal,
        variant_handle,
        amount_raw,
    };
    *frame.tables[TableId::Stockpiles.index()] = TableView::Complete(TableRows::Stockpiles(vec![
        stock(11, 0, 10, 100_000),
        stock(11, 1, 20, 5),
        stock(21, 0, 10, 999),
        stock(31, 0, 20, 7_000),
        stock(9001, 0, 30, 3),
    ]));
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(vec![
            line(1, 9001, 10, 360_045, 2_250_000, KIND_MILITARY),
            line(2, 21, 10, 100_000, 50_000, KIND_MILITARY),
            line(3, 11, 20, 100_000, 0, KIND_MILITARY),
            line(4, 31, 30, 50_000, 10_000, KIND_NAVAL),
        ]));
    *frame.tables[TableId::LogisticsEquipment.index()] =
        TableView::Complete(TableRows::LogisticsEquipment(vec![
            logistics(21, 1, 1, -5, 7),
            logistics(9001, 1, 1, 3, 9),
            logistics(9001, 2, 2, 0, 0),
        ]));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        [(1, "infantry_equipment_1"), (2, "destroyer_1")]
            .into_iter()
            .map(|(token, literal)| LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .collect(),
    ));
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(
            [(10, 1, ""), (20, 1, "Gewehr 43"), (30, 2, "")]
                .into_iter()
                .map(|(id, equipment_token, name)| VariantDefinitionRow {
                    id,
                    equipment_token,
                    creator_link: 9001,
                    display_name: name.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

fn stock_rows(frame: &mut DecodedFrame) -> &mut Vec<StockpileRow> {
    match &mut *frame.tables[TableId::Stockpiles.index()] {
        TableView::Complete(TableRows::Stockpiles(rows)) => rows,
        _ => unreachable!("fixture stock is complete"),
    }
}

fn line_rows(frame: &mut DecodedFrame) -> &mut Vec<ProductionLineRow> {
    match &mut *frame.tables[TableId::ProductionLines.index()] {
        TableView::Complete(TableRows::ProductionLines(rows)) => rows,
        _ => unreachable!("fixture lines are complete"),
    }
}

fn logistics_rows(frame: &mut DecodedFrame) -> &mut Vec<LogisticsEquipmentRow> {
    match &mut *frame.tables[TableId::LogisticsEquipment.index()] {
        TableView::Complete(TableRows::LogisticsEquipment(rows)) => rows,
        _ => unreachable!("fixture logistics are complete"),
    }
}

fn lexical_rows(frame: &mut DecodedFrame) -> &mut Vec<LexicalTokenRow> {
    match &mut *frame.tables[TableId::LexicalTokens.index()] {
        TableView::Complete(TableRows::LexicalTokens(rows)) => rows,
        _ => unreachable!("fixture tokens are complete"),
    }
}

fn country_rows(frame: &mut DecodedFrame) -> &mut Vec<CountryRow> {
    match &mut *frame.tables[TableId::Countries.index()] {
        TableView::Complete(TableRows::Countries(rows)) => rows,
        _ => unreachable!("fixture countries are complete"),
    }
}

fn link_rows(frame: &mut DecodedFrame) -> &mut Vec<CountryLinkRow> {
    match &mut *frame.tables[TableId::CountryLinks.index()] {
        TableView::Complete(TableRows::CountryLinks(rows)) => rows,
        _ => unreachable!("fixture links are complete"),
    }
}

fn variant_rows(frame: &mut DecodedFrame) -> &mut Vec<VariantDefinitionRow> {
    match &mut *frame.tables[TableId::VariantDefinitions.index()] {
        TableView::Complete(TableRows::VariantDefinitions(rows)) => rows,
        _ => unreachable!("fixture variants are complete"),
    }
}

/// `frame`'s hour with the stock and lines read every fourth hour, the logistics record
/// every sixth and the variant table every fifth, beside their daily slots.
fn dated(frame: &mut DecodedFrame, hour: i32) {
    frame.hour = hour;
    frame.metrics.table_ns.fill(0);
    frame.metrics.table_ns[TableId::Stockpiles.index()] = u64::from(hour % 4 == 0);
    frame.metrics.table_ns[TableId::LogisticsEquipment.index()] = u64::from(hour % 6 == 0);
    frame.metrics.table_ns[TableId::VariantDefinitions.index()] = u64::from(hour % 5 == 0);
}

/// Hours 100 to 135 of one run: stock appearing and disappearing, lines moving, a rename
/// and a change of type, a holder renamed, a link added, an overflowing total, and each
/// numeric, naming and country table missing or partial for an hour.
fn campaign() -> Vec<DecodedFrame> {
    let mut frame = frame(100);
    let mut inputs = Vec::new();
    for hour in 100..=135 {
        dated(&mut frame, hour);
        match hour {
            101 => stock_rows(&mut frame)[0].amount_raw += 1,
            102 => country_rows(&mut frame)[0].political_power_raw = Some(7),
            103 => line_rows(&mut frame)[0].efficiency_raw += 1,
            105 => {
                stock_rows(&mut frame)[2].amount_raw += 10;
                line_rows(&mut frame)[1].speed_raw += 1_000;
            }
            106 => {
                stock_rows(&mut frame).push(StockpileRow {
                    country: 41,
                    ordinal: 0,
                    variant_handle: 10,
                    amount_raw: 50,
                });
                stock_rows(&mut frame).sort_by_key(|row| (row.country, row.ordinal));
            }
            // A token the dictionary spells only later.
            107 => {
                logistics_rows(&mut frame).push(logistics(9001, 3, 3, 1, 1));
                logistics_rows(&mut frame).sort_by_key(|row| (row.country, row.archetype));
            }
            108 => variant_rows(&mut frame)[1].display_name = b"Gewehr 43/1".to_vec(),
            109 => lexical_rows(&mut frame).push(LexicalTokenRow {
                token: 3,
                literal: b"truck_1".to_vec(),
            }),
            110 => {
                logistics_rows(&mut frame)
                    .iter_mut()
                    .find(|row| (row.country, row.archetype) == (9001, 1))
                    .unwrap()
                    .balance_raw -= 4;
            }
            // A line moves from Germany's other link to Italy.
            111 => line_rows(&mut frame)[2].country = 21,
            112 => stock_rows(&mut frame).retain(|row| (row.country, row.ordinal) != (21, 0)),
            // Italy is AI-controlled at the next read.
            113 => logistics_rows(&mut frame).retain(|row| row.country != 21),
            // Variant 10 changes type: its holders change groups.
            114 => variant_rows(&mut frame)[0].equipment_token = 2,
            116 => stock_rows(&mut frame)[1].amount_raw += 1,
            117 => {
                country_rows(&mut frame)
                    .iter_mut()
                    .find(|row| row.id == 41)
                    .unwrap()
                    .tag = b"NOR".to_vec();
            }
            119 => line_rows(&mut frame).retain(|row| row.handle != 2),
            121 => {
                logistics_rows(&mut frame)
                    .iter_mut()
                    .find(|row| (row.country, row.archetype) == (9001, 2))
                    .unwrap()
                    .available_raw = Recorded::Value(None);
            }
            // A spelling both names and logistics read.
            123 => lexical_rows(&mut frame)[0].literal = b"infantry_equipment_2".to_vec(),
            126 => {
                stock_rows(&mut frame).push(StockpileRow {
                    country: 9001,
                    ordinal: 1,
                    variant_handle: 10,
                    amount_raw: i64::MAX,
                });
                stock_rows(&mut frame).sort_by_key(|row| (row.country, row.ordinal));
            }
            130 => line_rows(&mut frame)[0].cost_raw = 0,
            131 => {
                line_rows(&mut frame).push(line(5, 41, 20, 1, 1, KIND_MILITARY));
                line_rows(&mut frame).sort_by_key(|row| row.handle);
            }
            132 => line_rows(&mut frame).last_mut().unwrap().kind = KIND_NAVAL,
            133 => stock_rows(&mut frame).retain(|row| row.country != 41),
            // Germany gains a link holding stock of its own.
            134 => {
                link_rows(&mut frame).push(CountryLinkRow {
                    id: 51,
                    canonical_slot: 7,
                });
                link_rows(&mut frame).sort_by_key(|row| row.id);
                stock_rows(&mut frame).push(StockpileRow {
                    country: 51,
                    ordinal: 0,
                    variant_handle: 30,
                    amount_raw: 9,
                });
                stock_rows(&mut frame).sort_by_key(|row| (row.country, row.ordinal));
            }
            _ => {}
        }
        let mut input = frame.clone();
        let missing = |reason: &str| TableView::Unavailable {
            reason: reason.into(),
        };
        match hour {
            115 => *input.tables[TableId::Stockpiles.index()] = missing("stock unread"),
            118 => {
                let rows = TableRows::ProductionLines(line_rows(&mut frame).clone());
                *input.tables[TableId::ProductionLines.index()] =
                    TableView::Truncated { rows, missing: 1 };
            }
            120 => *input.tables[TableId::LogisticsEquipment.index()] = missing("logistics unread"),
            122 => *input.tables[TableId::LexicalTokens.index()] = missing("tokens unread"),
            124 => *input.tables[TableId::VariantDefinitions.index()] = missing("variants unread"),
            127 => *input.tables[TableId::Countries.index()] = missing("countries unread"),
            129 => *input.tables[TableId::CountryLinks.index()] = missing("links unread"),
            _ => {}
        }
        inputs.push(input);
    }
    inputs
}

/// Hours `first..=last` of a run starting afresh, Germany's stock changing at its second.
fn later(first: i32, last: i32) -> Vec<DecodedFrame> {
    let mut frame = frame(first);
    (first..=last)
        .map(|hour| {
            dated(&mut frame, hour);
            if hour == first + 1 {
                stock_rows(&mut frame)[0].amount_raw += 7;
            }
            frame.clone()
        })
        .collect()
}

/// Six days read only in their daily slots, so every other hour is quiet: Germany's stock
/// changes each day after its read, Italy's line on the second day, a new design reaches
/// France's stock on the third, and Sweden's logistics appear on the fourth.
fn days() -> Vec<DecodedFrame> {
    let mut frame = frame(96);
    let mut inputs = Vec::new();
    for hour in 96..240 {
        frame.hour = hour;
        frame.metrics.table_ns.fill(0);
        let day = (hour - 96) / 24;
        if hour % 24 == 13 {
            stock_rows(&mut frame)[0].amount_raw += 1_000;
        }
        match (day, hour % 24) {
            (1, 15) => line_rows(&mut frame)[1].speed_raw += 5_000,
            (2, 20) => {
                variant_rows(&mut frame).push(VariantDefinitionRow {
                    id: 40,
                    equipment_token: 1,
                    creator_link: 31,
                    display_name: b"Fusil".to_vec(),
                    ..Default::default()
                });
                stock_rows(&mut frame).push(StockpileRow {
                    country: 31,
                    ordinal: 1,
                    variant_handle: 40,
                    amount_raw: 12,
                });
                stock_rows(&mut frame).sort_by_key(|row| (row.country, row.ordinal));
            }
            (3, 2) => {
                logistics_rows(&mut frame).push(logistics(41, 1, 1, 2, 2));
                logistics_rows(&mut frame).sort_by_key(|row| (row.country, row.archetype));
            }
            _ => {}
        }
        inputs.push(frame.clone());
    }
    inputs
}

fn write<T: TableSchema>(walk: &mut Synthetic, input: &DecodedFrame) {
    let read = input.metrics.table_ns[T::ID.index()] != 0;
    let sample = Sample::FromReadBit;
    let input = match &*input.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

fn write_hour(walk: &mut Synthetic, input: &DecodedFrame) {
    write::<tables::Campaign>(walk, input);
    write::<tables::Countries>(walk, input);
    write::<tables::CountryLinks>(walk, input);
    write::<tables::CountryAppearance>(walk, input);
    write::<tables::LexicalTokens>(walk, input);
    write::<tables::VariantDefinitions>(walk, input);
    write::<tables::VariantModules>(walk, input);
    write::<tables::VariantUpgrades>(walk, input);
    write::<tables::VariantChanges>(walk, input);
    write::<tables::Stockpiles>(walk, input);
    write::<tables::ProductionLines>(walk, input);
    write::<tables::LogisticsEquipment>(walk, input);
}

fn flush(fixture: &Fixture, publish: impl FnOnce(&mut store::Writer)) {
    let mut writer = fixture.writer();
    publish(&mut writer);
    writer.commit().unwrap();
}

/// How a walk hands its projector on at a placement's restart.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Handover {
    /// A segment starts there, with a projector of its own.
    Segment,
    /// The segment goes on with a projector loaded from the one that walked it so far.
    Saved,
}

/// Where a run's row blocks end and its projection restarts, by the hours' ordinals: a
/// restart need not start a block.
struct Placement {
    label: &'static str,
    block: usize,
    restarts: BTreeSet<usize>,
}

impl Placement {
    fn ends(&self, ordinal: usize, hours: usize) -> bool {
        ordinal + 1 == hours || ordinal % self.block == self.block - 1
    }
}

fn placements(hours: usize) -> Vec<Placement> {
    vec![
        Placement {
            label: "segments on row blocks",
            block: 6,
            restarts: (1..hours).filter(|ordinal| ordinal % 6 == 0).collect(),
        },
        Placement {
            label: "restarts inside row blocks",
            block: 8,
            restarts: [1, 5, 7, 13, 22, 23, 30].into(),
        },
        Placement {
            label: "one walk through every row block",
            block: 5,
            restarts: BTreeSet::new(),
        },
        Placement {
            label: "a restart at every hour",
            block: 7,
            restarts: (1..hours).collect(),
        },
        Placement {
            label: "one row block, a restart every third hour",
            block: usize::MAX,
            restarts: (1..hours).filter(|ordinal| ordinal % 3 == 0).collect(),
        },
    ]
}

/// What a check walked: each branch's emit, and every hour the projectors pushed.
struct Walked {
    stores: [Fixture; 2],
    emits: BTreeMap<u64, Emit>,
    hours: Vec<columnar::Shape>,
}

/// Walks `runs` (each of a branch, in order) as the columnar walk and as production's row
/// blocks, placed as `placement` says, and holds every section either publishes equal at
/// each row block's end: the row path's store, then the projected one.
fn check(
    runs: &[(u64, Vec<DecodedFrame>)],
    content: &choices::Content,
    placement: &Placement,
    handover: Handover,
) -> Walked {
    let format = FormatVersion::CURRENT;
    let stores = [Fixture::new(), Fixture::new()];
    let mut expected: BTreeMap<u64, Prepared> = BTreeMap::new();
    let mut emits: BTreeMap<u64, Emit> = BTreeMap::new();
    let mut walked = Vec::new();
    for (run, (branch, inputs)) in runs.iter().enumerate() {
        let branch = *branch;
        let continues = runs[..run].iter().any(|(earlier, _)| *earlier == branch);
        let row = expected.entry(branch).or_insert_with(|| {
            let mut prepared = Prepared::default();
            prepared.bind_content(content);
            prepared
        });
        let emit = emits.entry(branch).or_insert_with(|| Emit::new(content));
        let mut walk = Synthetic::new(TABLES, branch, inputs[0].hour).unwrap();
        let mut world_held: Option<dashboard_world::World> = None;
        let mut tracker = FrameChangeTracker::new();
        let mut projector = columnar::Projector::new(format).unwrap();
        let mut hours: Vec<columnar::Hour> = Vec::new();
        let start = Start::Run { continues };
        assert!(projector.starts_at(start));
        projector.start(start, branch, inputs[0].hour).unwrap();
        let mut first = 0;
        for (ordinal, input) in inputs.iter().enumerate() {
            if ordinal > 0 && placement.restarts.contains(&ordinal) {
                match handover {
                    Handover::Segment => {
                        projector.end(&mut hours).unwrap();
                        projector = columnar::Projector::new(format).unwrap();
                        let start = Start::Restart(walk.state());
                        assert!(projector.starts_at(start));
                        projector.start(start, branch, input.hour).unwrap();
                        first = ordinal;
                    }
                    Handover::Saved => {
                        assert!(projector.carried().is_some());
                        let mut saved = Vec::new();
                        assert!(projector.save(&mut saved).unwrap());
                        projector = columnar::Projector::new(format).unwrap();
                        projector.load(&saved).unwrap();
                    }
                }
            }
            write_hour(&mut walk, input);
            walk.end(|state, manifest, frame, walk_starts| {
                let world = dashboard_world::World::at(
                    &mut world_held,
                    walk_starts,
                    &crate::products::equipment_stockpile::columnar::needs(),
                    (
                        replay_data::format::FormatVersion::CURRENT,
                        replay_data::scope::CaptureMode::Full,
                    ),
                    state,
                    manifest,
                )
                .unwrap();
                row.observe_changed(format, frame, &tracker.observe(frame, format))
                    .unwrap();
                projector
                    .hour(
                        &segments::Hour {
                            branch,
                            hour: frame.hour,
                            run: run as u32,
                            ordinal: ordinal as u32,
                            starts: ordinal == first,
                            opens: ordinal == first,
                            state,
                            manifest,
                            world,
                        },
                        &mut hours,
                    )
                    .unwrap();
            })
            .unwrap();
            for hour in std::mem::take(&mut hours) {
                walked.push(hour.shape());
                emit.observe(hour).unwrap();
            }
            if !placement.ends(ordinal, inputs.len()) {
                continue;
            }
            // The next row block starts from its keyframe, with no hour before it.
            tracker.invalidate();
            flush(&stores[0], |writer| {
                row.flush(writer, branch, &AtomicBool::new(false)).unwrap();
            });
            flush(&stores[1], |writer| {
                emit.flush(writer, branch, &AtomicBool::new(false)).unwrap();
            });
            assert_payloads_equal(
                &stores[1].payloads(),
                &stores[0].payloads(),
                &format!(
                    "{}, run {run}, through hour {}",
                    placement.label, input.hour
                ),
            );
        }
        projector.end(&mut hours).unwrap();
        assert!(hours.is_empty(), "a segment's end pushes nothing");
    }
    Walked {
        stores,
        emits,
        hours: walked,
    }
}

/// The published value of `tag`'s `field` of `id` at `hour`, as a reply reads it.
fn published(
    fixture: &Fixture,
    branch: u64,
    tag: &str,
    field: &str,
    id: u64,
    hour: i32,
) -> Option<i128> {
    let (keys, _): series::Catalogue<serde_json::Value> =
        series::read_catalogue(&fixture.store, &fixture.key, "stockpile", branch)
            .unwrap()
            .expect("published stockpile catalogue");
    let key = keys[&series::field_key(tag, &format!("{field}:{id}"))];
    let flag = AtomicBool::new(false);
    Reader::new(
        &fixture.store,
        &fixture.key,
        format!("stockpile-{branch}-{field}"),
        &flag,
    )
    .unwrap()
    .value_at(key, hour)
    .unwrap()
    .map(|run| run.value)
}

fn contents() -> [choices::Content; 2] {
    [
        choices::Content::default(),
        choices::Content::resolved(&[
            (
                "infantry_equipment_1",
                "infantry_equipment",
                Domain::Land,
                false,
            ),
            ("destroyer_1", "destroyer", Domain::Naval, false),
        ]),
    ]
}

#[test]
fn every_placement_publishes_the_row_paths_stockpile_across_gaps_and_branches() {
    let runs = [(1, campaign()), (1, later(140, 147)), (2, later(100, 105))];
    for content in contents() {
        for placement in placements(runs[0].1.len()) {
            let walked = check(&runs, &content, &placement, Handover::Segment);
            let [expected, actual] = &walked.stores;
            let sections: BTreeSet<String> = expected
                .payloads()
                .keys()
                .map(|(name, _)| name.clone())
                .collect();
            for section in [
                "catalogue",
                "stock",
                "stock-present",
                "daily",
                "balance",
                "available",
                "coverage",
                "reads",
                "names",
                "group-stock",
            ] {
                assert!(
                    sections.contains(&format!("stockpile-1-{section}")),
                    "{}: stockpile-1-{section} is published",
                    placement.label
                );
            }
            for store in [expected, actual] {
                // Read at 100; changed at 101 and read again at 104.
                assert_eq!(published(store, 1, "GER", "stock", 10, 102), Some(100_000));
                assert_eq!(published(store, 1, "GER", "stock", 10, 104), Some(100_001));
                assert_eq!(published(store, 1, "GER", "stock", 10, 124), Some(100_001));
                // A total past i64 is unknown, never clamped.
                assert_eq!(published(store, 1, "GER", "stock", 10, 128), None);
                // Sweden's stock appears at 106 and is first read at 108.
                assert_eq!(published(store, 1, "SWE", "stock", 10, 107), None);
                assert_eq!(published(store, 1, "SWE", "stock", 10, 108), Some(50));
                // Italy's row changed at 105 is read at 108; gone at its read at 112.
                assert_eq!(published(store, 1, "ITA", "stock", 10, 110), Some(1_009));
                assert_eq!(published(store, 1, "ITA", "stock", 10, 112), Some(0));
                // Nothing is carried over the gap; the later run reads afresh.
                assert_eq!(published(store, 1, "GER", "stock", 10, 137), None);
                assert_eq!(published(store, 1, "GER", "stock", 10, 140), Some(100_000));
                assert_eq!(published(store, 1, "GER", "stock", 10, 144), Some(100_007));
                assert_eq!(published(store, 2, "GER", "stock", 10, 101), Some(100_000));
                assert_eq!(published(store, 2, "GER", "stock", 10, 104), Some(100_007));
            }
        }
    }
}

#[test]
fn partial_and_unavailable_tables_match_at_every_restart() {
    for &table in TABLES.iter().filter(|&&table| table != TableId::Campaign) {
        for partial in [false, true] {
            let mut inputs = later(100, 111);
            for hour in [102, 104, 108] {
                let input = &mut inputs[(hour - 100) as usize];
                let TableView::Complete(rows) = &*input.tables[table.index()] else {
                    continue;
                };
                let replaced = if partial {
                    TableView::Truncated {
                        rows: rows.clone(),
                        missing: 1,
                    }
                } else {
                    TableView::Unavailable {
                        reason: "named synthetic refusal".into(),
                    }
                };
                *input.tables[table.index()] = replaced;
            }
            for placement in placements(inputs.len()) {
                check(
                    &[(1, inputs.clone())],
                    &contents()[1],
                    &placement,
                    Handover::Segment,
                );
            }
        }
    }
}

#[test]
fn daily_reads_and_quiet_spans_match_across_segments_and_flushes() {
    let inputs = days();
    let placements = [
        Placement {
            label: "day-and-a-quarter segments, flushes inside quiet spans",
            block: 50,
            restarts: (30..inputs.len()).step_by(30).collect(),
        },
        Placement {
            label: "one segment, daily flushes off the reads",
            block: 24,
            restarts: BTreeSet::new(),
        },
    ];
    for content in contents() {
        for placement in &placements {
            let walked = check(
                &[(1, inputs.clone())],
                &content,
                placement,
                Handover::Segment,
            );
            let quiet = walked.hours.iter().filter(|shape| shape.quiet).count();
            assert!(
                quiet >= inputs.len() - 6 * 4 - placement.restarts.len() - 1,
                "{}: {quiet} of {} hours are quiet",
                placement.label,
                inputs.len()
            );
            // Germany's stock changes after each read: a stock read inside a segment reads
            // Germany alone, and the third day's adds France's new design.
            for shape in &walked.hours {
                let hour = shape.hour;
                let first = placement
                    .restarts
                    .iter()
                    .copied()
                    .chain([0])
                    .filter(|&first| inputs[first].hour <= hour)
                    .max()
                    .unwrap();
                // A segment's first stock read reads every holder.
                let start = inputs[first].hour;
                let first_read = start + (8 - start).rem_euclid(24);
                if hour % 24 != 8 || hour == first_read {
                    continue;
                }
                let expected: Vec<u32> = match (hour - 96) / 24 {
                    2 => vec![7, 8],
                    3 => vec![7, 9],
                    _ => vec![7],
                };
                assert_eq!(
                    shape.slots(),
                    expected,
                    "{}: read at {hour}",
                    placement.label
                );
            }
        }
    }
}

#[test]
fn a_projector_loaded_mid_segment_goes_on_as_the_one_saved() {
    let runs = [(1, campaign()), (1, later(140, 147))];
    let content = &contents()[1];
    for placement in placements(runs[0].1.len()) {
        let saved = check(&runs, content, &placement, Handover::Saved);
        let unsaved = check(
            &runs,
            content,
            &Placement {
                label: placement.label,
                block: placement.block,
                restarts: BTreeSet::new(),
            },
            Handover::Segment,
        );
        assert_payloads_equal(
            &saved.stores[1].payloads(),
            &unsaved.stores[1].payloads(),
            placement.label,
        );
    }
    let mut projector = columnar::Projector::new(FormatVersion::CURRENT).unwrap();
    assert!(projector.load(b"not a projector").is_err());
}

/// `count` holders (native references 100,000 and up), each holding two variants, making
/// one of them on a line, and with one logistics row; every hour reads them all.
fn counted(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = dashboard_base::test_fixtures::scale_frame(1, hour, count);
    *frame.tables[TableId::Stockpiles.index()] = TableView::Complete(TableRows::Stockpiles(
        (0..count)
            .flat_map(|index| {
                [10, 20]
                    .into_iter()
                    .zip(0..)
                    .map(move |(variant_handle, ordinal)| StockpileRow {
                        country: 100_000 + index,
                        ordinal,
                        variant_handle,
                        amount_raw: i64::from(index + 1) * 100_000,
                    })
            })
            .collect(),
    ));
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(
            (0..count)
                .map(|index| {
                    line(
                        u64::from(index) + 1,
                        100_000 + index,
                        10,
                        100_000,
                        50_000,
                        KIND_MILITARY,
                    )
                })
                .collect(),
        ));
    *frame.tables[TableId::LogisticsEquipment.index()] =
        TableView::Complete(TableRows::LogisticsEquipment(
            (0..count)
                .map(|index| logistics(100_000 + index, 1, 1, -5, 7))
                .collect(),
        ));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 1,
            literal: b"infantry_equipment_1".to_vec(),
        }]));
    frame.metrics.table_ns[TableId::Stockpiles.index()] = 1;
    frame.metrics.table_ns[TableId::LogisticsEquipment.index()] = 1;
    frame
}

#[test]
fn fifteen_hundred_countries_step_only_the_holders_whose_rows_changed() {
    let count = 1_500;
    let mut frame = counted(100, count);
    let mut inputs = Vec::new();
    for hour in 100..=106 {
        frame.hour = hour;
        match hour {
            102 => stock_rows(&mut frame)[2 * 700 + 1].amount_raw += 1,
            104 => line_rows(&mut frame)[1_200].speed_raw += 1,
            105 => logistics_rows(&mut frame)[3].balance_raw -= 1,
            _ => {}
        }
        inputs.push(frame.clone());
    }
    let placement = Placement {
        label: "1,500 countries",
        block: 4,
        restarts: BTreeSet::new(),
    };
    let walked = check(
        &[(1, inputs)],
        &contents()[1],
        &placement,
        Handover::Segment,
    );
    let shape = |hour: i32| {
        walked
            .hours
            .iter()
            .find(|shape| shape.hour == hour)
            .unwrap()
    };
    // The first hour reads every holder, whole; then only those whose rows changed.
    assert_eq!(shape(100).slots().len(), count as usize);
    assert!(shape(100).holders);
    assert_eq!(shape(100).whole(0, 0), Some(true));
    for hour in [101, 103, 106] {
        assert!(
            shape(hour).slots().is_empty(),
            "nothing changed before {hour}"
        );
    }
    assert_eq!(shape(102).slots(), vec![700]);
    assert_eq!(shape(102).whole(700, 0), Some(false));
    assert_eq!(shape(104).slots(), vec![1_200]);
    assert_eq!(shape(105).slots(), vec![3]);
    let emit = &walked.emits[&1];
    assert_eq!(emit.holders(), count as usize);
    assert_eq!(
        emit.steps(),
        count as usize + 3,
        "every holder steps at its first hour, then only the three whose rows changed"
    );
}
