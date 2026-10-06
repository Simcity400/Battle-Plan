use super::{
    EncodedFrame, Frame, Metrics, TABLE_COUNT, TableUpdate, put_count, validate_complete,
    validate_delta, validate_rows,
};
use crate::{
    codec::{Decoder, invalid_static as invalid, put_string},
    format::FormatVersion,
    rows::{
        self, TableId, TableRows,
        compatibility::{HISTORY, LastChange},
    },
};
use engine::allocation;
use std::io;

impl Frame {
    pub fn encoded(&self) -> io::Result<EncodedFrame> {
        let mut bytes = allocation::vec(EncodedFrame::PREFIX_BYTES)?;
        bytes.resize(EncodedFrame::PREFIX_BYTES, 0);
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
        // Size decoded values directly: absent historical fields are not
        // re-encoded as current rows, and deltas retain their recorded mask width.
        TableId::ALL
            .map(|table| {
                let bytes = match &self.tables[table.index()] {
                    TableUpdate::Snapshot(rows) => 1 + rows.recorded_rows_bytes()?,
                    TableUpdate::Truncated { rows, .. } => 5 + rows.recorded_rows_bytes()?,
                    TableUpdate::Unavailable { reason } => 5 + reason.len(),
                    TableUpdate::Unchanged { .. } => 5,
                    TableUpdate::Delta {
                        changed, deleted, ..
                    } => {
                        validate_delta(table, changed, deleted)?;
                        let mut size = 13;
                        let mut previous = 0;
                        for row in changed {
                            size += rows::varint_bytes(row.key - previous)
                                + row.recorded_fields.div_ceil(8)
                                + row.values.len();
                            previous = row.key;
                        }
                        previous = 0;
                        for &key in deleted {
                            size += rows::varint_bytes(key - previous);
                            previous = key;
                        }
                        size
                    }
                };
                Ok(bytes)
            })
            .into_iter()
            .collect::<io::Result<Vec<_>>>()?
            .try_into()
            .map_err(|_| invalid("table size catalog mismatch"))
    }

    pub fn encode_with_sizes(&self, out: &mut Vec<u8>) -> io::Result<[usize; TABLE_COUNT]> {
        allocation::reserve(out, 37 + 8 * TABLE_COUNT)?;
        out.extend_from_slice(&self.branch.to_le_bytes());
        out.extend_from_slice(&self.hour.to_le_bytes());
        out.push(u8::from(self.keyframe));
        out.extend_from_slice(&self.metrics.total_ns.to_le_bytes());
        // Timing entries and updates use the same table-id order.
        for table in TableId::ALL {
            if !table
                .last_change()
                .present(FormatVersion::CURRENT.version())
            {
                continue;
            }
            out.extend_from_slice(&self.metrics.table_ns[table.index()].to_le_bytes());
        }
        out.extend_from_slice(&self.metrics.pool_bytes.to_le_bytes());
        out.extend_from_slice(&self.metrics.dropped_frames.to_le_bytes());
        let mut sizes = [0; TABLE_COUNT];
        for table in TableId::ALL {
            if !table
                .last_change()
                .present(FormatVersion::CURRENT.version())
            {
                continue;
            }
            allocation::reserve(out, 9)?;
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
        Self::decode_version(input, selected, FormatVersion::CURRENT)
    }

    pub(crate) fn decode_version(
        input: &mut Decoder<'_>,
        selected: Option<&[TableId]>,
        format: FormatVersion,
    ) -> io::Result<Self> {
        Self::decode_with_history(input, selected, format, &HISTORY)
    }

    pub(crate) fn decode_with_history(
        input: &mut Decoder<'_>,
        selected: Option<&[TableId]>,
        format: FormatVersion,
        history: &[LastChange; HISTORY.len()],
    ) -> io::Result<Self> {
        input.format = format.version();
        let branch = input.u64()?;
        let hour = input.i32()?;
        let keyframe = input.bool()?;
        let total_ns = input.u64()?;
        let mut table_ns = [0; TABLE_COUNT];
        for table in TableId::ALL {
            if history[table.index()].present(format.version()) {
                table_ns[table.index()] = input.u64()?;
            }
        }
        let metrics = Metrics {
            total_ns,
            table_ns,
            pool_bytes: input.u64()?,
            dropped_frames: input.u64()?,
        };
        let mut tables = allocation::vec(TABLE_COUNT)?;
        for table in TableId::ALL {
            let change = &history[table.index()];
            let gap = change.table_gap(table, format.version());
            if !change.present(format.version()) {
                let reason =
                    gap.ok_or_else(|| invalid("absent table has no catalogued lifetime gap"))?;
                tables.push(TableUpdate::Unavailable { reason });
                continue;
            }
            input.recorded_fields = change.fields(table, format.version());
            let status = input.u8()?;
            if let Some(reason) = gap {
                if keyframe && status != 0 && status != 4 {
                    return Err(invalid("keyframe contains a dependent or partial table"));
                }
                let fields = change.fields(table, format.version());
                match status {
                    0 => table.skip_snapshot(fields, input)?,
                    1 => {
                        input.i32()?;
                        table.skip_delta(fields, input)?;
                    }
                    2 => {
                        input.i32()?;
                    }
                    3 => {
                        if input.u32()? == 0 {
                            return Err(invalid("truncated table has no missing rows"));
                        }
                        table.skip_snapshot(fields, input)?;
                    }
                    4 => {
                        if input.string()?.is_empty() {
                            return Err(invalid("unavailable table needs a reason"));
                        }
                    }
                    _ => return Err(invalid("unknown table status")),
                }
                tables.push(TableUpdate::Unavailable { reason });
                continue;
            }
            let update = match status {
                0 => TableUpdate::Snapshot(decode_rows(table, input)?),
                1 => {
                    let baseline = input.i32()?;
                    let count =
                        decode_row_count(table, input, table.minimum_delta_bytes_at(input.format))?;
                    let mut changed = allocation::vec(count)?;
                    let mut previous_key = 0;
                    for _ in 0..count {
                        let row = rows::decode_delta(table, previous_key, input)?;
                        previous_key = row.key;
                        changed.push(row);
                    }
                    let count = decode_row_count(table, input, 1)?;
                    let mut deleted = allocation::vec(count)?;
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
    let count = decode_row_count(table, input, table.minimum_row_bytes_at(input.format))?;
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
