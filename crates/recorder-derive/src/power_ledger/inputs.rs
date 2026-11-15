//! How the writer reads each input table: one function per table, so a reshaped input (new keys,
//! a log in place of a state table) is a change here alone.

use crate::framework::{Columns, Hour, columns};
use replay_columnar::{
    manifest::Status,
    store::{TableView, Value as Held},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryCommandPowerRow, CountryLinkRow, CountryPowerChangeRow, CountryPowerDailyCostRow, CountryPowerDailyRow, CountryRow, PoliticsCaptureCoverageRow, PowerSpendCoverageRow, RowCodec,
        TableId,
    },
};
use std::{collections::HashMap, io};

pub(super) fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub(super) const COUNTRY_SLOT: [usize; 1] = columns::<CountryRow, 1>(["canonical_slot"]);
pub(super) const LINK_SLOT: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
pub(super) const ALLOCATED: [usize; 1] = columns::<CountryCommandPowerRow, 1>(["allocated_command_power_raw"]);
const WITNESS: [usize; 4] = columns::<PoliticsCaptureCoverageRow, 4>(["epoch", "since_hour", "through_hour", "streams"]);
const COVERAGE: [usize; 4] = columns::<PowerSpendCoverageRow, 4>(["political", "command", "political_since", "command_since"]);

/// A holder's PowerSpendCoverage row as the hour began: each currency covered and since when.
pub(super) struct HeldCoverage {
    pub political: bool,
    pub command: bool,
    pub political_since: Option<i32>,
    pub command_since: Option<i32>,
}

/// Holder `holder`'s row of the writer's own PowerSpendCoverage, `view`; none when it holds none.
pub(super) fn held_coverage(view: &TableView<'_>, holder: u32) -> Option<HeldCoverage> {
    let slot = view.slot(u128::from(holder))?;
    let number = |at: usize| view.number(COVERAGE[at], slot);
    Some(HeldCoverage { political: number(0)? != 0, command: number(1)? != 0, political_since: number(2).map(|hour| hour as i32), command_since: number(3).map(|hour| hour as i32) })
}

/// `table`'s row in `slot`, whole, as the row decoder reads it.
pub(super) fn row<R: RowCodec>(hour: &Hour<'_>, table: TableId, slot: u32) -> io::Result<R> {
    row_of(&hour.table(table), slot)
}

/// `view`'s row in `slot`, whole, as the row decoder reads it.
fn row_of<R: RowCodec>(view: &TableView<'_>, slot: u32) -> io::Result<R> {
    let key = view.key(slot).ok_or_else(|| invalid(format!("Slot {slot} holds no row")))?;
    let format = FormatVersion::CURRENT.version();
    R::from_wire_fields(key, u64::MAX, &mut |column| replay_frame::frame::field(view, slot, column, format))
}

/// `table`'s row `key`, whole; none when it holds none.
pub(super) fn keyed<R: RowCodec>(view: &TableView<'_>, key: u128) -> io::Result<Option<R>> {
    view.slot(key).map(|slot| row_of(view, slot)).transpose()
}

/// Every row `table` holds, whole, in key order.
pub(super) fn every<R: RowCodec>(view: &TableView<'_>) -> io::Result<Vec<R>> {
    let mut rows = view.rows().map(|slot| row_of::<R>(view, slot)).collect::<io::Result<Vec<_>>>()?;
    rows.sort_unstable_by_key(RowCodec::key);
    Ok(rows)
}

/// The rows of `table` the hour inserted or wrote, whole, in key order: every row in a run's
/// first hour.
fn written<R: RowCodec>(hour: &Hour<'_>, table: TableId) -> io::Result<Vec<R>> {
    let mut slots = hour.written_rows(table, Columns::Every);
    slots.sort_unstable();
    slots.dedup();
    let mut rows = slots.into_iter().map(|slot| row::<R>(hour, table, slot)).collect::<io::Result<Vec<_>>>()?;
    rows.sort_unstable_by_key(RowCodec::key);
    Ok(rows)
}

fn unsigned(view: &TableView<'_>, column: usize, slot: u32) -> Option<u32> {
    view.number(column, slot).map(|value| value as u32)
}

/// Every CountryLinks row: raw id and canonical slot.
pub(super) fn links(hour: &Hour<'_>) -> Vec<(u32, u32)> {
    let view = hour.table(TableId::CountryLinks);
    view.rows().filter_map(|slot| Some((view.key(slot)? as u32, unsigned(&view, LINK_SLOT[0], slot)?))).collect()
}

/// Every Countries row: raw id and canonical slot.
pub(super) fn countries(hour: &Hour<'_>) -> Vec<(u32, u32)> {
    let view = hour.table(TableId::Countries);
    view.rows().filter_map(|slot| Some((view.key(slot)? as u32, unsigned(&view, COUNTRY_SLOT[0], slot)?))).collect()
}

