
use crate::{
    query::{Query, Rows},
    table_name,
};
use dashboard_query::coverage::row_coverage;
use replay_columnar::manifest::Status;
use replay_data::rows::{Field, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::atomic::{AtomicU32, Ordering},
};

pub const CELLS: u32 = 1 << 31;

static NEXT: AtomicU32 = AtomicU32::new(0);

pub fn cells_id(lane: u32, lanes: u32) -> u32 {
    let lanes = lanes.max(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed) % (!CELLS / lanes);
    CELLS | (n * lanes + lane % lanes)
}

pub const NO_ROW: &str = "No live row has this key";
pub const NO_VALUE: &str = "No value";
pub const NOT_RECORDED: &str = "Not recorded at this hour";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Source {
    pub table: TableId,
    pub column: usize,
    pub key: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Value,
    Sum,
    Difference,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub table: TableId,
    pub columns: Vec<usize>,
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
        values.iter().all(|v| !matches!(v, CellValue::Number(n) if n.unsigned_abs() >= 1 << 53)).then_some(values)
    }

    pub fn query(&self) -> Query {
        Query::Cells { table: self.table.index(), columns: self.columns.clone(), rows: Rows::Live(self.keys.clone()) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellValue {
    Number(i64),
    Missing(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupState {
    Values(Vec<CellValue>),
    Follow(u32),
    Seeded { follow: u32, hour: i32, values: Vec<CellValue> },
    Unavailable(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    pub groups: Vec<Group>,
    rows: Vec<String>,
    kinds: Vec<(String, Op, f64)>,
    cells: Vec<u32>,
}

impl Live {
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

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
                let raw_values_json = |values: &[CellValue]| {
                    values
                        .iter()
                        .any(|value| matches!(value, CellValue::Number(number) if number.unsigned_abs() >= 1 << 53))
                        .then(|| {
                            values
                                .iter()
                                .map(|value| match value {
                                    CellValue::Number(number) if number.unsigned_abs() >= 1 << 53 => json!(number.to_string()),
                                    _ => Value::Null,
                                })
                                .collect::<Value>()
                        })
                };
                let (name, value) = match state {
                    GroupState::Values(values) => {
                        if let Some(raw) = raw_values_json(values) {
                            out["rawValues"] = raw;
                        }
                        ("values", values_json(values))
                    }
                    GroupState::Follow(id) => ("follow", json!(id)),
                    GroupState::Seeded { follow, hour, values } => {
                        out["seedHour"] = json!(hour);
                        out["values"] = values_json(values);
                        if let Some(raw) = raw_values_json(values) {
                            out["rawValues"] = raw;
                        }
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


    pub fn values(&self, frame: &DecodedFrame) -> Vec<Vec<CellValue>> {
        self.groups.iter().map(|group| group_values(group, &frame.tables[group.table.index()])).collect()
    }
}

fn group_values(group: &Group, view: &TableView) -> Vec<CellValue> {
    let width = group.columns.len();
    let (rows, absent_reason) = match view {
        TableView::Complete(rows) => (rows, NO_ROW.to_owned()),
        TableView::Truncated { rows, missing } => (
            rows,
            row_coverage(group.table, Status::Truncated { missing: *missing }, None, false).into_missing().unwrap_or_default(),
        ),
        TableView::NotRecorded { .. } => {
            let reason = row_coverage(group.table, Status::NotRecorded, None, false).into_missing().unwrap_or_default();
            return vec![CellValue::Missing(reason); group.keys.len() * width];
        }
        TableView::Unavailable { reason } => {
            let reason = row_coverage(group.table, Status::Unavailable { reason: 0 }, Some(reason), false).into_missing().unwrap_or_default();
            return vec![CellValue::Missing(reason); group.keys.len() * width];
        }
    };
    let mut out = Vec::with_capacity(group.keys.len() * width);
    for &key in &group.keys {
        let Some(row) = find(rows, key) else {
            out.extend(std::iter::repeat_n(CellValue::Missing(absent_reason.clone()), width));
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

#[derive(Default)]
pub struct Cells(RefCell<Building>);

impl Cells {
    pub fn value(&self, row: &str, column: &str, source: Source, scale: f64) {
        self.add(row, column, Op::Value, scale, &[source]);
    }

    pub fn sum(&self, row: &str, column: &str, sources: &[Source], scale: f64) {
        self.add(row, column, Op::Sum, scale, sources);
    }

    pub fn difference(&self, row: &str, column: &str, a: Source, b: Source, scale: f64) {
        self.add(row, column, Op::Difference, scale, &[a, b]);
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_values_carry_exact_raw_strings_only_for_unsafe_64_bit_integers() {
        let cells = Cells::default();
        cells.value("a", "total", Source { table: TableId::EquipmentLossTotals, column: 0, key: 1 }, 1.0);
        cells.value("b", "total", Source { table: TableId::EquipmentLossTotals, column: 0, key: 2 }, 1.0);
        cells.value("c", "total", Source { table: TableId::EquipmentLossTotals, column: 0, key: 3 }, 1.0);
        let live = cells.finish();

        let safe = live.json(&[GroupState::Values(vec![
            CellValue::Number((1_i64 << 53) - 1),
            CellValue::Number(-((1_i64 << 53) - 1)),
            CellValue::Missing(NO_ROW.to_owned()),
        ])]);
        assert!(safe["groups"][0].get("rawValues").is_none(), "safe integers omit rawValues: {safe}");

        let unsafe_json = live.json(&[GroupState::Values(vec![
            CellValue::Number(9_007_199_254_740_993),
            CellValue::Number(42),
            CellValue::Missing(NO_ROW.to_owned()),
        ])]);
        assert_eq!(
            unsafe_json["groups"][0]["rawValues"],
            json!(["9007199254740993", null, null]),
            "unsafe 64-bit integers must include exact rawValues strings"
        );
    }
}
