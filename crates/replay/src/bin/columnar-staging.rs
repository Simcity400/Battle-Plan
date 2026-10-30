//! Measures the columnar writer's staging on a recording: every table's hour as the recorder
//! hands it over (a complete view the recording leaves unchanged as the same rows again, so
//! not staged), staged by one set of writers a row at a time and by another as one batch by
//! column. Both must write the same sections, manifest entries and strings every hour, and the
//! same restarts at every run's end. Reports the wall each takes to prepare and commit, per
//! staged cell (a visited row's column), in total and per table, beside the wall of only
//! reading the rows' wire fields, which both pay.
//!
//! With `--fixed`, measures instead what a table's hour costs the writer by what it asks of
//! it, staged by column: no rows, rows that change nothing, and rows that write a section,
//! and, after each hour of a complete table, two probes of no rows: changes of no row, as the
//! recorder hands on a re-read table that did not change, and an empty batch prepared
//! directly. Writers clearing and finishing only the columns an hour touched run beside
//! writers sweeping every column, which must write the same. `--cold <MiB>` writes that much
//! memory before every hour, as the game does between the writer's hours.
//! `columnar-staging <recording.battleplan> [hours] [--fixed [--cold <MiB>]]`

use replay::{
    container::{Replay, ReplayStream},
    native_columns::{self, RowCells, Staging, TableHour, TableInput},
};
use replay_columnar::{
    manifest::Status,
    writer::{Batch, Census, Coverage, Dictionary, Evidence, Sample, Sweep, TableWriter},
};
use replay_data::rows::{ClockRow, Field, Opaque, RowCodec, RowsVisit, TABLE_COUNT, TableId};
use replay_frame::frame::{TableUpdate, TableView};
use serde_json::json;
use std::{
    io,
    path::{Path, PathBuf},
    time::Instant,
};

const USAGE: &str = "columnar-staging <recording.battleplan> [hours] [--fixed [--cold <MiB>]]";

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// One staging's writers for a run, and its wall.
struct Side {
    writers: Vec<TableWriter>,
    sweep: Sweep,
    dictionary: Dictionary,
    cells: RowCells,
    /// A batch of no rows, for the probe preparing one directly.
    empty: Batch,
    prepare_ns: Vec<u64>,
    commit_ns: Vec<u64>,
}

impl Side {
    fn new(staging: Staging, sweep: Sweep) -> io::Result<Self> {
        Ok(Self {
            writers: writers(sweep)?,
            sweep,
            dictionary: Dictionary::new(),
            cells: RowCells::new(staging),
            empty: Batch::new(),
            prepare_ns: vec![0; TABLE_COUNT],
            commit_ns: vec![0; TABLE_COUNT],
        })
    }

    /// A new run's writers, keeping the wall measured.
    fn restart(&mut self) -> io::Result<()> {
        self.writers = writers(self.sweep)?;
        self.dictionary = Dictionary::new();
        Ok(())
    }

    /// Table `table`'s wall so far.
    fn wall(&self, table: usize) -> u64 {
        self.prepare_ns[table] + self.commit_ns[table]
    }

    fn prepare(&mut self, table: usize, input: Input<'_>) -> io::Result<TableHour> {
        let begin = Instant::now();
        let writer = &mut self.writers[table];
        let (dictionary, cells) = (&mut self.dictionary, &mut self.cells);
        let sample = Sample::FromReadBit;
        let hour = match input {
            Input::Rows { rows, partial } => rows.visit(Stage {
                writer,
                dictionary,
                cells,
                partial,
            }),
            Input::Same => native_columns::table_hour::<ClockRow>(
                writer,
                dictionary,
                cells,
                TableInput::Same { read: true, sample },
            ),
            Input::Unavailable(reason) => native_columns::table_hour::<ClockRow>(
                writer,
                dictionary,
                cells,
                TableInput::Unavailable {
                    reason,
                    read: true,
                    sample,
                },
            ),
            Input::NotRecorded => native_columns::table_hour::<ClockRow>(
                writer,
                dictionary,
                cells,
                TableInput::NotRecorded { read: true, sample },
            ),
            Input::NoChanges => native_columns::table_hour::<ClockRow>(
                writer,
                dictionary,
                cells,
                TableInput::Changes {
                    rows: &[],
                    removed: &[],
                    read: false,
                    sample,
                },
            ),
            Input::EmptyBatch => self.empty.begin(writer.kinds(), 0).and_then(|()| {
                let coverage = Coverage::Changes { removed: &[] };
                Ok(TableHour {
                    prepared: Some(writer.prepare(dictionary, coverage, &self.empty)?),
                    evidence: Evidence {
                        status: Status::Complete,
                        read: false,
                        sample,
                    },
                })
            }),
        };
        self.prepare_ns[table] += begin.elapsed().as_nanos() as u64;
        hour
    }

