
use super::invalid;
use recorder_derive_framework::framework::{Columns, Hour, columns};
use replay_columnar::store::{Kept, TableView, Value as Held};
use replay_data::{
    event_feed_rows::EventFeedHistoryRow,
    format::FormatVersion,
    rows::{CountryCounterInputRow, CountryPowerChangeRow, PoliticsCaptureCoverageRow, PowerAction, PowerCause, PowerCurrency, Recorded, RelationTupleRow, RowCodec, TableId, TagHolderRow},
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

pub(super) const SLOT: [usize; 1] = [0];
pub(super) const OWNED: [usize; 1] = columns::<CountryCounterInputRow, 1>(["owned_state_count"]);
pub(super) const RELATION: [usize; 4] = columns::<RelationTupleRow, 4>(["first_country", "second_country", "start_hour", "cancel"]);

fn number(table: &TableView<'_>, column: usize, slot: u32) -> Option<i64> {
    match table.value(column, slot) {
        Held::Number(bits) => Some(bits as i64),
        _ => None,
    }
}

fn row<R: RowCodec>(table: TableId, view: &TableView<'_>, slot: u32) -> io::Result<R> {
    let key = view.key(slot).ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))?;
    let format = FormatVersion::CURRENT.version();
    R::from_wire(key, &mut |column| replay_frame::frame::field(view, slot, column, format)).map_err(|error| invalid(format!("{table:?} row {key}: {error}")))
}

pub(super) struct Pack {
    pub(super) row: EventFeedHistoryRow,
    pub(super) backlog: bool,
}

pub(super) fn pack(hour: &Hour<'_>) -> io::Result<Option<Pack>> {
    let view = hour.table(TableId::EventFeedHistory);
    if view.sampled_at() != Some(hour.hour) {
        return Ok(None);
    }
    let Some(slot) = view.slot(0) else { return Ok(None) };
    Ok(Some(Pack { row: row(TableId::EventFeedHistory, &view, slot)?, backlog: false }))
}

pub(super) fn derived_inputs_moved(hour: &Hour<'_>) -> bool {
    [(TableId::CountryCounterInputs, &OWNED[..]), (TableId::RelationTuples, &RELATION[..])]
        .into_iter()
        .any(|(table, columns)| !hour.complete(table) || hour.wrote(table, Columns::These(columns)))
}

pub(super) struct Names<'a> {
    countries: TableView<'a>,
    links: TableView<'a>,
}

impl<'a> Names<'a> {
    pub(super) fn new(hour: &Hour<'a>) -> Option<Self> {
        (hour.complete(TableId::Countries) && hour.complete(TableId::CountryLinks)).then(|| Self { countries: hour.table(TableId::Countries), links: hour.table(TableId::CountryLinks) })
    }

    fn canonical(&self, reference: u32) -> Option<u32> {
        let slot = self.links.slot(u128::from(reference))?;
        number(&self.links, SLOT[0], slot).map(|canonical| canonical as u32)
    }

    fn slot(&self, reference: u32, id: Option<u32>) -> Option<u32> {
        let slot = self.canonical(reference)?;
        let Some(id) = id else { return Some(slot) };
        let row = self.countries.slot(u128::from(id))?;
        (number(&self.countries, SLOT[0], row)? as u32 == slot && self.canonical(id)? == slot).then_some(slot)
    }
}

pub(super) fn slot(names: Option<&Names<'_>>, reference: u32, id: Option<u32>) -> Recorded<u32> {
    if reference as i32 <= 0 {
        return Recorded::Value(TagHolderRow::NONE);
    }
    names.and_then(|names| names.slot(reference, id)).map_or(Recorded::NotRecorded, Recorded::Value)
}

pub(super) type ChargeKey = (i32, u32, Box<[u8]>, bool);

pub(super) fn charges(hour: &Hour<'_>) -> io::Result<Option<HashMap<ChargeKey, [Option<i128>; 2]>>> {
    if !hour.complete(TableId::CountryPowerChanges) || !hour.complete(TableId::PoliticsCaptureCoverage) {
        return Ok(None);
    }
    let (changes, coverage) = (hour.table(TableId::CountryPowerChanges), hour.table(TableId::PoliticsCaptureCoverage));
    let streams = [PoliticsCaptureCoverageRow::PP, PoliticsCaptureCoverageRow::CP];
    let mut sums: HashMap<ChargeKey, [Option<i128>; 2]> = HashMap::new();
    for slot in changes.rows() {
        let change: CountryPowerChangeRow = row(TableId::CountryPowerChanges, &changes, slot)?;
        let timeout = match (change.cause, change.identity.action) {
            (PowerCause::Decision | PowerCause::TargetedDecision, PowerAction::Execute) => false,
            (PowerCause::DecisionTimeout, PowerAction::Timeout) => true,
            _ => continue,
        };
        let cover: Option<PoliticsCaptureCoverageRow> = match coverage.slot(u128::from(change.country)) {
            Some(at) => Some(row(TableId::PoliticsCaptureCoverage, &coverage, at)?),
            None => None,
        };
        let covered = streams.map(|stream| cover.as_ref().is_some_and(|cover| cover.since_hour <= change.hour && change.hour <= cover.through_hour && cover.streams & stream != 0));
        let token = content_reader_parser::parser::c_string(&change.identity.item_key);
        if token.is_empty() {
            continue;
        }
        let currency = match change.currency {
            PowerCurrency::Political => 0,
            PowerCurrency::Command => 1,
        };
        let sum = sums.entry((change.hour, change.country, token.into(), timeout)).or_insert(covered.map(|covered| covered.then_some(0)));
        if let Some(sum) = &mut sum[currency] {
            *sum += change.realized_delta_raw();
        }
    }
    Ok(Some(sums))
}

