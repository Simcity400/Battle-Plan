//! The reads every dashboard reply is made of, at the storage level, each reading one table's
//! lane: [`at`] (a table at an hour), [`at_each`] (rows at many hours), [`runs`] (a column's
//! values hour by hour over a span), [`history`] (any columns' cells over a span, at the hours
//! they change), [`chart`] (a column's buckets over a span from its summary pieces), [`next`]
//! (the next hour a column changes), [`log`] (a log's inserted rows counted per bin and facet),
//! [`lifetimes`] (every key a table held, with its first and last hour), [`reads`] (the hours a
//! table was read), [`statuses`] (its status as runs of hours) and [`logged_hours`] (the six-hour
//! buckets a log inserted rows in). Each takes a run of the recording and game hours; timelines,
//! identity and products are built on them elsewhere.

use crate::{cells::TableReader, columnar_frames::Recording, rows::TableId};
use replay_columnar::{
    envelope::{Extent, SPAN_BLOCKS, UNIT_ORDINALS},
    manifest::Status,
    state::Kind,
    store::{Strings, TableState, TableView},
    summary::{self, ColumnFrame, FANOUT, Fold, Piece, Shape},
    unit,
};
use rayon::prelude::*;
use std::{collections::HashMap, io};

pub use crate::points::Cell;

#[cfg(test)]
mod tests;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// The rows of a table a read at an hour asks for.
#[derive(Clone, Copy, Debug)]
pub enum Keys<'a> {
    All,
    These(&'a [u128]),
    /// Every key from the first through the second: for a table whose key holds its owner in its
    /// high bits, every row of one owner.
    Range(u128, u128),
    /// Keys within any of these ranges (first and last), ascending and apart: every row of
    /// several owners.
    Ranges(&'a [(u128, u128)]),
}

impl Keys<'_> {
    /// Whether it asks for the row keyed `key`; `sorted` is [`Keys::These`]' keys sorted.
    fn asks(&self, sorted: &[u128], key: u128) -> bool {
        match *self {
            Keys::All => true,
            Keys::These(_) => sorted.binary_search(&key).is_ok(),
            Keys::Range(low, high) => low <= key && key <= high,
            Keys::Ranges(ranges) => ranges.get(ranges.partition_point(|range| range.1 < key)).is_some_and(|range| range.0 <= key),
        }
    }
}

/// A table as it stood after an hour: its cells (the columns read; the others hold no value),
/// the strings they name, its status, and the rows asked.
pub struct Snapshot {
    pub table: TableState,
    pub strings: Strings,
    pub kinds: Vec<Kind>,
    pub status: Status,
    /// The hour of the read its rows hold; none when not known.
    pub sampled_at: Option<i32>,
    /// The hour read the table.
    pub was_read: bool,
    /// The rows asked that the hour shows, each key and slot, ascending by key: of a complete
    /// table its live rows, of one read in part the rows the hour read; none otherwise.
    pub rows: Vec<(u128, u32)>,
}

impl Snapshot {
    /// Its cells as the store's readers read a table: every live row, asked or not; the rows
    /// asked are [`Self::rows`].
    pub fn view(&self) -> TableView<'_> {
        TableView::new(&self.table, &self.kinds, &self.strings)
    }
}

/// Run `run`'s ordinal of game hour `hour`.
fn ordinal_of(recording: &Recording, run: u32, hour: i32) -> io::Result<u32> {
    recording.run(run)?.ordinal(hour).ok_or_else(|| invalid("The run holds no such hour"))
}

/// `table` of run `run` after hour `hour`, `columns` of it only when given (the others' cells
/// hold no value: never read them), every row: [`at_keys`] with [`Keys::All`].
pub fn at(recording: &Recording, run: u32, table: TableId, hour: i32, columns: Option<&[usize]>) -> io::Result<Snapshot> {
    at_keys(recording, run, table, hour, columns, Keys::All)
}

/// `table` of run `run` after hour `hour`, `columns` of it only when given (the others' cells
/// hold no value: never read them), the rows `keys` asks for: restored at its last restore before the hour, its parts
/// advanced to the hour. The cells are decoded whole (a unit is column-major); only the rows
/// asked are listed for a reader to build.
pub fn at_keys(recording: &Recording, run: u32, table: TableId, hour: i32, columns: Option<&[usize]>, keys: Keys<'_>) -> io::Result<Snapshot> {
    let ordinal = ordinal_of(recording, run, hour)?;
    let mut reader = TableReader::new(recording, run, table, columns)?;
    reader.seek(recording, ordinal)?;
    let rows = asked_rows(&reader, keys)?;
    let (status, sampled_at, was_read) = (reader.status(), reader.sampled_at(), reader.was_read());
    let (table, strings, kinds) = reader.into_parts();
    Ok(Snapshot { table, strings, kinds, status, sampled_at, was_read, rows })
}

/// The rows `keys` asks for that `reader`'s hour shows, each key and slot, ascending by key: of a
/// complete table its live rows, of one read in part the rows the hour read; none otherwise.
pub(crate) fn asked_rows(reader: &TableReader, keys: Keys<'_>) -> io::Result<Vec<(u128, u32)>> {
    let table = reader.table();
    let mut sorted = Vec::new();
    if let Keys::These(asked) = keys {
        sorted.extend_from_slice(asked);
        sorted.sort_unstable();
    }
    let view = reader.view();
    let mut rows: Vec<(u128, u32)> = Vec::new();
    let mut keep = |slot: u32| -> io::Result<()> {
        let key = view.key(slot).ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))?;
        if keys.asks(&sorted, key) {
            rows.push((key, slot));
        }
        Ok(())
    };
    match reader.status() {
        Status::Complete => view.rows().try_for_each(&mut keep)?,
        Status::Truncated { .. } => reader.read_rows().unwrap_or_default().iter().try_for_each(|&slot| keep(slot))?,
        _ => {}
    }
    rows.sort_unstable();
    Ok(rows)
}

