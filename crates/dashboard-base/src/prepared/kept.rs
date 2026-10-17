//! The hours a family prepared on its own keeps from a block read by its
//! changes: the observed hours' frames, with tables reduced to the fields the
//! family's observer reads, and the spans of quiet hours between them; and
//! the hour-by-hour observation of a block its fold does not continue. Shared
//! by the families whose extracts keep their hours this way (subjects, peace
//! and idea history).

use crate::prepared::frame_changes::{FrameChangeTracker, FrameChanges};
use replay::{
    container::{BlockIndex, QuietHours, Replay},
    frame::{DecodedFrame, SharedView, TableView, ViewIdentity},
    rows::{CountryRow, TableId, TableRows, tables},
};
use std::{io, ops::Range, sync::Arc};

/// One hour of a block's facts, in hour order.
pub enum Hour {
    /// An hour the family observes, with its frame and change facts.
    Observed {
        frame: DecodedFrame,
        changes: FrameChanges,
    },
    /// Consecutive recorded hours `first..=last`, after the hour before them,
    /// at which no field the family reads changed.
    Quiet { first: i32, last: i32 },
}

/// What `hours` hold ([`crate::prepared::prepass::Extract::bytes`]): each
/// kept frame's views once and its change facts.
#[allow(clippy::ptr_arg)]
pub fn hours_bytes(hours: &Vec<Hour>) -> u64 {
    let mut held = crate::prepared::prepass::HeldBytes::default();
    held.add(hours.capacity() * size_of::<Hour>());
    for hour in hours {
        if let Hour::Observed { frame, changes } = hour {
            held.frame(frame).changes(changes);
        }
    }
    held.total()
}

/// A block's hours as the extract kept them.
pub struct BlockFacts {
    pub hours: Vec<Hour>,
    /// Frames the extract was given or built.
    pub frames: usize,
}

/// The change facts of a kept hour. A block read by its changes records no
/// row updates, so none are kept.
pub fn kept_changes(changes: &FrameChanges) -> FrameChanges {
    FrameChanges {
        rows_changed: changes.rows_changed,
        table_read: changes.table_read,
        slot_read: changes.slot_read,
        updates: None,
    }
}

/// The recorded hours of `hours`' `span`, which the pre-pass gives in order
/// with no gap between them: its first and last, or None when empty. An
/// error when the hours are not consecutive, so the block is observed hour
/// by hour instead.
pub fn consecutive(hours: &QuietHours, span: Range<usize>) -> io::Result<Option<(i32, i32)>> {
    let span = &hours.hours()[span];
    let (Some(&first), Some(&last)) = (span.first(), span.last()) else {
        return Ok(None);
    };
    if i64::from(last) - i64::from(first) + 1 != span.len() as i64 {
        return Err(io::Error::other("Quiet hours are not consecutive"));
    }
    Ok(Some((first, last)))
}

/// Push hours `first..=last` after `hours`, joined to a span just before.
pub fn push_quiet(hours: &mut Vec<Hour>, first: i32, last: i32) {
    if let Some(Hour::Quiet { last: end, .. }) = hours.last_mut()
        && end.checked_add(1) == Some(first)
    {
        *end = last;
    } else {
        hours.push(Hour::Quiet { first, last });
    }
}

/// One table of a family's kept frames reduced to the fields its observer
/// reads, so a kept frame does not hold the hourly-moving rest of a large
/// table. Kept hours whose reduced rows are equal share one reduced view.
/// Only a complete view is reduced; another stays as read. The change facts
/// are kept apart, so a reduced view changes nothing the observer is told.
pub struct Reduced<R> {
    table: TableId,
    held: Option<(Vec<R>, SharedView)>,
    source: Option<ViewIdentity>,
}

impl<R: Clone + PartialEq> Reduced<R> {
    pub fn new(table: TableId) -> Self {
        Self {
            table,
            held: None,
            source: None,
        }
    }

    /// Reuse the last complete projection where the caller's field report
    /// proves its row keys and projected fields unchanged. Incomplete
    /// sources stay incomplete, and a reused view takes this hour's sample.
    pub fn reuse(&self, frame: &mut DecodedFrame) -> bool {
        let index = self.table.index();
        let Some((_, view)) = &self.held else {
            return false;
        };
        if !matches!(*frame.tables[index], TableView::Complete(_)) {
            return false;
        }
        frame.tables[index] = view
            .clone()
            .with_sampled_at(frame.tables[index].sampled_at());
        true
    }

    /// Reduce only a new source view. A carried view has the same rows;
    /// its supplying date still belongs to this hour's original view.
    pub fn reduce(
        &mut self,
        frame: &mut DecodedFrame,
        rows: impl FnOnce(&DecodedFrame) -> Option<Vec<R>>,
        wrap: impl FnOnce(Vec<R>) -> TableRows,
    ) {
        let index = self.table.index();
        if self
            .source
            .as_ref()
            .is_some_and(|source| source.is(&frame.tables[index]))
            && let Some((_, held)) = &self.held
        {
            frame.tables[index] = held
                .clone()
                .with_sampled_at(frame.tables[index].sampled_at());
            return;
        }
        #[cfg(any(test, feature = "test-support"))]
        crate::prepared::prepass::note("kept.reduce");
        let rows = rows(frame);
        self.apply(frame, rows, wrap);
    }

