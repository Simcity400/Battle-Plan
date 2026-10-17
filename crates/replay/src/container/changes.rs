//! A block read only at the hours its watched tables' fields change.
//!
//! [`Replay::read_block_changes`] decodes a block's selected tables as
//! [`Replay::read_block_alone`] does and follows each table's view hour by
//! hour with the decoder's own per-table step, but builds a frame only at the
//! block's first hour and at the hours a watched field, row key or
//! availability changes. Every other hour is reported in a quiet span with the
//! read evidence `read_block_alone`'s frame holds there.
//!
//! Every value change a block encodes without an entry, a field moving by its
//! prediction or ramp, reaches the reader as the same plain delta the grouped
//! reader reconstructs for that hour, with the field in the row's mask, so
//! such an hour is never quiet for a watched field.

use super::*;
use crate::{
    frame::{SharedView, TableUpdate, decode_update},
    rows::{DeltaRow, compatibility::HISTORY, full_mask},
};

/// The fields of one table a [`Replay::read_block_changes`] reader watches:
/// bit `i` for the table's current schema field `i` ([`TableId::field_name`]).
/// A nested record is one field. With `read`, every hour the table was read
/// (its metrics' read time) is visited too, whether or not a field changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableFields {
    pub table: TableId,
    pub fields: u64,
    pub read: bool,
}

impl TableFields {
    /// Every field: any change of the table's rows.
    pub fn all(table: TableId) -> Self {
        Self {
            table,
            fields: full_mask(table.schema().len()),
            read: false,
        }
    }

    /// No field: only the table's row keys and availability.
    pub fn keys(table: TableId) -> Self {
        Self {
            table,
            fields: 0,
            read: false,
        }
    }

    /// These fields, and every hour the table was read: for a daily or
    /// sliced table whose reads matter to its consumer although the rows
    /// read are equal.
    pub fn when_read(self) -> Self {
        Self { read: true, ..self }
    }

    /// The fields `names` name, as [`TableId::field_name`] spells them.
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

/// One step of [`Replay::read_block_changes`], in hour order.
#[derive(Debug)]
pub enum BlockChange<'a> {
    /// The block's first hour, or an hour at which a watched table's watched
    /// fields, row keys or availability changed (a truncated table's missing
    /// count and an unavailable table's reason included) or a table watched
    /// when read ([`TableFields::when_read`]) was read: the frame
    /// [`Replay::read_block_alone`] reconstructs at that hour. `tables` are
    /// the watched tables that changed or were so read at this hour, every
    /// watched table at the first. `rebuilt` are the reconstructed tables whose view this
    /// hour's update rebuilt rather than carried unchanged from the hour
    /// before, every one at the first: a reader holding the hour before's
    /// `read_block_alone` frame finds exactly these at a new allocation, even
    /// where their rows are equal.
    Changed {
        frame: &'a DecodedFrame,
        tables: &'a [TableId],
        rebuilt: &'a [TableId],
    },
    /// Consecutive hours after a changed hour at which every watched table's
    /// watched fields, row keys and availability equal that hour's, and no
    /// table watched when read was read.
    Quiet(&'a QuietHours),
}

/// The hours of a [`BlockChange::Quiet`] span and, for each, what
/// [`Replay::read_block_alone`]'s frame there holds besides rows: its metrics
/// (which tables were read) and each table's sample hour. Unwatched fields
/// may change in these hours; their rows are not reported.
#[derive(Debug, Default)]
pub struct QuietHours {
    hours: Vec<Hour>,
    metrics: Vec<Metrics>,
    /// The reconstructed tables, in TableId order.
    tables: Vec<TableId>,
    /// Each hour's sample hour of each of `tables`.
    sampled: Vec<Option<Hour>>,
    /// Whether each hour's update rebuilt each of `tables`.
    rebuilt: Vec<bool>,
}

impl QuietHours {
    /// The span's first hour.
    pub fn first(&self) -> Hour {
        self.hours[0]
    }

    /// The span's last hour.
    pub fn last(&self) -> Hour {
        self.hours[self.hours.len() - 1]
    }

    /// The span's hours, in order: the block's recorded hours between the
    /// changed hours around it.
    pub fn hours(&self) -> &[Hour] {
        &self.hours
    }

    /// Each hour's metrics, as `read_block_alone`'s frame holds them.
    pub fn metrics(&self) -> &[Metrics] {
        &self.metrics
    }

    /// Whether `table` was read at the span's `position`th hour.
    pub fn read(&self, position: usize, table: TableId) -> bool {
        self.metrics[position].table_ns[table.index()] != 0
    }

    /// `table`'s [`SharedView::sampled_at`] at the span's `position`th hour,
    /// as in `read_block_alone`'s frame: None for a table not reconstructed.
    pub fn sampled_at(&self, position: usize, table: TableId) -> Option<Hour> {
        let column = self.tables.iter().position(|&watched| watched == table)?;
        self.sampled[position * self.tables.len() + column]
    }

