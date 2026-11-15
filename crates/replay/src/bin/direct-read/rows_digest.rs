//! `rows-digest`: each table's rows as a consumer is handed them, at chosen hours, as digests:
//! a line a table and hour, then a total. Nothing in a line depends on how the recording stores
//! the table, only on the rows, their keys and the view's status.

use crate::{Options, invalid, open, table_named};
use rayon::prelude::*;
use replay::{cells::TableReader, columnar_frames::Recording};
use replay_columnar::unit::BLOCK;
use replay_data::{
    format::FormatVersion,
    rows::{Field, RowCodec, RowsVisit, TableId, TableRows},
    scope::CaptureMode,
};
use replay_frame::frame::{TableView, Unsayable, cells_view};
use serde_json::{Value, json};
use std::io::{self, Write};

/// One table at one hour: its view's status, and its rows' count and digest when it has rows.
#[derive(Debug, PartialEq, Eq)]
pub struct Line {
    table: TableId,
    run: u32,
    ordinal: u32,
    hour: i32,
    status: Value,
    rows: Option<(usize, String)>,
}

impl Line {
    fn json(&self) -> Value {
        json!({
            "table": format!("{:?}", self.table),
            "run": self.run,
            "ordinal": self.ordinal,
            "hour": self.hour,
            "status": self.status,
            "rows": self.rows.as_ref().map(|(count, _)| count),
            "digest": self.rows.as_ref().map(|(_, digest)| digest),
        })
    }
}

/// Every table, or those `--tables` names, at each block's first hour of every run, or every
/// `--every` hours: a line each, tables in the order named, then the total.
pub fn rows_digest(path: &str, options: &Options) -> io::Result<Value> {
    let tables = match options.get("tables") {
        Some(names) => names.split(',').map(table_named).collect::<io::Result<Vec<_>>>()?,
        None => TableId::ALL.to_vec(),
    };
    let every = match options.get("every") {
        None | Some("block") => BLOCK,
        Some(hours) => hours.parse().ok().filter(|&hours: &u32| hours > 0).ok_or_else(|| invalid("--every=block|<hours>"))?,
    };
    let lines = digests(&open(path)?, &tables, every)?;
    let mut total = blake3::Hasher::new();
    let (mut with_rows, mut rows) = (0_u64, 0_u64);
    let mut out = io::stdout().lock();
    for line in &lines {
        let text = serde_json::to_string(&line.json())?;
        total.update(text.as_bytes());
        total.update(b"\n");
        if let Some((count, _)) = &line.rows {
            with_rows += 1;
            rows += *count as u64;
        }
        writeln!(out, "{text}")?;
    }
    out.flush()?;
    Ok(json!({
        "command": "rows-digest",
        "tables": tables.len(),
        "every": every,
        "lines": lines.len(),
        "lines_with_rows": with_rows,
        "rows": rows,
        "digest": total.finalize().to_hex().to_string(),
    }))
}

/// `tables` of `recording` at every `every`th ordinal of each run from its first, each as the
/// product's frames read it: sought there, its view built at the recording's capture mode.
pub fn digests(recording: &Recording, tables: &[TableId], every: u32) -> io::Result<Vec<Line>> {
    let mode = CaptureMode::from_byte(recording.root_identity().capture_mode)?;
    let format = FormatVersion::CURRENT.version();
    let mut hours = Vec::new();
    for run in 0..recording.run_count() {
        let held = recording.run(run)?;
        hours.extend((0..held.ordinals).step_by(every as usize).map(|ordinal| (run, ordinal, held.first_hour + ordinal as i32)));
    }
    let per_table = tables.par_iter().map(|&table| table_lines(recording, table, &hours, mode, format)).collect::<io::Result<Vec<_>>>()?;
    Ok(per_table.into_iter().flatten().collect())
}

fn table_lines(recording: &Recording, table: TableId, hours: &[(u32, u32, i32)], mode: CaptureMode, format: u16) -> io::Result<Vec<Line>> {
    let withheld = recording.withheld_reason(table.index());
    let mut held: Option<TableReader> = None;
    let mut lines = Vec::with_capacity(hours.len());
    for &(run, ordinal, hour) in hours {
        let (status, rows) = match withheld {
            Some(reason) => (json!({ "kind": "Withheld", "reason": reason }), None),
            None => {
                let reader = match held.take() {
                    Some(reader) if reader.run() == run => held.insert(reader),
                    _ => held.insert(TableReader::new(recording, run, table, None)?),
                };
                reader.seek(recording, ordinal)?;
                match cells_view(table, &reader.view(), reader.status(), reader.read_rows(), mode, format) {
                    Ok(view) => described(&view)?,
                    Err(error) if Unsayable::of(&error).is_some() => (json!({ "kind": "Unavailable", "reason": error.to_string() }), None),
                    Err(error) => return Err(error),
                }
            }
        };
        lines.push(Line { table, run, ordinal, hour, status, rows });
    }
    Ok(lines)
}

fn described(view: &TableView) -> io::Result<(Value, Option<(usize, String)>)> {
    Ok(match view {
        TableView::Complete(rows) => (json!({ "kind": "Complete" }), Some(digest(rows)?)),
        TableView::Truncated { rows, missing } => (json!({ "kind": "Truncated", "missing": missing }), Some(digest(rows)?)),
        TableView::Unavailable { reason } => (json!({ "kind": "Unavailable", "reason": reason }), None),
        TableView::NotRecorded { mode } => (json!({ "kind": "NotRecorded", "mode": format!("{mode:?}") }), None),
    })
}

