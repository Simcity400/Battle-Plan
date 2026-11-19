//! Live cells: the numbers a product shows that move with the playhead, named once by the product
//! beside the structure it shows (rows, names, art). A cell is an id the UI's structure names (a
//! row and a column) and a small formula over recorded cells (table, column, key): a value, a sum,
//! a difference or a ratio, each times a scale, evaluated where it is painted.
//!
//! The cells' sources are grouped by table and by the columns a key is read in, so one group is
//! one followed query ([`Rows::Live`]). On the request path a group carries its values at the
//! asked hour ([`Live::values`]); at the head the panel's host follows each group by key under an
//! id of its own ([`CELLS`]) and the follow layer pushes the rows that changed.

use crate::{
    evaluate,
    query::{Query, Rows},
    table_name,
};
use replay_columnar::manifest::Status;
use replay_data::rows::{Field, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::atomic::{AtomicU32, Ordering},
};

/// The bit of every follow id a panel's host takes for a group of its cells; the UI's own ids
/// never carry it.
pub const CELLS: u32 = 1 << 31;

static NEXT: AtomicU32 = AtomicU32::new(0);

/// A new id for a group of a panel's cells, greater than every one taken before it (until 2^31
/// have been taken), so a reader can tell an answer of a group it dropped from one of a group
/// its next structure will name.
pub fn cells_id() -> u32 {
    CELLS | (NEXT.fetch_add(1, Ordering::Relaxed) & !CELLS)
}

/// Why a cell's value is missing; the UI's followed cells say the same.
pub const NO_ROW: &str = "No live row has this key";
pub const NO_VALUE: &str = "No value";
pub const NOT_RECORDED: &str = "Not recorded at this hour";

/// A recorded cell: a table's column of the row with a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Source {
    pub table: TableId,
    pub column: usize,
    pub key: u128,
}

/// How a cell is made from its sources, then multiplied by its scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// Its one source.
    Value,
    /// Its sources added.
    Sum,
    /// The first source less the second.
    Difference,
    /// The first source over the second; missing where the second is zero.
    Ratio,
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::Sum => "sum",
            Self::Difference => "difference",
            Self::Ratio => "ratio",
        }
    }
}

/// One followed query of a product's cells: these keys of a table, each read in these columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub table: TableId,
    /// Ascending.
    pub columns: Vec<usize>,
    /// Ascending, each once.
    pub keys: Vec<u128>,
}

impl Group {
    pub(crate) fn seed(&self, frame: &DecodedFrame) -> Option<Vec<CellValue>> {
        let view = &frame.tables[self.table.index()];
        if view.is_not_declared() || view.held().is_not_selected()
            || view.fields().is_some_and(|fields| self.columns.iter().any(|&column| column >= 64 || fields & (1 << column) == 0))
        {
            return None;
        }
        let values = group_values(self, view.held());
        // The binary follow preserves all i64s; the panel's JSON is read as JavaScript numbers.
        values.iter().all(|v| !matches!(v, CellValue::Number(n) if n.unsigned_abs() >= 1 << 53)).then_some(values)
    }

    /// The query following it: the rows that changed pushed after the first answer.
    pub fn query(&self) -> Query {
        Query::Cells { table: self.table.index(), columns: self.columns.clone(), rows: Rows::Live(self.keys.clone()) }
    }
}

/// A cell's value read on the request path: the source's number, or why it has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellValue {
    Number(i64),
    Missing(String),
}

/// Where a group's values come from in the JSON the UI reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupState {
    /// Read at the asked hour: its cells row by row.
    Values(Vec<CellValue>),
    /// Followed at the head under this id.
    Follow(u32),
    Seeded { follow: u32, hour: i32, values: Vec<CellValue> },
    /// Not followed, and why: every one of its cells is missing for it.
    Unavailable(String),
}

/// A product's live cells as named: the groups their sources are read in, and each cell's id and
/// formula.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    pub groups: Vec<Group>,
    /// The row and column ids cells name, each once.
    rows: Vec<String>,
    /// Each distinct (column id, op, scale) cells use.
    kinds: Vec<(String, Op, f64)>,
    /// Per cell: its row, its kind, its sources' count, then each source's group, row and column.
    cells: Vec<u32>,
}

