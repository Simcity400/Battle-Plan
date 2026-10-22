//! Grouped deltas assembled directly into their final owned rows.
use super::*;
use crate::rows::{DeltaRow, varint_bytes};
use engine_allocation as allocation;

/// Successful sections keep their directly assembled output. On refusal,
/// restart the whole table section through body framing followed by wire
/// decoding, with fresh frame-before patch bases, predictors and budget. That
/// route owns refusal precedence and may succeed where direct validation failed.
/// Allocation failures and stops are not refusals: they propagate unchanged.
pub(super) fn table_updates(context: &Context<'_>, table: TableId) -> io::Result<Vec<TableUpdate>> {
    super::table_updates_body(context, table, true).or_else(|error| {
        if allocation::is_allocation_error(&error) {
            Err(error)
        } else {
            super::table_updates_body(context, table, false)
        }
    })
}

/// One row buffer goes to its final DeltaRow. Validation and patch-base
/// observation share a field traversal after recovery against the old bases.
pub(super) struct Delta<'a> {
    table: TableId,
    fields: usize,
    format: u16,
    baseline: Hour,
    shadow: &'a mut Shadow,
    changed: Vec<DeltaRow>,
    deleted: Vec<u128>,
    bytes: usize,
    previous: u128,
}

impl<'a> Delta<'a> {
    pub(super) fn new(
        table: TableId,
        fields: usize,
        format: u16,
        baseline: Hour,
        shadow: &'a mut Shadow,
    ) -> Self {
        Self {
            table,
            fields,
            format,
            baseline,
            shadow,
            changed: Vec::new(),
            deleted: Vec::new(),
            bytes: 9,
            previous: 0,
        }
    }

    pub(super) fn bases(&self) -> &Shadow {
        self.shadow
    }
    pub(super) fn len(&self) -> usize {
        self.bytes
    }
    pub(super) fn is_empty(&self) -> bool {
        self.changed.is_empty()
    }

    pub(super) fn row(&mut self, key: u128, mask: u64, values: Vec<u8>) -> io::Result<()> {
        if self.table.key_bits() == 0 && !self.changed.is_empty() {
            return Err(invalid_static(
                "singleton table cannot contain multiple rows",
            ));
        }
        self.table.check_key(key)?;
        if !self.changed.is_empty() && key <= self.previous {
            return Err(invalid_static("delta keys must be unique and ascending"));
        }
        if mask == 0 || mask & !crate::rows::full_mask(self.fields) != 0 {
            return Err(invalid_static("invalid recorded delta mask"));
        }
        u32::try_from(self.changed.len() + 1)
            .map_err(|_| invalid_static("too many residual rows"))?;
        // No body/predictor operation for this row may read its old bases
        // after here. In particular the movement advance and list prediction
        // have both finished before either base is changed.
        let mut input = Decoder::new(&values);
        input.format = self.format;
        input.recorded_fields = self.fields;
        for field in 0..self.fields {
            if mask & (1 << field) != 0 {
                let span = self.table.change_span(field, &mut input)?;
                observe_shadow(self.table, DELTA, key, field, span, self.shadow)?;
            }
        }
        input.finish()?;
        self.bytes += varint_bytes(key - self.previous) + self.fields.div_ceil(8) + values.len();
        self.previous = key;
        allocation::reserve(&mut self.changed, 1)?;
        self.changed.push(DeltaRow {
            format: self.format,
            key,
            mask,
            recorded_fields: self.fields,
            values,
        });
        Ok(())
    }

    pub(super) fn deleted(&mut self, deleted: Vec<u128>) -> io::Result<()> {
        if self.table.key_bits() == 0 && deleted.len() > 1 {
            return Err(invalid_static(
                "singleton table cannot contain multiple rows",
            ));
        }
        let mut previous = 0;
        let mut changed = self.changed.iter().peekable();
        self.bytes += 4;
        for (index, &key) in deleted.iter().enumerate() {
            self.table.check_key(key)?;
            if index != 0 && key <= previous {
                return Err(invalid_static("delta keys must be unique and ascending"));
            }
            while changed.peek().is_some_and(|row| row.key < key) {
                changed.next();
            }
            if changed.peek().is_some_and(|row| row.key == key) {
                return Err(invalid_static("delta both changes and deletes a key"));
            }
            self.bytes += varint_bytes(key - previous);
            previous = key;
            self.shadow.remove(&key);
        }
        self.deleted = deleted;
        Ok(())
    }