    fn commit(&mut self, table: usize, hour: TableHour, at: i32) -> io::Result<()> {
        let begin = Instant::now();
        self.writers[table].commit(hour.prepared, hour.evidence, at)?;
        self.commit_ns[table] += begin.elapsed().as_nanos() as u64;
        Ok(())
    }
}

fn total(values: &[u64]) -> u64 {
    values.iter().sum()
}

fn writers(sweep: Sweep) -> io::Result<Vec<TableWriter>> {
    TableId::ALL
        .iter()
        .map(|id| {
            let mut writer = TableWriter::new(&native_columns::columns(id.schema())?)?;
            writer.set_sweep(sweep);
            Ok(writer)
        })
        .collect()
}

/// A table's hour as the recorder hands it to its writer.
#[derive(Clone, Copy)]
enum Input<'a> {
    Rows {
        rows: &'a replay_data::rows::TableRows,
        partial: Option<u32>,
    },
    Same,
    Unavailable(&'a str),
    NotRecorded,
    /// Changes of no row, through [`native_columns::table_hour`].
    NoChanges,
    /// A batch of no rows, through [`TableWriter::prepare`].
    EmptyBatch,
}

/// A table's rows by their own type, staged as a complete or truncated snapshot.
struct Stage<'a> {
    writer: &'a mut TableWriter,
    dictionary: &'a mut Dictionary,
    cells: &'a mut RowCells,
    partial: Option<u32>,
}

impl RowsVisit for Stage<'_> {
    type Output = io::Result<TableHour>;

    fn rows<R: RowCodec>(self, rows: &[R]) -> io::Result<TableHour> {
        native_columns::table_hour(
            self.writer,
            self.dictionary,
            self.cells,
            TableInput::Snapshot {
                rows,
                partial: self.partial,
                read: true,
                sample: Sample::FromReadBit,
            },
        )
    }
}

/// Reads every row's wire fields, encoding opaque values, and keeps none: what a staging
/// pays before it compares a cell.
struct Wire<'a> {
    scratch: &'a mut Vec<u8>,
}

