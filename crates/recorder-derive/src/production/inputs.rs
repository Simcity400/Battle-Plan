//! How the writer reads each input table: one function per table, so a reshaped input is a
//! change here alone. A row is read from a cell source: the table as the hour left it, or a
//! row's cells before the hour wrote them ([`before`]), or a deleted row's last cells.

use crate::framework::{Hour, columns};
use replay_columnar::store::{Kept, TableView, Value};
use replay_data::rows::{LexicalTokenRow, OutputPurpose, ProductionLineOutputRow, RocketProductionOutputRow, TableId, VariantDefinitionRow};
use std::collections::HashMap;

pub(super) const OUTPUT: [usize; 8] = columns::<ProductionLineOutputRow, 8>(["country", "variant", "units", "since_hour", "ended", "exact", "spent_raw", "purpose"]);
const COUNTRY: usize = OUTPUT[0];
const VARIANT: usize = OUTPUT[1];
const UNITS: usize = OUTPUT[2];
const SINCE: usize = OUTPUT[3];
const ENDED: usize = OUTPUT[4];
const EXACT: usize = OUTPUT[5];
const SPENT: usize = OUTPUT[6];
const PURPOSE: usize = OUTPUT[7];
pub(super) const ROCKET_COLUMNS: [usize; 12] = columns::<RocketProductionOutputRow, 12>([
    "spent_raw",
    "instant_raw",
    "since_hour",
    "exact",
    "ticks",
    "inexact_ticks",
    "unknown_design_ticks",
    "unknown_country_ticks",
    "units",
    "units_exact",
    "deliveries",
    "inexact_deliveries",
]);
pub(super) const TOKEN: [usize; 2] = columns::<VariantDefinitionRow, 2>(["equipment_token", "archetype_token"]);
pub(super) const LITERAL: [usize; 1] = columns::<LexicalTokenRow, 1>(["literal"]);

/// A row's applied IC as the collector recorded it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Spent {
    Value(i64),
    /// Refused at the read.
    Refused,
    NotRecorded,
}

/// A ProductionLineOutput row as the totals fold it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Output {
    pub country: u32,
    pub variant: u64,
    pub units: i64,
    pub since: i32,
    pub ended: bool,
    pub exact: bool,
    pub spent: Spent,
    /// [`OutputPurpose::raw`]; Unknown when not recorded.
    pub purpose: u8,
}

fn number(value: Value<'_>) -> Option<i64> {
    match value {
        Value::Number(bits) => Some(bits as i64),
        _ => None,
    }
}

/// An output row from its cells, `cell(column)`; none while a cell its codec requires is not
/// recorded.
fn output<'a>(cell: impl Fn(usize) -> Value<'a>) -> Option<Output> {
    let spent = match cell(SPENT) {
        Value::Number(bits) => Spent::Value(bits as i64),
        Value::Missing => Spent::Refused,
        _ => Spent::NotRecorded,
    };
    let purpose = number(cell(PURPOSE)).map_or(OutputPurpose::Unknown, |raw| OutputPurpose::from_raw(raw as u8).unwrap_or_default());
    Some(Output {
        country: number(cell(COUNTRY))? as u32,
        variant: number(cell(VARIANT))? as u64,
        units: number(cell(UNITS))?,
        since: number(cell(SINCE))? as i32,
        ended: number(cell(ENDED))? != 0,
        exact: number(cell(EXACT))? != 0,
        spent,
        purpose: purpose.raw(),
    })
}

/// The cell of `column` of `view`'s row in `slot` before the hour wrote `written` of its cells.
fn held<'a>(view: &TableView<'a>, slot: u32, written: &'a [(u16, Kept)], column: usize) -> Value<'a> {
    match written.iter().find(|(held, _)| usize::from(*held) == column) {
        Some((_, kept)) => kept.value(),
        None => view.value(column, slot),
    }
}

