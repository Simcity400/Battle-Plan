
use dashboard_base::products::Country;
use dashboard_base::{Source, span::HourSpan};
use dashboard_query::{Query, identity::Holder};
use replay_lanes::{lanes::{self, Keys}, points::Cell};
use replay_data::{cadence::STAGGER_PERIOD, rows::TableId};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, PoisonError},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stock {
    Absent,
    Unknown,
    Amount(i64),
}

impl Stock {
    pub fn raw(self) -> Option<i128> {
        match self {
            Self::Absent => Some(0),
            Self::Unknown => None,
            Self::Amount(amount) => Some(i128::from(amount)),
        }
    }

    pub fn holds(self) -> bool {
        matches!(self, Self::Unknown) || matches!(self, Self::Amount(amount) if amount != 0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Logistics {
    pub balance: Option<i64>,
    pub stock: Option<i64>,
    pub available: Option<i64>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Replay {
    pub stock: Vec<HourSpan>,
    pub lines: Vec<HourSpan>,
    pub balance: Vec<HourSpan>,
    pub ai: Vec<HourSpan>,
    pub reads: Vec<i32>,
    pub line_reads: Vec<i32>,
    pub logistics_reads: Vec<i32>,
    pub variants: BTreeMap<u64, Vec<(i32, Stock)>>,
    pub logistics: BTreeMap<u32, Vec<(i32, Option<Logistics>)>>,
    pub produced: BTreeSet<u64>,
    pub unresolved: Vec<HourSpan>,
}

fn read_hours(reads: &[(i32, i32)]) -> Vec<i32> {
    reads.iter().flat_map(|&(first, last)| first..=last).collect()
}

pub fn carried(reads: &[i32], last: i32) -> Vec<HourSpan> {
    let mut spans: Vec<HourSpan> = Vec::new();
    for (at, &read) in reads.iter().enumerate() {
        let end = reads.get(at + 1).map_or(last, |next| next - 1).min(read.saturating_add(STAGGER_PERIOD - 1)).min(last);
        match spans.last_mut() {
            Some(span) if span.last_hour + 1 == read => span.last_hour = end,
            _ => spans.push(HourSpan { first_hour: read, last_hour: end }),
        }
    }
    spans
}

pub fn split_logistics(reads: &[i32], held: &[(i32, bool)], last: i32) -> (Vec<HourSpan>, Vec<HourSpan>) {
    let (mut with, mut without) = (Vec::new(), Vec::new());
    for (at, &read) in reads.iter().enumerate() {
        let index = held.partition_point(|(hour, _)| *hour <= read);
        let any = index.checked_sub(1).is_some_and(|index| held[index].1);
        let end = reads.get(at + 1).map_or(last, |next| next - 1).min(read.saturating_add(STAGGER_PERIOD - 1)).min(last);
        let spans: &mut Vec<HourSpan> = if any { &mut with } else { &mut without };
        match spans.last_mut() {
            Some(span) if span.last_hour + 1 == read => span.last_hour = end,
            _ => spans.push(HourSpan { first_hour: read, last_hour: end }),
        }
    }
    (with, without)
}

fn number(cell: Option<&Cell>) -> Option<i64> {
    match cell {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

fn note<T: PartialEq>(changes: &mut Vec<(i32, T)>, hour: i32, value: T) {
    if changes.last().is_none_or(|(_, last)| *last != value) {
        changes.push((hour, value));
    }
}

fn reads_of(query: &Query<'_>, run: u32, table: TableId, first: i32, last: i32) -> io::Result<Vec<i32>> {
    let recording = query.replay.recording();
    Ok(read_hours(&dashboard_query::cost::storage(|| lanes::reads(recording, run, table, first, last))?))
}

pub fn replay(source: &Source, query: &Query<'_>, tag: &Country) -> io::Result<Replay> {
    let recording = query.replay.recording();
    let field = |table, name| dashboard_declare::field(table, name);
    let amount = field(TableId::StockByVariant, "amount")?;
    let logistics_columns = [
        field(TableId::LogisticsEquipment, "token")?,
        field(TableId::LogisticsEquipment, "balance_raw")?,
        field(TableId::LogisticsEquipment, "stock_raw")?,
        field(TableId::LogisticsEquipment, "available_raw")?,
    ];
    let (variant, kind) = (field(TableId::ProductionLines, "variant")?, field(TableId::ProductionLines, "kind")?);
    let mut replay = Replay::default();
    let mut logistics_held: Vec<(i32, bool)> = Vec::new();
    let (mut reads, mut line_reads, mut logistics_reads) = (Vec::new(), Vec::new(), Vec::new());
    for run in query.holders.within(tag, query.timeline.first, query.timeline.last) {
        let Holder::Found { slot, id } = run.holder else {
            replay.unresolved.push(HourSpan { first_hour: run.first, last_hour: run.last });
            continue;
        };
        for stretch in query.timeline.within(run.first, run.last) {
            let (first, last) = (stretch.first, stretch.last);
            let stock = dashboard_query::cost::storage(|| {
                lanes::history(recording, stretch.run, TableId::StockByVariant, &[amount], Keys::Range(u128::from(slot) << 64, u128::from(slot) << 64 | u128::from(u64::MAX)), first, last)
            })?;
            for (key, steps) in &stock.keys {
                let changes = replay.variants.entry(*key as u64).or_default();
                for step in steps {
                    let value = match step.cells.as_deref() {
                        None => Stock::Absent,
                        Some([Cell::Number(value), ..]) => Stock::Amount(*value),
                        Some(_) => Stock::Unknown,
                    };
                    note(changes, step.hour, value);
                }
                note(changes, last.saturating_add(1), Stock::Absent);
            }
            let held = dashboard_query::cost::storage(|| {
                lanes::history(recording, stretch.run, TableId::LogisticsEquipment, &logistics_columns, Keys::Range(u128::from(id) << 32, u128::from(id) << 32 | u128::from(u32::MAX)), first, last)
            })?;
            let mut moves: BTreeMap<i32, i64> = BTreeMap::new();
            for (_, steps) in &held.keys {
                let mut shown: Option<u32> = None;
                for step in steps {
                    let row = step.cells.as_deref().and_then(|cells| {
                        let token = number(cells.first()).and_then(|token| u32::try_from(token).ok())?;
                        Some((token, Logistics { balance: number(cells.get(1)), stock: number(cells.get(2)), available: number(cells.get(3)) }))
                    });
                    if let Some(before) = shown.filter(|before| row.is_none_or(|(token, _)| token != *before)) {
                        note(replay.logistics.entry(before).or_default(), step.hour, None);
                    }
                    match (shown.is_some(), row) {
                        (false, Some(_)) => *moves.entry(step.hour).or_default() += 1,
                        (true, None) => *moves.entry(step.hour).or_default() -= 1,
                        _ => {}
                    }
                    if let Some((token, values)) = row {
                        note(replay.logistics.entry(token).or_default(), step.hour, Some(values));
                    }
                    shown = row.map(|(token, _)| token);
                }
                if let Some(before) = shown {
                    note(replay.logistics.entry(before).or_default(), last.saturating_add(1), None);
                }
            }
            let mut count = 0_i64;
            for (hour, change) in moves {
                count += change;
                note(&mut logistics_held, hour, count > 0);
            }
            note(&mut logistics_held, last.saturating_add(1), false);
            let lines = crate::products::production::cached_production_lines(source, stretch.run)?;
            let links = dashboard_base::queries::links(source, stretch.run)?;
            let handles: Vec<u128> = lines
                .lines
                .iter()
                .filter(|life| life.first <= last && life.last >= first && links.slot_at(life.country, life.first.max(first)) == Some(slot))
                .map(|life| u128::from(life.handle))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            if !handles.is_empty() {
                let made = dashboard_query::cost::storage(|| lanes::history(recording, stretch.run, TableId::ProductionLines, &[variant, kind], Keys::These(&handles), first, last))?;
                let at = made.columns.binary_search(&variant).ok();
                for (_, steps) in &made.keys {
                    for step in steps {
                        if let Some(design) = number(step.cells.as_deref().and_then(|cells| cells.get(at?))) {
                            replay.produced.insert(design as u64);
                        }
                    }
                }
            }
            let lines_read = reads_of(query, stretch.run, TableId::ProductionLines, first, last)?;
            let stock_read = reads_of(query, stretch.run, TableId::Stockpiles, first, last)?;
            reads.push((last, if stock_read.is_empty() { lines_read.clone() } else { stock_read }));
            line_reads.push((last, lines_read));
            logistics_reads.push((last, reads_of(query, stretch.run, TableId::LogisticsEquipment, first, last)?));
        }
    }
    for (into, spans, held) in [(&mut replay.reads, &mut replay.stock, reads), (&mut replay.line_reads, &mut replay.lines, line_reads)] {
        for (last, found) in held {
            spans.extend(carried(&found, last));
            into.extend(found);
        }
    }
    for (last, found) in logistics_reads {
        let (with, without) = split_logistics(&found, &logistics_held, last);
        replay.balance.extend(with);
        replay.ai.extend(without);
        replay.logistics_reads.extend(found);
    }
    Ok(replay)
}

#[derive(Default)]
struct Kept {
    replays: HashMap<(u64, i32, Country), Arc<Replay>>,
}

pub fn kept(source: &Source, query: &Query<'_>, tag: &Country) -> io::Result<Arc<Replay>> {
    let key = (query.timeline.id, query.timeline.last, tag.clone());
    let state = source.state::<Kept>();
    if let Some(found) = state.lock().unwrap_or_else(PoisonError::into_inner).replays.get(&key) {
        return Ok(Arc::clone(found));
    }
    let made = Arc::new(replay(source, query, tag)?);
    let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
    held.replays.retain(|(timeline, last, _), _| *timeline != key.0 || *last == key.1);
    held.replays.insert(key, Arc::clone(&made));
    Ok(made)
}
