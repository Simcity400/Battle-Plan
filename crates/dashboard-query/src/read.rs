
use replay_recording::{cells::TableReader, columnar_frames::Recording};
use replay_lanes::lanes::Snapshot;
pub use replay_lanes::at::{Keys, Read};
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

pub fn ordinal(recording: &Recording, run: u32, hour: i32) -> io::Result<u32> {
    recording.run(run)?.ordinal(hour).ok_or_else(|| invalid("The run holds no such hour"))
}

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

pub fn at(recording: &Recording, run: u32, reads: &[Read<'_>], hour: i32) -> io::Result<Vec<Snapshot>> {
    crate::cost::storage(|| replay_lanes::at::snapshots(recording, run, hour, reads))
}

pub fn text<'a>(view: &TableView<'a>, column: usize, slot: u32) -> Option<&'a [u8]> {
    view.text(column, slot).and_then(|id| view.string(id))
}

pub type Runs = replay_lanes::lanes::Runs;

pub fn runs(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: summary::Source,
    keys: Option<&[u128]>,
    from: i32,
    to: i32,
) -> io::Result<HashMap<u128, Runs>> {
    let held = crate::cost::storage(|| replay_lanes::lanes::keyed_runs(recording, run, table, source, keys, from, to))?;
    Ok(held.into_iter().collect())
}

pub fn statuses(recording: &Recording, run: u32, table: TableId, from: i32, to: i32) -> io::Result<Vec<replay_lanes::lanes::StatusRun>> {
    crate::cost::storage(|| replay_lanes::lanes::statuses(recording, run, table, from, to))
}

pub fn status_reasons(recording: &Recording, run: u32, table: TableId, runs: &[replay_lanes::lanes::StatusRun]) -> io::Result<HashMap<u32, String>> {
    crate::cost::storage(|| replay_lanes::lanes::status_reasons(recording, run, table, runs))
}

pub fn columns_of(source: &summary::Source) -> Vec<usize> {
    match *source {
        summary::Source::Cell(column) => vec![column],
        summary::Source::Computed { inputs, .. } => (0..64).filter(|bit| inputs & (1 << bit) != 0).collect(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Nothing,
    Steady(i64),
    Varies,
}

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