fn kept(cells: &[Kept], column: usize) -> Value<'_> {
    cells.get(column).map_or(Value::Unrecorded, Kept::value)
}

/// The output row in `slot` as the hour left it.
pub(super) fn output_now(view: &TableView<'_>, slot: u32) -> Option<Output> {
    output(|column| view.value(column, slot))
}

/// The output row in `slot` before the hour wrote `written` of its cells.
pub(super) fn output_before<'a>(view: &TableView<'a>, slot: u32, written: &'a [(u16, Kept)]) -> Option<Output> {
    output(|column| held(view, slot, written, column))
}

/// A deleted output row as the deletion found it.
pub(super) fn output_dropped(cells: &[Kept]) -> Option<Output> {
    output(|column| kept(cells, column))
}

/// Each written row of `table` the hour kept, by slot: its columns' values before the hour,
/// each column once.
pub(super) fn written(hour: &Hour<'_>, table: TableId) -> HashMap<u32, Vec<(u16, Kept)>> {
    let mut rows: HashMap<u32, Vec<(u16, Kept)>> = HashMap::new();
    for (slot, column, kept) in hour.previous(table) {
        let cells = rows.entry(*slot).or_default();
        if !cells.iter().any(|(held, _)| held == column) {
            cells.push((*column, kept.clone()));
        }
    }
    rows
}

/// A RocketProductionOutput row keyed `key` from its cells; none while one is not recorded.
fn rocket<'a>(key: u128, cell: impl Fn(usize) -> Value<'a>) -> Option<RocketProductionOutputRow> {
    let [spent, instant, since, exact, ticks, inexact_ticks, unknown_design, unknown_country, units, units_exact, deliveries, inexact_deliveries] = ROCKET_COLUMNS.map(|column| number(cell(column)));
    Some(RocketProductionOutputRow {
        country: (key >> 64) as u32,
        variant: key as u64,
        spent_raw: spent?,
        instant_raw: instant?,
        since_hour: since? as i32,
        exact: exact? != 0,
        ticks: ticks? as u64,
        inexact_ticks: inexact_ticks? as u64,
        unknown_design_ticks: unknown_design? as u64,
        unknown_country_ticks: unknown_country? as u64,
        units: units?,
        units_exact: units_exact? != 0,
        deliveries: deliveries? as u64,
        inexact_deliveries: inexact_deliveries? as u64,
    })
}

/// The Rocket row in `slot` as the hour left it.
pub(super) fn rocket_now(view: &TableView<'_>, slot: u32) -> Option<RocketProductionOutputRow> {
    rocket(view.key(slot)?, |column| view.value(column, slot))
}

/// The Rocket row in `slot` before the hour wrote `written` of its cells.
pub(super) fn rocket_before<'a>(view: &TableView<'a>, slot: u32, written: &'a [(u16, Kept)]) -> Option<RocketProductionOutputRow> {
    rocket(view.key(slot)?, |column| held(view, slot, written, column))
}

/// A deleted Rocket row as the deletion found it.
pub(super) fn rocket_dropped(key: u128, cells: &[Kept]) -> Option<RocketProductionOutputRow> {
    rocket(key, |column| kept(cells, column))
}

/// Design `variant`'s equipment token and archetype as VariantDefinitions records them (lexer
/// tokens); none without its row.
pub(super) fn definition(hour: &Hour<'_>, variant: u64) -> Option<(u32, u32)> {
    let definitions = hour.table(TableId::VariantDefinitions);
    let slot = definitions.slot(u128::from(variant))?;
    Some((number(definitions.value(TOKEN[0], slot))? as u32, number(definitions.value(TOKEN[1], slot))? as u32))
}

/// The string id of lexer token `token`'s spelling; none while LexicalTokens does not hold it.
pub(super) fn spelling(hour: &Hour<'_>, token: u32) -> Option<u32> {
    let tokens = hour.table(TableId::LexicalTokens);
    tokens.text(LITERAL[0], tokens.slot(u128::from(token))?)
}
