//! Which tables two recordings of one save record differently at the same hours, for proving a
//! collector change byte-identical: compare its differences from main with main's own
//! run-to-run differences. With `-` for the second recording and a table name, prints that
//! table's state and encoded size bound in each hour of the first instead. A complete table whose rows differ reports
//! each side's row count and its rows with no equal row of the same key in the other.
//! Measurement only:
//! `hour-diff <a.battleplan> <b.battleplan | -> <first-hour> <hours> [table]`.

use replay::campaign::Replay;
use replay_data::rows::{RowCodec, RowsPairVisit, RowsVisit, TableId, TableRows};
use replay_frame::frame::TableView;
use serde_json::json;
use std::{collections::BTreeMap, io, path::Path};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Each requested hour's table views, from the recording's first branch.
fn hours(path: &str, first: i32, count: i32) -> io::Result<BTreeMap<i32, Vec<TableView>>> {
    let replay = Replay::open_path(Path::new(path))?;
    let branch = replay
        .branches
        .first()
        .ok_or_else(|| invalid(format!("{path} has no branch")))?
        .id;
    let last = first + count - 1;
    let blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|index| index.branch == branch && index.first_hour <= last && index.last_hour >= first)
        .collect();
    let mut out = BTreeMap::new();
    replay.visit_blocks(
        &blocks,
        &TableId::ALL,
        None,
        |_, _, frame| {
            if (first..=last).contains(&frame.hour) {
                out.insert(
                    frame.hour,
                    TableId::ALL
                        .iter()
                        .map(|table| (*frame.tables[table.index()]).clone())
                        .collect(),
                );
            }
            Ok(frame.hour < last)
        },
        || Ok(()),
    )?;
    Ok(out)
}

/// Rows of one table in each recording with no equal row of the same key in the other.
struct Unmatched;
impl RowsPairVisit for Unmatched {
    type Output = (usize, usize);
    fn rows<R: RowCodec>(self, rows: &[R], other: &[R]) -> (usize, usize) {
        let mut by_key: BTreeMap<u128, Vec<&R>> = BTreeMap::new();
        for row in other {
            by_key.entry(row.key()).or_default().push(row);
        }
        let mut only_a = 0;
        for row in rows {
            let same = by_key.get_mut(&row.key()).and_then(|candidates| {
                let at = candidates.iter().position(|candidate| *candidate == row)?;
                Some(candidates.swap_remove(at))
            });
            only_a += usize::from(same.is_none());
        }
        (only_a, by_key.values().map(Vec::len).sum())
    }
}
/// The table's encoded size bound, summed over its rows.
struct Encoded;
impl RowsVisit for Encoded {
    type Output = usize;
    fn rows<R: RowCodec>(self, rows: &[R]) -> usize {
        rows.iter().map(|row| row.encoded_bound().unwrap_or(0)).sum()
    }
}
fn unmatched(a: &TableRows, b: &TableRows) -> serde_json::Value {
    match a.visit_pair(b, Unmatched) {
        Ok((only_a, only_b)) => json!([a.len(), b.len(), only_a, only_b]),
        Err(_) => json!("row types differ"),
    }
}

fn kind(view: &TableView) -> &'static str {
    match view {
        TableView::Complete(_) => "complete",
        TableView::Truncated { .. } => "truncated",
        TableView::Unavailable { .. } => "unavailable",
        TableView::NotRecorded { .. } => "not recorded",
    }
}

fn state(view: &TableView) -> serde_json::Value {
    match view {
        TableView::Complete(rows) => json!(["complete", rows.len()]),
        TableView::Truncated { rows, missing } => json!(["truncated", rows.len(), missing]),
        TableView::Unavailable { reason } => json!(["unavailable", reason]),
        TableView::NotRecorded { .. } => json!(["not recorded"]),
    }
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = || invalid("hour-diff <a.battleplan> <b.battleplan | -> <first-hour> <hours> [table]");
    let (a, b, first, count) = match args.as_slice() {
        [a, b, first, count] | [a, b, first, count, _] => (a, b, first, count),
        _ => return Err(usage()),
    };
    let first: i32 = first.parse().map_err(|_| invalid("first-hour"))?;
    let count: i32 = count.parse().map_err(|_| invalid("hours"))?;
    if b == "-" {
        let name = args.get(4).ok_or_else(usage)?;
        let table = TableId::ALL
            .iter()
            .position(|table| format!("{table:?}") == *name)
            .ok_or_else(|| invalid(format!("no table {name}")))?;
        for (hour, views) in hours(a, first, count)? {
            // Only a complete table has a whole size; any other prints none.
            let bytes = match &views[table] {
                TableView::Complete(rows) => json!(rows.visit(Encoded)),
                _ => json!(null),
            };
            println!("{hour} {} {bytes}", state(&views[table]));
        }
        return Ok(());
    }
    let (a, b) = (hours(a, first, count)?, hours(b, first, count)?);
    let mut report = Vec::new();
    for (hour, left) in &a {
        let Some(right) = b.get(hour) else {
            report.push(json!({ "hour": hour, "missing_in": "b" }));
            continue;
        };
        let mut rows = Vec::new();
        let mut availability = Vec::new();
        for (index, table) in TableId::ALL.iter().enumerate() {
            let (x, y) = (&left[index], &right[index]);
            if x == y {
                continue;
            }
            let name = format!("{table:?}");
            if let (TableView::Complete(x), TableView::Complete(y)) = (x, y) {
                rows.push(json!([name, unmatched(x, y)]));
            } else if kind(x) == kind(y) {
                rows.push(json!([name]));
            } else {
                availability.push(json!([name, state(x), state(y)]));
            }
        }
        report.push(json!({ "hour": hour, "rows_differ": rows, "availability_differs": availability }));
    }
    for hour in b.keys().filter(|hour| !a.contains_key(hour)) {
        report.push(json!({ "hour": hour, "missing_in": "a" }));
    }
    println!("{}", serde_json::to_string_pretty(&report).map_err(io::Error::other)?);
    Ok(())
}
