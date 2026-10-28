//! Offline census of what the columnar change grammar would emit for a
//! recording: per table and column, the inserts, deletes, changed values,
//! explicit equal-value writes, fills and dictionary appends, as JSON.
use replay::container::{Replay, ReplayStream};
use replay_data::{
    codec::Decoder,
    rows::{Kind, TABLE_COUNT, TableId, TableRows},
};
use replay_frame::frame::{TableUpdate, TableView};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io,
    path::PathBuf,
};

/// A row's encoded fields and where each field ends within them.
struct Row {
    bytes: Vec<u8>,
    ends: Vec<u32>,
}
impl Row {
    fn field(&self, index: usize) -> &[u8] {
        let start = if index == 0 { 0 } else { self.ends[index - 1] as usize };
        &self.bytes[start..self.ends[index] as usize]
    }
}

#[derive(Default, Clone)]
struct Column {
    set: u64,
    noop: u64,
    /// Fixed-width bytes of the changed values, and the varint bytes of their exact differences.
    set_bytes: u64,
    add_bytes: u64,
    fill_hours: u64,
    fill_exceptions: u64,
    dictionary_appends: u64,
    dictionary_bytes: u64,
}

#[derive(Default)]
struct Table {
    rows: HashMap<u128, Row>,
    status: [u64; 4],
    updates: [u64; 3],
    inserts: u64,
    deletes: u64,
    changed_slots: u64,
    insert_bytes: u64,
    live_sum: u64,
    live_max: u64,
    columns: Vec<Column>,
}

fn fixed_width(kind: Kind) -> Option<usize> {
    Some(match kind {
        Kind::U8 | Kind::I8 | Kind::Bool => 1,
        Kind::U16 | Kind::I16 => 2,
        Kind::U32 | Kind::I32 => 4,
        Kind::U64 | Kind::I64 => 8,
        _ => return None,
    })
}

fn integer(kind: Kind, bytes: &[u8]) -> Option<i128> {
    let width = fixed_width(kind)?;
    if matches!(kind, Kind::Bool) || bytes.len() != width {
        return None;
    }
    let mut wide = [0_u8; 16];
    wide[..width].copy_from_slice(bytes);
    let value = i128::from_le_bytes(wide);
    let shift = 128 - width as u32 * 8;
    Some(match kind {
        Kind::I8 | Kind::I16 | Kind::I32 | Kind::I64 => (value << shift) >> shift,
        _ => value,
    })
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::U8 => "U8",
        Kind::I8 => "I8",
        Kind::U16 => "U16",
        Kind::I16 => "I16",
        Kind::U32 => "U32",
        Kind::I32 => "I32",
        Kind::U64 => "U64",
        Kind::I64 => "I64",
        Kind::Bool => "Bool",
        Kind::OptionalI32 => "OptionalI32",
        Kind::OptionalU32 => "OptionalU32",
        Kind::OptionalI64 => "OptionalI64",
        Kind::OptionalU64 => "OptionalU64",
        Kind::Bytes => "Bytes",
        Kind::U32List => "U32List",
        Kind::U64List => "U64List",
        Kind::Opaque => "Opaque",
    }
}

fn varint_bytes(value: i128) -> u64 {
    let zigzag = ((value as u128) << 1) ^ ((value >> 127) as u128);
    (128 - zigzag.leading_zeros() as u64).div_ceil(7).max(1)
}

fn row(table: TableId, rows: &TableRows, index: usize, buffer: &mut Vec<u8>) -> io::Result<Row> {
    buffer.clear();
    rows.encode_row_fields(index, buffer)?;
    let mut decoder = Decoder::new(buffer.as_slice());
    let total = buffer.len();
    let mut ends = Vec::with_capacity(table.schema().len());
    for kind in table.schema() {
        kind.decode(&mut decoder)?;
        ends.push((total - decoder.remaining()) as u32);
    }
    Ok(Row {
        bytes: buffer.clone(),
        ends,
    })
}

