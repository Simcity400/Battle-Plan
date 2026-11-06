//! A walk's hour as the changes of the tables a reader keeps its own frame of: each table whole
//! where the walk reads it whole or its status moved, else the rows whose read fields changed and
//! the keys it deleted ([`read`]); and that frame patched with them ([`apply_frame`]).

use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_world::World;
use replay::{columnar_frames::view_heap_bytes, columns, segments};
use replay_data::{
    format::FormatVersion,
    rows::{RowChanges, TABLE_COUNT, TableId, TableRows},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TableView};
use std::{collections::BTreeSet, io, mem::size_of, sync::Arc};

/// A table's change at an hour: its whole view, or its changed rows and deleted keys.
#[derive(replay_spool::Spooled)]
pub enum Change {
    Replace(SharedView),
    Patch {
        rows: TableRows,
        deleted: Vec<u128>,
        keys_changed: bool,
    },
}

impl Change {
    /// The bytes it holds: a replaced view's are its own once the walk's next build copies it.
    pub fn heap(&self) -> usize {
        size_of::<Change>()
            + match self {
                Change::Replace(view) => view_heap_bytes(view),
                Change::Patch { rows, deleted, .. } => {
                    rows.heap_bytes() + deleted.capacity() * size_of::<u128>()
                }
            }
    }
}

/// `table`'s change at `hour` for a reader of its fields `mask`, whose rows it builds through
/// `reduce`; `shape` is the table's status the hour before. None when the hour changed none of
/// those fields and left the status as it was. Incomplete rows are never read: a truncated table
/// changes only its status.
pub fn read(
    hour: &segments::Hour<'_, World>,
    table: TableId,
    mask: u64,
    shape: Option<columns::Status>,
    (format, mode): (FormatVersion, CaptureMode),
    reduce: &dyn Fn(TableRows) -> io::Result<TableRows>,
) -> io::Result<Option<Change>> {
    let status = columns::status(hour.state, table);
    let held = columns::table(hour.state, table)?;
    let edits = columns::changes(hour.state, table)?;
    let whole = hour.starts || hour.opens || shape != Some(columns::Status::Complete);
    let rows = |slots: Vec<u32>| -> io::Result<TableRows> {
        reduce(columns::rows_at(
            hour.state,
            table,
            &slots,
            format.version(),
        )?)
    };
    let replace = match status {
        columns::Status::Complete if whole => {
            Some(TableView::Complete(rows(held.live().ones().collect())?))
        }
        columns::Status::Complete => {
            let mut slots: BTreeSet<u32> = edits.inserted.iter().copied().collect();
            for (changed, masks) in [
                (&edits.changed, &edits.masks),
                (&edits.fill_slots, &edits.fill_masks),
            ] {
                if changed.len() != masks.len() {
                    return Err(invalid("A table's slot masks miss their rows"));
                }
                slots.extend(
                    changed
                        .iter()
                        .zip(masks)
                        .filter(|(_, fields)| **fields & mask != 0)
                        .map(|(&slot, _)| slot),
                );
            }
            if slots.is_empty() && edits.deleted_keys.is_empty() {
                return Ok(None);
            }
            let rows = rows(slots.into_iter().collect())?;
            let mut deleted = edits.deleted_keys.clone();
            deleted.sort_unstable();
            deleted.dedup();
            deleted.retain(|&key| !rows.contains_key(key));
            return Ok(Some(Change::Patch {
                rows,
                deleted,
                keys_changed: !edits.inserted.is_empty() || !edits.deleted_keys.is_empty(),
            }));
        }
        columns::Status::Truncated { missing } => Some(TableView::Truncated {
            rows: TableRows::empty(table),
            missing,
        }),
        columns::Status::Unavailable { reason } => {
            let bytes = hour
                .state
                .dictionary()
                .get(reason)
                .ok_or_else(|| invalid("A table's reason is missing from its dictionary"))?;
            Some(TableView::Unavailable {
                reason: std::str::from_utf8(bytes)
                    .map_err(|_| invalid("A table's reason is not text"))?
                    .to_owned(),
            })
        }
        columns::Status::NotRecorded => Some(TableView::NotRecorded { mode }),
    };
    if !hour.starts && !hour.opens && shape == Some(status) {
        return Ok(None);
    }
    Ok(replace.map(|view| Change::Replace(view.into())))
}

/// A frame of `branch` at `hour` holding no table.
pub fn empty_frame(branch: u64, hour: i32, metrics: Metrics) -> DecodedFrame {
    DecodedFrame {
        branch,
        hour,
        metrics,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: String::new(),
            }
            .into()
        }),
    }
}

/// `changes` applied to `held`, the frame of the hour before, and the hour's change facts: each
/// changed table's rows changed, every table's at a frame's first hour (`opens`).
pub fn apply_frame(
    held: &mut Option<DecodedFrame>,
    (branch, hour): (u64, i32),
    opens: bool,
    metrics: &Metrics,
    changes: &[(TableId, Arc<Change>)],
    format: FormatVersion,
) -> io::Result<FrameChanges> {
    let frame = held.get_or_insert_with(|| empty_frame(branch, hour, Metrics::default()));
    if frame.branch != branch {
        return Err(invalid("A frame's changes crossed branches"));
    }
    frame.hour = hour;
    frame.metrics = metrics.clone();
    let mut changed = [opens; TABLE_COUNT];
    for (table, change) in changes {
        changed[table.index()] = true;
        match &**change {
            Change::Replace(view) => frame.tables[table.index()] = view.clone(),
            Change::Patch { rows, deleted, .. } => {
                let TableView::Complete(held) = &mut *frame.tables[table.index()] else {
                    return Err(invalid("A table's patch has no complete baseline"));
                };
                let previous = std::mem::replace(held, TableRows::empty(*table));
                *held = previous.apply_changes(RowChanges::new(rows.clone(), deleted.clone()))?;
            }
        }
    }
    let mut changes = FrameChanges::alone(frame, format);
    changes.rows_changed = changed;
    Ok(changes)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