impl Live {
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// The JSON the UI reads, each group's values or follow as `states` gives it, in group order.
    pub fn json(&self, states: &[GroupState]) -> Value {
        let groups: Vec<Value> = self
            .groups
            .iter()
            .zip(states)
            .map(|(group, state)| {
                let mut out = json!({
                    "table": group.table.index(),
                    "columns": group.columns,
                    "keys": group.keys.iter().map(u128::to_string).collect::<Vec<_>>(),
                });
                let values_json = |values: &[CellValue]| values.iter().map(|value| match value {
                        CellValue::Number(number) => json!(number),
                        CellValue::Missing(reason) => json!(reason),
                    }).collect::<Value>();
                let (name, value) = match state {
                    GroupState::Values(values) => ("values", values_json(values)),
                    GroupState::Follow(id) => ("follow", json!(id)),
                    GroupState::Seeded { follow, hour, values } => {
                        out["seedHour"] = json!(hour);
                        out["values"] = values_json(values);
                        ("follow", json!(follow))
                    }
                    GroupState::Unavailable(reason) => ("unavailable", json!(reason)),
                };
                out[name] = value;
                out
            })
            .collect();
        let kinds: Vec<Value> = self.kinds.iter().map(|(id, op, scale)| json!({ "id": id, "op": op.name(), "scale": scale })).collect();
        json!({ "groups": groups, "rows": self.rows, "kinds": kinds, "cells": self.cells })
    }

    /// Patch the seeded value array at the positions of changed source cells.
    pub fn patch(&self, frame: &DecodedFrame, changed: &BTreeMap<TableId, BTreeMap<u128, u64>>, data: &mut Value) {
        for (i, group) in self.groups.iter().enumerate() {
            let Some(keys) = changed.get(&group.table) else { continue };
            let Ok(rows) = rows_of(group.table, frame.tables[group.table.index()].held()) else { continue };
            let values = &mut data["live"]["groups"][i]["values"];
            for (&key, &mask) in keys {
                let Ok(place) = group.keys.binary_search(&key) else { continue };
                let Some(row) = find(rows, key) else { continue };
                let _ = row.visit_wire(&mut |col, raw| {
                    if mask & (1u64 << col) != 0 && let Ok(c) = group.columns.binary_search(&col) {
                        values[place * group.columns.len() + c] = match cell(group.table, col, raw) {
                            CellValue::Number(n) => json!(n),
                            CellValue::Missing(m) => json!(m),
                        };
                    }
                    Ok(())
                });
            }
        }
    }

    /// Each group's cells at `frame`'s hour, row by row, from the frame's rows of its table:
    /// what the follow layer answers for the same hour, a missing cell named, never zero.
    pub fn values(&self, frame: &DecodedFrame) -> Vec<Vec<CellValue>> {
        self.groups.iter().map(|group| group_values(group, &frame.tables[group.table.index()])).collect()
    }
}

/// The table's rows, or why the whole group is missing (as the follow layer's lane status says it).
fn rows_of(table: TableId, view: &TableView) -> Result<&TableRows, String> {
    match view {
        TableView::Complete(rows) => Ok(rows),
        TableView::Truncated { missing, .. } => Err(evaluate::unrecorded(table.index(), Status::Truncated { missing: *missing }, None).unwrap_or_default()),
        TableView::NotRecorded { .. } => Err(evaluate::unrecorded(table.index(), Status::NotRecorded, None).unwrap_or_default()),
        TableView::Unavailable { reason } => Err(evaluate::unrecorded(table.index(), Status::Unavailable { reason: 0 }, Some(reason)).unwrap_or_default()),
    }
}

fn group_values(group: &Group, view: &TableView) -> Vec<CellValue> {
    let width = group.columns.len();
    let rows = match rows_of(group.table, view) {
        Ok(rows) => rows,
        Err(reason) => return vec![CellValue::Missing(reason); group.keys.len() * width],
    };
    let mut out = Vec::with_capacity(group.keys.len() * width);
    for &key in &group.keys {
        let Some(row) = find(rows, key) else {
            out.extend(std::iter::repeat_n(CellValue::Missing(NO_ROW.to_owned()), width));
            continue;
        };
        let start = out.len();
        out.extend(std::iter::repeat_n(CellValue::Missing(NOT_RECORDED.to_owned()), width));
        let _ = row.visit_wire(&mut |field, value| {
            if let Ok(at) = group.columns.binary_search(&field) {
                out[start + at] = cell(group.table, field, value);
            }
            Ok(())
        });
    }
    out
}

/// The row of `rows` (in key order) with `key`.
fn find(rows: &TableRows, key: u128) -> Option<replay_data::rows::RowRef<'_>> {
    let (mut low, mut high) = (0, rows.len());
    while low < high {
        let mid = low + (high - low) / 2;
        let row = rows.get(mid)?;
        match row.key().cmp(&key) {
            std::cmp::Ordering::Less => low = mid + 1,
            std::cmp::Ordering::Greater => high = mid,
            std::cmp::Ordering::Equal => return Some(row),
        }
    }
    None
}

