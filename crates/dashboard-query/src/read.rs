//! The reads replies are made of, over one run of the recording: a walk of some tables through
//! some hours, a table at an hour ([`replay::lanes::at`]), hourly runs of a column for some keys,
//! and the nearest hour a column changes for any of some keys, found from the summary pieces and
//! read exactly from the one block that holds it. Timelines join these per stretch
//! ([`crate::timeline`]).

use replay::{cells::TableReader, columnar_frames::Recording, lanes::Snapshot};
pub use replay::at::{Keys, Read};
use replay_data::rows::TableId;
use replay_columnar::{
    envelope::UNIT_ORDINALS,
    store::TableView,
    summary::{self, Piece},
};
use std::{collections::HashMap, io};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// Run `run`'s ordinal of game hour `hour`.
pub fn ordinal(recording: &Recording, run: u32, hour: i32) -> io::Result<u32> {
    recording.run(run)?.ordinal(hour).ok_or_else(|| invalid("The run holds no such hour"))
}

/// Readers of `tables` of run `run`, each projected to its columns when given.
fn readers(recording: &Recording, run: u32, tables: &[(TableId, Option<&[usize]>)]) -> io::Result<Vec<TableReader>> {
    tables
        .iter()
        .map(|&(table, columns)| {
            let mut reader = TableReader::new(recording, run, table, columns)?;
            reader.index_keys();
            Ok(reader)
        })
        .collect()
}

/// Reads `tables` (each projected to its columns when given) through hours `from..=to` of run
/// `run`: restored before `from`, then stepped; `visit` is told each hour with every table's
/// reader, in the order asked.
pub fn walk(recording: &Recording, run: u32, tables: &[(TableId, Option<&[usize]>)], from: i32, to: i32, mut visit: impl FnMut(i32, &[TableReader]) -> io::Result<()>) -> io::Result<()> {
    crate::cost::storage(|| {
        let first = ordinal(recording, run, from)?;
        let last = ordinal(recording, run, to)?;
        let mut held = readers(recording, run, tables)?;
        for reader in &mut held {
            reader.seek(recording, first)?;
        }
        let mut at = first;
        loop {
            crate::cost::outside(|| visit(from + (at - first) as i32, &held))?;
            if at == last {
                return Ok(());
            }
            for reader in &mut held {
                reader.step(recording)?;
            }
            at += 1;
        }
    })
}

/// `reads`' tables (each projected to its columns when given, the rows its keys ask for listed)
/// of run `run` after hour `hour`, in the order asked, each read on its own thread.
pub fn at(recording: &Recording, run: u32, reads: &[Read<'_>], hour: i32) -> io::Result<Vec<Snapshot>> {
    crate::cost::storage(|| replay::at::snapshots(recording, run, hour, reads))
}

/// A text cell's bytes, when it holds one.
pub fn text<'a>(view: &TableView<'a>, column: usize, slot: u32) -> Option<&'a [u8]> {
    view.text(column, slot).and_then(|id| view.string(id))
}

/// One key's runs over a span: each run's first and last hour and its value, none where the row
/// was not live, the cell not recorded or the table not available.
pub type Runs = replay::lanes::Runs;

/// `source` of `table` of run `run` hour by hour through `from..=to` for `keys` (every key when
/// none), by key.
pub fn runs(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: summary::Source,
    keys: Option<&[u128]>,
    from: i32,
    to: i32,
) -> io::Result<HashMap<u128, Runs>> {
    let held = crate::cost::storage(|| replay::lanes::keyed_runs(recording, run, table, source, keys, from, to))?;
    Ok(held.into_iter().collect())
}

/// `table`'s status of run `run` through `from..=to`, as runs of hours, from the blocks' page
/// entries.
pub fn statuses(recording: &Recording, run: u32, table: TableId, from: i32, to: i32) -> io::Result<Vec<replay::lanes::StatusRun>> {
    crate::cost::storage(|| replay::lanes::statuses(recording, run, table, from, to))
}

/// The columns `source` reads.
pub fn columns_of(source: &summary::Source) -> Vec<usize> {
    match *source {
        summary::Source::Cell(column) => vec![column],
        summary::Source::Computed { inputs, .. } => (0..64).filter(|bit| inputs & (1 << bit) != 0).collect(),
    }
}

/// A key's value over one bucket, as far as its summary tells: none, one value held throughout,
/// or more than that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Nothing,
    Steady(i64),
    Varies,
}

/// What each of `keys` held over each level-0 bucket of block `block` of run `run`; none when
/// the table has no piece there (it held no key).
fn block_buckets(recording: &Recording, run: u32, table: TableId, column: u32, keys: &[u128], block: u32) -> io::Result<Option<Vec<Vec<Held>>>> {
    let Some(extent) = recording.block_of(run, block as usize, table.index())? else {
        return Ok(None);
    };
    let bytes = recording.stored_bytes(&extent)?;
    let piece = Piece::read(&bytes)?;
    let at = piece.column_of(column).ok_or_else(|| invalid("The summary holds no such column"))?;
    let frame = piece.column(at)?;
    let buckets = piece.directory.buckets as usize;
    let mut out = Vec::with_capacity(keys.len());
    for &key in keys {
        let Some(row) = piece.key_at(key) else {
            out.push(vec![Held::Nothing; buckets]);
            continue;
        };
        let lives = piece.life(row)?;
        let held = frame
            .buckets(&piece, row)?
            .into_iter()
            .enumerate()
            .map(|(bucket, fold)| match fold {
                None if lives[bucket].hours == 0 => Held::Nothing,
                Some(fold)
                    if fold.first == fold.last
                        && fold.min == fold.max
                        && fold.first == fold.min
                        && fold.hours == piece.directory.covered(bucket as u32) =>
                {
                    Held::Steady(fold.first)
                }
                _ => Held::Varies,
            })
            .collect();
        out.push(held);
    }
    Ok(Some(out))
}

