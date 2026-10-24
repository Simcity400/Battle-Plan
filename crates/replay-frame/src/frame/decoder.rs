use super::{
    DecodedFrame, SharedView, TABLE_COUNT, TableUpdate, TableView, validate_applied,
    validate_complete, validate_delta, validate_rows, validate_singleton,
};
use crate::{Hour, codec::invalid, rows::TableId};
use engine_allocation as allocation;
use std::{io, sync::LazyLock};

mod resume;

#[derive(Clone, Default)]
pub struct FrameDecoder {
    current: Option<DecodedFrame>,
    selected: Option<[bool; TABLE_COUNT]>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconstruct only these tables, retaining the normal wire validation and
    /// baseline rules. Omitted tables are explicitly unavailable, never empty.
    pub fn for_tables(tables: &[TableId]) -> Self {
        let mut selected = [false; TABLE_COUNT];
        for table in tables {
            selected[table.index()] = true;
        }
        Self {
            current: None,
            selected: Some(selected),
        }
    }

    /// Decode an hour and return a frame callers can retain. It shares every
    /// table with the decoder, so it copies no rows; a table is copied only
    /// when either side writes to it.
    pub fn apply(
        &mut self,
        frame: &(impl super::FrameSource + ?Sized),
    ) -> io::Result<DecodedFrame> {
        self.advance(frame).cloned()
    }

    /// Advance the same decoder without copying its rows into a separate output.
    /// The borrowed hour remains available for validation before the next advance.
    pub fn advance(
        &mut self,
        frame: &(impl super::FrameSource + ?Sized),
    ) -> io::Result<&DecodedFrame> {
        // Measurement only: each table's reconstruct cycles, when enabled.
        let _cpu = crate::cpu::scope("reconstruct", None);
        let measured = crate::cpu::enabled();
        let mut spent: Vec<(usize, u64)> = Vec::new();
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.branch == frame.branch() && current.hour >= frame.hour())
        {
            return Err(invalid("frame hour did not advance on its branch"));
        }
        // Reject frame-wide errors before moving any baseline. From here onward,
        // a failed update clears only its table and becomes a named unavailable view.
        if frame.keyframe() && !(0..TABLE_COUNT).all(|index| keyframe_update(frame.update(index))) {
            return Err(invalid("keyframe contains a dependent or partial table"));
        }
        let selected = |index: usize| {
            self.selected
                .as_ref()
                .is_none_or(|selected| selected[index])
        };
        // The hour before's views. An unselected table's is the shared
        // unselected view, which moves on unchanged; a selected table's is
        // its baseline only on the same branch and outside a keyframe, and
        // is released before this hour's views are built otherwise.
        let current = self.current.take();
        let continues = current
            .as_ref()
            .is_some_and(|current| !frame.keyframe() && current.branch == frame.branch());
        let (previous_hour, mut held) = match current {
            Some(current) => (current.hour, current.tables.map(Some)),
            None => (frame.hour(), std::array::from_fn(|_| None)),
        };
        if !continues {
            for (index, view) in held.iter_mut().enumerate() {
                if selected(index) {
                    *view = None;
                }
            }
        }
        let mut allocation_failure = None;
        let tables = std::array::from_fn(|index| {
            if !selected(index) {
                return held[index].take().unwrap_or_else(not_selected);
            }
            if allocation_failure.is_some() {
                return not_selected();
            }
            let table = TableId::ALL[index];
            // Every complete table advances its baseline to the enclosing
            // hour, including unchanged tables after a recorded gap.
            let previous = held[index]
                .take()
                .filter(|view| matches!(**view, TableView::Complete(_)))
                .map(|view| (previous_hour, view));
            let started = if measured { crate::cpu::stamp() } else { 0 };
            let advanced = advance_public_table(
                table,
                frame.update(index),
                previous,
                frame.hour(),
                frame.metrics().table_ns[index] != 0,
            );
            if measured {
                spent.push((index, crate::cpu::stamp().wrapping_sub(started)));
            }
            match advanced {
                Ok(view) => view,
                Err(error) => {
                    allocation_failure = Some(error);
                    not_selected()
                }
            }
        });
        if measured {
            crate::cpu::reconstructed(frame.hour(), &spent);
        }
        if let Some(error) = allocation_failure {
            return Err(error);
        }
        self.current = Some(DecodedFrame {
            branch: frame.branch(),
            hour: frame.hour(),
            tables,
            metrics: frame.metrics().clone(),
        });
        Ok(self.current.as_ref().unwrap())
    }
}

/// Whether a keyframe may hold `update`: one that needs no earlier hour.
/// [`FrameDecoder::advance`] refuses a keyframe holding any other, whichever
/// tables it reconstructs.
pub fn keyframe_update(update: &TableUpdate) -> bool {
    matches!(
        update,
        TableUpdate::Snapshot(_)
            | TableUpdate::Unavailable { .. }
            | TableUpdate::NotRecorded { .. }
            | TableUpdate::Incompatible { .. }
    )
}

/// Apply a public table view, separately attributed from private alias source applications.
pub fn advance_public_table(
    table: TableId,
    update: &TableUpdate,
    previous: Option<(Hour, SharedView)>,
    hour: Hour,
    read: bool,
) -> io::Result<SharedView> {
    let _cpu = crate::cpu::reader("reader_public_apply", table);
    advance_table(table, update, previous, hour, read)
}