pub(super) struct Counters<'a> {
    view: TableView<'a>,
    pub(super) rows: Vec<u32>,
}

impl Counters<'_> {
    pub(super) fn owned(&self, slot: u32) -> Option<(u32, i64)> {
        Some((self.view.key(slot)? as u32, number(&self.view, OWNED[0], slot)?))
    }
}

pub(super) enum Counted<'a> {
    Incomplete,
    Unread,
    Read(Counters<'a>, Vec<u32>),
}

pub(super) fn counters_read(hour: &Hour<'_>) -> bool {
    hour.complete(TableId::CountryCounterInputs) && hour.table(TableId::CountryCounterInputs).sampled_at() == Some(hour.hour)
}

pub(super) fn counters<'a>(hour: &Hour<'a>, whole: bool) -> Counted<'a> {
    let table = TableId::CountryCounterInputs;
    let view = hour.table(table);
    if !hour.complete(table) {
        return Counted::Incomplete;
    }
    if view.sampled_at() != Some(hour.hour) {
        return Counted::Unread;
    }
    let (rows, deleted) = if whole {
        (view.rows().collect(), Vec::new())
    } else if hour.wrote(table, Columns::These(&OWNED)) {
        (hour.written_rows(table, Columns::These(&OWNED)), hour.deleted_keys(table).map(|key| key as u32).collect())
    } else {
        (Vec::new(), Vec::new())
    };
    Counted::Read(Counters { view, rows }, deleted)
}

pub(super) type Guarantee = (u32, u32, i32);

#[derive(Default)]
pub(super) struct Guarantees {
    pub(super) listed: HashSet<Guarantee>,
    pub(super) held: HashMap<Guarantee, i64>,
    pub(super) moved: HashMap<Guarantee, i64>,
}

fn is_guarantee(key: u128) -> bool {
    (key >> 16) as u32 == engine_native_parts::cguarantee_relation::TYPE
}

fn guarantee([first, second, start]: [Option<i64>; 3]) -> Option<Guarantee> {
    Some((first? as u32, second? as u32, start? as i32))
}

fn kept(cell: Option<&Kept>) -> Option<i64> {
    match cell.map(Kept::value) {
        Some(Held::Number(bits)) => Some(bits as i64),
        _ => None,
    }
}

pub(super) fn guarantees(hour: &Hour<'_>, whole: bool) -> Option<Guarantees> {
    let table = TableId::RelationTuples;
    if !hour.complete(table) {
        return None;
    }
    let mut out = Guarantees::default();
    if !whole && !hour.wrote(table, Columns::These(&RELATION)) {
        return Some(out);
    }
    let view = hour.table(table);
    let changes = hour.changes(table);
    let inserted: HashSet<u32> = if whole { HashSet::new() } else { changes.inserted.iter().copied().collect() };
    let mut before: HashMap<u32, [Option<Option<i64>>; 3]> = HashMap::new();
    if !whole {
        for (slot, column, cell) in hour.previous(table) {
            let Some(at) = RELATION[..3].iter().position(|&held| held == usize::from(*column)) else { continue };
            let earlier = &mut before.entry(*slot).or_default()[at];
            if earlier.is_none() {
                *earlier = Some(kept(Some(cell)));
            }
        }
    }
    let rows: Vec<u32> = if whole { view.rows().collect() } else { hour.written_rows(table, Columns::These(&RELATION)) };
    let mut read = HashSet::new();
    for slot in rows {
        if !read.insert(slot) || !view.key(slot).is_some_and(is_guarantee) {
            continue;
        }
        let [first, second, start, cancel] = RELATION.map(|column| number(&view, column, slot));
        let now = guarantee([first, second, start]);
        if let Some(now) = now {
            *out.held.entry(now).or_default() += 1;
            if cancel == Some(0) {
                out.listed.insert(now);
            }
        }
        if whole {
            continue;
        }
        let earlier = match inserted.contains(&slot) {
            true => None,
            false => guarantee([0, 1, 2].map(|at| before.get(&slot).and_then(|earlier| earlier[at]).unwrap_or_else(|| number(&view, RELATION[at], slot)))),
        };
        if earlier != now {
            earlier.into_iter().for_each(|earlier| *out.moved.entry(earlier).or_default() -= 1);
            now.into_iter().for_each(|now| *out.moved.entry(now).or_default() += 1);
        }
    }
    if !whole {
        for (&key, cells) in hour.dropped(table) {
            if is_guarantee(key)
                && let Some(earlier) = guarantee([0, 1, 2].map(|at| kept(cells.get(RELATION[at]))))
            {
                *out.moved.entry(earlier).or_default() -= 1;
            }
        }
    }
    Some(out)
}
