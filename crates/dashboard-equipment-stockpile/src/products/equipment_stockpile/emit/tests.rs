//! The publication of synthetic walks: what it holds of every holder at every hour is what the
//! recording gives there, and it is the same publication however the walk is cut into
//! segments, published and resumed.

use super::*;
use crate::products::equipment_stockpile::columnar::{self, TABLES};
use dashboard_base::products::equipment::daily_units_raw;
use dashboard_series::{
    EventOrder, EventQuery, Identity, Publication, Selection, Snapshot,
    codec::Binary,
    family::Scalar,
    memory::{Memory, publish},
    query::EventView,
};
use engine_native::cequipment_production_line::{KIND_MILITARY, KIND_NAVAL};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator, Start},
};
use replay_data::{
    rows::{
        CountryLinkRow, CountryRow, LexicalTokenRow, LogisticsEquipmentRow, ProductionLineRow,
        Recorded, StockpileRow, TableRows, TableSchema, VariantDefinitionRow, tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::collections::{BTreeMap, BTreeSet};

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
pub(crate) fn frame(hour: i32) -> DecodedFrame {
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

macro_rules! rows {
    ($name:ident, $variant:ident, $row:ty) => {
        fn $name(frame: &mut DecodedFrame) -> &mut Vec<$row> {
            match &mut *frame.tables[TableId::$variant.index()] {
                TableView::Complete(TableRows::$variant(rows)) => rows,
                _ => unreachable!("the fixture's table is complete"),
            }
        }
    };
}

rows!(stock_rows, Stockpiles, StockpileRow);
rows!(line_rows, ProductionLines, ProductionLineRow);
rows!(logistics_rows, LogisticsEquipment, LogisticsEquipmentRow);
rows!(lexical_rows, LexicalTokens, LexicalTokenRow);
rows!(country_rows, Countries, CountryRow);
rows!(link_rows, CountryLinks, CountryLinkRow);
rows!(variant_rows, VariantDefinitions, VariantDefinitionRow);

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
pub(crate) fn campaign() -> Vec<DecodedFrame> {
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
            // Variant 10 changes type.
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
pub(crate) fn later(first: i32, last: i32) -> Vec<DecodedFrame> {
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

/// Thirty days read only in their daily slots: Germany's stock changes each day after its
/// read, Italy's line every fifth day, France's stock once, and Sweden's logistics appear on
/// the fourth day.
pub(crate) fn days() -> Vec<DecodedFrame> {
    let mut frame = frame(96);
    let mut inputs = Vec::new();
    for hour in 96..96 + 30 * 24 {
        frame.hour = hour;
        frame.metrics.table_ns.fill(0);
        let day = (hour - 96) / 24;
        if hour % 24 == 13 {
            stock_rows(&mut frame)[0].amount_raw += 1_000;
        }
        match (day, hour % 24) {
            (day, 15) if day % 5 == 1 => line_rows(&mut frame)[1].speed_raw += 5_000,
            (2, 20) => stock_rows(&mut frame)[3].amount_raw += 12,
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

/// Each branch's projected hours from `runs` (each of a branch, in order), a projector of its
/// own starting a segment at each ordinal `restarts` holds.
pub(crate) fn project(
    runs: &[(u64, Vec<DecodedFrame>)],
    restarts: &BTreeSet<usize>,
) -> BTreeMap<u64, Vec<columnar::Hour>> {
    let format = FormatVersion::CURRENT;
    let mut projected: BTreeMap<u64, Vec<columnar::Hour>> = BTreeMap::new();
    for (run, (branch, inputs)) in runs.iter().enumerate() {
        let branch = *branch;
        let continues = runs[..run].iter().any(|(earlier, _)| *earlier == branch);
        let hours = projected.entry(branch).or_default();
        let mut walk = Synthetic::new(TABLES, branch, inputs[0].hour).unwrap();
        let mut held = None;
        let mut projector = columnar::Projector::new(format).unwrap();
        projector
            .start(Start::Run { continues }, branch, inputs[0].hour)
            .unwrap();
        let mut first = 0;
        for (ordinal, input) in inputs.iter().enumerate() {
            if ordinal > 0 && restarts.contains(&ordinal) {
                projector.end(hours).unwrap();
                projector = columnar::Projector::new(format).unwrap();
                projector
                    .start(Start::Restart(walk.state()), branch, input.hour)
                    .unwrap();
                first = ordinal;
            }
            write_hour(&mut walk, input);
            walk.end(|state, manifest, frame, walk_starts| {
                let world = dashboard_world::World::at(
                    &mut held,
                    walk_starts,
                    &columnar::needs(),
                    (format, CaptureMode::Full),
                    state,
                    manifest,
                )
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
                        hours,
                    )
                    .unwrap();
            })
            .unwrap();
        }
        projector.end(hours).unwrap();
    }
    projected
}

pub(crate) fn identity() -> Identity {
    Identity {
        source: [1; 32],
        content: [2; 32],
        branch: [3; 32],
        schema: SCHEMA,
    }
}

fn content() -> Content {
    Content::resolved(&[
        ("infantry_equipment_1", "infantry_equipment", Domain::Land),
        ("destroyer_1", "destroyer", Domain::Naval),
    ])
}

/// A publication of `hours`, cut after each of `cuts`' hours; at each of `resume`'s, the emit
/// is saved and goes on from the publication, as a preparation of a grown recording does.
pub(crate) fn build(hours: Vec<columnar::Hour>, cuts: &[i32], resume: &[i32]) -> (Memory, u64) {
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(identity(), first).unwrap();
    let mut emit = Emit::new(FormatVersion::CURRENT, content());
    let mut root = 0;
    let mut hours = hours.into_iter().peekable();
    for (generation, &cut) in (1..).zip(cuts) {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        let publication = Publication {
            generation,
            segment: generation - 1,
            accepted: Span::new(first, cut).unwrap(),
            reached: cut,
            refusal: None,
        };
        root = publish(&mut writer, &mut pages, publication, Blob::EMPTY).unwrap();
        if resume.contains(&cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::load(FormatVersion::CURRENT, content(), &directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none(), "every hour is cut");
    (pages, root)
}

/// A stream's coverage at an hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Seen {
    Known(u64),
    Unknown(u64),
    Nothing,
}

pub(crate) type Stock = BTreeMap<u64, Option<i64>>;
pub(crate) type Lines = BTreeMap<u64, [Option<i64>; 4]>;
pub(crate) type Logistic = BTreeMap<u32, [Option<i64>; 3]>;

/// What a holder holds at an hour: each stream's coverage and, where it is known, its values
/// and the counts of its latest read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct State {
    pub(crate) coverage: [Seen; 3],
    pub(crate) stock: Option<Stock>,
    pub(crate) lines: Option<Lines>,
    pub(crate) logistics: Option<Logistic>,
    pub(crate) counts: [Option<u64>; 11],
}

const PARTS: [std::ops::Range<usize>; 3] = [0..1, 1..7, 7..11];

/// Every holder's state at every walked hour from its first resolution on, by tag and hour.
pub(crate) type States = BTreeMap<(String, i32), State>;

fn complete(frame: &DecodedFrame, table: TableId) -> bool {
    matches!(&*frame.tables[table.index()], TableView::Complete(_))
}

fn slot_of(frame: &DecodedFrame, raw: u32) -> Option<u32> {
    match &*frame.tables[TableId::CountryLinks.index()] {
        TableView::Complete(TableRows::CountryLinks(links)) => links
            .iter()
            .find(|link| link.id == raw)
            .map(|link| link.canonical_slot),
        _ => None,
    }
}

/// Each tag's first holder by id the links resolve, while the countries are complete.
fn identities(frame: &DecodedFrame) -> BTreeMap<String, (u32, u32)> {
    let mut identities = BTreeMap::new();
    if !complete(frame, TableId::CountryLinks) {
        return identities;
    }
    let TableView::Complete(TableRows::Countries(countries)) =
        &*frame.tables[TableId::Countries.index()]
    else {
        return identities;
    };
    let mut countries: Vec<&CountryRow> = countries.iter().collect();
    countries.sort_by_key(|country| country.id);
    for country in countries {
        let tag = String::from_utf8(country.tag.clone()).unwrap();
        if !identities.contains_key(&tag)
            && slot_of(frame, country.id) == Some(country.canonical_slot)
        {
            identities.insert(tag, (country.canonical_slot, country.id));
        }
    }
    identities
}

fn fits(sum: Option<i128>, overflowed: &mut u64) -> Option<i64> {
    sum.and_then(|sum| {
        let fits = i64::try_from(sum).ok();
        *overflowed += u64::from(fits.is_none());
        fits
    })
}

fn plus(sum: Option<i128>, value: Option<i64>) -> Option<i128> {
    sum.zip(value)
        .and_then(|(sum, value)| sum.checked_add(i128::from(value)))
}

/// A slot's totals of each numeric table, and the read's counts.
fn totals(frame: &DecodedFrame, slot: u32) -> (Stock, Lines, Logistic, [u64; 11]) {
    let mut counts = [0; 11];
    let own = |raw: u32| slot_of(frame, raw) == Some(slot);
    let mut stock: BTreeMap<u64, Option<i128>> = BTreeMap::new();
    if let TableView::Complete(TableRows::Stockpiles(rows)) =
        &*frame.tables[TableId::Stockpiles.index()]
    {
        for row in rows.iter().filter(|row| own(row.country)) {
            let sum = stock.entry(row.variant_handle).or_insert(Some(0));
            *sum = plus(*sum, Some(row.amount_raw));
        }
    }
    let stock = stock
        .into_iter()
        .map(|(variant, sum)| (variant, fits(sum, &mut counts[0])))
        .collect();
    let mut lines: BTreeMap<u64, (Option<i128>, Option<i128>, Option<Option<(i64, i64)>>)> =
        BTreeMap::new();
    if let TableView::Complete(TableRows::ProductionLines(rows)) =
        &*frame.tables[TableId::ProductionLines.index()]
    {
        for row in rows.iter().filter(|row| own(row.country)) {
            let military = row.kind == KIND_MILITARY;
            let units = if military {
                let units = daily_units_raw(row.speed_raw, row.cost_raw);
                counts[1] += u64::from(units.is_none());
                units
            } else {
                counts[2] += 1;
                None
            };
            let cost = (military && row.cost_raw > 0).then_some(row.cost_raw);
            let (daily, ic, costs) = lines.entry(row.variant).or_insert((Some(0), Some(0), None));
            *daily = plus(*daily, units);
            *ic = plus(*ic, military.then_some(row.speed_raw));
            *costs = Some(match *costs {
                Some(range) => range
                    .zip(cost)
                    .map(|((low, high), cost)| (low.min(cost), high.max(cost))),
                None => cost.map(|cost| (cost, cost)),
            });
        }
    }
    let lines = lines
        .into_iter()
        .map(|(variant, (daily, ic, costs))| {
            let range = costs.flatten();
            (
                variant,
                [
                    fits(daily, &mut counts[3]),
                    fits(ic, &mut counts[4]),
                    range.map(|range| range.0),
                    range.map(|range| range.1),
                ],
            )
        })
        .collect();
    let mut logistic: BTreeMap<u32, [Option<i128>; 3]> = BTreeMap::new();
    if let TableView::Complete(TableRows::LogisticsEquipment(rows)) =
        &*frame.tables[TableId::LogisticsEquipment.index()]
    {
        for row in rows.iter().filter(|row| own(row.country)) {
            let available = match row.available_raw {
                Recorded::Value(value) => value,
                Recorded::NotRecorded => None,
            };
            counts[7] += u64::from(available.is_none());
            let sums = logistic.entry(row.token).or_insert([Some(0); 3]);
            sums[0] = plus(sums[0], Some(row.balance_raw));
            sums[1] = plus(sums[1], Some(row.stock_raw));
            sums[2] = plus(sums[2], available);
        }
    }
    let logistic = logistic
        .into_iter()
        .map(|(token, [balance, stock, available])| {
            (
                token,
                [
                    fits(balance, &mut counts[8]),
                    fits(stock, &mut counts[9]),
                    fits(available, &mut counts[10]),
                ],
            )
        })
        .collect();
    (stock, lines, logistic, counts)
}

/// What the recording gives each holder of `branch` at every hour `runs` walk of it: a
/// stream is known from a read of its usable table by its resolved holder until either stops
/// or an hour is not walked, and holds that read's totals until the next.
pub(crate) fn expected(runs: &[(u64, Vec<DecodedFrame>)], branch: u64) -> States {
    let format = FormatVersion::CURRENT;
    let mut states = States::new();
    let mut last: Option<i32> = None;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut held: BTreeMap<String, State> = BTreeMap::new();
    for frame in runs
        .iter()
        .filter(|(held, _)| *held == branch)
        .flat_map(|(_, frames)| frames)
    {
        let hour = frame.hour;
        if last.and_then(|last| last.checked_add(1)) != Some(hour) {
            for state in held.values_mut() {
                state.coverage = [Seen::Nothing; 3];
            }
        }
        last = Some(hour);
        let countries =
            complete(frame, TableId::Countries) && complete(frame, TableId::CountryLinks);
        let identities = identities(frame);
        let usable = [
            TableId::Stockpiles,
            TableId::ProductionLines,
            TableId::LogisticsEquipment,
        ]
        .map(|table| countries && complete(frame, table));
        let read = |table| dashboard_text::metadata::slot_read(format, &frame.metrics, table, hour);
        let reads = [
            read(TableId::Stockpiles),
            read(TableId::Stockpiles),
            read(TableId::LogisticsEquipment),
        ];
        seen.extend(identities.keys().cloned());
        for tag in &seen {
            let state = held.entry(tag.clone()).or_insert_with(|| State {
                coverage: [Seen::Nothing; 3],
                stock: None,
                lines: None,
                logistics: None,
                counts: [None; 11],
            });
            let identity = identities.get(tag).copied();
            let read_now = identity.map(|(slot, _)| totals(frame, slot));
            for stream in 0..3 {
                let status = match identity {
                    _ if !countries => Seen::Unknown(reason::COUNTRIES),
                    None => Seen::Unknown(reason::UNRESOLVED),
                    Some(_) if !usable[stream] => Seen::Unknown(reason::TABLE),
                    Some(identity) => {
                        let known = Seen::Known(packed(identity));
                        if state.coverage[stream] == known || reads[stream] {
                            known
                        } else {
                            Seen::Unknown(reason::UNREAD)
                        }
                    }
                };
                state.coverage[stream] = status;
                if !matches!(status, Seen::Known(_)) {
                    match stream {
                        0 => state.stock = None,
                        1 => state.lines = None,
                        _ => state.logistics = None,
                    }
                    for at in PARTS[stream].clone() {
                        state.counts[at] = None;
                    }
                    continue;
                }
                if reads[stream] {
                    let (stock, lines, logistic, counts) = read_now.clone().unwrap();
                    match stream {
                        0 => state.stock = Some(stock),
                        1 => state.lines = Some(lines),
                        _ => state.logistics = Some(logistic),
                    }
                    for at in PARTS[stream].clone() {
                        state.counts[at] = Some(counts[at]);
                    }
                }
            }
            states.insert((tag.clone(), hour), state.clone());
        }
    }
    states
}

/// A decoded event of a key.
#[derive(Clone, Debug, PartialEq)]
struct Decoded {
    hour: i32,
    read: Option<i32>,
    kind: u64,
    cells: Vec<Scalar>,
}

fn scalar(snapshot: &mut Snapshot<'_, Memory>, cell: Cell) -> Scalar {
    match cell {
        Cell::Null => Scalar::Null,
        Cell::Integer(value) => Scalar::Integer(value),
        Cell::Unsigned(value) => Scalar::Unsigned(value),
        Cell::Number(value) => Scalar::Number(value),
        Cell::Boolean(value) => Scalar::Boolean(value),
        Cell::Text(blob) => {
            let mut bytes = vec![0; blob.len as usize];
            if blob.len != 0 {
                snapshot.cell_bytes(blob, 0, &mut bytes).unwrap();
            }
            Scalar::Text(bytes.into_boxed_slice())
        }
        Cell::List(_) => unreachable!("the stockpile writes no list"),
    }
}

/// Every event of `key`, in order, its fields read whole as a reader reads them.
fn events(snapshot: &mut Snapshot<'_, Memory>, key: KeyHandle, span: Span) -> Vec<Decoded> {
    let mut raw = Vec::new();
    snapshot
        .each_event(
            EventQuery {
                selection: &[Selection {
                    key,
                    membership: None,
                }],
                native: span,
                as_of: None,
                order: EventOrder::Native,
            },
            1 << 20,
            |event| {
                let EventView::Family(event) = event else {
                    panic!("the stockpile presents no evidence");
                };
                raw.push(event);
                Ok(())
            },
        )
        .unwrap();
    raw.sort_by_key(|event| (event.native_hour, event.sequence));
    raw.into_iter()
        .map(|event| {
            let mut bytes = vec![0; event.fields.len as usize];
            if event.fields.len != 0 {
                snapshot.cell_bytes(event.fields, 0, &mut bytes).unwrap();
            }
            let cells = bytes
                .chunks_exact(Cell::SIZE)
                .map(|chunk| Cell::decode(&mut Decode::new(chunk)).unwrap())
                .collect::<Vec<_>>();
            Decoded {
                hour: event.native_hour,
                read: event.read_hour,
                kind: event.kind,
                cells: cells
                    .into_iter()
                    .map(|cell| scalar(snapshot, cell))
                    .collect(),
            }
        })
        .collect()
}

fn evidence(snapshot: &mut Snapshot<'_, Memory>, key: KeyHandle, span: Span) -> Vec<Evidence> {
    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot
            .evidence_page(key, span, cursor, 1 << 16, |row| {
                rows.push(row);
                Ok(())
            })
            .unwrap();
        cursor = page.next;
        if cursor.is_none() {
            return rows;
        }
    }
}

/// A key's runs over `span` as a reader reads them: each run's hours, value and whether it
/// starts with a break.
fn runs(
    snapshot: &mut Snapshot<'_, Memory>,
    key: KeyHandle,
    span: Span,
) -> Vec<(Span, i128, bool)> {
    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot
            .run_page(
                Selection {
                    key,
                    membership: None,
                },
                span,
                cursor,
                1 << 16,
                |run| {
                    rows.push((run.span, run.value, run.break_before));
                    Ok(())
                },
            )
            .unwrap();
        cursor = page.next;
        if cursor.is_none() {
            return rows;
        }
    }
}

fn amount_of(cell: &Scalar) -> Option<i64> {
    match cell {
        Scalar::Null => None,
        Scalar::Integer(value) => Some(i64::try_from(*value).unwrap()),
        other => panic!("not an amount: {other:?}"),
    }
}

fn unsigned(cell: &Scalar) -> u64 {
    match cell {
        Scalar::Unsigned(value) => *value,
        other => panic!("not unsigned: {other:?}"),
    }
}

fn boolean(cell: &Scalar) -> bool {
    match cell {
        Scalar::Boolean(value) => *value,
        other => panic!("not a flag: {other:?}"),
    }
}

/// A stream's rows after each of its events, checking its chains: every delta names its
/// chain's keyframe, and no chain is longer than [`CHAIN`].
fn replay_chain<R: Clone>(
    events: &[Decoded],
    width: usize,
    row: impl Fn(&[Scalar]) -> (u64, Option<R>),
) -> Vec<(i32, BTreeMap<u64, R>)> {
    let mut state = BTreeMap::new();
    let mut chain: Option<(i32, u8)> = None;
    let mut after = Vec::new();
    for event in events {
        assert_eq!(
            event.read,
            Some(event.hour),
            "an event is dated by its read"
        );
        let header = match event.kind {
            kind::KEYFRAME => {
                state.clear();
                chain = Some((event.hour, 1));
                2
            }
            kind::DELTA => {
                let (first, length) = chain.expect("a delta follows its keyframe");
                assert_eq!(
                    event.cells[0],
                    Scalar::Integer(i128::from(first)),
                    "a delta names its chain's keyframe"
                );
                assert!(length < CHAIN, "a chain holds {CHAIN} events at most");
                chain = Some((first, length + 1));
                1
            }
            other => panic!("unexpected event kind {other}"),
        };
        let rows = &event.cells[header..];
        assert_eq!(rows.len() % width, 0, "rows are {width} cells each");
        for cells in rows.chunks_exact(width) {
            let (key, value) = row(cells);
            match value {
                Some(value) => {
                    state.insert(key, value);
                }
                None => {
                    assert_eq!(event.kind, kind::DELTA, "a keyframe lists present rows");
                    assert!(state.remove(&key).is_some(), "a delta removes a held row");
                }
            }
        }
        after.push((event.hour, state.clone()));
    }
    after
}

type DailyRow = (Option<Option<i64>>, Option<[Option<i64>; 4]>);

fn daily_row(cells: &[Scalar]) -> (u64, Option<DailyRow>) {
    let stock = boolean(&cells[2]).then(|| amount_of(&cells[1]));
    let lines = boolean(&cells[3]).then(|| [4, 5, 6, 7].map(|at| amount_of(&cells[at])));
    if stock.is_none() {
        assert_eq!(cells[1], Scalar::Null, "an absent stock is null");
    }
    let present = stock.is_some() || lines.is_some();
    (unsigned(&cells[0]), present.then_some((stock, lines)))
}

fn logistics_row(cells: &[Scalar]) -> (u64, Option<[Option<i64>; 3]>) {
    let values = [1, 2, 3].map(|at| amount_of(&cells[at]));
    let present = boolean(&cells[4]);
    if !present {
        assert_eq!(values, [None; 3], "an absent row is null");
    }
    (unsigned(&cells[0]), present.then_some(values))
}

/// The state after the last of `after` at or before `hour`.
fn at<R: Clone>(after: &[(i32, BTreeMap<u64, R>)], hour: i32) -> BTreeMap<u64, R> {
    after
        .iter()
        .rev()
        .find(|(at, _)| *at <= hour)
        .map(|(_, state)| state.clone())
        .unwrap_or_default()
}

/// The publication read back.
struct Published {
    states: States,
    /// Every key's events, evidence and runs, by its place.
    dump: BTreeMap<(u64, u64, Option<u64>), (Vec<Decoded>, Vec<Evidence>, Vec<(Span, i128, bool)>)>,
    /// Each tag's daily and logistics events.
    streams: BTreeMap<String, [Vec<Decoded>; 2]>,
}

/// What the publication at `root` holds of every holder at every one of `hours`.
fn published(pages: &Memory, root: u64, hours: &[i32]) -> Published {
    let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let span = Span::new(hours[0], *hours.last().unwrap()).unwrap();
    let mut dump = BTreeMap::new();
    let mut places = Vec::new();
    for actor in (0..directory.actors.len() as u64).chain([FAMILY]) {
        for measure in 0..6 {
            if let Some(key) = directory.key(actor, measure, None) {
                places.push(((actor, measure, None), key));
            }
        }
    }
    for measure in [NAMES, CLASSES] {
        for &(variant, key) in directory.dimensions(FAMILY, measure) {
            places.push(((FAMILY, measure, Some(variant)), key));
        }
    }
    for (place, key) in places {
        let held = (
            events(&mut snapshot, key, span),
            evidence(&mut snapshot, key, span),
            runs(&mut snapshot, key, span),
        );
        dump.insert(place, held);
    }
    let mut states = States::new();
    let mut streams = BTreeMap::new();
    for (actor, named) in directory.actors.iter().enumerate() {
        let tag = String::from_utf8(named.name.to_vec()).unwrap();
        let actor = actor as u64;
        let held = |measure: u64| &dump[&(actor, measure, None)];
        let daily = replay_chain(&held(DAILY).0, DAILY_CELLS, daily_row);
        let logistic = replay_chain(&held(LOGISTICS).0, LOGISTICS_CELLS, logistics_row);
        let counted: Vec<(i32, [u64; 11])> = held(COUNTS)
            .0
            .iter()
            .map(|event| {
                assert_eq!(event.kind, kind::COUNTS);
                let counts: Vec<u64> = event.cells.iter().map(unsigned).collect();
                (event.hour, counts.try_into().unwrap())
            })
            .collect();
        for &hour in hours.iter().filter(|&&hour| hour >= named.first_hour) {
            let coverage = [STOCK_HOURS, LINES_HOURS, LOGISTICS_HOURS].map(|measure| {
                held(measure)
                    .1
                    .iter()
                    .find(|row| row.span.contains(hour))
                    .map_or(Seen::Nothing, |row| match row.known {
                        Knownness::Known => Seen::Known(row.holder.unwrap()),
                        Knownness::Unknown(reason) => {
                            assert!(row.holder.is_none());
                            Seen::Unknown(reason)
                        }
                    })
            });
            let known = coverage.map(|seen| matches!(seen, Seen::Known(_)));
            let rows = at(&daily, hour);
            let counts = counted
                .iter()
                .rev()
                .find(|(at, _)| *at <= hour)
                .map_or([0; 11], |(_, counts)| *counts);
            let mut masked = [None; 11];
            for (stream, part) in PARTS.iter().enumerate() {
                if known[stream] {
                    for at in part.clone() {
                        masked[at] = Some(counts[at]);
                    }
                }
            }
            states.insert(
                (tag.clone(), hour),
                State {
                    coverage,
                    stock: known[0].then(|| {
                        rows.iter()
                            .filter_map(|(&variant, (stock, _))| {
                                stock.map(|stock| (variant, stock))
                            })
                            .collect()
                    }),
                    lines: known[1].then(|| {
                        rows.iter()
                            .filter_map(|(&variant, (_, lines))| {
                                lines.map(|lines| (variant, lines))
                            })
                            .collect()
                    }),
                    logistics: known[2].then(|| {
                        at(&logistic, hour)
                            .into_iter()
                            .map(|(token, values)| (token as u32, values))
                            .collect()
                    }),
                    counts: masked,
                },
            );
        }
        streams.insert(tag, [held(DAILY).0.clone(), held(LOGISTICS).0.clone()]);
    }
    Published {
        states,
        dump,
        streams,
    }
}

/// Each branch's walked hours.
pub(crate) fn walked(runs: &[(u64, Vec<DecodedFrame>)], branch: u64) -> Vec<i32> {
    runs.iter()
        .filter(|(held, _)| *held == branch)
        .flat_map(|(_, frames)| frames.iter().map(|frame| frame.hour))
        .collect()
}

/// The ordinals at which each placement starts a segment, and the hours it cuts after.
struct Placement {
    label: &'static str,
    restarts: BTreeSet<usize>,
    /// Every how many walked hours it cuts, and the cuts it resumes at.
    every: usize,
    resume: bool,
}

fn placements(hours: usize) -> Vec<Placement> {
    vec![
        Placement {
            label: "one segment, one cut",
            restarts: BTreeSet::new(),
            every: usize::MAX,
            resume: false,
        },
        Placement {
            label: "segments every sixth hour, cuts every eighth",
            restarts: (1..hours).filter(|ordinal| ordinal % 6 == 0).collect(),
            every: 8,
            resume: false,
        },
        Placement {
            label: "a segment at every hour, cuts every fifth, resumed at each",
            restarts: (1..hours).collect(),
            every: 5,
            resume: true,
        },
        Placement {
            label: "segments at odd places, cuts every third, resumed at each",
            restarts: [1, 5, 7, 13, 22, 23, 30].into(),
            every: 3,
            resume: true,
        },
    ]
}

/// `branch`'s publication from `runs` as `placement` walks and cuts it.
fn publication(runs: &[(u64, Vec<DecodedFrame>)], branch: u64, placement: &Placement) -> Published {
    let hours = walked(runs, branch);
    let mut cuts: Vec<i32> = hours
        .iter()
        .copied()
        .enumerate()
        .filter(|&(at, _)| {
            placement.every != usize::MAX && at % placement.every == placement.every - 1
        })
        .map(|(_, hour)| hour)
        .collect();
    if cuts.last() != hours.last() {
        cuts.push(*hours.last().unwrap());
    }
    let resume: Vec<i32> = if placement.resume {
        cuts.clone()
    } else {
        Vec::new()
    };
    let mut projected = project(runs, &placement.restarts);
    let (pages, root) = build(projected.remove(&branch).unwrap(), &cuts, &resume);
    published(&pages, root, &hours)
}

pub(crate) fn campaign_runs() -> Vec<(u64, Vec<DecodedFrame>)> {
    vec![(1, campaign()), (1, later(140, 147)), (2, later(100, 105))]
}

#[test]
fn every_holder_holds_what_the_recording_gives_at_every_hour_however_it_is_walked() {
    let runs = campaign_runs();
    for branch in [1, 2] {
        let expected = expected(&runs, branch);
        let mut first: Option<Published> = None;
        for placement in placements(runs[0].1.len()) {
            let published = publication(&runs, branch, &placement);
            let label = format!("branch {branch}, {}", placement.label);
            for (at, state) in &expected {
                assert_eq!(published.states.get(at), Some(state), "{label}: {at:?}");
            }
            assert_eq!(
                published.states.len(),
                expected.len(),
                "{label}: no holder the recording does not give"
            );
            // The same events, evidence and runs, wherever segments start, cuts fall or the
            // emit resumes.
            match &first {
                None => first = Some(published),
                Some(first) => assert!(first.dump == published.dump, "{label}"),
            }
        }
    }
}

#[test]
fn the_campaign_publishes_its_reads_coverage_and_counts_as_they_happen() {
    let runs = campaign_runs();
    let states = expected(&runs, 1);
    let state = |tag: &str, hour: i32| &states[&(tag.to_owned(), hour)];
    // Read at 100; changed at 101 and read again at 104.
    assert_eq!(
        state("GER", 102).stock.as_ref().unwrap()[&10],
        Some(100_000)
    );
    assert_eq!(
        state("GER", 104).stock.as_ref().unwrap()[&10],
        Some(100_001)
    );
    // The stock table missing at 115 ends its coverage; the next read at 116 starts it.
    assert_eq!(state("GER", 115).coverage[0], Seen::Unknown(reason::TABLE));
    assert_eq!(
        state("GER", 115).coverage[1],
        Seen::Known(packed((7, 9001)))
    );
    assert!(matches!(state("GER", 116).coverage[0], Seen::Known(_)));
    // A total past i64 is unknown, never clamped, and counted.
    assert_eq!(state("GER", 128).stock.as_ref().unwrap()[&10], None);
    assert_eq!(state("GER", 128).counts[0], Some(1));
    // Italy is AI-controlled from its read at 114: known, with no row.
    assert_eq!(state("ITA", 114).logistics, Some(BTreeMap::new()));
    // A costless line and a naval line are counted, their totals unknown.
    assert_eq!(state("GER", 100).counts[1], Some(1));
    assert_eq!(state("FRA", 100).lines.as_ref().unwrap()[&30], [None; 4]);
    assert_eq!(state("FRA", 100).counts[2], Some(1));
    // Countries missing at 127 leave every holder unknown; the next read resumes them.
    assert_eq!(
        state("GER", 127).coverage[0],
        Seen::Unknown(reason::COUNTRIES)
    );
    assert_eq!(
        state("GER", 127).coverage[2],
        Seen::Unknown(reason::COUNTRIES)
    );
    assert_eq!(state("GER", 128).coverage[2], Seen::Unknown(reason::UNREAD));
    // Sweden's tag goes at 117: NOR holds it from there.
    assert_eq!(
        state("SWE", 117).coverage[0],
        Seen::Unknown(reason::UNRESOLVED)
    );
    assert_eq!(state("NOR", 117).coverage[0], Seen::Unknown(reason::UNREAD));
    // Nothing goes on over the gap; the later run reads afresh.
    assert!(!states.contains_key(&("GER".to_owned(), 137)));
    assert_eq!(
        state("GER", 140).stock.as_ref().unwrap()[&10],
        Some(100_000)
    );
    assert_eq!(
        state("GER", 144).stock.as_ref().unwrap()[&10],
        Some(100_007)
    );
}

/// The hours a stream must write an event at: a read starting any of its coverages, or
/// changing what a coverage known before and after holds.
fn event_hours(states: &States, tag: &str, hours: &[i32], stream: usize) -> Vec<i32> {
    let parts: &[usize] = if stream == 0 { &[0, 1] } else { &[2] };
    let mut due = Vec::new();
    for (index, &hour) in hours.iter().enumerate() {
        let Some(now) = states.get(&(tag.to_owned(), hour)) else {
            continue;
        };
        let before = index
            .checked_sub(1)
            .filter(|&before| hours[before] + 1 == hour)
            .and_then(|before| states.get(&(tag.to_owned(), hours[before])));
        let mut writes = false;
        for &part in parts {
            if !matches!(now.coverage[part], Seen::Known(_)) {
                continue;
            }
            let continues =
                before.is_some_and(|before| before.coverage[part] == now.coverage[part]);
            let changed = before.is_some_and(|before| match part {
                0 => before.stock != now.stock,
                1 => before.lines != now.lines,
                _ => before.logistics != now.logistics,
            });
            writes |= !continues || changed;
        }
        if writes {
            due.push(hour);
        }
    }
    due
}

#[test]
fn a_read_writes_an_event_only_where_it_starts_or_changes_a_stream() {
    for runs in [campaign_runs(), vec![(1, days())]] {
        let hours = walked(&runs, 1);
        let states = expected(&runs, 1);
        let placement = Placement {
            label: "segments every day and a quarter",
            restarts: (30..hours.len()).step_by(30).collect(),
            every: 24,
            resume: true,
        };
        let published = publication(&runs, 1, &placement);
        for (tag, streams) in &published.streams {
            for (stream, events) in streams.iter().enumerate() {
                let written: Vec<i32> = events.iter().map(|event| event.hour).collect();
                assert_eq!(
                    written,
                    event_hours(&states, tag, &hours, stream),
                    "{tag} stream {stream}"
                );
                // A coverage starting writes a keyframe.
                for event in events {
                    let now = &states[&(tag.clone(), event.hour)];
                    let before = states.get(&(tag.clone(), event.hour - 1));
                    let parts: &[usize] = if stream == 0 { &[0, 1] } else { &[2] };
                    let starts = parts.iter().any(|&part| {
                        matches!(now.coverage[part], Seen::Known(_))
                            && before
                                .is_none_or(|before| before.coverage[part] != now.coverage[part])
                    });
                    if starts {
                        assert_eq!(event.kind, kind::KEYFRAME, "{tag} at {}", event.hour);
                    }
                }
            }
        }
    }
    // Thirty days of Germany's daily changes make chains of sixteen.
    let runs = vec![(1, days())];
    let published = publication(
        &runs,
        1,
        &Placement {
            label: "one segment",
            restarts: BTreeSet::new(),
            every: usize::MAX,
            resume: false,
        },
    );
    let daily = &published.streams["GER"][0];
    let keyframes: Vec<usize> = daily
        .iter()
        .enumerate()
        .filter(|(_, event)| event.kind == kind::KEYFRAME)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(keyframes, [0, 16], "a keyframe every sixteen events");
    // Sweden holds nothing: one empty keyframe, and nothing at the reads after.
    assert_eq!(published.streams["SWE"][0].len(), 1);
    assert_eq!(published.streams["SWE"][0][0].cells.len(), 2);
}

#[test]
fn the_read_schedules_date_every_read_and_no_other_hour() {
    let runs = campaign_runs();
    let published = publication(&runs, 1, &placements(runs[0].1.len())[2]);
    let format = FormatVersion::CURRENT;
    for (stream, table) in [
        (DAILY_READS, TableId::Stockpiles),
        (LOGISTICS_READS, TableId::LogisticsEquipment),
    ] {
        let runs_of = &published.dump[&(FAMILY, stream, None)].2;
        for frame in runs
            .iter()
            .filter(|(branch, _)| *branch == 1)
            .flat_map(|(_, frames)| frames)
        {
            let hour = frame.hour;
            let read = dashboard_text::metadata::slot_read(format, &frame.metrics, table, hour);
            let scheduled = runs_of
                .iter()
                .find(|(span, _, _)| span.contains(hour))
                .is_some_and(|&(span, value, break_before)| {
                    let period = i32::try_from(value).unwrap();
                    assert!(break_before, "every schedule starts with a break");
                    hour == span.first || (period > 0 && (hour - span.first) % period == 0)
                });
            assert_eq!(scheduled, read, "{table:?} at {hour}");
        }
    }
}

#[test]
fn names_classes_and_spellings_are_written_where_they_change() {
    let runs = campaign_runs();
    let published = publication(&runs, 1, &placements(runs[0].1.len())[1]);
    let names = |variant: u64| -> Vec<(i32, serde_json::Value)> {
        published.dump[&(FAMILY, NAMES, Some(variant))]
            .0
            .iter()
            .map(|event| {
                assert_eq!(event.kind, kind::NAME);
                let Scalar::Text(json) = &event.cells[0] else {
                    panic!("a name is JSON text");
                };
                (event.hour, serde_json::from_slice(json).unwrap())
            })
            .collect()
    };
    let forgets: Vec<i32> = published.dump[&(FAMILY, FORGETS, None)]
        .0
        .iter()
        .map(|event| {
            assert_eq!(event.kind, kind::FORGET);
            assert!(event.cells.is_empty());
            event.hour
        })
        .collect();
    // The variant table missing at 124, and the hour after the run's last.
    assert_eq!(forgets, [124, 136]);
    for variant in [10, 20, 30] {
        let names = names(variant);
        assert_eq!(
            names[0].0, 100,
            "variant {variant} is named at the first read"
        );
        // A name repeats its operands only after a forget.
        for pair in names.windows(2) {
            let [(before, was), (hour, now)] = pair else {
                unreachable!()
            };
            let mut same = was.clone();
            same["first_hour"] = now["first_hour"].clone();
            let forgotten = forgets
                .iter()
                .any(|&forget| *before < forget && forget <= *hour);
            assert!(forgotten || same != *now, "variant {variant} at {hour}");
            assert_eq!(now["first_hour"], *hour);
        }
    }
    let renamed = names(20)
        .into_iter()
        .find(|(_, raw)| raw["display_name"] == "Gewehr 43/1")
        .expect("the rename is read");
    assert!((108..=110).contains(&renamed.0), "{renamed:?}");
    let classes = |variant: u64| -> Vec<Vec<Scalar>> {
        published.dump[&(FAMILY, CLASSES, Some(variant))]
            .0
            .iter()
            .map(|event| {
                assert_eq!(event.kind, kind::CLASS);
                event.cells.clone()
            })
            .collect()
    };
    let text = |text: &str| Scalar::Text(text.as_bytes().into());
    let infantry = vec![
        text("infantry_equipment_1"),
        text("infantry_equipment"),
        Scalar::Unsigned(0),
    ];
    // Variant 10 changes type at 114; the later run's frames give it its first type again.
    assert_eq!(
        classes(10),
        [
            infantry.clone(),
            vec![text("destroyer_1"), text("destroyer"), Scalar::Unsigned(2)],
            infantry.clone(),
        ]
    );
    // Its token's spelling changes at 123, to one content does not resolve.
    assert_eq!(
        classes(20),
        [
            infantry.clone(),
            vec![text("infantry_equipment_2"), Scalar::Null, Scalar::Null],
            infantry.clone(),
        ]
    );
    let spelled = &published.dump[&(FAMILY, TOKENS, None)].0;
    let tokens: BTreeMap<u64, Vec<Scalar>> = spelled
        .iter()
        .map(|event| {
            assert_eq!(event.kind, kind::TOKEN);
            (unsigned(&event.cells[0]), event.cells[1..].to_vec())
        })
        .collect();
    assert_eq!(
        tokens[&1],
        [
            text("infantry_equipment_1"),
            text("infantry_equipment"),
            Scalar::Unsigned(0)
        ]
    );
    assert_eq!(tokens[&3], [text("truck_1"), Scalar::Null, Scalar::Null]);
    assert_eq!(tokens.len(), spelled.len(), "each token is spelled once");
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
fn fifteen_hundred_countries_write_events_only_for_the_holders_whose_rows_changed() {
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
    let runs = vec![(1, inputs)];
    let mut projected = project(&runs, &BTreeSet::new());
    let hours = projected.remove(&1).unwrap();
    let shapes: Vec<columnar::Shape> = hours.iter().map(columnar::Hour::shape).collect();
    // The first hour reads every holder, whole; then only those whose rows changed.
    assert_eq!(shapes[0].slots().len(), count as usize);
    assert_eq!(shapes[0].whole(0, 0), Some(true));
    assert_eq!(shapes[2].slots(), [700]);
    assert_eq!(shapes[2].whole(700, 0), Some(false));
    assert_eq!(shapes[4].slots(), [1_200]);
    assert_eq!(shapes[5].slots(), [3]);
    let (pages, root) = build(hours, &[103, 106], &[103]);
    let published = published(&pages, root, &walked(&runs, 1));
    let written = |stream: usize| -> BTreeMap<i32, usize> {
        let mut written = BTreeMap::new();
        for streams in published.streams.values() {
            for event in &streams[stream] {
                *written.entry(event.hour).or_default() += 1;
            }
        }
        written
    };
    assert_eq!(
        written(0),
        BTreeMap::from([(100, count as usize), (102, 1), (104, 1)])
    );
    assert_eq!(
        written(1),
        BTreeMap::from([(100, count as usize), (105, 1)])
    );
    assert_eq!(published.states, expected(&runs, 1));
}

#[test]
fn a_damaged_saved_emit_is_refused() {
    let runs = vec![(1, later(100, 103))];
    let mut projected = project(&runs, &BTreeSet::new());
    let mut pages = Memory::default();
    let mut writer = Writer::new(identity(), 100).unwrap();
    let mut emit = Emit::new(FormatVersion::CURRENT, content());
    for hour in projected.remove(&1).unwrap() {
        emit.hour(&mut writer, hour).unwrap();
    }
    emit.cut(&mut writer, 103).unwrap();
    let publication = Publication {
        generation: 1,
        segment: 0,
        accepted: Span::new(100, 103).unwrap(),
        reached: 103,
        refusal: None,
    };
    let root = publish(&mut writer, &mut pages, publication, Blob::EMPTY).unwrap();
    let mut saved = Vec::new();
    emit.save(&mut saved).unwrap();
    let mut snapshot = Snapshot::open(pages, root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    assert!(Emit::load(FormatVersion::CURRENT, content(), &directory, &saved).is_ok());
    for cut in [1, saved.len() / 2, saved.len() - 1] {
        assert!(
            Emit::load(FormatVersion::CURRENT, content(), &directory, &saved[..cut]).is_err(),
            "cut at {cut}"
        );
    }
}