fn position(rows: &TableRows, key: u128) -> Option<usize> {
    let (mut low, mut high) = (0, rows.len());
    while low < high {
        let middle = (low + high) / 2;
        match rows.get(middle)?.key().cmp(&key) {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => return Some(middle),
        }
    }
    None
}

/// Per-hour column tallies for fill detection.
struct Hour {
    changed: Vec<u64>,
    first: Vec<Option<Vec<u8>>>,
    uniform: Vec<bool>,
    slots: HashSet<u128>,
}

fn compare(
    schema: &[Kind],
    state: &mut Table,
    dictionary: &mut HashSet<Vec<u8>>,
    hour: &mut Hour,
    key: u128,
    new: Row,
    mask: Option<u64>,
) {
    let Some(old) = state.rows.get(&key) else {
        state.inserts += 1;
        state.insert_bytes += new.bytes.len() as u64;
        for (column, kind) in schema.iter().enumerate() {
            if matches!(kind, Kind::Bytes) && dictionary.insert(new.field(column).to_vec()) {
                state.columns[column].dictionary_appends += 1;
                state.columns[column].dictionary_bytes += new.field(column).len() as u64;
            }
        }
        state.rows.insert(key, new);
        return;
    };
    for (column, kind) in schema.iter().enumerate() {
        let (value, previous) = (new.field(column), old.field(column));
        let entry = &mut state.columns[column];
        if value == previous {
            if mask.is_some_and(|mask| mask & (1 << column) != 0) {
                entry.noop += 1;
                hour.slots.insert(key);
            }
            continue;
        }
        hour.slots.insert(key);
        hour.changed[column] += 1;
        entry.set += 1;
        entry.set_bytes += fixed_width(*kind).unwrap_or(value.len()) as u64;
        if let (Some(value), Some(previous)) = (integer(*kind, value), integer(*kind, previous)) {
            entry.add_bytes += varint_bytes(value - previous);
        }
        if matches!(kind, Kind::Bytes) && dictionary.insert(value.to_vec()) {
            entry.dictionary_appends += 1;
            entry.dictionary_bytes += value.len() as u64;
        }
        match &hour.first[column] {
            None => hour.first[column] = Some(value.to_vec()),
            Some(first) if first.as_slice() != value => hour.uniform[column] = false,
            _ => {}
        }
    }
    state.rows.insert(key, new);
}

