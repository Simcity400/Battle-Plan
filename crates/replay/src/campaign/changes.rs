use super::{BlockIndex, Replay};
use replay_data::{Hour, codec::{Decoder, invalid}, format::FormatVersion, rows::{DeltaRow, TableId, TableRows, compatibility::HISTORY, full_mask}};
use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TableUpdate, TableView, decode_update};
use std::{io, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableFields {
    pub table: TableId,
    pub fields: u64,
    pub read: bool,
}

impl TableFields {
    pub fn all(table: TableId) -> Self {
        Self {
            table,
            fields: full_mask(table.schema().len()),
            read: false,
        }
    }

    pub fn keys(table: TableId) -> Self {
        Self {
            table,
            fields: 0,
            read: false,
        }
    }

    pub fn when_read(self) -> Self {
        Self { read: true, ..self }
    }

    pub fn named(table: TableId, names: &[&str]) -> io::Result<Self> {
        let mut fields = 0;
        for name in names {
            let field = (0..table.schema().len())
                .find(|&field| table.field_name(field) == Some(*name))
                .ok_or_else(|| invalid(format!("{table:?} has no field {name}")))?;
            fields |= 1_u64 << field;
        }
        Ok(Self {
            table,
            fields,
            read: false,
        })
    }
}

#[derive(Debug)]
pub enum BlockChange<'a> {
    Changed {
        frame: &'a DecodedFrame,
        tables: &'a [TableId],
        rebuilt: &'a [TableId],
    },
    Quiet(&'a QuietHours),
}

#[derive(Debug, Default)]
pub struct QuietHours {
    hours: Vec<Hour>,
    metrics: Vec<Arc<Metrics>>,
    tables: Vec<TableId>,
    sampled: Vec<Option<Hour>>,
    rebuilt: Vec<bool>,
}

impl QuietHours {
    pub fn first(&self) -> Hour {
        self.hours[0]
    }

    pub fn last(&self) -> Hour {
        self.hours[self.hours.len() - 1]
    }

    pub fn hours(&self) -> &[Hour] {
        &self.hours
    }

    pub fn metrics(&self) -> &[Arc<Metrics>] {
        &self.metrics
    }

    pub fn read(&self, position: usize, table: TableId) -> bool {
        self.metrics[position].table_ns[table.index()] != 0
    }

    pub fn sampled_at(&self, position: usize, table: TableId) -> Option<Hour> {
        let column = self.tables.binary_search(&table).ok()?;
        self.sampled[position * self.tables.len() + column]
    }

    pub fn rebuilt(&self, position: usize, table: TableId) -> bool {
        self.tables
            .binary_search(&table)
            .ok()
            .is_some_and(|column| self.rebuilt[position * self.tables.len() + column])
    }

    fn push(
        &mut self,
        hour: Hour,
        metrics: Arc<Metrics>,
        tables: impl Iterator<Item = (Option<Hour>, bool)>,
    ) -> io::Result<()> {
        engine_allocation::reserve(&mut self.hours, 1)?;
        engine_allocation::reserve(&mut self.metrics, 1)?;
        engine_allocation::reserve(&mut self.sampled, self.tables.len())?;
        engine_allocation::reserve(&mut self.rebuilt, self.tables.len())?;
        self.hours.push(hour);
        self.metrics.push(metrics);
        for (sampled, rebuilt) in tables {
            self.sampled.push(sampled);
            self.rebuilt.push(rebuilt);
        }
        Ok(())
    }

    fn flush(
        &mut self,
        visit: &mut dyn FnMut(BlockChange<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.hours.is_empty() {
            return Ok(());
        }
        let visited = visit(BlockChange::Quiet(self));
        self.hours.clear();
        self.metrics.clear();
        self.sampled.clear();
        self.rebuilt.clear();
        visited
    }
}

impl Replay {
    pub fn read_block_changes(
        &self,
        index: &BlockIndex,
        fields: &[TableFields],
        mut visit: impl FnMut(BlockChange<'_>) -> io::Result<()>,
        check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        let mut tables: Vec<TableId> = TableId::ALL
            .into_iter()
            .filter(|&table| table == TableId::Campaign || fields.iter().any(|entry| entry.table == table))
            .collect();
        tables.dedup();
        let watched: Vec<(Option<u64>, bool)> = tables
            .iter()
            .map(|&table| {
                let named = fields
                    .iter()
                    .filter(|entry| entry.table == table)
                    .fold(None, |union, entry| Some(union.unwrap_or(0) | entry.fields));
                let read = fields.iter().any(|entry| entry.table == table && entry.read);
                (named, read)
            })
            .collect();
        let mut quiet = QuietHours {
            tables: tables.clone(),
            ..QuietHours::default()
        };
        let mut before: Vec<Option<SharedView>> = vec![None; tables.len()];
        let mut changed = Vec::new();
        let mut rebuilt = Vec::new();
        let mut first = true;
        self.read_block_alone(
            index,
            &tables,
            |recorded, frame| {
                changed.clear();
                rebuilt.clear();
                for (position, (&table, &(watch, visit_read))) in tables.iter().zip(&watched).enumerate() {
                    let view = &frame.tables[table.index()];
                    let renewed = before[position]
                        .as_ref()
                        .is_none_or(|before| !SharedView::ptr_eq(before, view));
                    let read = frame.metrics.table_ns[table.index()] != 0;
                    let differs = renewed
                        && match &recorded.tables[table.index()] {
                            TableUpdate::Delta { changed, deleted, .. } => {
                                !deleted.is_empty()
                                    || changed.iter().any(|row| touches(table, watch, row))
                            }
                            _ => before[position]
                                .as_ref()
                                .is_none_or(|before| view_differs(table, watch, before, view)),
                        };
                    if watch.is_some() && (first || differs || (visit_read && read)) {
                        changed.push(table);
                    }
                    if first || renewed {
                        rebuilt.push(table);
                    }
                    before[position] = Some(view.clone());
                }
                let visits = first || !changed.is_empty();
                first = false;
                if !visits {
                    let metrics = Arc::new(frame.metrics.clone());
                    return quiet.push(
                        frame.hour,
                        metrics,
                        tables.iter().map(|table| {
                            let view = &frame.tables[table.index()];
                            (view.sampled_at(), rebuilt.contains(table))
                        }),
                    );
                }
                quiet.flush(&mut visit)?;
                visit(BlockChange::Changed {
                    frame,
                    tables: &changed,
                    rebuilt: &rebuilt,
                })
            },
            check,
        )?;
        quiet.flush(&mut visit)
    }
}

fn touches(table: TableId, fields: Option<u64>, row: &DeltaRow) -> bool {
    let Some(fields) = fields else {
        return false;
    };
    let unrecorded = if row.recorded_fields < table.schema().len() {
        u32::try_from(row.recorded_fields)
            .ok()
            .and_then(|recorded| fields.checked_shr(recorded))
            .unwrap_or(0)
    } else {
        0
    };
    row.mask & fields != 0 || unrecorded != 0
}

fn view_differs(table: TableId, fields: Option<u64>, before: &SharedView, after: &SharedView) -> bool {
    let Some(fields) = fields else {
        return false;
    };
    if SharedView::ptr_eq(before, after) {
        return false;
    }
    match (&**before, &**after) {
        (TableView::Complete(before), TableView::Complete(after)) => {
            rows_differ(table, fields, before, after)
        }
        (
            TableView::Truncated {
                rows: before,
                missing: before_missing,
            },
            TableView::Truncated {
                rows: after,
                missing: after_missing,
            },
        ) => before_missing != after_missing || rows_differ(table, fields, before, after),
        (TableView::Unavailable { reason: before }, TableView::Unavailable { reason: after }) => {
            before != after
        }
        _ => true,
    }
}

fn rows_differ(table: TableId, fields: u64, before: &TableRows, after: &TableRows) -> bool {
    if before == after {
        return false;
    }
    let all = full_mask(table.schema().len());
    if fields & all == all || before.len() != after.len() {
        return true;
    }
    changed_fields(table, before, after).is_none_or(|changed| match changed {
        Changed::Keys => true,
        Changed::Fields(changed) => changed & fields != 0,
    })
}

enum Changed {
    Keys,
    Fields(u64),
}

fn changed_fields(table: TableId, before: &TableRows, after: &TableRows) -> Option<Changed> {
    let mut plain = vec![1_u8];
    plain.extend_from_slice(&0_i32.to_le_bytes());
    let mut deleted = Vec::new();
    after.encode_changes(before, &mut plain, &mut deleted).ok()?;
    let mut input = Decoder::new(&plain);
    let update = decode_update(
        table,
        &HISTORY[table.index()],
        FormatVersion::CURRENT,
        false,
        &mut input,
    )
    .ok()?;
    input.finish().ok()?;
    match update {
        TableUpdate::Delta {
            changed, deleted, ..
        } if deleted.is_empty() => Some(Changed::Fields(
            changed.iter().fold(0, |mask, row| mask | row.mask),
        )),
        TableUpdate::Delta { .. } => Some(Changed::Keys),
        _ => None,
    }
}