/// Whether the hour moved a link's slot or a country's.
pub(super) fn identities_moved(hour: &Hour<'_>) -> bool {
    hour.wrote(TableId::CountryLinks, Columns::These(&LINK_SLOT)) || hour.wrote(TableId::Countries, Columns::These(&COUNTRY_SLOT))
}

/// Rows of an input as a log hands them: the hour's own, or in a run's first hour the whole list
/// as it stood when the run began (`backlog`), which are not that hour's events.
pub(super) struct Logged<T> {
    pub backlog: bool,
    pub rows: Vec<T>,
}

/// Whether every power change of the hour was read.
pub(super) fn changes_complete(hour: &Hour<'_>) -> bool {
    hour.complete(TableId::CountryPowerChanges)
}

/// The power changes read this hour, each living the hour it was read (those a truncated read
/// holds; none when the table is not read). Today the state table's rows the hour wrote; once a
/// log, `Hour::appended` of it, its backlog flag as handed.
pub(super) fn changes(hour: &Hour<'_>) -> io::Result<Logged<CountryPowerChangeRow>> {
    let table = TableId::CountryPowerChanges;
    let rows = match hour.status(table) {
        Status::Complete | Status::Truncated { .. } => written(hour, table)?,
        Status::Unavailable { .. } | Status::NotRecorded => Vec::new(),
    };
    Ok(Logged { backlog: hour.first, rows })
}

/// Whether the daily rows and their costs are read whole.
pub(super) fn daily_complete(hour: &Hour<'_>) -> bool {
    hour.complete(TableId::CountryPowerDaily) && hour.complete(TableId::CountryPowerDailyCosts)
}

const DAILY: [usize; 10] = columns::<CountryPowerDailyRow, 10>([
    "epoch",
    "day_hour",
    "pp_gross_gain_raw",
    "pp_cost_raw",
    "pp_net_gain_raw",
    "pp_realized_delta_raw",
    "pp_sequence",
    "cp_gain_raw",
    "cp_realized_delta_raw",
    "cp_sequence",
]);

fn held_number(value: Held<'_>) -> Option<i64> {
    match value {
        Held::Number(bits) => Some(bits as i64),
        _ => None,
    }
}

/// A daily row of `country` from its cells by `DAILY` column.
fn daily_of(country: u32, cell: impl Fn(usize) -> Option<i64>) -> Option<CountryPowerDailyRow> {
    let [epoch, day_hour, pp_gross, pp_cost, pp_net, pp_realized, pp_sequence, cp_gain, cp_realized, cp_sequence] = DAILY.map(cell);
    Some(CountryPowerDailyRow {
        country,
        epoch: epoch? as u64,
        day_hour: day_hour? as i32,
        pp_gross_gain_raw: pp_gross,
        pp_cost_raw: pp_cost,
        pp_net_gain_raw: pp_net,
        pp_realized_delta_raw: pp_realized,
        pp_sequence: pp_sequence.map(|value| value as u64),
        cp_gain_raw: cp_gain,
        cp_realized_delta_raw: cp_realized,
        cp_sequence: cp_sequence.map(|value| value as u64),
        ..CountryPowerDailyRow::default()
    })
}

/// Each day the hour ended of a raw country's daily row (CountryPowerDaily stays a state table):
/// its last cells, where the hour gave the row another day or epoch, or deleted it.
pub(super) fn closed_days(hour: &Hour<'_>) -> io::Result<Vec<CountryPowerDailyRow>> {
    let table = TableId::CountryPowerDaily;
    let view = hour.table(table);
    let mut before: HashMap<u32, HashMap<usize, Held<'_>>> = HashMap::new();
    for (slot, column, kept) in hour.previous(table) {
        before.entry(*slot).or_default().entry(usize::from(*column)).or_insert(kept.value());
    }
    let mut days = Vec::new();
    for (slot, cells) in before {
        if !cells.contains_key(&DAILY[0]) && !cells.contains_key(&DAILY[1]) {
            continue;
        }
        let Some(country) = view.key(slot).map(|key| key as u32) else { continue };
        let cell = |column: usize| cells.get(&column).map_or_else(|| held_number(view.value(column, slot)), |&value| held_number(value));
        days.extend(daily_of(country, cell));
    }
    for (key, cells) in hour.dropped(table) {
        days.extend(daily_of(*key as u32, |column| cells.get(column).and_then(|kept| held_number(kept.value()))));
    }
    days.sort_unstable_by_key(|day| day.country);
    Ok(days)
}

