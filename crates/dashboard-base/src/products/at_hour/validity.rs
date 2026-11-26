use dashboard_text::{metadata::check_cancel, triggers::inputs::{Resolution, TableRead}};
use replay::campaign::Replay;
use replay_data::rows::TableId;
use replay_lanes::lanes::Cell;
use std::{collections::{BTreeMap, BTreeSet}, io, sync::atomic::AtomicBool};

pub fn reads_through(replay: &Replay, run: u32, reads: &[TableRead], hour: i32, through: i32, cancelled: &AtomicBool) -> io::Result<i32> {
    let mut tables: BTreeMap<TableId, (u64, Option<Vec<(u128, u128)>>)> = BTreeMap::new();
    let mut ending: BTreeSet<TableId> = BTreeSet::new();
    let mut merge = |read: &TableRead| {
        if read.ends {
            ending.insert(read.table);
        }
        let held = tables.entry(read.table).or_insert((0, Some(Vec::new())));
        held.0 |= read.fields;
        match (read.rows, read.owner, held.1.as_mut()) {
            (Some(rows), None, Some(ranges)) => ranges.push(rows),
            _ => held.1 = None,
        }
    };
    for read in reads {
        merge(read);
        if let Some(owner) = read.owner {
            merge(&match owner.by {
                Resolution::Links => TableRead::fields(TableId::CountryLinks, &["canonical_slot"]),
                Resolution::Countries => TableRead::fields(TableId::Countries, &["canonical_slot"]),
            });
        }
    }
    let mut next = through.saturating_add(1);
    for (table, (fields, ranges)) in tables {
        check_cancel(cancelled)?;
        let mut columns: Vec<usize> = (0..64).filter(|bit| fields & (1_u64 << bit) != 0).collect();
        if columns.is_empty() {
            columns.push(0);
        }
        let ranges = ranges.map(|mut ranges| {
            ranges.sort_unstable();
            let mut merged: Vec<(u128, u128)> = Vec::with_capacity(ranges.len());
            for (first, last) in ranges {
                match merged.last_mut() {
                    Some(held) if first <= held.1.saturating_add(1) => held.1 = held.1.max(last),
                    _ => merged.push((first, last)),
                }
            }
            merged
        });
        let keys = match &ranges {
            Some(ranges) => replay_lanes::lanes::Keys::Ranges(ranges),
            None => replay_lanes::lanes::Keys::All,
        };
        let Ok(history) = replay_lanes::lanes::history(replay.recording(), run, table, &columns, keys, hour, next.saturating_sub(1)) else {
            return Ok(hour);
        };
        let edges = history.keys.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour)).chain(history.statuses.iter().map(|status| status.first));
        if let Some(edge) = edges.filter(|&edge| edge > hour).min() {
            next = next.min(edge);
        }
        if ending.contains(&table)
            && let Some(at) = columns.iter().position(|&column| table.field_name(column) == Some("end_hour"))
        {
            for (_, steps) in &history.keys {
                let held = steps[..steps.partition_point(|step| step.hour <= hour)].last();
                if let Some(Cell::Number(end)) = held.and_then(|step| step.cells.as_deref()).and_then(|cells| cells.get(at)).copied()
                    && let Ok(end) = i32::try_from(end)
                    && end > hour
                {
                    next = next.min(end);
                }
            }
        }
    }
    Ok(next.saturating_sub(1).max(hour))
}