/// Whether any of the keys may change within the buckets, or between `before` (the last held
/// value of each key in the block before, when known) and the first bucket.
fn may_change(buckets: &[Vec<Held>], before: Option<&[Held]>) -> bool {
    buckets.iter().enumerate().any(|(at, held)| {
        let mut previous = before.map(|before| before[at]);
        for &now in held {
            if now == Held::Varies || previous.is_some_and(|previous| previous != now) {
                return true;
            }
            previous = Some(now);
        }
        false
    })
}

/// The nearest hour of run `run` within `bounds` strictly after (`forward`) or before `origin`
/// at which `source` of any of `keys` holds another value than the hour before it (a row coming
/// or going counts). `column` is the summary id of `source`; a table it does not summarize is
/// walked hour by hour.
#[allow(clippy::too_many_arguments, reason = "a change search names its run, table, column, keys and bounds")]
#[allow(clippy::too_many_arguments, reason = "a lane query names its run, table, columns, keys and range")]
pub fn nearest_change(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: summary::Source,
    column: Option<u32>,
    keys: &[u128],
    origin: i64,
    forward: bool,
    bounds: (i32, i32),
) -> io::Result<Option<i32>> {
    crate::cost::storage(|| nearest_change_counted(recording, run, table, source, column, keys, origin, forward, bounds))
}

#[allow(clippy::too_many_arguments, reason = "a lane query names its run, table, columns, keys and range")]
fn nearest_change_counted(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: summary::Source,
    column: Option<u32>,
    keys: &[u128],
    origin: i64,
    forward: bool,
    bounds: (i32, i32),
) -> io::Result<Option<i32>> {
    let held = recording.run(run)?;
    let run_first = held.first_hour;
    let run_last = held.first_hour + held.ordinals as i32 - 1;
    // A change at an hour needs the hour before it in the run.
    let low = bounds.0.max(run_first + 1);
    let high = bounds.1.min(run_last);
    let (low, high) = if forward {
        (i64::from(low).max(origin + 1), i64::from(high))
    } else {
        (i64::from(low), i64::from(high).min(origin - 1))
    };
    if low > high || keys.is_empty() {
        return Ok(None);
    }
    let (low, high) = (low as i32, high as i32);
    let block_of = |hour: i32| ((i64::from(hour) - i64::from(run_first)) / i64::from(UNIT_ORDINALS)) as u32;
    let block_hours = |block: u32| {
        let first = run_first + (block * UNIT_ORDINALS) as i32;
        (first, (first + UNIT_ORDINALS as i32 - 1).min(run_last))
    };
    let exact = |from: i32, to: i32| -> io::Result<Option<i32>> {
        let found = runs(recording, run, table, source, Some(keys), (from - 1).max(run_first), to)?;
        let mut best: Option<i32> = None;
        for runs in found.values() {
            for (at, &(first, _, _)) in runs.iter().enumerate() {
                if at == 0 || first < from || first > to {
                    continue;
                }
                best = Some(best.map_or(first, |best| if forward { best.min(first) } else { best.max(first) }));
            }
        }
        Ok(best)
    };
    let Some(column) = column else {
        return exact(low, high);
    };
    let (first_block, last_block) = (block_of(low), block_of(high));
    let order: Box<dyn Iterator<Item = u32>> = if forward {
        Box::new(first_block..=last_block)
    } else {
        Box::new((first_block..=last_block).rev())
    };
    let mut neighbour: Option<(u32, Option<Vec<Vec<Held>>>)> = None;
    for block in order {
        let buckets = block_buckets(recording, run, table, column, keys, block)?;
        let (first, last) = block_hours(block);
        let changes = match &buckets {
            None => true,
            Some(buckets) => {
                // The block before's last bucket of each key: read it, or the neighbour's.
                let before_block = block.checked_sub(1);
                let before = match (&neighbour, before_block) {
                    (Some((held, Some(other))), Some(previous)) if *held == previous => {
                        Some(other.iter().map(|held| *held.last().unwrap_or(&Held::Nothing)).collect::<Vec<_>>())
                    }
                    (_, Some(previous)) => block_buckets(recording, run, table, column, keys, previous)?
                        .map(|other| other.iter().map(|held| *held.last().unwrap_or(&Held::Nothing)).collect()),
                    (_, None) => None,
                };
                may_change(buckets, before.as_deref())
            }
        };
        if changes && let Some(hour) = exact(first.max(low), last.min(high))? {
            return Ok(Some(hour));
        }
        neighbour = Some((block, buckets));
    }
    Ok(None)
}
