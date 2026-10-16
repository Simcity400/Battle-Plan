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
    BranchId,
    frame::{SharedView, TableUpdate, advance_table, decode_update, not_selected},
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
    pub fn read_block_changes(
        &self,
        index: &BlockIndex,
        fields: &[TableFields],
        mut visit: impl FnMut(BlockChange<'_>) -> io::Result<()>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        check()?;
        let mut reader = ChangeReader::new(&self.campaign_id, index.branch, fields)?;
        let tables = engine::allocation::collect_vec(reader.quiet.tables.iter().copied())?;
        if let Some(block) = self.grouped_block(index)? {
            // Admission as `read_block_selected`: the selected tables
            // decoded, then every section's checksum on a first read.
            let read = super::grouped::read_selected_updates(self, index, &block, &tables)?;
            if !self.is_verified(index) {
                super::grouped::verify(&block, index)?;
            }
            self.mark_verified(index);
            drop(block);
            check()?;
            let super::grouped::SelectedUpdates {
                hours,
                metrics,
                updates,
                dependent_keyframe,
            } = read;
            let updates = &updates;
            for (position, (hour, metrics)) in hours.into_iter().zip(metrics).enumerate() {
                check()?;
                let keyframe = position == 0;
                if keyframe && dependent_keyframe {
                    return Err(invalid_static(
                        "keyframe contains a dependent or partial table",
                    ));
                }
                reader.hour(
                    hour,
                    keyframe,
                    metrics,
                    move |table| &updates[table][position],
                    &mut visit,
                )?;
            }
            return reader.flush(&mut visit);
        }
        // Interleaved blocks keep their bounded payload streaming: an
        // admission pass, then reconstruction.
        self.visit_block(index, Some(&tables), |_| {
            check()?;
            Ok(true)
        })?;
        check()?;
        let watched = &tables;
        self.visit_block(index, Some(&tables), |frame| {
            check()?;
            let Frame {
                hour,
                keyframe,
                metrics,
                tables: updates,
                ..
            } = frame;
            if keyframe && updates.iter().any(|update| !keyframe_holds(update)) {
                return Err(invalid_static(
                    "keyframe contains a dependent or partial table",
                ));
            }
            let updates = &updates;
            reader.hour(
                hour,
                keyframe,
                metrics,
                move |table| &updates[watched[table].index()],
                &mut visit,
            )?;
            Ok(true)
        })?;
        reader.flush(&mut visit)
    }
}

/// An update a keyframe may hold.
fn keyframe_holds(update: &TableUpdate) -> bool {
    matches!(
        update,
        TableUpdate::Snapshot(_)
            | TableUpdate::Unavailable { .. }
            | TableUpdate::Incompatible { .. }
    )
}

/// A reconstructed table and the fields watched in it; None for the Campaign
/// table the caller did not name, which is reconstructed and checked only.
/// With `read`, the hours it was read are visited.
struct Watch {
    table: TableId,
    fields: Option<u64>,
    read: bool,
}

/// Each reconstructed table's view, advanced every hour exactly as
/// `FrameDecoder::advance` advances it, and the quiet hours not yet visited.
struct ChangeReader<'r> {
    campaign_id: &'r [u8],
    branch: BranchId,
    watches: Vec<Watch>,
    /// Each table's position in `watches`, by table index.
    slots: Vec<Option<usize>>,
    views: Vec<Option<SharedView>>,
    last_hour: Option<Hour>,
    changed: Vec<TableId>,
    /// The reconstructed tables the current hour's update rebuilt.
    rebuilt: Vec<TableId>,
    quiet: QuietHours,
}

impl<'r> ChangeReader<'r> {
    fn new(campaign_id: &'r [u8], branch: BranchId, fields: &[TableFields]) -> io::Result<Self> {
        let mut watches = Vec::new();
        for table in TableId::ALL {
            let named = fields
                .iter()
                .filter(|entry| entry.table == table)
                .fold(None, |union, entry| Some(union.unwrap_or(0) | entry.fields));
            if named.is_some() || table == TableId::Campaign {
                engine::allocation::reserve(&mut watches, 1)?;
                watches.push(Watch {
                    table,
                    fields: named,
                    read: fields
                        .iter()
                        .any(|entry| entry.table == table && entry.read),
                });
            }
        }
        let mut slots = engine::allocation::collect_vec((0..TABLE_COUNT).map(|_| None))?;
        for (position, watch) in watches.iter().enumerate() {
            slots[watch.table.index()] = Some(position);
        }
        Ok(Self {
            campaign_id,
            branch,
            views: engine::allocation::collect_vec(watches.iter().map(|_| None))?,
            quiet: QuietHours {
                tables: engine::allocation::collect_vec(watches.iter().map(|watch| watch.table))?,
                ..QuietHours::default()
            },
            watches,
            slots,
            last_hour: None,
            changed: Vec::new(),
            rebuilt: Vec::new(),
        })
    }