fn main() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        return Err(io::Error::other("p3-census <recording.battleplan> <census.json>"));
    };
    let replay = Replay::open_path(&PathBuf::from(input))?;
    let mut stream = ReplayStream::new(&TableId::ALL)?;
    let mut tables: Vec<Table> = TableId::ALL
        .iter()
        .map(|table| Table {
            columns: vec![Column::default(); table.schema().len()],
            ..Table::default()
        })
        .collect();
    let mut dictionary: HashSet<Vec<u8>> = HashSet::new();
    let mut buffer = Vec::new();
    let mut hours = 0_u64;
    while stream.next_block_recorded_with(
        &replay,
        |recorded, decoded| {
            hours += 1;
            for (index, (table, update)) in TableId::ALL.iter().zip(recorded.tables.iter()).enumerate() {
                let state = &mut tables[index];
                let schema = table.schema();
                let (rows, complete) = match &*decoded.tables[index] {
                    TableView::Complete(rows) => (rows, true),
                    TableView::Truncated { rows, .. } => (rows, false),
                    TableView::Unavailable { .. } => {
                        state.status[2] += 1;
                        continue;
                    }
                    TableView::NotRecorded { .. } => {
                        state.status[3] += 1;
                        continue;
                    }
                };
                state.status[usize::from(!complete)] += 1;
                let mut hour = Hour {
                    changed: vec![0; schema.len()],
                    first: vec![None; schema.len()],
                    uniform: vec![true; schema.len()],
                    slots: HashSet::new(),
                };
                match update {
                    TableUpdate::Unchanged { .. } => state.updates[0] += 1,
                    TableUpdate::Delta { changed, deleted, .. } if complete => {
                        state.updates[1] += 1;
                        for key in deleted {
                            if state.rows.remove(key).is_some() {
                                state.deletes += 1;
                            }
                        }
                        for delta in changed {
                            let at = position(rows, delta.key)
                                .ok_or_else(|| io::Error::other("delta key absent from its hour"))?;
                            let new = row(*table, rows, at, &mut buffer)?;
                            compare(schema, state, &mut dictionary, &mut hour, delta.key, new, Some(delta.mask));
                        }
                    }
                    _ => {
                        // Snapshots and partial walks: compare every row read this hour.
                        state.updates[2] += 1;
                        let mut keys = HashSet::with_capacity(rows.len());
                        for at in 0..rows.len() {
                            let key = rows.get(at).map(|row| row.key()).unwrap_or_default();
                            keys.insert(key);
                            let new = row(*table, rows, at, &mut buffer)?;
                            compare(schema, state, &mut dictionary, &mut hour, key, new, None);
                        }
                        if complete {
                            let before = state.rows.len();
                            state.rows.retain(|key, _| keys.contains(key));
                            state.deletes += (before - state.rows.len()) as u64;
                        }
                    }
                }
                let live = rows.len() as u64;
                for column in 0..schema.len() {
                    if hour.changed[column] >= 16 && hour.uniform[column] && hour.changed[column] * 2 >= live {
                        state.columns[column].fill_hours += 1;
                        state.columns[column].fill_exceptions += live - hour.changed[column];
                    }
                }
                state.changed_slots += hour.slots.len() as u64;
                state.live_sum += live;
                state.live_max = state.live_max.max(live);
            }
            Ok(())
        },
        || Ok(()),
    )? {}
    let report: Vec<Value> = TableId::ALL
        .iter()
        .zip(&tables)
        .filter(|(_, table)| table.status.iter().sum::<u64>() > 0)
        .map(|(id, table)| {
            let present = table.status[0] + table.status[1];
            json!({
                "table": format!("{id:?}"),
                "hours": {"complete": table.status[0], "truncated": table.status[1], "unavailable": table.status[2], "not_recorded": table.status[3]},
                "updates": {"unchanged": table.updates[0], "delta": table.updates[1], "full": table.updates[2]},
                "live_rows_mean": table.live_sum.checked_div(present),
                "live_rows_max": table.live_max,
                "inserts": table.inserts,
                "deletes": table.deletes,
                "changed_slots": table.changed_slots,
                "insert_bytes": table.insert_bytes,
                "columns": id.schema().iter().zip(&table.columns).enumerate().filter(|(_, (_, column))| column.set + column.noop + column.dictionary_appends > 0).map(|(index, (kind, column))| json!({
                    "column": index,
                    "kind": kind_name(*kind),
                    "set": column.set,
                    "noop": column.noop,
                    "set_bytes": column.set_bytes,
                    "add_varint_bytes": column.add_bytes,
                    "fill_hours": column.fill_hours,
                    "fill_exceptions": column.fill_exceptions,
                    "dictionary_appends": column.dictionary_appends,
                    "dictionary_bytes": column.dictionary_bytes,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let changed: u64 = tables.iter().map(|table| table.changed_slots).sum();
    let value = json!({
        "hours": hours,
        "tables": TABLE_COUNT,
        "changed_slots_per_hour": changed as f64 / hours.max(1) as f64,
        "inserts": tables.iter().map(|table| table.inserts).sum::<u64>(),
        "deletes": tables.iter().map(|table| table.deletes).sum::<u64>(),
        "dictionary_entries": dictionary.len(),
        "per_table": report,
    });
    std::fs::write(output, serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}