    pub(super) fn finish(self) -> TableUpdate {
        TableUpdate::Delta {
            baseline: self.baseline,
            changed: self.changed,
            deleted: self.deleted,
        }
    }
}

/// The unpredicted delta body, with the same column/count/budget accounting
/// as take_body, but no intermediate whole-update plain buffer.
#[allow(clippy::too_many_arguments)]
pub(super) fn take_delta(
    table: TableId,
    fields: usize,
    format: u16,
    baseline: Hour,
    input: &mut ColumnReader<'_>,
    shadow: &mut Shadow,
    budget: &Budget,
    layout: Layout,
    mut coding: Option<RowCoding<'_>>,
) -> io::Result<TableUpdate> {
    let mut output = Delta::new(table, fields, format, baseline, shadow);
    let width = fields.div_ceil(8);
    let changed = varint_u32(&mut input.0[CONTROL])?;
    match &coding {
        Some(coding) if coding.modelled() => {
            if changed as usize > input.0[KEYS].remaining().saturating_add(coding.held_rows()) {
                return Err(invalid_static("grouped row count exceeds its rows"));
            }
        }
        _ if changed as usize > input.0[KEYS].remaining() => {
            return Err(invalid_static("grouped row count exceeds its keys"));
        }
        _ => {}
    }
    let mut run = match &mut coding {
        Some(coding) => Some(coding.read(table, changed as usize, &mut input.0)?),
        None => None,
    };
    let mut row = 0_u128;
    for _ in 0..changed {
        let mask = match (&mut coding, &mut run) {
            (Some(coding), Some(run)) => {
                let (key, mask) = coding.take_row(run, table, width, &mut input.0)?;
                row = key;
                mask
            }
            _ => {
                row = row
                    .checked_add(input.0[KEYS].varint()?)
                    .ok_or_else(|| invalid_static("row key addition overflow"))?;
                let mut mask = [0; 8];
                mask[..width].copy_from_slice(input.0[MASKS].take(width)?);
                u64::from_le_bytes(mask)
            }
        };
        if mask == 0 || (fields < 64 && mask >> fields != 0) {
            return Err(invalid_static("invalid recorded delta mask"));
        }
        let mut values = Vec::new();
        // take_body charges the body only, excluding its already-written
        // five-byte status/baseline head. Include this row's key and mask
        // before reconstructing byte fields, as the old path does.
        let difference = row
            .checked_sub(output.previous)
            .ok_or_else(|| invalid_static("delta keys must be unique and ascending"))?;
        let used = output.len() - 5 + varint_bytes(difference) + width;
        for field in (0..fields).filter(|field| mask & (1 << field) != 0) {
            let column = &mut input.0[FIELDS + field];
            if !table.byte_field(field) {
                values.extend_from_slice(table.change_span(field, column)?);
            } else {
                let base = output
                    .bases()
                    .get(&row)
                    .and_then(|fields| fields.get(&field))
                    .map(Vec::as_slice);
                let used = used + values.len();
                take_byte_field(
                    table,
                    field,
                    column,
                    base,
                    None,
                    &mut values,
                    used,
                    budget,
                    layout,
                )?;
            }
        }
        output.row(row, mask, values)?;
        budget.check(output.len() - 5)?;
    }
    let count = varint_u32(&mut input.0[CONTROL])?;
    if count as usize > input.0[KEYS].remaining() {
        return Err(invalid_static("grouped row count exceeds its keys"));
    }
    let mut deleted = allocation::vec(count as usize)?;
    let mut run = coding.as_ref().map(|coding| KeyReader::new(coding.keys));
    let mut previous = 0_u128;
    for _ in 0..count {
        let key = match (&mut run, &mut coding) {
            (Some(run), Some(coding)) => {
                let (key, _) = run.take(table, &mut input.0[KEYS])?;
                coding.note_deleted(key);
                key
            }
            _ => previous
                .checked_add(input.0[KEYS].varint()?)
                .ok_or_else(|| invalid_static("row key addition overflow"))?,
        };
        deleted.push(key);
        previous = key;
    }
    output.deleted(deleted)?;
    budget.spend(output.len() - 5)?;
    Ok(output.finish())
}