/// Every raw country whose daily row the hour wrote or deleted.
pub(super) fn moved_daily(hour: &Hour<'_>) -> Vec<u32> {
    let table = TableId::CountryPowerDaily;
    let view = hour.table(table);
    let mut raws: Vec<u32> = hour.written_rows(table, Columns::Every).into_iter().filter_map(|slot| view.key(slot)).map(|key| key as u32).collect();
    raws.extend(hour.deleted_keys(table).map(|key| key as u32));
    raws.sort_unstable();
    raws.dedup();
    raws
}

/// Raw country `raw`'s daily row.
pub(super) fn daily(hour: &Hour<'_>, raw: u32) -> io::Result<Option<CountryPowerDailyRow>> {
    keyed(&hour.table(TableId::CountryPowerDaily), u128::from(raw))
}

/// One raw country's daily costs as published, its batch whole, in ordinal order.
pub(super) struct Publication {
    pub country: u32,
    pub costs: Vec<CountryPowerDailyCostRow>,
}

/// The hour's daily cost publications. Today: each raw country with a cost row the hour wrote,
/// with every cost row it holds (a batch takes ordinals from zero, so the walk ends at the first
/// missing). Once a log: `Hour::appended` of it grouped by raw country, which needs the capture to
/// append a republished batch whole; its backlog flag as handed.
pub(super) fn publications(hour: &Hour<'_>) -> io::Result<Logged<Publication>> {
    let table = TableId::CountryPowerDailyCosts;
    if !hour.complete(table) {
        return Ok(Logged { backlog: hour.first, rows: Vec::new() });
    }
    let view = hour.table(table);
    let mut raws: Vec<u32> = hour.written_rows(table, Columns::Every).into_iter().filter_map(|slot| view.key(slot)).map(|key| (key >> 32) as u32).collect();
    raws.sort_unstable();
    raws.dedup();
    let mut rows = Vec::with_capacity(raws.len());
    for country in raws {
        let mut costs = Vec::new();
        for ordinal in 0..=u32::MAX {
            let Some(cost) = keyed::<CountryPowerDailyCostRow>(&hour.table(table), u128::from(country) << 32 | u128::from(ordinal))? else { break };
            costs.push(cost);
        }
        rows.push(Publication { country, costs });
    }
    Ok(Logged { backlog: hour.first, rows })
}

/// The reason, by name, the first of the payments' inputs not read whole this hour gave.
pub(super) fn input_reason(hour: &Hour<'_>) -> Option<String> {
    for table in [TableId::CountryPowerChanges, TableId::CountryPowerDaily, TableId::CountryPowerDailyCosts, TableId::PoliticsCaptureCoverage] {
        let view = hour.table(table);
        match view.status() {
            Status::Complete => {}
            Status::Truncated { missing } => return Some(format!("{table:?} was read in part: {missing} missing")),
            Status::Unavailable { reason } => return Some(format!("{table:?}: {}", view.string(reason).map_or_else(|| "unavailable".into(), |text| String::from_utf8_lossy(text).into_owned()))),
            Status::NotRecorded => return Some(format!("{table:?} is not recorded")),
        }
    }
    None
}

/// A raw country's politics capture as the hour's coverage holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Witness {
    pub country: u32,
    pub epoch: u64,
    pub since: i32,
    pub through: i32,
    pub streams: u8,
}

impl Witness {
    /// Whether it covers an event of `epoch` dated `event` in all of `bits` at read `hour`.
    pub fn covers(&self, epoch: u64, event: i32, bits: u8, hour: i32) -> bool {
        self.epoch == epoch && self.through == hour && self.streams & bits == bits && (self.since..=self.through).contains(&event)
    }
}

/// The politics coverage, when read whole this hour.
pub(super) fn witnesses<'a>(hour: &Hour<'a>) -> Option<TableView<'a>> {
    hour.complete(TableId::PoliticsCaptureCoverage).then(|| hour.table(TableId::PoliticsCaptureCoverage))
}

/// Raw country `raw`'s capture in `witnesses`.
pub(super) fn witness(witnesses: &TableView<'_>, raw: u32) -> Option<Witness> {
    let slot = witnesses.slot(u128::from(raw))?;
    let [epoch, since, through, streams] = WITNESS.map(|column| witnesses.number(column, slot));
    Some(Witness { country: raw, epoch: epoch? as u64, since: since? as i32, through: through? as i32, streams: streams? as u8 })
}

/// The command power slot `slot`'s allocators hold; none unless read.
pub(super) fn allocated(hour: &Hour<'_>, slot: u32) -> Option<i64> {
    if !hour.complete(TableId::CountryCommandPower) {
        return None;
    }
    let view = hour.table(TableId::CountryCommandPower);
    view.number(ALLOCATED[0], view.slot(u128::from(slot))?)
}