/// `table` of run `run` at each of `asks` (an hour and a key), `columns` of it only when given:
/// one reader moved forward through the hours asked in order, each block advanced through once
/// and a restore lying between two hours sought. `each` is given each ask's hour and key, and its
/// row when the hour shows it (live, its table available and, read in part, read), in hour then
/// key order.
pub fn at_each(
    recording: &Recording,
    run: u32,
    table: TableId,
    columns: Option<&[usize]>,
    asks: &[(i32, u128)],
    each: &mut dyn FnMut(i32, u128, Option<(TableView<'_>, u32)>) -> io::Result<()>,
) -> io::Result<()> {
    let mut asks = asks.to_vec();
    asks.sort_unstable();
    asks.dedup();
    let mut reader = TableReader::new(recording, run, table, columns)?;
    reader.index_keys();
    for hour_asks in asks.chunk_by(|a, b| a.0 == b.0) {
        let hour = hour_asks[0].0;
        reader.go_to(recording, ordinal_of(recording, run, hour)?)?;
        let (view, read, available) = (reader.view(), reader.read_rows(), reader.available());
        for &(_, key) in hour_asks {
            let row = view.slot(key).filter(|&slot| available && read.is_none_or(|read| read.binary_search(&slot).is_ok()));
            each(hour, key, row.map(|slot| (view, slot)))?;
        }
    }
    Ok(())
}

/// One key's runs of a column: each run's first and last hour and its value, none where the
/// row was not live, its cell not recorded or its table not available.
pub type Runs = Vec<(i32, i32, Option<i64>)>;

/// Column `column` of `table` of run `run` from `from` through `to`, by key: its runs of equal
/// value, read from the column's tracks ([`crate::points`]). A text column's value is a string id
/// of its text, equal values equal texts within the read; [`history`] gives the texts.
pub fn runs(recording: &Recording, run: u32, table: TableId, column: usize, from: i32, to: i32) -> io::Result<Vec<(u128, Runs)>> {
    let held = keyed_runs(recording, run, table, summary::Source::Cell(column), None, from, to)?;
    let mut out: Vec<(u128, Runs)> = held.into_iter().collect();
    out.sort_unstable_by_key(|(key, _)| *key);
    Ok(out)
}

/// `source` of `table`'s rows from `from` through `to`, for `keys` (every key the hours hold
/// when none), by key: each key's runs of equal value, from its columns' tracks.
pub fn keyed_runs(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: summary::Source,
    keys: Option<&[u128]>,
    from: i32,
    to: i32,
) -> io::Result<HashMap<u128, Runs, foldhash::fast::RandomState>> {
    let mut held: HashMap<u128, Runs, foldhash::fast::RandomState> = HashMap::default();
    crate::points::points(recording, run, table, &inputs(source), Some(source), keys.map_or(Keys::All, Keys::These), from, to, &mut |point| {
        let runs = held.entry(point.key).or_default();
        let value = if point.covered { point.value } else { None };
        match runs.last_mut() {
            Some((_, _, now)) if *now == value => {}
            Some((_, last, _)) => {
                *last = point.hour - 1;
                runs.push((point.hour, point.hour, value));
            }
            None => runs.push((point.hour, point.hour, value)),
        }
        true
    })?;
    for runs in held.values_mut() {
        if let Some((_, last, _)) = runs.last_mut() {
            *last = to;
        }
    }
    Ok(held)
}

/// One key's cells from an hour on, as [`history`] reads them: none while its row is not shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub hour: i32,
    pub cells: Option<Box<[Cell]>>,
}

/// Some columns of a table's rows over a span ([`history`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    /// The columns read, ascending: each step's cells in their order.
    pub columns: Vec<usize>,
    /// The span's last hour: each key's last step holds through it.
    pub to: i32,
    /// The texts a [`Cell::Text`] names, by its index.
    pub texts: Vec<Box<[u8]>>,
    /// By key ascending, its steps in hour order.
    pub keys: Vec<(u128, Vec<Step>)>,
}

impl History {
    /// A text cell's text.
    pub fn text(&self, cell: Cell) -> Option<&[u8]> {
        match cell {
            Cell::Text(index) => self.texts.get(index as usize).map(|text| &**text),
            _ => None,
        }
    }

    /// Column `column`'s runs by key: each run's first and last hour and its cell, none where the
    /// row is not shown, equal cells one run; none when the history holds no such column.
    pub fn runs(&self, column: usize) -> Option<Vec<(u128, Vec<(i32, i32, Option<Cell>)>)>> {
        let at = self.columns.binary_search(&column).ok()?;
        Some(
            self.keys
                .iter()
                .map(|(key, steps)| {
                    let mut runs: Vec<(i32, i32, Option<Cell>)> = Vec::new();
                    for (index, step) in steps.iter().enumerate() {
                        let cell = step.cells.as_ref().map(|cells| cells[at]);
                        let last = steps.get(index + 1).map_or(self.to, |next| next.hour - 1);
                        match runs.last_mut() {
                            Some(run) if run.2 == cell => run.1 = last,
                            _ => runs.push((step.hour, last, cell)),
                        }
                    }
                    (*key, runs)
                })
                .collect(),
        )
    }
}

/// Columns `columns` of `table` of run `run` from `from` through `to`, for the keys `keys` asks
/// for: each key's cells at `from` (each key named, live or not; each live key asked otherwise)
/// and at every hour after at which one of them changes, its row comes or goes, or the table's
/// status or partial read hides or shows it, read from the tracks a block at a time
/// ([`crate::points::cells`]). Numbers and texts come with their values; a list or opaque cell
/// comes with the hour its value was written, to read it then with [`at_each`].
pub fn history(recording: &Recording, run: u32, table: TableId, columns: &[usize], keys: Keys<'_>, from: i32, to: i32) -> io::Result<History> {
    let mut held: HashMap<u128, Vec<Step>, foldhash::fast::RandomState> = HashMap::default();
    let (columns, texts) = crate::points::cells(recording, run, table, columns, keys, from, to, &mut |hour, key, cells| {
        held.entry(key).or_default().push(Step { hour, cells: cells.map(Box::from) });
        true
    })?;
    let mut keys: Vec<(u128, Vec<Step>)> = held.into_iter().collect();
    keys.sort_unstable_by_key(|(key, _)| *key);
    Ok(History { columns, to, texts, keys })
}

