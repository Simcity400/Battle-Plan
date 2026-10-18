//! Prepared decoder snapshots. This packet never enters a recording or spool.
use super::*;
use crate::{
    FORMAT_VERSION,
    codec::{invalid_static, put_string},
    value::Value,
};

const MAGIC: &[u8; 4] = b"H4CP";
const VERSION: u16 = 2;

impl DecodedFrame {
    /// Append a prepared snapshot, keeping decoded values and sample provenance
    /// exact. Its independent schema invalidates only prepared caches.
    pub fn encode_checkpoint(&self, output: &mut Vec<u8>) -> io::Result<()> {
        let start = output.len();
        let result = self.encode_checkpoint_inner(output);
        if result.is_err() {
            output.truncate(start);
        }
        result
    }

    fn encode_checkpoint_inner(&self, output: &mut Vec<u8>) -> io::Result<()> {
        allocation::reserve(output, 8)?;
        output.extend_from_slice(MAGIC);
        output.extend_from_slice(&VERSION.to_le_bytes());
        output.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        Value::encode(&self.branch, output)?;
        Value::encode(&self.hour, output)?;
        Value::encode(&self.metrics.total_ns, output)?;
        Value::encode(&self.metrics.table_ns, output)?;
        Value::encode(&self.metrics.pool_bytes, output)?;
        Value::encode(&self.metrics.dropped_frames, output)?;
        for table in TableId::ALL {
            let view = &self.tables[table.index()];
            if view
                .sampled_at()
                .is_some_and(|hour| hour > self.hour || !matches!(&**view, TableView::Complete(_)))
            {
                return Err(invalid_static("invalid checkpoint sample hour"));
            }
            Value::encode(&view.sampled_at(), output)?;
            match &**view {
                TableView::Complete(rows) => {
                    validate_complete(table, rows, self.hour)?;
                    Value::encode(&0_u8, output)?;
                    rows.encode_checkpoint_rows(output)?;
                }
                TableView::Truncated { rows, missing } => {
                    validate_rows(table, rows)?;
                    if *missing == 0 {
                        return Err(invalid_static("truncated checkpoint has no missing rows"));
                    }
                    Value::encode(&1_u8, output)?;
                    Value::encode(missing, output)?;
                    rows.encode_checkpoint_rows(output)?;
                }
                TableView::Unavailable { reason } => {
                    if reason.is_empty() {
                        return Err(invalid_static("unavailable checkpoint needs a reason"));
                    }
                    Value::encode(&2_u8, output)?;
                    put_string(output, reason)?;
                }
            }
        }
        Ok(())
    }

    /// Read a snapshot from a bounded page payload; its owner checks `finish`.
    pub fn decode_checkpoint(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.take(4)? != MAGIC || input.u16()? != VERSION || input.u16()? != FORMAT_VERSION {
            return Err(invalid_static("unsupported prepared checkpoint schema"));
        }
        input.format = FORMAT_VERSION;
        input.recorded_fields = usize::MAX;
        let branch = input.u64()?;
        let hour = input.i32()?;
        let metrics = Metrics {
            total_ns: input.u64()?,
            table_ns: Value::decode(input)?,
            pool_bytes: input.u64()?,
            dropped_frames: input.u64()?,
        };
        let mut tables = allocation::vec(TABLE_COUNT)?;
        let mut samples = [None; TABLE_COUNT];
        for table in TableId::ALL {
            samples[table.index()] = Value::decode(input)?;
            let view = match input.u8()? {
                0 => {
                    let rows = table.decode_checkpoint_rows(input)?;
                    validate_complete(table, &rows, hour)?;
                    TableView::Complete(rows)
                }
                1 => {
                    let missing = input.u32()?;
                    if missing == 0 {
                        return Err(invalid_static("truncated checkpoint has no missing rows"));
                    }
                    let rows = table.decode_checkpoint_rows(input)?;
                    validate_rows(table, &rows)?;
                    TableView::Truncated { rows, missing }
                }
                2 => {
                    let reason = input.string()?;
                    if reason.is_empty() {
                        return Err(invalid_static("unavailable checkpoint needs a reason"));
                    }
                    TableView::Unavailable { reason }
                }
                _ => return Err(invalid_static("invalid checkpoint availability")),
            };
            tables.push(SharedView::from(view));
        }
        let mut frame = Self {
            branch,
            hour,
            tables: tables.try_into().unwrap(),
            metrics,
        };
        frame.restore_samples(samples)?;
        Ok(frame)
    }
}

#[cfg(test)]
mod tests;