/// A field as the follow layer reads the same cell from a lane.
fn cell(table: TableId, column: usize, field: Field<'_>) -> CellValue {
    let number = |value: i64| CellValue::Number(value);
    let missing = |reason: &str| CellValue::Missing(reason.to_owned());
    match field {
        Field::U8(value) => number(value.into()),
        Field::I8(value) => number(value.into()),
        Field::U16(value) => number(value.into()),
        Field::I16(value) => number(value.into()),
        Field::U32(value) => number(value.into()),
        Field::I32(value) => number(value.into()),
        Field::U64(value) => number(value as i64),
        Field::I64(value) => number(value),
        Field::Bool(value) => number(value.into()),
        Field::OptionalI32(value) => value.map_or_else(|| missing(NO_VALUE), |value| number(value.into())),
        Field::OptionalU32(value) => value.map_or_else(|| missing(NO_VALUE), |value| number(value.into())),
        Field::OptionalI64(value) => value.map_or_else(|| missing(NO_VALUE), number),
        Field::OptionalU64(value) => value.map_or_else(|| missing(NO_VALUE), |value| number(value as i64)),
        Field::Empty | Field::NotRecorded => missing(NOT_RECORDED),
        Field::Bytes(_) | Field::U32List(_) | Field::U64List(_) | Field::Opaque(_) => {
            let name = table.field_name(column).unwrap_or("?");
            CellValue::Missing(format!("{}'s {name} is not a number, which a live cell does not read", table_name(table.index())))
        }
    }
}

#[derive(Default)]
struct Building {
    rows: Vec<String>,
    row_at: HashMap<String, u32>,
    kinds: Vec<(String, Op, f64)>,
    cells: Vec<(u32, u32, Vec<Source>)>,
}

/// A product's live cells while its reply is made; [`Cells::finish`] groups them. Named through a
/// shared reference, as the reply's context holds it.
#[derive(Default)]
pub struct Cells(RefCell<Building>);

impl Cells {
    /// `source` times `scale`, as cell (`row`, `column`).
    pub fn value(&self, row: &str, column: &str, source: Source, scale: f64) {
        self.add(row, column, Op::Value, scale, &[source]);
    }

    /// `sources` added, times `scale`.
    pub fn sum(&self, row: &str, column: &str, sources: &[Source], scale: f64) {
        self.add(row, column, Op::Sum, scale, sources);
    }

    /// `a` less `b`, times `scale`.
    pub fn difference(&self, row: &str, column: &str, a: Source, b: Source, scale: f64) {
        self.add(row, column, Op::Difference, scale, &[a, b]);
    }

    /// `a` over `b`, times `scale`; missing where `b` is zero.
    pub fn ratio(&self, row: &str, column: &str, a: Source, b: Source, scale: f64) {
        self.add(row, column, Op::Ratio, scale, &[a, b]);
    }

    fn add(&self, row: &str, column: &str, op: Op, scale: f64, sources: &[Source]) {
        let held = &mut *self.0.borrow_mut();
        let row = match held.row_at.get(row).copied() {
            Some(at) => at,
            None => {
                let at = held.rows.len() as u32;
                held.rows.push(row.to_owned());
                held.row_at.insert(row.to_owned(), at);
                at
            }
        };
        let kind = match held.kinds.iter().position(|(id, held_op, held_scale)| id == column && *held_op == op && held_scale.to_bits() == scale.to_bits()) {
            Some(at) => at as u32,
            None => {
                held.kinds.push((column.to_owned(), op, scale));
                held.kinds.len() as u32 - 1
            }
        };
        held.cells.push((row, kind, sources.to_vec()));
    }

    /// The cells grouped: a key's columns of one table are one row of the group holding every key
    /// read in those columns.
    pub fn finish(self) -> Live {
        let Building { rows, kinds, cells, .. } = self.0.into_inner();
        let mut columns_of: BTreeMap<(TableId, u128), BTreeSet<usize>> = BTreeMap::new();
        for source in cells.iter().flat_map(|(.., sources)| sources) {
            columns_of.entry((source.table, source.key)).or_default().insert(source.column);
        }
        let mut grouped: BTreeMap<(TableId, Vec<usize>), Vec<u128>> = BTreeMap::new();
        for ((table, key), columns) in &columns_of {
            grouped.entry((*table, columns.iter().copied().collect())).or_default().push(*key);
        }
        let mut place: HashMap<(TableId, u128), (u32, u32)> = HashMap::with_capacity(columns_of.len());
        let groups: Vec<Group> = grouped
            .into_iter()
            .enumerate()
            .map(|(at, ((table, columns), keys))| {
                for (row, &key) in keys.iter().enumerate() {
                    place.insert((table, key), (at as u32, row as u32));
                }
                Group { table, columns, keys }
            })
            .collect();
        let mut flat = Vec::with_capacity(cells.len() * 6);
        for (row, kind, sources) in &cells {
            flat.extend([*row, *kind, sources.len() as u32]);
            for source in sources {
                let (group, at) = place[&(source.table, source.key)];
                let column = groups[group as usize].columns.binary_search(&source.column).expect("a grouped column") as u32;
                flat.extend([group, at, column]);
            }
        }
        Live { groups, rows, kinds, cells: flat }
    }
}