impl RowsVisit for Wire<'_> {
    type Output = io::Result<()>;

    fn rows<R: RowCodec>(self, rows: &[R]) -> io::Result<()> {
        for row in rows {
            row.visit_wire(&mut |_, field| {
                if let Field::Opaque(Opaque::Native(value)) = field {
                    self.scratch.clear();
                    value.encode_value(self.scratch)?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }
}

/// A table's hour as the recorder hands it to its writer.
fn handed<'a>(
    view: &'a TableView,
    update: &TableUpdate,
    continues: bool,
    writer: &TableWriter,
) -> Input<'a> {
    match view {
        // The collector hands an unchanged complete table on as the rows it shared before,
        // which the live writer takes as the same rows.
        TableView::Complete(_)
            if continues
                && matches!(update, TableUpdate::Unchanged { .. })
                && writer.status() == Status::Complete =>
        {
            Input::Same
        }
        TableView::Complete(table) => Input::Rows {
            rows: table,
            partial: None,
        },
        TableView::Truncated {
            rows: table,
            missing,
        } => Input::Rows {
            rows: table,
            partial: Some(*missing),
        },
        TableView::Unavailable { reason } => Input::Unavailable(reason),
        TableView::NotRecorded { .. } => Input::NotRecorded,
    }
}

/// What an hour writes: its section with its manifest entry and census, if it has one.
fn written(hour: &TableHour) -> Option<(&[u8], &[u64], u8, Census)> {
    let prepared = hour.prepared.as_ref()?;
    let section = prepared.section.as_deref()?;
    Some((
        section,
        prepared.columns.as_slice(),
        prepared.ops,
        prepared.census,
    ))
}

/// Both hours write the same section and evidence.
fn same_hour(one: &TableHour, other: &TableHour) -> bool {
    written(one) == written(other) && one.evidence == other.evidence
}

/// Both stagings' restarts of every table are the same.
fn same_restarts(rows: &Side, columns: &Side, hour: i32) -> io::Result<()> {
    for (index, (expected, actual)) in rows.writers.iter().zip(&columns.writers).enumerate() {
        if expected.restart()? != actual.restart()? {
            return Err(invalid(format!(
                "{:?} restarts differ after hour {hour}",
                TableId::ALL[index]
            )));
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let (mut fixed, mut cold) = (false, 0_usize);
    let mut positional = Vec::new();
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--fixed") => fixed = true,
            Some("--cold") => {
                cold = arguments
                    .next()
                    .and_then(|mib| mib.to_str()?.parse::<usize>().ok())
                    .ok_or_else(|| io::Error::other("--cold takes MiB"))?;
            }
            _ => positional.push(argument),
        }
    }
    let mut positional = positional.into_iter();
    let input = PathBuf::from(positional.next().ok_or_else(|| io::Error::other(USAGE))?);
    let limit = match positional.next() {
        Some(hours) => hours
            .to_str()
            .and_then(|hours| hours.parse::<u64>().ok())
            .ok_or_else(|| io::Error::other("hours is a count"))?,
        None => u64::MAX,
    };
    if fixed {
        return per_table(&input, limit, cold);
    }
    let replay = Replay::open_path(&input)?;
    let mut stream = ReplayStream::new(&TableId::ALL)?;
    let mut rows = Side::new(Staging::Rows, Sweep::Touched)?;
    let mut columns = Side::new(Staging::Columns, Sweep::Touched)?;
    let schema: Vec<usize> = TableId::ALL.iter().map(|id| id.schema().len()).collect();
    let mut cells = vec![0_u64; TABLE_COUNT];
    let mut visited = vec![0_u64; TABLE_COUNT];
    let mut wire_ns = vec![0_u64; TABLE_COUNT];
    let mut scratch = Vec::new();
    let (mut hours, mut runs) = (0_u64, 0_u64);
    // The branch and the next hour of the run under way.
    let mut run: Option<(u64, i32)> = None;
    let started = Instant::now();
    while hours < limit
        && stream.next_block_recorded_with(
            &replay,
            |recorded, decoded| {
                if hours >= limit {
                    return Ok(());
                }
                let continues = run == Some((decoded.branch, decoded.hour));
                if !continues {
                    if run.is_some() {
                        same_restarts(&rows, &columns, decoded.hour)?;
                    }
                    rows.restart()?;
                    columns.restart()?;
                    runs += 1;
                }
                for index in 0..TABLE_COUNT {
                    let input = handed(
                        &decoded.tables[index],
                        &recorded.tables[index],
                        continues,
                        &rows.writers[index],
                    );
                    if let Input::Rows { rows: table, .. } = input {
                        visited[index] += table.len() as u64;
                        cells[index] += (table.len() * schema[index]) as u64;
                        let begin = Instant::now();
                        table.visit(Wire {
                            scratch: &mut scratch,
                        })?;
                        wire_ns[index] += begin.elapsed().as_nanos() as u64;
                    }
                    // Each staging goes first every other hour, so neither always finds the
                    // rows warm.
                    let (first, second) = if hours % 2 == 0 {
                        (&mut rows, &mut columns)
                    } else {
                        (&mut columns, &mut rows)
                    };
                    let one = first.prepare(index, input)?;
                    let other = second.prepare(index, input)?;
                    if !same_hour(&one, &other) {
                        return Err(invalid(format!(
                            "{:?} hour {} differs between the stagings",
                            TableId::ALL[index],
                            decoded.hour
                        )));
                    }
                    first.commit(index, one, decoded.hour)?;
                    second.commit(index, other, decoded.hour)?;
                }
                if rows.dictionary.take_appended() != columns.dictionary.take_appended() {
                    return Err(invalid(format!(
                        "hour {} appends other strings between the stagings",
                        decoded.hour
                    )));
                }
                run = Some((decoded.branch, decoded.hour + 1));
                hours += 1;
                Ok(())
            },
            || Ok(()),
        )?
    {}
    if let Some((_, next)) = run {
        same_restarts(&rows, &columns, next - 1)?;
    }
    let per_hour = |value: u64| value as f64 / hours.max(1) as f64;
    let per_cell = |ns: u64, cells: u64| ns as f64 / cells.max(1) as f64;
    let wall = |side: &Side, table: usize| side.prepare_ns[table] + side.commit_ns[table];
    let all_cells = total(&cells);
    let rows_ns = total(&rows.prepare_ns) + total(&rows.commit_ns);
    let columns_ns = total(&columns.prepare_ns) + total(&columns.commit_ns);
    let mut tables: Vec<usize> = (0..TABLE_COUNT).collect();
    tables.sort_unstable_by_key(|&table| std::cmp::Reverse(wall(&rows, table)));
    let side = |side: &Side| {
        json!({
            "prepare_ms_per_hour": per_hour(total(&side.prepare_ns)) / 1e6,
            "commit_ms_per_hour": per_hour(total(&side.commit_ns)) / 1e6,
            "ns_per_cell": per_cell(total(&side.prepare_ns) + total(&side.commit_ns), all_cells),
        })
    };
    let report = json!({
        "recording": input.display().to_string(),
        "hours": hours,
        "runs": runs,
        "wall_seconds": started.elapsed().as_secs_f64(),
        "cells_per_hour": per_hour(all_cells),
        "wire_ms_per_hour": per_hour(total(&wire_ns)) / 1e6,
        "wire_ns_per_cell": per_cell(total(&wire_ns), all_cells),
        "rows": side(&rows),
        "columns": side(&columns),
        "speedup": rows_ns as f64 / columns_ns.max(1) as f64,
        "tables": tables
            .iter()
            .filter(|&&table| cells[table] > 0)
            .map(|&table| json!({
                "table": format!("{:?}", TableId::ALL[table]),
                "rows_per_hour": per_hour(visited[table]),
                "cells_per_hour": per_hour(cells[table]),
                "rows_us_per_hour": per_hour(wall(&rows, table)) / 1e3,
                "columns_us_per_hour": per_hour(wall(&columns, table)) / 1e3,
                "rows_ns_per_cell": per_cell(wall(&rows, table), cells[table]),
                "columns_ns_per_cell": per_cell(wall(&columns, table), cells[table]),
                "wire_ns_per_cell": per_cell(wire_ns[table], cells[table]),
            }))
            .collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// The table-hours [`per_table`] tells apart: no rows visited, rows that change nothing, rows
/// that write a section, then the probes after an hour of a complete table.
const KINDS: [&str; 5] = ["same", "quiet", "changed", "no_changes", "empty_batch"];
const SAME: usize = 0;
const QUIET: usize = 1;
const CHANGED: usize = 2;
const NO_CHANGES: usize = 3;
const EMPTY_BATCH: usize = 4;

/// A count or wall per table and kind of hour.
type ByKind = Vec<[u64; KINDS.len()]>;

/// Writers working only the columns an hour touched beside writers sweeping every column,
/// and what their hours cost.
struct Sweeps {
    touched: Side,
    swept: Side,
    counts: ByKind,
    /// The touched writers' wall, then the swept ones'.
    walls: [ByKind; 2],
}

impl Sweeps {
    /// One table-hour through both, which must write the same; the one going first
    /// alternates every hour, so neither always finds the rows warm. Its wall goes to `kind`,
    /// or to the kind its rows make when none is given.
    fn hour(
        &mut self,
        table: usize,
        input: Input<'_>,
        kind: Option<usize>,
        hour: i32,
        flip: bool,
    ) -> io::Result<()> {
        let Self {
            touched,
            swept,
            counts,
            walls,
        } = self;
        let before = [touched.wall(table), swept.wall(table)];
        let (first, second) = if flip {
            (&mut *swept, &mut *touched)
        } else {
            (&mut *touched, &mut *swept)
        };
        let one = first.prepare(table, input)?;
        let other = second.prepare(table, input)?;
        if !same_hour(&one, &other) {
            return Err(invalid(format!(
                "{:?} hour {hour} differs between the sweeps",
                TableId::ALL[table]
            )));
        }
        let kind = kind.unwrap_or(if written(&one).is_some() {
            CHANGED
        } else {
            QUIET
        });
        first.commit(table, one, hour)?;
        second.commit(table, other, hour)?;
        let after = [touched.wall(table), swept.wall(table)];
        counts[table][kind] += 1;
        for ((side, after), before) in walls.iter_mut().zip(after).zip(before) {
            side[table][kind] += after - before;
        }
        Ok(())
    }
}

/// Every table's hours staged by column, by writers working only the columns an hour touched
/// and by writers sweeping every column, timed by what each hour asked of them.
fn per_table(input: &Path, limit: u64, cold: usize) -> io::Result<()> {
    let replay = Replay::open_path(input)?;
    let mut stream = ReplayStream::new(&TableId::ALL)?;
    let mut sweeps = Sweeps {
        touched: Side::new(Staging::Columns, Sweep::Touched)?,
        swept: Side::new(Staging::Columns, Sweep::Every)?,
        counts: vec![[0; KINDS.len()]; TABLE_COUNT],
        walls: [
            vec![[0; KINDS.len()]; TABLE_COUNT],
            vec![[0; KINDS.len()]; TABLE_COUNT],
        ],
    };
    let mut evicted = vec![0_u8; cold << 20];
    let (mut hours, mut runs) = (0_u64, 0_u64);
    let mut run: Option<(u64, i32)> = None;
    let started = Instant::now();
    while hours < limit
        && stream.next_block_recorded_with(
            &replay,
            |recorded, decoded| {
                if hours >= limit {
                    return Ok(());
                }
                let continues = run == Some((decoded.branch, decoded.hour));
                if !continues {
                    if run.is_some() {
                        same_restarts(&sweeps.touched, &sweeps.swept, decoded.hour)?;
                    }
                    sweeps.touched.restart()?;
                    sweeps.swept.restart()?;
                    runs += 1;
                }
                for line in evicted.chunks_exact_mut(64) {
                    line[0] = line[0].wrapping_add(1);
                }
                let _ = std::hint::black_box(&mut evicted);
                let (at, flip) = (decoded.hour, hours % 2 == 1);
                for index in 0..TABLE_COUNT {
                    let input = handed(
                        &decoded.tables[index],
                        &recorded.tables[index],
                        continues,
                        &sweeps.touched.writers[index],
                    );
                    let kind = match input {
                        Input::Rows { .. } => None,
                        _ => Some(SAME),
                    };
                    sweeps.hour(index, input, kind, at, flip)?;
                    // A complete table's hours of no rows change nothing held.
                    if sweeps.touched.writers[index].status() == Status::Complete {
                        sweeps.hour(index, Input::NoChanges, Some(NO_CHANGES), at, flip)?;
                        sweeps.hour(index, Input::EmptyBatch, Some(EMPTY_BATCH), at, flip)?;
                    }
                }
                let appended = sweeps.touched.dictionary.take_appended();
                if sweeps.swept.dictionary.take_appended() != appended {
                    return Err(invalid(format!(
                        "hour {} appends other strings between the sweeps",
                        decoded.hour
                    )));
                }
                run = Some((decoded.branch, decoded.hour + 1));
                hours += 1;
                Ok(())
            },
            || Ok(()),
        )?
    {}
    if let Some((_, next)) = run {
        same_restarts(&sweeps.touched, &sweeps.swept, next - 1)?;
    }
    let Sweeps { counts, walls, .. } = &sweeps;
    let us = |ns: u64, count: u64| ns as f64 / count.max(1) as f64 / 1e3;
    let sum = |values: &ByKind, of: &[usize]| -> u64 {
        values
            .iter()
            .map(|table| of.iter().map(|&kind| table[kind]).sum::<u64>())
            .sum()
    };
    let kinds = |walls: &ByKind, of: &[usize]| {
        let (ns, count) = (sum(walls, of), sum(counts, of));
        json!({
            "table_hours": count,
            "us_per_table_hour": us(ns, count),
            "ms_per_hour": ns as f64 / hours.max(1) as f64 / 1e6,
        })
    };
    let side = |walls: &ByKind| {
        let mut report = serde_json::Map::new();
        for (kind, name) in KINDS.iter().enumerate() {
            report.insert(name.to_string(), kinds(walls, &[kind]));
        }
        // Every hour that wrote nothing: what a recording's unchanged tables cost.
        report.insert("unwritten".to_string(), kinds(walls, &[SAME, QUIET]));
        serde_json::Value::Object(report)
    };
    let mut tables: Vec<usize> = (0..TABLE_COUNT)
        .filter(|&table| counts[table].iter().any(|&count| count > 0))
        .collect();
    // The tables whose hours of no change cost the sweeping writers most first.
    tables.sort_unstable_by_key(|&table| {
        let swept = &walls[1][table];
        std::cmp::Reverse(swept[QUIET] + swept[NO_CHANGES] + swept[EMPTY_BATCH])
    });
    let entry = |table: usize| {
        let mut entry = serde_json::Map::new();
        entry.insert(
            "table".to_string(),
            json!(format!("{:?}", TableId::ALL[table])),
        );
        entry.insert(
            "columns".to_string(),
            json!(TableId::ALL[table].schema().len()),
        );
        for (kind, name) in KINDS.iter().enumerate() {
            let count = counts[table][kind];
            if count > 0 {
                let (touched, swept) = (walls[0][table][kind], walls[1][table][kind]);
                entry.insert(
                    name.to_string(),
                    json!({
                        "table_hours": count,
                        "touched_us_per_table_hour": us(touched, count),
                        "swept_us_per_table_hour": us(swept, count),
                    }),
                );
            }
        }
        serde_json::Value::Object(entry)
    };
    let report = json!({
        "recording": input.display().to_string(),
        "hours": hours,
        "runs": runs,
        "cold_mib": cold,
        "wall_seconds": started.elapsed().as_secs_f64(),
        "touched": side(&walls[0]),
        "swept": side(&walls[1]),
        "tables": tables.iter().map(|&table| entry(table)).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