    /// Advance every reconstructed table to `hour` from its update
    /// (`update(position)`, by position in `watches`), then visit the hour
    /// when it is the first or a watched table changed, after the quiet hours
    /// before it; otherwise keep it quiet.
    fn hour<'u>(
        &mut self,
        hour: Hour,
        keyframe: bool,
        metrics: Metrics,
        update: impl Fn(usize) -> &'u TableUpdate,
        visit: &mut impl FnMut(BlockChange<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        if keyframe && (0..self.watches.len()).any(|position| !keyframe_holds(update(position))) {
            return Err(invalid_static(
                "keyframe contains a dependent or partial table",
            ));
        }
        // As the decoder: only the hour before's complete views are baselines,
        // and a keyframe has none.
        let before_hour = self.last_hour.filter(|_| !keyframe);
        self.changed.clear();
        self.rebuilt.clear();
        for position in 0..self.watches.len() {
            let table = self.watches[position].table;
            let fields = self.watches[position].fields;
            let visit_read = self.watches[position].read;
            let update = update(position);
            let read = metrics.table_ns[table.index()] != 0;
            let before = self.views[position].take();
            let (view, differs, rebuilt) = if let TableUpdate::Delta {
                changed: rows,
                deleted,
                ..
            } = update
            {
                // The view moves into the delta, which applies in place when
                // no visited frame still holds it. A delta always yields a new
                // view, as it does for a reader holding the hour before.
                let count = before.as_deref().and_then(complete_len);
                let view = advance_table(table, update, baseline(before_hour, before), hour, read)?;
                let differs = match (&*view, count) {
                    (TableView::Complete(after), Some(count)) => {
                        !deleted.is_empty()
                            || after.len() != count
                            || rows.iter().any(|row| touches(table, fields, row))
                    }
                    _ => true,
                };
                (view, differs, true)
            } else {
                let view = advance_table(
                    table,
                    update,
                    baseline(before_hour, before.clone()),
                    hour,
                    read,
                )?;
                let differs = before
                    .as_ref()
                    .is_none_or(|before| view_differs(table, fields, before, &view));
                // Only an unchanged table keeps the hour before's view; it is
                // still held here, so a new view cannot reuse its address.
                let rebuilt = before
                    .as_ref()
                    .is_none_or(|before| !SharedView::ptr_eq(before, &view));
                (view, differs, rebuilt)
            };
            self.views[position] = Some(view);
            if fields.is_some() && (keyframe || differs || (visit_read && read)) {
                engine::allocation::reserve(&mut self.changed, 1)?;
                self.changed.push(table);
            }
            if keyframe || rebuilt {
                engine::allocation::reserve(&mut self.rebuilt, 1)?;
                self.rebuilt.push(table);
            }
        }
        if let Some(position) = self.slots[TableId::Campaign.index()]
            && let Some(view) = &self.views[position]
        {
            validate_campaign_view(view, self.campaign_id)?;
        }
        self.last_hour = Some(hour);
        if !keyframe && self.changed.is_empty() {
            engine::allocation::reserve(&mut self.quiet.hours, 1)?;
            engine::allocation::reserve(&mut self.quiet.metrics, 1)?;
            engine::allocation::reserve(&mut self.quiet.sampled, self.views.len())?;
            engine::allocation::reserve(&mut self.quiet.rebuilt, self.views.len())?;
            self.quiet.hours.push(hour);
            self.quiet.metrics.push(metrics);
            for view in &self.views {
                self.quiet
                    .sampled
                    .push(view.as_ref().and_then(SharedView::sampled_at));
            }
            for watch in &self.watches {
                self.quiet.rebuilt.push(self.rebuilt.contains(&watch.table));
            }
            return Ok(());
        }
        self.flush(visit)?;
        let frame = DecodedFrame {
            branch: self.branch,
            hour,
            tables: std::array::from_fn(|index| match self.slots[index] {
                Some(position) => self.views[position].clone().unwrap_or_else(not_selected),
                None => not_selected(),
            }),
            metrics,
        };
        visit(BlockChange::Changed {
            frame: &frame,
            tables: &self.changed,
            rebuilt: &self.rebuilt,
        })
    }

    /// Visit the quiet hours not yet visited.
    fn flush(
        &mut self,
        visit: &mut impl FnMut(BlockChange<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.quiet.hours.is_empty() {
            return Ok(());
        }
        let visited = visit(BlockChange::Quiet(&self.quiet));
        self.quiet.clear();
        visited
    }
}

/// The hour before's view as the decoder's baseline: only a complete one.
fn baseline(hour: Option<Hour>, view: Option<SharedView>) -> Option<(Hour, SharedView)> {
    let hour = hour?;
    view.filter(|view| matches!(**view, TableView::Complete(_)))
        .map(|view| (hour, view))
}

fn complete_len(view: &TableView) -> Option<usize> {
    match view {
        TableView::Complete(rows) => Some(rows.len()),
        _ => None,
    }
}

/// Whether a delta row can change a watched field: one in its mask, or one
/// at or past the fields an older format recorded, which every row a delta
/// names takes as not recorded.
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

/// Whether a view replaced by an update other than a delta differs from the
/// hour before's in availability, a truncated table's missing count, an
/// unavailable table's reason, row keys or watched fields.
fn view_differs(
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