/// The hours `table` of run `run` was read from `from` through `to`, as runs of first and last
/// hour, ascending: those whose sample hour is the hour itself (what a writer watching every read
/// is told), from the blocks' page entries, no row decoded.
pub fn reads(recording: &Recording, run: u32, table: TableId, from: i32, to: i32) -> io::Result<Vec<(i32, i32)>> {
    let held = recording.run(run)?;
    let (from, to) = (from.max(held.first_hour), to.min(held.last_hour()));
    let (Some(first), Some(last)) = (held.ordinal(from), held.ordinal(to)) else { return Ok(Vec::new()) };
    if first > last {
        return Ok(Vec::new());
    }
    let blocks = held.place(first).0..held.place(last).0 + 1;
    let found = blocks
        .into_par_iter()
        .map(|block| -> io::Result<Vec<(i32, i32)>> {
            let entry = recording.table_entry(run, block, table.index())?.unit;
            let mut runs = Vec::new();
            unit::reader::reads(&entry, held.block(block as usize).1 as u16, held.block_first_hour(block), &mut |start, end| {
                let (start, end) = (start.max(from), end.min(to));
                if start <= end {
                    runs.push((start, end));
                }
            })?;
            Ok(runs)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let mut out: Vec<(i32, i32)> = Vec::new();
    for (start, end) in found.into_iter().flatten() {
        match out.last_mut() {
            Some((_, last)) if *last + 1 >= start => *last = (*last).max(end),
            _ => out.push((start, end)),
        }
    }
    Ok(out)
}

/// A stretch of hours over which a table held one status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusRun {
    pub first: i32,
    pub last: i32,
    /// An unavailable table's reason is its string id: its text is read with [`at`] at the run's
    /// first hour.
    pub status: Status,
}

/// [`reads`] less the hours `table` was not available (its [`statuses`]): an hour read but refused
/// holds no rows, and is not a read of them.
pub fn available_reads(recording: &Recording, run: u32, table: TableId, from: i32, to: i32) -> io::Result<Vec<(i32, i32)>> {
    let reads = reads(recording, run, table, from, to)?;
    let available: Vec<(i32, i32)> = statuses(recording, run, table, from, to)?
        .into_iter()
        .filter(|held| crate::columnar_frames::available(held.status))
        .map(|held| (held.first, held.last))
        .collect();
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < reads.len() && j < available.len() {
        let (first, last) = (reads[i].0.max(available[j].0), reads[i].1.min(available[j].1));
        if first <= last {
            out.push((first, last));
        }
        if reads[i].1 < available[j].1 { i += 1 } else { j += 1 }
    }
    Ok(out)
}

/// The statuses of `table` of run `run` from `from` through `to`, as runs of one status covering
/// the span, ascending: from the blocks' page entries, no row decoded.
pub fn statuses(recording: &Recording, run: u32, table: TableId, from: i32, to: i32) -> io::Result<Vec<StatusRun>> {
    let held = recording.run(run)?;
    let (from, to) = (from.max(held.first_hour), to.min(held.last_hour()));
    let (Some(first), Some(last)) = (held.ordinal(from), held.ordinal(to)) else { return Ok(Vec::new()) };
    if first > last {
        return Ok(Vec::new());
    }
    let blocks = held.place(first).0..held.place(last).0 + 1;
    let found = blocks
        .into_par_iter()
        .map(|block| -> io::Result<Vec<StatusRun>> {
            let entry = recording.table_entry(run, block, table.index())?.unit;
            let mut runs = Vec::new();
            unit::reader::statuses(&entry, held.block(block as usize).1 as u16, held.block_first_hour(block), &mut |start, end, status| {
                let (first, last) = (start.max(from), end.min(to));
                if first <= last {
                    runs.push(StatusRun { first, last, status });
                }
            })?;
            Ok(runs)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let mut out: Vec<StatusRun> = Vec::new();
    for run in found.into_iter().flatten() {
        match out.last_mut() {
            Some(held) if held.status == run.status && held.last + 1 == run.first => held.last = run.last,
            _ => out.push(run),
        }
    }
    Ok(out)
}

/// The hours of the six-hour buckets in which a log of run `run` inserted rows from `from`
/// through `to`, as runs of first and last hour clipped to the span, ascending: from its level-0
/// summary pieces, no row decoded.
pub fn logged_hours(recording: &Recording, run: u32, table: TableId, from: i32, to: i32) -> io::Result<Vec<(i32, i32)>> {
    let held = recording.run(run)?;
    let (from, to) = (from.max(held.first_hour), to.min(held.last_hour()));
    if from > to {
        return Ok(Vec::new());
    }
    let first = ordinal_of(recording, run, from)? / UNIT_ORDINALS;
    let last = ordinal_of(recording, run, to)? / UNIT_ORDINALS;
    let extents = pieces(recording, run, table.index(), first..last + 1, 0)?;
    let found = extents
        .par_iter()
        .map(|extent| -> io::Result<Vec<(i64, i64)>> {
            let bytes = recording.stored_bytes(extent)?;
            let piece = Piece::read(&bytes)?;
            let frame = piece.column(0)?;
            let size = summary::bucket_hours(piece.directory.level);
            let mut held = vec![false; piece.directory.buckets as usize];
            for at in 0..piece.keys.len() {
                for (bucket, count) in frame.counts(&piece, at)?.into_iter().enumerate() {
                    if count > 0
                        && let Some(held) = held.get_mut(bucket)
                    {
                        *held = true;
                    }
                }
            }
            Ok(held
                .iter()
                .enumerate()
                .filter(|(_, held)| **held)
                .map(|(bucket, _)| {
                    let start = (piece.directory.first_bucket + bucket as i64) * size;
                    (start, start + size - 1)
                })
                .collect())
        })
        .collect::<io::Result<Vec<_>>>()?;
    let mut buckets: Vec<(i64, i64)> = found.into_iter().flatten().collect();
    buckets.sort_unstable();
    buckets.dedup();
    let mut out: Vec<(i32, i32)> = Vec::new();
    for (start, end) in buckets {
        let (start, end) = (start.max(i64::from(from)), end.min(i64::from(to)));
        if start > end {
            continue;
        }
        let (start, end) = (start as i32, end as i32);
        match out.last_mut() {
            Some((_, last)) if *last + 1 >= start => *last = (*last).max(end),
            _ => out.push((start, end)),
        }
    }
    Ok(out)
}

/// The summary pieces of `table` of run `run` covering the blocks `blocks`, each of the highest
/// level up to `level` the file holds for its part: whole spans' pieces from their span rows, a
/// finished run's last span from its directory's tail, the rest from their blocks'. A value
/// table's blocks hold none: a part no piece covers is the walk's.
pub(crate) fn pieces(recording: &Recording, run: u32, table: usize, blocks: std::ops::Range<u32>, level: u8) -> io::Result<Vec<Extent>> {
    let held = recording.run(run)?;
    let closed = held.directory().spans.len() as u32 * SPAN_BLOCKS;
    let mut out = Vec::new();
    cover(recording, run, table, blocks.start..blocks.end.min(closed), level, closed, &mut out)?;
    if blocks.end > closed {
        let tail = held.directory().tail.iter().find(|(held, _)| *held as usize == table);
        match tail {
            Some((_, extent)) if level > 0 => out.push(*extent),
            _ => cover(recording, run, table, blocks.start.max(closed)..blocks.end, 0, closed, &mut out)?,
        }
    }
    Ok(out)
}

fn cover(
    recording: &Recording,
    run: u32,
    table: usize,
    blocks: std::ops::Range<u32>,
    level: u8,
    closed: u32,
    out: &mut Vec<Extent>,
) -> io::Result<()> {
    if blocks.is_empty() {
        return Ok(());
    }
    if level == 0 {
        let found = blocks
            .into_par_iter()
            .map(|block| recording.block_of(run, block as usize, table))
            .collect::<io::Result<Vec<_>>>()?;
        out.extend(found.into_iter().flatten());
        return Ok(());
    }
    // A level whose pieces span more blocks than the run has closed holds none yet.
    let group = u64::from(SPAN_BLOCKS) * (FANOUT as u64).saturating_pow(u32::from(level) - 1);
    if group > u64::from(closed) {
        return cover(recording, run, table, blocks, level - 1, closed, out);
    }
    let group = group as u32;
    let groups: Vec<u32> = (blocks.start - blocks.start % group..blocks.end).step_by(group as usize).collect();
    // Each closed group's span row read on its own thread.
    let found = groups
        .par_iter()
        .map(|&start| -> io::Result<Option<Extent>> {
            let end = start + group;
            if end > closed {
                return Ok(None);
            }
            Ok(recording
                .span_summaries(run, end / SPAN_BLOCKS - 1, table)?
                .into_iter()
                .find(|(held, _)| *held == level)
                .map(|(_, extent)| extent))
        })
        .collect::<io::Result<Vec<_>>>()?;
    for (&start, piece) in groups.iter().zip(found) {
        let part = start.max(blocks.start)..(start + group).min(blocks.end);
        match piece {
            Some(extent) => out.push(extent),
            // A group whose pieces held no key, or not closed yet, is read from the level below.
            None => cover(recording, run, table, part, level - 1, closed, out)?,
        }
    }
    Ok(())
}

/// The ordinal ranges of run `run`'s blocks `blocks` that none of `covered` (game-hour ranges
/// of pieces read) holds, clipped to ordinals `first..=last`.
fn uncovered(recording: &Recording, run: u32, blocks: std::ops::Range<u32>, covered: &[(i64, i64)], first: u32, last: u32) -> io::Result<Vec<(u32, u32)>> {
    let held = recording.run(run)?;
    let first_hour = i64::from(held.first_hour);
    let mut out: Vec<(u32, u32)> = Vec::new();
    for block in blocks {
        let (start, hours) = held.block(block as usize);
        let (from, to) = (first_hour + i64::from(start), first_hour + i64::from(start + hours));
        if covered.iter().any(|&(begin, end)| begin <= from && to <= end) {
            continue;
        }
        let (a, b) = (start.max(first), (start + hours - 1).min(last));
        if a > b {
            continue;
        }
        match out.last_mut() {
            Some((_, end)) if *end + 1 == a => *end = b,
            _ => out.push((a, b)),
        }
    }
    Ok(out)
}

/// One key's bin: what its hours with a value held. None held a value when `hours` is 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bin {
    /// Hours with a value.
    pub hours: u32,
    pub first: i64,
    pub last: i64,
    pub min: i64,
    pub max: i64,
    /// The level-0 buckets of the first, last, least and greatest value; read from a coarser level,
    /// the first level-0 bucket of the bucket holding the first, least and greatest, the last of the last.
    pub first_bucket: i32,
    pub last_bucket: i32,
    pub min_bucket: i32,
    pub max_bucket: i32,
}

impl Bin {
    /// `fold`, the hours after those it holds, added.
    fn add(&mut self, fold: &Fold) {
        let bucket = |bucket: i64| bucket as i32;
        if self.hours == 0 {
            *self = Self {
                hours: fold.hours,
                first: fold.first,
                last: fold.last,
                min: fold.min,
                max: fold.max,
                first_bucket: bucket(fold.first_bucket),
                last_bucket: bucket(fold.last_bucket),
                min_bucket: bucket(fold.min_bucket),
                max_bucket: bucket(fold.max_bucket),
            };
            return;
        }
        if fold.min < self.min {
            (self.min, self.min_bucket) = (fold.min, bucket(fold.min_bucket));
        }
        if fold.max > self.max {
            (self.max, self.max_bucket) = (fold.max, bucket(fold.max_bucket));
        }
        (self.last, self.last_bucket) = (fold.last, bucket(fold.last_bucket));
        self.hours += fold.hours;
    }
}

/// A bin's aggregates, for a column that declares them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    /// The hour-weighted sum of the value, and of its magnitude.
    pub sum: i128,
    pub abs: u128,
    /// Hour-to-hour decreases and changes.
    pub falls: u64,
    pub changes: u64,
}

impl Totals {
    fn add(&mut self, fold: &Fold) {
        self.sum += fold.sum;
        self.abs += fold.abs;
        self.falls += fold.falls;
        self.changes += fold.changes;
    }
}

/// A chart of one column: per key, each bin's values, aggregates and covered hours, key after
/// key in one buffer.
#[derive(Debug)]
pub struct Chart {
    /// The summary level read; [`WALKED`] when the hours were walked.
    pub level: u8,
    /// Hours per bin, and the first bin's first hour.
    pub bin_hours: i64,
    pub first_bin_hour: i64,
    pub bins: usize,
    pub keys: Vec<u128>,
    values: Vec<Bin>,
    /// Empty when the column declares no aggregates.
    totals: Vec<Totals>,
    life: Vec<u32>,
    /// Pieces read.
    pub pieces: usize,
    /// Blocks walked where no piece answered.
    pub walked: usize,
}

impl Chart {
    /// The `at`th key's bins.
    pub fn values(&self, at: usize) -> &[Bin] {
        &self.values[at * self.bins..(at + 1) * self.bins]
    }

    /// The `at`th key's bins' aggregates; none when the column declares none.
    pub fn totals(&self, at: usize) -> Option<&[Totals]> {
        (!self.totals.is_empty()).then(|| &self.totals[at * self.bins..(at + 1) * self.bins])
    }

    /// The `at`th key's covered hours by bin.
    pub fn life(&self, at: usize) -> &[u32] {
        &self.life[at * self.bins..(at + 1) * self.bins]
    }

    /// The row of `key`, made when it has none.
    fn row(&mut self, key: u128, rows: &mut HashMap<u128, usize, foldhash::fast::RandomState>) -> usize {
        let bins = self.bins;
        let declares = !self.totals.is_empty();
        *rows.entry(key).or_insert_with(|| {
            self.keys.push(key);
            self.values.resize(self.values.len() + bins, Bin::default());
            self.life.resize(self.life.len() + bins, 0);
            if declares {
                self.totals.resize(self.totals.len() + bins, Totals::default());
            }
            self.keys.len() - 1
        })
    }

    /// `fold` added to the `at`th key's bin `bin`.
    fn add(&mut self, at: usize, bin: usize, fold: &Fold) {
        let index = at * self.bins + bin;
        self.values[index].add(fold);
        if let Some(totals) = self.totals.get_mut(index) {
            totals.add(fold);
        }
    }
}

/// Column `source` (a column, or a computed column's id) of value table `table` of run `run` from
/// `from` through `to` drawn `width` bins wide, for `keys` (every key when none), on the absolute
/// grid. Bins of the coarse summary level's buckets or wider are read from summary pieces, a part
/// no piece covers walked; finer bins walk the blocks in view.
#[allow(clippy::too_many_arguments, reason = "a lane query names its run, table, columns, keys and range")]
pub fn chart(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: u32,
    keys: Option<&[u128]>,
    from: i32,
    to: i32,
    width: u32,
) -> io::Result<Chart> {
    let per_pixel = ((i64::from(to) - i64::from(from) + 1) / i64::from(width.max(1))).max(1);
    if per_pixel < summary::bucket_hours(0) {
        return chart_at(recording, run, table, source, keys, from, to, WALKED, per_pixel);
    }
    let mut level = 0u8;
    while summary::bucket_hours(level + 1) <= per_pixel {
        level += 1;
    }
    let bin_hours = (per_pixel / summary::bucket_hours(level)) * summary::bucket_hours(level);
    chart_at(recording, run, table, source, keys, from, to, level, bin_hours)
}

/// The level [`chart_at`] answers from the table's lane alone, walked.
pub const WALKED: u8 = u8::MAX;

/// [`chart`] in bins of `bin_hours`: from pieces of `level` (or the levels below where the file
/// holds none), `bin_hours` a multiple of its bucket, the hours no piece covers walked; or with
/// [`WALKED`], every hour walked.
#[allow(clippy::too_many_arguments, reason = "a lane query names its run, table, columns, keys and range")]
pub fn chart_at(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: u32,
    keys: Option<&[u128]>,
    from: i32,
    to: i32,
    level: u8,
    bin_hours: i64,
) -> io::Result<Chart> {
    let first_bin_hour = i64::from(from).div_euclid(bin_hours) * bin_hours;
    let bins = ((i64::from(to) - first_bin_hour) / bin_hours + 1) as usize;
    let (first, last) = (ordinal_of(recording, run, from)?, ordinal_of(recording, run, to)?);
    let blocks = first / UNIT_ORDINALS..last / UNIT_ORDINALS + 1;
    let extents = match level {
        WALKED => Vec::new(),
        _ => pieces(recording, run, table.index(), blocks.clone(), level)?,
    };
    let registry = crate::summaries::registry()?;
    let Some((_, Shape::Values(columns))) = registry.iter().find(|(held, _)| *held == table.index()) else {
        return Err(invalid("The table has no summarized columns"));
    };
    let column = *columns
        .iter()
        .find(|column| column.source.id() == source)
        .ok_or_else(|| invalid("The summary holds no such column"))?;
    let mut chart = Chart {
        level,
        bin_hours,
        first_bin_hour,
        bins,
        keys: Vec::new(),
        values: Vec::new(),
        totals: Vec::new(),
        life: Vec::new(),
        pieces: extents.len(),
        walked: 0,
    };
    let mut rows: HashMap<u128, usize, foldhash::fast::RandomState> = HashMap::default();
    // Every piece read, then each one's column decoded on its own thread.
    let read = extents.par_iter().map(|extent| recording.stored_bytes(extent)).collect::<io::Result<Vec<_>>>()?;
    let decoded = read
        .par_iter()
        .map(|bytes| -> io::Result<(Piece<'_>, ColumnFrame)> {
            let piece = Piece::read(bytes)?;
            let column = piece.column_of(source).ok_or_else(|| invalid("The summary holds no such column"))?;
            let frame = piece.column(column)?;
            Ok((piece, frame))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let covered: Vec<(i64, i64)> = decoded
        .iter()
        .map(|(piece, _)| (piece.directory.first_hour, piece.directory.first_hour + i64::from(piece.directory.hours)))
        .collect();
    let mut asked: Vec<u128> = match keys {
        Some(keys) => keys.iter().copied().filter(|&key| decoded.iter().any(|(piece, _)| piece.key_at(key).is_some())).collect(),
        None => decoded.iter().flat_map(|(piece, _)| piece.keys.iter().copied()).collect(),
    };
    asked.sort_unstable();
    asked.dedup();
    let grid = Grid {
        from: i64::from(from),
        to: i64::from(to),
        first_bin_hour,
        bin_hours,
        bins,
    };
    // Each key's bins on its own thread, into its own slice: nothing is merged across threads.
    chart.values = vec![Bin::default(); asked.len() * bins];
    chart.life = vec![0; asked.len() * bins];
    if column.aggregates != 0 {
        chart.totals = vec![Totals::default(); asked.len() * bins];
        chart
            .values
            .par_chunks_mut(bins.max(1))
            .zip(chart.life.par_chunks_mut(bins.max(1)))
            .zip(chart.totals.par_chunks_mut(bins.max(1)))
            .zip(asked.par_iter())
            .try_for_each(|(((values, life), totals), &key)| grid.fold_key(&decoded, key, values, life, Some(totals), None))?;
    } else {
        chart
            .values
            .par_chunks_mut(bins.max(1))
            .zip(chart.life.par_chunks_mut(bins.max(1)))
            .zip(asked.par_iter())
            .try_for_each(|((values, life), &key)| grid.fold_key(&decoded, key, values, life, None, None))?;
    }
    for (at, &key) in asked.iter().enumerate() {
        rows.insert(key, at);
    }
    chart.keys = asked;
    let ranges = uncovered(recording, run, blocks, &covered, first, last)?;
    if !ranges.is_empty() {
        if column.aggregates != 0 && chart.totals.is_empty() {
            chart.totals = vec![Totals::default(); chart.values.len()];
        }
        let mut walk = Walk::new(Some(column.source), keys, first_bin_hour, bin_hours, bins);
        chart.walked = walk_ranges(recording, run, table, &inputs(column.source), &ranges, &mut walk)?;
        let mut walked: Vec<(u128, Row)> = walk.rows.into_iter().collect();
        walked.sort_unstable_by_key(|(key, _)| *key);
        for (key, folded) in walked {
            let row = chart.row(key, &mut rows);
            for bin in 0..bins {
                chart.life[row * bins + bin] += folded.life[bin];
                if let Some(fold) = folded.fold(bin) {
                    chart.add(row, bin, &fold);
                }
            }
        }
    }
    Ok(chart)
}

/// Every key `table` of run `run` held, with the first and last hour it was covered, from the
/// highest summary pieces the file holds, a value table's hours no piece covers walked.
pub fn lifetimes(recording: &Recording, run: u32, table: TableId) -> io::Result<Vec<(u128, i64, i64)>> {
    let held = recording.run(run)?;
    let blocks = held.pages() as u32;
    let extents = pieces(recording, run, table.index(), 0..blocks, 8)?;
    let mut spans: HashMap<u128, (i64, i64), foldhash::fast::RandomState> = HashMap::default();
    let mut covered = Vec::new();
    for extent in &extents {
        let bytes = recording.stored_bytes(extent)?;
        let piece = Piece::read(&bytes)?;
        let size = summary::bucket_hours(piece.directory.level);
        covered.push((piece.directory.first_hour, piece.directory.first_hour + i64::from(piece.directory.hours)));
        for (at, &key) in piece.keys.iter().enumerate() {
            for (bucket, life) in piece.life(at)?.iter().enumerate() {
                if life.hours == 0 {
                    continue;
                }
                let start = (piece.directory.first_bucket + bucket as i64) * size;
                let (first, last) = (start + i64::from(life.first), start + i64::from(life.end) - 1);
                let span = spans.entry(key).or_insert((first, last));
                span.0 = span.0.min(first);
                span.1 = span.1.max(last);
            }
        }
    }
    let values = matches!(
        crate::summaries::registry()?.iter().find(|(held, _)| *held == table.index()),
        Some((_, Shape::Values(_)))
    );
    if values && blocks > 0 {
        let ranges = uncovered(recording, run, 0..blocks, &covered, 0, held.ordinals - 1)?;
        if !ranges.is_empty() {
            let mut walk = Walk::new(None, None, i64::from(held.first_hour), i64::from(held.ordinals), 1);
            walk_ranges(recording, run, table, &[], &ranges, &mut walk)?;
            for (key, row) in walk.rows {
                if let Some((first, last)) = row.lived {
                    let span = spans.entry(key).or_insert((first, last));
                    span.0 = span.0.min(first);
                    span.1 = span.1.max(last);
                }
            }
        }
    }
    let mut out: Vec<(u128, i64, i64)> = spans.into_iter().map(|(key, (first, last))| (key, first, last)).collect();
    out.sort_unstable();
    Ok(out)
}

/// The columns a summarized column's value reads.
fn inputs(source: summary::Source) -> Vec<usize> {
    match source {
        summary::Source::Cell(column) => vec![column],
        summary::Source::Computed { inputs, .. } => (0..64).filter(|bit| inputs >> bit & 1 != 0).collect(),
    }
}

/// Reads `table`'s `columns` over each of run `run`'s ordinal ranges `ranges` into `walk`, from
/// their tracks ([`crate::points`]); the blocks read.
fn walk_ranges(recording: &Recording, run: u32, table: TableId, columns: &[usize], ranges: &[(u32, u32)], walk: &mut Walk<'_>) -> io::Result<usize> {
    let first_hour = recording.run(run)?.first_hour;
    let mut blocks = 0;
    for &(first, last) in ranges {
        blocks += (last / UNIT_ORDINALS - first / UNIT_ORDINALS + 1) as usize;
        let start = i64::from(first_hour) + i64::from(first);
        walk.tracks.clear();
        // From the hour before the range, its state is where the walk starts; at a run's first
        // hour nothing is held before.
        let before = first.checked_sub(1);
        let from = first_hour + before.unwrap_or(0) as i32;
        let to = first_hour + last as i32;
        crate::points::points(recording, run, table, columns, walk.source, Keys::All, from, to, &mut |point| {
            if before.is_some() && point.hour == from {
                walk.begin(point.key, point.covered, point.value, start);
            } else {
                walk.point(point.key, point.covered, point.value, i64::from(point.hour));
            }
            true
        })?;
        walk.end(start + i64::from(last - first) + 1);
    }
    Ok(blocks)
}

/// One key's bins as a walk folds them.
struct Row {
    values: Vec<Option<Fold>>,
    life: Vec<u32>,
    /// Changes and falls counted at the hour they happen, by bin.
    counts: Vec<(u64, u64)>,
    /// The first and last hour it was covered.
    lived: Option<(i64, i64)>,
}

impl Row {
    fn fold(&self, bin: usize) -> Option<Fold> {
        let mut fold = self.values[bin]?;
        fold.changes += self.counts[bin].0;
        fold.falls += self.counts[bin].1;
        Some(fold)
    }
}

/// One key as a walk passes its hours: whether it is covered since when, and its value since
/// when with the value the hour before.
struct Track {
    covered: bool,
    since: i64,
    value: Option<i64>,
    value_since: i64,
    before: Option<i64>,
}

/// A table's hours folded into bins as a walk passes them, exactly as the summary builder folds
/// them: a key is covered while live, its table available and, read in part, read; a value is
/// none where not covered or not recorded; a change counts where both sides hold a value.
struct Walk<'a> {
    source: Option<summary::Source>,
    keys: Option<&'a [u128]>,
    first_bin: i64,
    bin_hours: i64,
    bins: usize,
    tracks: HashMap<u128, Track, foldhash::fast::RandomState>,
    rows: HashMap<u128, Row, foldhash::fast::RandomState>,
}

impl<'a> Walk<'a> {
    fn new(source: Option<summary::Source>, keys: Option<&'a [u128]>, first_bin: i64, bin_hours: i64, bins: usize) -> Self {
        Self {
            source,
            keys,
            first_bin,
            bin_hours,
            bins,
            tracks: HashMap::default(),
            rows: HashMap::default(),
        }
    }

    fn wanted(&self, key: u128) -> bool {
        self.keys.is_none_or(|keys| keys.contains(&key))
    }

    fn row(&mut self, key: u128) -> &mut Row {
        let bins = self.bins;
        self.rows.entry(key).or_insert_with(|| Row {
            values: vec![None; bins],
            life: vec![0; bins],
            counts: vec![(0, 0); bins],
            lived: None,
        })
    }

    /// Each bin's part of the hours `[from, to)`, as (bin, start, end).
    fn parts(&self, from: i64, to: i64) -> Vec<(usize, i64, i64)> {
        let mut out = Vec::new();
        let mut at = from.max(self.first_bin);
        let last = self.first_bin + self.bins as i64 * self.bin_hours;
        while at < to.min(last) {
            let bin = (at - self.first_bin) / self.bin_hours;
            let end = (self.first_bin + (bin + 1) * self.bin_hours).min(to);
            out.push((bin as usize, at, end));
            at = end;
        }
        out
    }

    fn add_life(&mut self, key: u128, from: i64, to: i64) {
        if to <= from {
            return;
        }
        let parts = self.parts(from, to);
        let row = self.row(key);
        for (bin, start, end) in parts {
            row.life[bin] += (end - start) as u32;
        }
        let lived = row.lived.get_or_insert((from, to - 1));
        lived.0 = lived.0.min(from);
        lived.1 = lived.1.max(to - 1);
    }

    fn add_value(&mut self, key: u128, value: i64, from: i64, to: i64) {
        let parts = self.parts(from, to);
        let row = self.row(key);
        for (bin, start, end) in parts {
            let mut part = Fold::constant(value, (end - start) as u32, summary::bucket_of(start, 0));
            part.last_bucket = summary::bucket_of(end - 1, 0);
            match &mut row.values[bin] {
                Some(held) => held.merge(&part),
                slot @ None => *slot = Some(part),
            }
        }
    }

    fn cover(&mut self, key: u128, covered: bool, hour: i64) {
        let track = self.tracks.get_mut(&key).expect("a track");
        if track.covered == covered {
            return;
        }
        let (was, since) = (track.covered, track.since);
        track.covered = covered;
        track.since = hour;
        if was {
            self.add_life(key, since, hour);
        }
    }

    fn set(&mut self, key: u128, value: Option<i64>, hour: i64) {
        let track = self.tracks.get_mut(&key).expect("a track");
        if track.value == value && track.value_since <= hour {
            return;
        }
        let before = if track.value_since < hour { track.value } else { track.before };
        let (held, since) = (track.value, track.value_since);
        track.before = before;
        track.value = value;
        track.value_since = hour;
        if let Some(held) = held {
            self.add_value(key, held, since, hour);
        }
        if let (Some(before), Some(now)) = (before, value)
            && before != now
            && let Some(&(bin, _, _)) = self.parts(hour, hour + 1).first()
        {
            let counts = &mut self.row(key).counts[bin];
            counts.0 += 1;
            counts.1 += u64::from(now < before);
        }
    }

    fn track(&mut self, key: u128, hour: i64) {
        self.tracks.entry(key).or_insert(Track {
            covered: false,
            since: hour,
            value: None,
            value_since: hour,
            before: None,
        });
    }

    /// A key's state the hour before the walk's first, `hour`.
    fn begin(&mut self, key: u128, covered: bool, value: Option<i64>, hour: i64) {
        if !covered || !self.wanted(key) {
            return;
        }
        let value = self.source.and(value);
        self.tracks.insert(key, Track { covered: true, since: hour, value, value_since: hour, before: value });
    }

    /// A key's state from `hour` on.
    fn point(&mut self, key: u128, covered: bool, value: Option<i64>, hour: i64) {
        if !self.wanted(key) {
            return;
        }
        self.track(key, hour);
        self.cover(key, covered, hour);
        if self.source.is_some() {
            self.set(key, if covered { value } else { None }, hour);
        }
    }

    /// The walk ends before `hour`.
    fn end(&mut self, hour: i64) {
        let keys: Vec<u128> = self.tracks.keys().copied().collect();
        for key in keys {
            self.cover(key, false, hour);
            self.set(key, None, hour);
        }
        self.tracks.clear();
    }
}

/// The rows a log of run `run` inserted from `from` through `to`, by facet key, counted per bin of
/// `bin_hours` hours (a multiple of six) on the absolute grid.
pub fn log(recording: &Recording, run: u32, table: TableId, from: i32, to: i32, bin_hours: i64) -> io::Result<Vec<(u128, Vec<u64>)>> {
    log_of(recording, run, table, (from, to), bin_hours, &|_| true)
}

/// [`log`] of the facet keys `keep` takes: no other key's counts are decoded or held.
pub fn log_of(recording: &Recording, run: u32, table: TableId, (from, to): (i32, i32), bin_hours: i64, keep: &dyn Fn(u128) -> bool) -> io::Result<Vec<(u128, Vec<u64>)>> {
    let bin_hours = bin_hours.max(summary::bucket_hours(0));
    let mut level = 0u8;
    while summary::bucket_hours(level + 1) <= bin_hours && bin_hours % summary::bucket_hours(level + 1) == 0 {
        level += 1;
    }
    let first_bin = i64::from(from).div_euclid(bin_hours) * bin_hours;
    let bins = ((i64::from(to) - first_bin) / bin_hours + 1) as usize;
    let first = ordinal_of(recording, run, from)? / UNIT_ORDINALS;
    let last = ordinal_of(recording, run, to)? / UNIT_ORDINALS;
    let extents = pieces(recording, run, table.index(), first..last + 1, level)?;
    let mut counts: HashMap<u128, Vec<u64>, foldhash::fast::RandomState> = HashMap::default();
    for extent in &extents {
        let bytes = recording.stored_bytes(extent)?;
        let piece = Piece::read(&bytes)?;
        let frame = piece.column(0)?;
        let size = summary::bucket_hours(piece.directory.level);
        for (at, &key) in piece.keys.iter().enumerate() {
            if !keep(key) {
                continue;
            }
            for (bucket, count) in frame.counts(&piece, at)?.into_iter().enumerate() {
                let start = (piece.directory.first_bucket + bucket as i64) * size;
                if count == 0 || start + size <= i64::from(from) || start > i64::from(to) {
                    continue;
                }
                let bin = ((start - first_bin).div_euclid(bin_hours)) as usize;
                if bin < bins {
                    counts.entry(key).or_insert_with(|| vec![0; bins])[bin] += count;
                }
            }
        }
    }
    let mut out: Vec<(u128, Vec<u64>)> = counts.into_iter().collect();
    out.sort_unstable_by_key(|(key, _)| *key);
    Ok(out)
}

/// The first hour after `from` at which column `column` of `key` of `table` of run `run` holds
/// another value than at `from` (or holds one, or stops holding one); none through the run's end.
/// The coarse summary pieces pass the buckets it holds steady in; the table's lane is walked from
/// the first that is not, or from where no piece covers.
pub fn next(recording: &Recording, run: u32, table: TableId, column: usize, key: u128, from: i32) -> io::Result<Option<i32>> {
    let held = recording.run(run)?;
    let last_hour = held.first_hour + (held.ordinals - 1) as i32;
    if from >= last_hour {
        return Ok(None);
    }
    let start = at_keys(recording, run, table, from, Some(&[column]), Keys::These(&[key]))?;
    let now = {
        let available = matches!(start.status, Status::Complete | Status::Truncated { .. });
        let view = start.view();
        available.then(|| view.slot(key).and_then(|slot| view.number(column, slot))).flatten()
    };
    // The first hour not yet known to hold `now`.
    let mut cursor = i64::from(from) + 1;
    let first_block = ordinal_of(recording, run, from + 1)? / UNIT_ORDINALS;
    for extent in pieces(recording, run, table.index(), first_block..held.pages() as u32, 1)? {
        let bytes = recording.stored_bytes(&extent)?;
        let piece = Piece::read(&bytes)?;
        let directory = &piece.directory;
        if directory.first_hour > cursor {
            break;
        }
        let size = summary::bucket_hours(directory.level);
        // A column the summaries do not hold is read from its tracks alone.
        let Some(column_at) = piece.column_of(column as u32) else { break };
        let frame = piece.column(column_at)?;
        let mut tokens = piece.key_at(key).map(|at| frame.tokens(at));
        for bucket in 0..directory.buckets {
            let index = directory.first_bucket + i64::from(bucket);
            let covered = directory.covered(bucket);
            let fold = match &mut tokens {
                Some(tokens) => tokens.next(index, covered)?,
                None => None,
            };
            let begin = index * size;
            let end = (begin + size).min(directory.first_hour + i64::from(directory.hours));
            if end <= cursor {
                continue;
            }
            let steady = match (fold, now) {
                (None, None) => true,
                (Some(fold), Some(now)) => fold.first == now && fold.last == now && fold.min == now && fold.max == now && fold.hours == covered,
                _ => false,
            };
            if !steady {
                return walk_to_change(recording, run, table, column, key, (begin.max(cursor) - 1) as i32, last_hour);
            }
            cursor = end;
        }
    }
    walk_to_change(recording, run, table, column, key, (cursor - 1) as i32, last_hour)
}

/// The first hour after `after` at which `key`'s column `column` of `table` holds another value
/// than at `after` (a row gone or the table unavailable holding none), from the column's tracks.
fn walk_to_change(recording: &Recording, run: u32, table: TableId, column: usize, key: u128, after: i32, last_hour: i32) -> io::Result<Option<i32>> {
    if after >= last_hour {
        return Ok(None);
    }
    let (mut held, mut found) = (None, None);
    crate::points::points(recording, run, table, &[column], Some(summary::Source::Cell(column)), Keys::These(&[key]), after, last_hour, &mut |point| {
        let now = if point.covered { point.value } else { None };
        if point.hour == after {
            held = now;
        } else if now != held {
            found = Some(point.hour);
            return false;
        }
        true
    })?;
    Ok(found)
}

/// The bins a chart folds a piece's buckets into.
struct Grid {
    from: i64,
    to: i64,
    first_bin_hour: i64,
    bin_hours: i64,
    bins: usize,
}

impl Grid {
    /// The bin piece bucket `bucket` of `directory` falls in; none outside the view.
    fn bin(&self, directory: &summary::Directory, bucket: u32) -> Option<usize> {
        let size = summary::bucket_hours(directory.level);
        let start = (directory.first_bucket + i64::from(bucket)) * size;
        if start + size <= self.from || start > self.to {
            return None;
        }
        usize::try_from((start - self.first_bin_hour).div_euclid(self.bin_hours))
            .ok()
            .filter(|&bin| bin < self.bins)
    }

    /// `key`'s bins, aggregates when asked and covered hours from `pieces`, into `values`,
    /// `totals` and `life`, and when asked into `marks` the [`mark`] of the bucket of each bin's
    /// last value: a run of buckets without a value is passed, a run holding one value added once
    /// per bin it meets.
    fn fold_key(
        &self,
        pieces: &[(Piece<'_>, ColumnFrame)],
        key: u128,
        values: &mut [Bin],
        life: &mut [u32],
        mut totals: Option<&mut [Totals]>,
        mut marks: Option<&mut [u64]>,
    ) -> io::Result<()> {
        // A piece's folds name buckets of its level: each made level-0, the last value at its
        // bucket's last level-0 bucket, the others at its first.
        let mut merge = |bin: usize, mut fold: Fold, level: u8, at: u64| {
            if let Some(marks) = marks.as_deref_mut() {
                marks[bin] = at;
            }
            let ratio = summary::bucket_hours(level) / summary::bucket_hours(0);
            fold.first_bucket *= ratio;
            fold.min_bucket *= ratio;
            fold.max_bucket *= ratio;
            fold.last_bucket = (fold.last_bucket + 1) * ratio - 1;
            values[bin].add(&fold);
            if let Some(totals) = totals.as_deref_mut() {
                totals[bin].add(&fold);
            }
        };
        for (place, (piece, frame)) in pieces.iter().enumerate() {
            let Some(at) = piece.key_at(key) else { continue };
            let directory = &piece.directory;
            let buckets = directory.buckets;
            let run = |count: u64, at: u32| -> io::Result<u32> {
                u32::try_from(count)
                    .ok()
                    .filter(|&count| count <= buckets - at)
                    .ok_or_else(|| invalid("A key's tokens pass its piece's buckets"))
            };
            let mut tokens = frame.tokens(at);
            let mut bucket = 0;
            while bucket < buckets {
                let index = directory.first_bucket + i64::from(bucket);
                match tokens.span(index, directory.covered(bucket))? {
                    summary::Span::None(count) => bucket += run(count, bucket)?,
                    summary::Span::One(fold) => {
                        if let Some(bin) = self.bin(directory, bucket) {
                            merge(bin, fold, directory.level, mark(place, bucket));
                        }
                        bucket += 1;
                    }
                    summary::Span::Same(value, count) => {
                        let end = bucket + run(count, bucket)?;
                        while bucket < end {
                            let bin = self.bin(directory, bucket);
                            let (start, mut hours) = (bucket, directory.covered(bucket));
                            bucket += 1;
                            while bucket < end && self.bin(directory, bucket) == bin {
                                hours += directory.covered(bucket);
                                bucket += 1;
                            }
                            if let Some(bin) = bin {
                                let mut fold = Fold::constant(value, hours, directory.first_bucket + i64::from(start));
                                fold.last_bucket = directory.first_bucket + i64::from(bucket - 1);
                                merge(bin, fold, directory.level, mark(place, bucket - 1));
                            }
                        }
                    }
                }
            }
            let mut lives = piece.lives(at);
            let mut bucket = 0;
            while bucket < buckets {
                match lives.span()? {
                    summary::LifeSpan::Empty(count) => bucket += run(count, bucket)?,
                    summary::LifeSpan::Full(count) => {
                        let end = bucket + run(count, bucket)?;
                        for full in bucket..end {
                            if let Some(bin) = self.bin(directory, full) {
                                life[bin] += directory.covered(full);
                            }
                        }
                        bucket = end;
                    }
                    summary::LifeSpan::Part(lived) => {
                        if let Some(bin) = self.bin(directory, bucket) {
                            life[bin] += lived.hours;
                        }
                        bucket += 1;
                    }
                }
            }
        }
        Ok(())
    }
}

/// What [`top`] ranks a bin's keys by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    /// The value at the bin's last hour with one.
    Last,
    /// The greatest value in the bin.
    Max,
    /// The least value in the bin.
    Min,
}

impl By {
    fn of(self, bin: &Bin) -> i64 {
        match self {
            By::Last => bin.last,
            By::Max => bin.max,
            By::Min => bin.min,
        }
    }
}

/// Where bucket `bucket` of the `piece`th piece read lies among them all, in time order; 0 is
/// before any.
fn mark(piece: usize, bucket: u32) -> u64 {
    ((piece as u64) << 32 | u64::from(bucket)) + 1
}

/// The `n` keys of value table `table`'s column `source` of run `run` ranking first by `by` in
/// each bin of the view from `from` through `to` drawn `width` bins wide, greatest first (ties by
/// key), each with its bin: the top-N of every key without holding every key's bins. From the
/// view's summary pieces of six-hour bins or wider; none narrower.
///
/// By [`By::Last`] a bin ranks the keys holding a value in its last bucket read that holds any
/// (a bucket split between two pieces counts as two), by their last value there: a key gone
/// earlier in the bin is not ranked. A column whose pieces rank at least `n` keys per bucket is
/// read from those ranks and then only their keys' buckets; any other view folds every key.
#[allow(clippy::too_many_arguments, reason = "a lane query names its run, table, column, view, count and order")]
pub fn top(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: u32,
    from: i32,
    to: i32,
    width: u32,
    n: usize,
    by: By,
) -> io::Result<Vec<Vec<(u128, Bin)>>> {
    top_with(recording, run, table, source, from, to, width, n, by, true)
}

/// [`top`], from the column's ranks only when `ranked` allows it.
#[allow(clippy::too_many_arguments, reason = "a lane query names its run, table, column, view, count and order")]
fn top_with(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: u32,
    from: i32,
    to: i32,
    width: u32,
    n: usize,
    by: By,
    ranked: bool,
) -> io::Result<Vec<Vec<(u128, Bin)>>> {
    let per_pixel = ((i64::from(to) - i64::from(from) + 1) / i64::from(width.max(1))).max(1);
    if per_pixel < summary::bucket_hours(0) {
        return Err(invalid("A top-N view's bins are six hours or wider"));
    }
    let mut level = 0u8;
    while summary::bucket_hours(level + 1) <= per_pixel {
        level += 1;
    }
    let bin_hours = (per_pixel / summary::bucket_hours(level)) * summary::bucket_hours(level);
    let first_bin_hour = i64::from(from).div_euclid(bin_hours) * bin_hours;
    let bins = ((i64::from(to) - first_bin_hour) / bin_hours + 1) as usize;
    if n == 0 {
        return Ok(vec![Vec::new(); bins]);
    }
    let (first, last) = (ordinal_of(recording, run, from)?, ordinal_of(recording, run, to)?);
    let extents = pieces(recording, run, table.index(), first / UNIT_ORDINALS..last / UNIT_ORDINALS + 1, level)?;
    let read = extents.par_iter().map(|extent| recording.stored_bytes(extent)).collect::<io::Result<Vec<_>>>()?;
    let decoded = read
        .par_iter()
        .map(|bytes| -> io::Result<(Piece<'_>, ColumnFrame, usize)> {
            let piece = Piece::read(bytes)?;
            let column = piece.column_of(source).ok_or_else(|| invalid("The summary holds no such column"))?;
            let frame = piece.column(column)?;
            Ok((piece, frame, column))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let from_ranks = ranked && by == By::Last && !decoded.is_empty() && decoded.iter().all(|(piece, _, column)| usize::from(piece.rank(*column)) >= n);
    let ranks = match from_ranks {
        true => decoded
            .par_iter()
            .map(|(piece, _, column)| piece.ranks(*column)?.ok_or_else(|| invalid("A ranked column's piece holds no ranks")))
            .collect::<io::Result<Vec<_>>>()?,
        false => Vec::new(),
    };
    let decoded: Vec<(Piece<'_>, ColumnFrame)> = decoded.into_iter().map(|(piece, frame, _)| (piece, frame)).collect();
    let grid = Grid {
        from: i64::from(from),
        to: i64::from(to),
        first_bin_hour,
        bin_hours,
        bins,
    };
    match from_ranks {
        true => grid.top_ranked(&decoded, &ranks, n),
        false => grid.top_folded(&decoded, n, by),
    }
}

/// Greater by `by` first, then the lesser key.
fn better(by: By) -> impl Fn(&(u128, Bin), &(u128, Bin)) -> std::cmp::Ordering + Copy + Send + Sync {
    move |a, b| by.of(&b.1).cmp(&by.of(&a.1)).then(a.0.cmp(&b.0))
}

impl Grid {
    /// [`top`] from the ranks of `pieces` (each piece's own, in its order): each bin's last bucket
    /// ranking a key names its first `n` keys, whose bins alone are folded.
    fn top_ranked(&self, pieces: &[(Piece<'_>, ColumnFrame)], ranks: &[summary::Ranks], n: usize) -> io::Result<Vec<Vec<(u128, Bin)>>> {
        let mut chosen: Vec<Vec<u128>> = vec![Vec::new(); self.bins];
        for ((piece, _), ranks) in pieces.iter().zip(ranks) {
            for bucket in 0..piece.directory.buckets {
                let list = ranks.bucket(bucket);
                if list.is_empty() {
                    continue;
                }
                if let Some(bin) = self.bin(&piece.directory, bucket) {
                    chosen[bin] = list.iter().take(n).map(|&at| piece.keys[at as usize]).collect();
                }
            }
        }
        let mut keys: Vec<u128> = chosen.iter().flatten().copied().collect();
        keys.sort_unstable();
        keys.dedup();
        let folded = keys
            .par_iter()
            .map(|&key| -> io::Result<Vec<Bin>> {
                let (mut values, mut life) = (vec![Bin::default(); self.bins], vec![0u32; self.bins]);
                self.fold_key(pieces, key, &mut values, &mut life, None, None)?;
                Ok(values)
            })
            .collect::<io::Result<Vec<_>>>()?;
        let better = better(By::Last);
        Ok(chosen
            .into_iter()
            .enumerate()
            .map(|(bin, held)| {
                let mut list: Vec<(u128, Bin)> = held.into_iter().map(|key| (key, folded[keys.binary_search(&key).expect("a chosen key")][bin])).collect();
                list.sort_unstable_by(better);
                list
            })
            .collect())
    }

    /// [`top`] folding every key of `pieces`: each thread folds its keys one at a time into one
    /// scratch row and keeps its own best `n` per bin (by [`By::Last`], of the keys of the latest
    /// bucket it saw holding a value); the threads' lists are merged per bin.
    fn top_folded(&self, pieces: &[(Piece<'_>, ColumnFrame)], n: usize, by: By) -> io::Result<Vec<Vec<(u128, Bin)>>> {
        use std::cmp::Ordering;
        let mut keys: Vec<u128> = pieces.iter().flat_map(|(piece, _)| piece.keys.iter().copied()).collect();
        keys.sort_unstable();
        keys.dedup();
        let bins = self.bins;
        let better = better(by);
        let empty = || (vec![Bin::default(); bins], vec![0u32; bins], vec![0u64; bins], vec![(0u64, Vec::<(u128, Bin)>::new()); bins]);
        let (_, _, _, best) = keys
            .par_iter()
            .try_fold(empty, |(mut values, mut life, mut marks, mut best), &key| -> io::Result<_> {
                values.fill(Bin::default());
                marks.fill(0);
                self.fold_key(pieces, key, &mut values, &mut life, None, Some(&mut marks))?;
                for (bin, held) in values.iter().enumerate() {
                    if held.hours == 0 {
                        continue;
                    }
                    let (latest, list) = &mut best[bin];
                    if by == By::Last {
                        match marks[bin].cmp(latest) {
                            Ordering::Less => continue,
                            Ordering::Greater => {
                                *latest = marks[bin];
                                list.clear();
                            }
                            Ordering::Equal => {}
                        }
                    }
                    let entry = (key, *held);
                    if list.len() == n && better(&entry, list.last().expect("full")) != Ordering::Less {
                        continue;
                    }
                    let at = list.partition_point(|other| better(other, &entry) == Ordering::Less);
                    list.insert(at, entry);
                    list.truncate(n);
                }
                Ok((values, life, marks, best))
            })
            .try_reduce(empty, |(values, life, marks, mut ours), (_, _, _, theirs)| {
                for ((latest, list), (their_latest, their_list)) in ours.iter_mut().zip(theirs) {
                    match their_latest.cmp(latest) {
                        Ordering::Greater => (*latest, *list) = (their_latest, their_list),
                        Ordering::Equal => {
                            list.extend(their_list);
                            list.sort_unstable_by(better);
                            list.truncate(n);
                        }
                        Ordering::Less => {}
                    }
                }
                Ok((values, life, marks, ours))
            })?;
        Ok(best.into_iter().map(|(_, list)| list).collect())
    }
}

#[cfg(test)]
pub(crate) fn top_folding(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: u32,
    view: (i32, i32, u32),
    n: usize,
    by: By,
) -> io::Result<Vec<Vec<(u128, Bin)>>> {
    top_with(recording, run, table, source, view.0, view.1, view.2, n, by, false)
}