    /// Whether the span's `position`th hour's update rebuilt `table`'s view
    /// rather than carrying the hour before's unchanged, as
    /// [`BlockChange::Changed`]'s `rebuilt`: a reader holding the hour
    /// before's `read_block_alone` frame finds it at a new allocation there,
    /// although its watched fields, keys and availability are equal (an
    /// unwatched field changed, or an equal view was decoded again). False
    /// for a table not reconstructed.
    pub fn rebuilt(&self, position: usize, table: TableId) -> bool {
        self.tables
            .iter()
            .position(|&watched| watched == table)
            .is_some_and(|column| self.rebuilt[position * self.tables.len() + column])
    }

    fn clear(&mut self) {
        self.hours.clear();
        self.metrics.clear();
        self.sampled.clear();
        self.rebuilt.clear();
    }
}

impl QuietHours {
    /// An empty span of a reader reconstructing `tables`, in TableId order.
    pub(super) fn for_tables(tables: Vec<TableId>) -> Self {
        Self {
            tables,
            ..Self::default()
        }
    }

    /// One more quiet hour: its metrics and each reconstructed table's sample
    /// hour and whether its update rebuilt it, in the tables' order.
    pub(super) fn push(
        &mut self,
        hour: Hour,
        metrics: Metrics,
        tables: impl Iterator<Item = (Option<Hour>, bool)>,
    ) -> io::Result<()> {
        engine::allocation::reserve(&mut self.hours, 1)?;
        engine::allocation::reserve(&mut self.metrics, 1)?;
        engine::allocation::reserve(&mut self.sampled, self.tables.len())?;
        engine::allocation::reserve(&mut self.rebuilt, self.tables.len())?;
        self.hours.push(hour);
        self.metrics.push(metrics);
        for (sampled, rebuilt) in tables {
            self.sampled.push(sampled);
            self.rebuilt.push(rebuilt);
        }
        Ok(())
    }

    /// Visit the quiet hours not yet visited.
    pub(super) fn flush(
        &mut self,
        visit: &mut dyn FnMut(super::Watched<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.hours.is_empty() {
            return Ok(());
        }
        let visited = visit(super::Watched::Change(BlockChange::Quiet(self)));
        self.clear();
        visited
    }
}

impl Replay {
    /// One block read alone, as [`Replay::read_block_alone`] reads it with
    /// `fields`' tables, visiting only its first hour and the hours at which a
    /// watched table's watched fields, row keys or availability change or a
    /// table watched when read is read, each
    /// with `read_block_alone`'s frame at that hour, and the quiet spans
    /// between them with each hour's read evidence. Changed hours share a
    /// table's allocation exactly when `read_block_alone`'s frames at those
    /// hours do. Duplicate entries of a table watch the union of their
    /// fields. Campaign is reconstructed and checked every hour as
    /// `read_block_alone` does, and watched only when `fields` names it.
    /// Admission, verification, cancellation and errors are
    /// `read_block_alone`'s; a failed read may have visited earlier hours.
    /// It is [`Replay::read_block_watched`] with this one watch.
    pub fn read_block_changes(
        &self,
        index: &BlockIndex,
        fields: &[TableFields],
        mut visit: impl FnMut(BlockChange<'_>) -> io::Result<()>,
        check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        let mut watches = [super::BlockWatch::new(
            super::Watching::Changes(fields),
            |step| match step {
                super::Watched::Change(change) => visit(change),
                super::Watched::Hour { .. } | super::Watched::Replacing(_) => {
                    unreachable!("a watch of changes")
                }
            },
        )];
        self.read_block_watched(index, &mut watches, check)?;
        let [watch] = watches;
        watch.finish()
    }
}

/// The row count of a complete view.
pub(super) fn complete_len(view: &TableView) -> Option<usize> {
    match view {
        TableView::Complete(rows) => Some(rows.len()),
        _ => None,
    }
}

/// Whether a delta row can change a watched field: one in its mask, or one
/// at or past the fields an older format recorded, which every row a delta
/// names takes as not recorded.
pub(super) fn touches(table: TableId, fields: Option<u64>, row: &DeltaRow) -> bool {
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

/// Whether a view replaced by an update other than a delta differs from the
/// hour before's in availability, a truncated table's missing count, an
/// unavailable table's reason, row keys or watched fields.
pub(super) fn view_differs(
    table: TableId,
    fields: Option<u64>,
    before: &SharedView,
    after: &SharedView,
) -> bool {
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

/// Whether two row sets differ in keys or watched fields. Rows that cannot be
/// compared field by field differ.
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

/// How two equally long row sets differ.
enum Changed {
    /// A row is deleted, and so another inserted: the keys differ, whatever
    /// fields are watched.
    Keys,
    /// The same keys; the fields any row changes.
    Fields(u64),
}

/// How `after` differs from `before` by the plain delta between them; None
/// when they cannot be encoded as one (fields an older format did not record).
fn changed_fields(table: TableId, before: &TableRows, after: &TableRows) -> Option<Changed> {
    // A plain delta: its status, a baseline hour, then the changes.
    let mut plain = vec![1_u8];
    plain.extend_from_slice(&0_i32.to_le_bytes());
    let mut deleted = Vec::new();
    after
        .encode_changes(before, &mut plain, &mut deleted)
        .ok()?;
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

#[cfg(test)]
#[path = "changes_tests.rs"]
mod tests;