/// One table's view at `hour`, from its update and the hour before's complete
/// view with that hour (`previous`), as [`FrameDecoder::advance`] reconstructs
/// it: a failed update becomes a named unavailable view, and a complete table
/// `read` this hour (its own nonzero timing) is sampled at it. Only an
/// allocation failure is an error.
pub fn advance_table(
    table: TableId,
    update: &TableUpdate,
    previous: Option<(Hour, SharedView)>,
    hour: Hour,
    read: bool,
) -> io::Result<SharedView> {
    let _table = replay_data::measurement::table(table);
    match apply_table(table, update, previous, hour) {
        Ok(mut view) => {
            // The collector exports zero timing for skipped tables
            // (sampling::exported_table_ticks). Equality may encode a
            // successful acquisition as Unchanged; completeness and this
            // table's own timing, not the wire update, witness it.
            if matches!(*view, TableView::Complete(_)) && read {
                view.1 = Some(hour);
            }
            Ok(view)
        }
        Err(error) if allocation::is_allocation_error(&error) => Err(error),
        Err(error) => Ok(TableView::Unavailable {
            reason: error.to_string(),
        }
        .into()),
    }
}

/// One view shared by every unselected table of every hour.
pub fn not_selected() -> SharedView {
    #[cfg(any(test, feature = "test-support"))]
    UNSELECTED_SHARES.with(|count| count.set(count.get() + 1));
    static VIEW: LazyLock<SharedView> = LazyLock::new(|| {
        TableView::Unavailable {
            reason: "Table not selected for reconstruction".into(),
        }
        .into()
    });
    VIEW.clone()
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Times this thread shared the unselected view anew.
    static UNSELECTED_SHARES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `read`'s result and the times it shared the unselected view anew on this thread.
#[cfg(any(test, feature = "test-support"))]
pub fn count_unselected<T>(read: impl FnOnce() -> T) -> (T, usize) {
    let before = UNSELECTED_SHARES.with(std::cell::Cell::get);
    let result = read();
    (
        result,
        UNSELECTED_SHARES.with(std::cell::Cell::get) - before,
    )
}

/// An unchanged table is the previous hour's allocation, shared rather than
/// copied; a delta takes the baseline by value, which copies it only when an
/// earlier hour's frame still holds it.
fn apply_table(
    table: TableId,
    update: &TableUpdate,
    previous: Option<(Hour, SharedView)>,
    hour: Hour,
) -> io::Result<SharedView> {
    match update {
        TableUpdate::Snapshot(values) => {
            validate_complete(table, values, hour)?;
            let rows = values.try_clone_validated()?;
            replay_data::measurement::count(replay_data::measurement::Counter::SnapshotCopies, 1);
            replay_data::measurement::count(
                replay_data::measurement::Counter::SnapshotRows,
                values.len(),
            );
            Ok(TableView::Complete(rows).into())
        }
        TableUpdate::Truncated { rows, missing } => {
            validate_rows(table, rows)?;
            if *missing == 0 {
                return Err(invalid("truncated table has no missing rows"));
            }
            let rows = rows.try_clone_validated()?;
            replay_data::measurement::count(replay_data::measurement::Counter::SnapshotCopies, 1);
            replay_data::measurement::count(
                replay_data::measurement::Counter::SnapshotRows,
                rows.len(),
            );
            Ok(TableView::Truncated {
                rows,
                missing: *missing,
            }
            .into())
        }
        TableUpdate::Unavailable { reason } | TableUpdate::Incompatible { reason, .. } => {
            if reason.is_empty() {
                return Err(invalid("unavailable table needs a reason"));
            }
            Ok(TableView::Unavailable {
                reason: allocation::string(reason)?,
            }
            .into())
        }
        TableUpdate::NotRecorded { mode } => Ok(TableView::NotRecorded { mode: *mode }.into()),
        TableUpdate::Delta { baseline, .. } | TableUpdate::Unchanged { baseline } => {
            let Some((_, previous_view)) =
                previous.filter(|(previous_hour, _)| previous_hour == baseline && *baseline < hour)
            else {
                return Err(invalid(format!(
                    "missing table baseline at hour {baseline}"
                )));
            };
            if let TableUpdate::Delta {
                changed, deleted, ..
            } = update
            {
                validate_delta(table, changed, deleted)?;
                // The baseline passed full row validation, so an in-place
                // delta checks only the keys it can have changed.
                let rows = match previous_view.try_unwrap() {
                    Ok(TableView::Complete(previous_rows)) => {
                        previous_rows.apply_delta_validated(changed, deleted)?
                    }
                    Err(shared) => match &*shared {
                        TableView::Complete(previous_rows) => {
                            let started = if crate::cpu::enabled() {
                                crate::cpu::stamp()
                            } else {
                                0
                            };
                            let rows = previous_rows.apply_delta_shared(changed, deleted)?;
                            crate::cpu::copied(table, previous_rows.len(), started);
                            rows.validate_keys()?;
                            rows
                        }
                        _ => unreachable!("only complete tables become baselines"),
                    },
                    Ok(_) => unreachable!("only complete tables become baselines"),
                };
                validate_applied(table, &rows, hour)?;
                Ok(TableView::Complete(rows).into())
            } else {
                // This private baseline already passed full row validation. Only
                // the singleton's relation to the new enclosing hour can have changed.
                let TableView::Complete(previous_rows) = &*previous_view else {
                    unreachable!("only complete tables become baselines");
                };
                validate_singleton(table, previous_rows, hour)?;
                Ok(previous_view)
            }
        }
    }
}