/// The rows' count and a blake3 of them in their order, each row as [`digest_row`] hashes it.
fn digest(rows: &TableRows) -> io::Result<(usize, String)> {
    struct Digest;
    impl RowsVisit for Digest {
        type Output = io::Result<(usize, String)>;
        fn rows<R: RowCodec>(self, rows: &[R]) -> Self::Output {
            let mut hasher = blake3::Hasher::new();
            let mut encoded = Vec::new();
            for row in rows {
                digest_row(&mut hasher, &mut encoded, row.key(), |visit| row.visit_wire(visit))?;
            }
            Ok((rows.len(), hasher.finalize().to_hex().to_string()))
        }
    }
    rows.visit(Digest)
}

const EMPTY: u8 = 0;
const NOT_RECORDED: u8 = 1;
const VALUE: u8 = 2;

/// A row's key, then each field `fields` visits: a marker for an empty or a not-recorded cell,
/// or a value's marker and its wire encoding behind its length.
fn digest_row(
    hasher: &mut blake3::Hasher,
    encoded: &mut Vec<u8>,
    key: u128,
    fields: impl FnOnce(&mut dyn FnMut(usize, Field<'_>) -> io::Result<()>) -> io::Result<()>,
) -> io::Result<()> {
    hasher.update(&key.to_le_bytes());
    fields(&mut |_, field| {
        match field {
            Field::Empty => hasher.update(&[EMPTY]),
            Field::NotRecorded => hasher.update(&[NOT_RECORDED]),
            field => {
                encoded.clear();
                field.encode(encoded)?;
                hasher.update(&[VALUE]).update(&(encoded.len() as u64).to_le_bytes()).update(encoded)
            }
        };
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::columnar_frames::{
        fixtures,
        synthetic::{Input, Sample},
    };
    use replay_data::rows::{CountryRow, Recorded};

    const CHANGED: u32 = BLOCK + 10;

    /// Countries over two blocks of one run, `change` made to the row keyed 2 at the hour of
    /// ordinal [`CHANGED`] alone.
    fn recording(campaign: &[u8], change: fn(&mut CountryRow)) -> Recording {
        let (bytes, _) = fixtures::written(campaign, &[(7, 1000, 2)], &[TableId::Countries], |synthetic, _, ordinal| {
            let rows: Vec<CountryRow> = (1..=3)
                .map(|id| {
                    let mut row = CountryRow { id, tag: format!("tag {id}").into_bytes(), stability_raw: i64::from(ordinal / 100), ..Default::default() };
                    if id == 2 && ordinal == CHANGED {
                        change(&mut row);
                    }
                    row
                })
                .collect();
            synthetic.table(TableId::Countries, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        });
        Recording::open(bytes).unwrap()
    }

    #[test]
    fn rows_written_alike_digest_alike_and_one_cell_changes_its_hour_alone() {
        let (first, again, changed) = (recording(b"first", |_| {}), recording(b"again", |_| {}), recording(b"changed", |row| row.tag = b"changed".to_vec()));
        for every in [BLOCK, 10] {
            let lines = digests(&first, &[TableId::Countries], every).unwrap();
            assert_eq!(lines.len(), (2 * BLOCK).div_ceil(every) as usize);
            assert!(lines.iter().all(|line| line.status == json!({ "kind": "Complete" }) && line.rows.as_ref().is_some_and(|(count, _)| *count == 3)));
            assert_eq!(lines, digests(&again, &[TableId::Countries], every).unwrap());
        }
        let (before, after) = (digests(&first, &[TableId::Countries], 10).unwrap(), digests(&changed, &[TableId::Countries], 10).unwrap());
        for (before, after) in before.iter().zip(&after) {
            assert_eq!(before.ordinal, after.ordinal);
            if before.ordinal == CHANGED {
                assert_ne!(before.rows, after.rows);
                assert_eq!(before.rows.as_ref().map(|(count, _)| count), after.rows.as_ref().map(|(count, _)| count));
            } else {
                assert_eq!(before, after);
            }
        }
    }

    #[test]
    fn a_not_recorded_cell_digests_unlike_a_value_and_unlike_an_empty_cell() {
        let unrecorded = recording(b"unrecorded", |row| row.border_land_doctrine_level = Recorded::NotRecorded);
        let valued = recording(b"valued", |row| row.border_land_doctrine_level = Recorded::Value(Some(0)));
        let first = recording(b"first", |_| {});
        let lines = |recording: &Recording| digests(recording, &[TableId::Countries], 10).unwrap();
        let (unrecorded, valued, first) = (lines(&unrecorded), lines(&valued), lines(&first));
        let at = |lines: &[Line]| lines.iter().find(|line| line.ordinal == CHANGED).unwrap().rows.clone().unwrap();
        let (unrecorded, valued, first) = (at(&unrecorded), at(&valued), at(&first));
        assert_eq!((unrecorded.0, valued.0, first.0), (3, 3, 3));
        assert_ne!(unrecorded.1, valued.1);
        assert_ne!(unrecorded.1, first.1);
        assert_ne!(valued.1, first.1);

        let row = |last: Field<'static>| {
            let mut hasher = blake3::Hasher::new();
            digest_row(&mut hasher, &mut Vec::new(), 2, |visit| {
                visit(0, Field::U32(7))?;
                visit(1, last)
            })
            .unwrap();
            hasher.finalize()
        };
        let cells = [row(Field::NotRecorded), row(Field::Empty), row(Field::OptionalI32(None)), row(Field::OptionalI32(Some(0)))];
        for (at, cell) in cells.iter().enumerate() {
            assert!(cells[at + 1..].iter().all(|other| other != cell), "cell {at}");
        }
    }
}
