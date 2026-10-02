use super::{
    EncodedFrame, Frame, Metrics, TABLE_COUNT, TableUpdate, put_count, validate_complete,
    validate_delta, validate_rows,
};
use crate::{
    codec::{Decoder, invalid, put_string},
    rows::{self, TableId, TableRows},
};
use std::io;

impl Frame {
    pub fn encoded(&self) -> io::Result<EncodedFrame> {
        let mut bytes = vec![0; EncodedFrame::PREFIX_BYTES];
        self.encode(&mut bytes)?;
        Ok(EncodedFrame {
            branch: self.branch,
            hour: self.hour,
            keyframe: self.keyframe,
            bytes,
        })
    }
    pub fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.encode_with_sizes(out).map(|_| ())
    }

    pub fn table_encoded_sizes(&self) -> io::Result<[usize; TABLE_COUNT]> {
        self.encode_with_sizes(&mut Vec::new())
    }

    pub fn encode_with_sizes(&self, out: &mut Vec<u8>) -> io::Result<[usize; TABLE_COUNT]> {
        out.extend_from_slice(&self.branch.to_le_bytes());
        out.extend_from_slice(&self.hour.to_le_bytes());
        out.push(u8::from(self.keyframe));
        out.extend_from_slice(&self.metrics.total_ns.to_le_bytes());
        // Timing entries and updates use the same table-id order.
        for table in TableId::ALL {
            out.extend_from_slice(&self.metrics.table_ns[table.index()].to_le_bytes());
        }
        out.extend_from_slice(&self.metrics.pool_bytes.to_le_bytes());
        out.extend_from_slice(&self.metrics.dropped_frames.to_le_bytes());
        let mut sizes = [0; TABLE_COUNT];
        for table in TableId::ALL {
            let update = &self.tables[table.index()];
            let table_start = out.len();
            if self.keyframe
                && !matches!(
                    update,
                    TableUpdate::Snapshot(_) | TableUpdate::Unavailable { .. }
                )
            {
                return Err(invalid("keyframe contains a dependent or partial table"));
            }
            match update {
                TableUpdate::Snapshot(values) => {
                    validate_complete(table, values, self.hour)?;
                    out.push(0);
                    encode_rows(table, values, out)?;
                }
                TableUpdate::Delta {
                    baseline,
                    changed,
                    deleted,
                } => {
                    validate_delta(table, changed, deleted)?;
                    out.push(1);
                    out.extend_from_slice(&baseline.to_le_bytes());
                    put_count(out, changed.len())?;
                    let mut previous_key = 0;
                    for value in changed {
                        rows::encode_delta(table, value, previous_key, out)?;
                        previous_key = value.key;
                    }
                    put_count(out, deleted.len())?;
                    let mut previous_key = 0;
                    for key in deleted {
                        table.encode_key(*key, previous_key, out)?;
                        previous_key = *key;
                    }
                }
                TableUpdate::Unchanged { baseline } => {
                    out.push(2);
                    out.extend_from_slice(&baseline.to_le_bytes());
                }
                TableUpdate::Truncated { rows, missing } => {
                    if *missing == 0 {
                        return Err(invalid("truncated table has no missing rows"));
                    }
                    out.push(3);
                    out.extend_from_slice(&missing.to_le_bytes());
                    encode_rows(table, rows, out)?;
                }
                TableUpdate::Unavailable { reason } => {
                    if reason.is_empty() {
                        return Err(invalid("unavailable table needs a reason"));
                    }
                    out.push(4);
                    put_string(out, reason)?;
                }
            }
            sizes[table.index()] = out.len() - table_start;
        }
        Ok(sizes)
    }

    /// Read one timing entry and one update for every recorded table.
    pub fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        Self::decode_selected(input, None)
    }

    /// Parse and validate every update, retaining only the requested tables.
    /// Discarding unrelated rows immediately bounds their allocation lifetime.
    pub(crate) fn decode_selected(
        input: &mut Decoder<'_>,
        selected: Option<&[TableId]>,
    ) -> io::Result<Self> {
        let branch = input.u64()?;
        let hour = input.i32()?;
        let keyframe = input.bool()?;
        let total_ns = input.u64()?;
        let mut table_ns = [0; TABLE_COUNT];
        for table in TableId::ALL {
            table_ns[table.index()] = input.u64()?;
        }
        let metrics = Metrics {
            total_ns,
            table_ns,
            pool_bytes: input.u64()?,
            dropped_frames: input.u64()?,
        };
        let mut tables = Vec::with_capacity(TABLE_COUNT);
        for table in TableId::ALL {
            let update = match input.u8()? {
                0 => TableUpdate::Snapshot(decode_rows(table, input)?),
                1 => {
                    let baseline = input.i32()?;
                    let count = decode_row_count(table, input, table.minimum_delta_bytes())?;
                    let mut changed = Vec::with_capacity(count);
                    let mut previous_key = 0;
                    for _ in 0..count {
                        let row = rows::decode_delta(table, previous_key, input)?;
                        previous_key = row.key;
                        changed.push(row);
                    }
                    let count = decode_row_count(table, input, 1)?;
                    let mut deleted = Vec::with_capacity(count);
                    let mut previous_key = 0;
                    for _ in 0..count {
                        let key = table.decode_key(previous_key, input)?;
                        previous_key = key;
                        deleted.push(key);
                    }
                    validate_delta(table, &changed, &deleted)?;
                    TableUpdate::Delta {
                        baseline,
                        changed,
                        deleted,
                    }
                }
                2 => TableUpdate::Unchanged {
                    baseline: input.i32()?,
                },
                3 => {
                    let missing = input.u32()?;
                    if missing == 0 {
                        return Err(invalid("truncated table has no missing rows"));
                    }
                    TableUpdate::Truncated {
                        rows: decode_rows(table, input)?,
                        missing,
                    }
                }
                4 => {
                    let reason = input.string()?;
                    if reason.is_empty() {
                        return Err(invalid("unavailable table needs a reason"));
                    }
                    TableUpdate::Unavailable { reason }
                }
                _ => return Err(invalid("unknown table status")),
            };
            if keyframe
                && !matches!(
                    update,
                    TableUpdate::Snapshot(_) | TableUpdate::Unavailable { .. }
                )
            {
                return Err(invalid("keyframe contains a dependent or partial table"));
            }
            tables.push(if selected.is_none_or(|tables| tables.contains(&table)) {
                update
            } else {
                TableUpdate::Unavailable {
                    reason: "Table not selected for reconstruction".into(),
                }
            });
        }
        Ok(Self {
            branch,
            hour,
            keyframe,
            metrics,
            tables: tables.try_into().unwrap(),
        })
    }
}

fn encode_rows(table: TableId, values: &TableRows, out: &mut Vec<u8>) -> io::Result<()> {
    validate_rows(table, values)?;
    put_count(out, values.len())?;
    values.encode_rows(out)
}
pub(super) fn decode_rows(table: TableId, input: &mut Decoder<'_>) -> io::Result<TableRows> {
    let count = decode_row_count(table, input, table.minimum_row_bytes())?;
    let values = table.decode_rows(count, input)?;
    validate_rows(table, &values)?;
    Ok(values)
}

pub(super) fn decode_row_count(
    table: TableId,
    input: &mut Decoder<'_>,
    minimum_bytes: usize,
) -> io::Result<usize> {
    let count = input.count(minimum_bytes)?;
    if count > 1 && table.key_bits() == 0 {
        return Err(invalid("singleton table cannot contain multiple rows"));
    }
    Ok(count)
}