    /// Replace `frame`'s view of the table by `rows`, its reduced rows (None
    /// when the view is not complete), which `wrap` makes the table's rows.
    pub fn apply(
        &mut self,
        frame: &mut DecodedFrame,
        rows: Option<Vec<R>>,
        wrap: impl FnOnce(Vec<R>) -> TableRows,
    ) {
        let Some(rows) = rows else {
            return;
        };
        let index = self.table.index();
        self.source = Some(frame.tables[index].identity());
        let view = match &self.held {
            Some((held, view)) if *held == rows => view.clone(),
            _ => {
                let view: SharedView = TableView::Complete(wrap(rows.clone())).into();
                self.held = Some((rows, view.clone()));
                view
            }
        };
        let sampled = frame.tables[index].sampled_at();
        frame.tables[index] = view.with_sampled_at(sampled);
    }
}

impl Reduced<CountryRow> {
    /// The countries as these families read them: the id, canonical slot and
    /// tag of each (`CountryKeys`).
    pub fn countries(&mut self, frame: &mut DecodedFrame) {
        let index = TableId::Countries.index();
        if self
            .source
            .as_ref()
            .is_none_or(|source| !source.is(&frame.tables[index]))
            && let Some((held, view)) = &self.held
            && frame.complete::<tables::Countries>().is_ok_and(|rows| {
                held.len() == rows.len()
                    && held.iter().zip(rows).all(|(held, row)| {
                        (held.id, held.canonical_slot, &held.tag)
                            == (row.id, row.canonical_slot, &row.tag)
                    })
            })
        {
            self.source = Some(frame.tables[index].identity());
            frame.tables[index] = view
                .clone()
                .with_sampled_at(frame.tables[index].sampled_at());
            return;
        }
        self.reduce(
            frame,
            |frame| {
                frame.complete::<tables::Countries>().ok().map(|rows| {
                    #[cfg(any(test, feature = "test-support"))]
                    crate::prepared::prepass::note("kept.countries");
                    rows.iter()
                        .map(|row| CountryRow {
                            id: row.id,
                            canonical_slot: row.canonical_slot,
                            tag: row.tag.clone(),
                            ..Default::default()
                        })
                        .collect()
                })
            },
            TableRows::Countries,
        );
    }
}

/// A family's observation of one hour's outcome: `Ok(true)` while it goes on,
/// `Ok(false)` once it refused the branch.
pub type Observed<'a> = dyn FnMut(i32, io::Result<()>) -> io::Result<bool> + 'a;

/// Read the block alone with `tables` and give `observe` each hour from
/// `from` (every hour without it), with the change facts of the block alone;
/// each outcome goes to `observed`, until the family refuses. Returns the
/// last hour observed.
pub fn observe_hours(
    replay: &Replay,
    index: &BlockIndex,
    tables: &[TableId],
    from: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
    mut observe: impl FnMut(&DecodedFrame, &FrameChanges) -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let _cpu = ::replay::cpu::scope("fold driver", None);
    let mut tracker = FrameChangeTracker::new();
    let mut reached = None;
    let mut stopped = false;
    let mut failure = None;
    let read = replay.read_block_alone(
        index,
        tables,
        |raw, frame| {
            let _cpu = ::replay::cpu::scope("fold driver", None);
            let facts = tracker.observe_updates(frame, Arc::new(raw), replay.format);
            if stopped || from.is_some_and(|from| frame.hour < from) {
                return Ok(());
            }
            reached = Some(frame.hour);
            let result = {
                let _cpu = ::replay::cpu::scope("fold observe", None);
                observe(frame, &facts)
            };
            match observed(frame.hour, result) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Family observation stopped"))
                }
            }
        },
        || check(),
    );
    if let Some(error) = failure {
        return Err(error);
    }
    read?;
    Ok(reached)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// By family: the hours its fold observed on carried state, and the
    /// frames its extracts were given or built.
    static COUNTS: std::cell::RefCell<std::collections::BTreeMap<&'static str, (usize, usize)>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn note_observed(family: &'static str) {
    COUNTS.with(|counts| counts.borrow_mut().entry(family).or_default().0 += 1);
}

#[cfg(any(test, feature = "test-support"))]
pub fn note_frames(family: &'static str, frames: usize) {
    COUNTS.with(|counts| counts.borrow_mut().entry(family).or_default().1 += frames);
}

/// The hours `family`'s folds observed on this thread (kept hours and hours
/// observed one by one), and the frames the extracts of the blocks it folded
/// were given or built: every hour of a block read whole, only the hours
/// read and those observed between them of a block read by its changes.
#[cfg(any(test, feature = "test-support"))]
pub fn counted(family: &str) -> (usize, usize) {
    COUNTS.with(|counts| counts.borrow().get(family).copied().unwrap_or_default())
}

#[cfg(test)]
mod tests;
