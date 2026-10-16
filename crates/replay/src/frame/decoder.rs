use super::{
    DecodedFrame, Frame, SharedView, TABLE_COUNT, TableUpdate, TableView, validate_applied,
    validate_complete, validate_delta, validate_rows, validate_singleton,
};
use crate::{Hour, codec::invalid, rows::TableId};
use engine::allocation;
use std::{io, sync::LazyLock};

mod resume;

#[derive(Default)]
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
    pub fn apply(&mut self, frame: &Frame) -> io::Result<DecodedFrame> {
        self.advance(frame).cloned()
    }

    /// Advance the same decoder without copying its rows into a separate output.
    /// The borrowed hour remains available for validation before the next advance.
    pub fn advance(&mut self, frame: &Frame) -> io::Result<&DecodedFrame> {
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.branch == frame.branch && current.hour >= frame.hour)
        {
            return Err(invalid("frame hour did not advance on its branch"));
        }
        // Reject frame-wide errors before moving any baseline. From here onward,
        // a failed update clears only its table and becomes a named unavailable view.
        if frame.keyframe
            && frame.tables.iter().any(|update| {
                !matches!(
                    update,
                    TableUpdate::Snapshot(_)
                        | TableUpdate::Unavailable { .. }
                        | TableUpdate::Incompatible { .. }
                )
            })
        {
            return Err(invalid("keyframe contains a dependent or partial table"));
        }
        let mut previous = self
            .current
            .take()
            .filter(|current| !frame.keyframe && current.branch == frame.branch)
            .map(|current| {
                // Every complete table advances its baseline to the enclosing
                // hour, including unchanged tables after a recorded gap.
                current.tables.map(|table| {
                    matches!(*table, TableView::Complete(_)).then_some((current.hour, table))
                })
            })
            .unwrap_or_else(|| std::array::from_fn(|_| None));
        let mut allocation_failure = None;
        let tables = std::array::from_fn(|index| {
            if allocation_failure.is_some() {
                return not_selected();
            }
            let table = TableId::ALL[index];
            if self
                .selected
                .as_ref()
                .is_some_and(|selected| !selected[index])
            {
                return not_selected();
            }
            match advance_table(
                table,
                &frame.tables[index],
                previous[index].take(),
                frame.hour,
                frame.metrics.table_ns[index] != 0,
            ) {
                Ok(view) => view,
                Err(error) => {
                    allocation_failure = Some(error);
                    not_selected()
                }
            }
        });
        if let Some(error) = allocation_failure {
            return Err(error);
        }
        self.current = Some(DecodedFrame {
            branch: frame.branch,
            hour: frame.hour,
            tables,
            metrics: frame.metrics.clone(),
        });
        Ok(self.current.as_ref().unwrap())
    }
}

/// One table's view at `hour`, from its update and the hour before's complete
/// view with that hour (`previous`), as [`FrameDecoder::advance`] reconstructs
/// it: a failed update becomes a named unavailable view, and a complete table
/// `read` this hour (its own nonzero timing) is sampled at it. Only an
/// allocation failure is an error.
pub(crate) fn advance_table(
    table: TableId,
    update: &TableUpdate,
    previous: Option<(Hour, SharedView)>,
    hour: Hour,
    read: bool,
) -> io::Result<SharedView> {
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
pub(crate) fn not_selected() -> SharedView {
    static VIEW: LazyLock<SharedView> = LazyLock::new(|| {
        TableView::Unavailable {
            reason: "Table not selected for reconstruction".into(),
        }
        .into()
    });
    VIEW.clone()
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
            Ok(TableView::Complete(values.try_clone_validated()?).into())
        }
        TableUpdate::Truncated { rows, missing } => {
            validate_rows(table, rows)?;
            if *missing == 0 {
                return Err(invalid("truncated table has no missing rows"));
            }
            Ok(TableView::Truncated {
                rows: rows.try_clone_validated()?,
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
                            let rows = previous_rows.apply_delta_shared(changed, deleted)?;
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
