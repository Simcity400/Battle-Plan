//! A keyed family prepared block by block. A pre-pass worker reads a block
//! alone with the observer's tables, only at the hours the fields it watches
//! change ([`Observer::fields`]), and keeps those hours' frames with their
//! change facts and, for each hour between them, whether the dating table was
//! read there (its slot read, from the hour's read evidence).
//!
//! The family folds each block in order on the carried scan: each kept hour
//! is observed as an hour-by-hour walk observes it, and each hour between
//! takes the scan's unchanged-hour step (`Scan::advance_quiet`). When the
//! carried scan does not continue into such an hour, the rest of the block is
//! observed hour by hour.

use crate::Observer;
#[cfg(any(test, feature = "test-support"))]
use crate::Scan;
#[cfg(any(test, feature = "test-support"))]
use dashboard_base::prepared::kept::{Observed, consecutive, kept_changes, observe_hours};
#[cfg(any(test, feature = "test-support"))]
use dashboard_base::prepared::kept::{note_frames, note_observed};
use dashboard_base::prepared::prepass::HeldBytes;
#[cfg(any(test, feature = "test-support"))]
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    prepass::{Extract, Family, Folded},
};
#[cfg(any(test, feature = "test-support"))]
use replay::campaign::{BlockIndex, QuietHours, Replay};
use replay_data::format::FormatVersion;
use replay_frame::frame::DecodedFrame;
#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::FrameLog;
use std::io;
#[cfg(any(test, feature = "test-support"))]
use std::{any::Any, marker::PhantomData, ops::Range};

/// The pre-pass entry of observer `O`, named `name` with its timing rows.
#[cfg(any(test, feature = "test-support"))]
pub const fn family<O: Observer>(
    name: &'static str,
    extract: &'static str,
    sequential: &'static str,
    fallback: &'static str,
) -> Family {
    Family {
        name,
        extract,
        sequential,
        fallback,
        tables: O::TABLES,
        fields: Some(O::fields),
        start: start::<O>,
    }
}

#[cfg(any(test, feature = "test-support"))]
fn start<O: Observer>(format: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract::<O> {
        format,
        hours: Vec::new(),
        kept: FrameLog::new(),
        frames: 0,
        keeper: O::keeper(),
        observer: PhantomData,
    })
}

/// Only the operands an observer needs, detached from the reconstructed source tables.
pub trait Operands<O: Observer>: Send {
    fn observe(&self, observer: &mut O, values: &mut crate::Values) -> io::Result<bool>;
    fn observe_changes(
        &self,
        observer: &mut O,
        names: &crate::delta::Names<'_>,
        delta: &mut crate::Delta,
    ) -> io::Result<Option<bool>> {
        let _ = (observer, names, delta);
        Ok(None)
    }
    /// Whether the hour gave `table` whole, as `observer` holds it from the last hour it
    /// observed: a native segment's first hour restating it, which is then unchanged.
    fn restates(&self, observer: &O, table: replay_data::rows::TableId) -> bool {
        let _ = (observer, table);
        false
    }
    /// A native emit found the hour quiet and observes none of it: what the observer mirrors
    /// of its keeper's hours goes on past it, publishing nothing.
    fn skipped(&self, observer: &mut O) -> io::Result<()> {
        let _ = observer;
        Ok(())
    }
    fn held(&self, bytes: &mut HeldBytes);
    /// Writes them for a native segment's emit, which reads them back by
    /// [`Observer::unspool_operands`].
    fn spool(&self, out: &mut replay_spool::Writer<'_>) -> io::Result<()> {
        let _ = out;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "These operands are never spooled",
        ))
    }
}

/// Copies changed operands while the driver lends the current hour; never keeps source views.
#[cfg(any(test, feature = "test-support"))]
pub trait Keeper<O: Observer>: Send {
    fn keep(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
    ) -> io::Result<Box<dyn Operands<O>>>;
    fn held(&self, bytes: &mut HeldBytes);
}

#[cfg(any(test, feature = "test-support"))]
enum Input<O: Observer> {
    /// The hour's frame: the next its block's kept frames hold.
    Frame,
    Operands(Box<dyn Operands<O>>),
}

pub(crate) struct Borrowed<'a> {
    pub format: FormatVersion,
    pub frame: &'a DecodedFrame,
}

impl<O: Observer> Operands<O> for Borrowed<'_> {
    fn observe(&self, observer: &mut O, values: &mut crate::Values) -> io::Result<bool> {
        observer.observe(self.format, self.frame, values)
    }
    fn observe_changes(
        &self,
        observer: &mut O,
        names: &crate::delta::Names<'_>,
        delta: &mut crate::Delta,
    ) -> io::Result<Option<bool>> {
        observer.observe_changes(self.format, self.frame, names, delta)
    }
    fn held(&self, _: &mut HeldBytes) {}
}

/// One hour of a block's facts, in hour order.
#[cfg(any(test, feature = "test-support"))]
#[allow(
    clippy::large_enum_variant,
    reason = "Block facts retain observed frames inline without a per-hour allocation."
)]
enum Hour<O: Observer> {
    /// An hour the observer observes, with its frame and change facts.
    Observed {
        hour: i32,
        input: Input<O>,
        changes: FrameChanges,
    },
    /// Consecutive recorded hours from `first`, after the hour before them,
    /// at which nothing the observer reads changed: whether the dating table
    /// was read at each.
    Quiet { first: i32, reads: Vec<bool> },
}

/// A block's hours as the extract kept them.
#[cfg(any(test, feature = "test-support"))]
struct BlockFacts<O: Observer> {
    hours: Vec<Hour<O>>,
    kept: FrameLog,
    frames: usize,
}

#[cfg(any(test, feature = "test-support"))]
struct BlockExtract<O: Observer> {
    format: FormatVersion,
    hours: Vec<Hour<O>>,
    kept: FrameLog,
    frames: usize,
    keeper: Option<Box<dyn Keeper<O>>>,
    observer: PhantomData<fn() -> O>,
}

#[cfg(any(test, feature = "test-support"))]
impl<O: Observer> Extract for BlockExtract<O> {
    fn bytes(&self) -> u64 {
        let mut held = dashboard_base::prepared::prepass::HeldBytes::default();
        held.add(self.hours.capacity() * size_of::<Hour<O>>());
        for hour in &self.hours {
            match hour {
                Hour::Observed { input, changes, .. } => {
                    if let Input::Operands(input) = input {
                        input.held(&mut held);
                    }
                    held.changes(changes);
                }
                Hour::Quiet { reads, .. } => {
                    held.add(reads.capacity());
                }
            }
        }
        if let Some(keeper) = &self.keeper {
            keeper.held(&mut held);
        }
        for view in self.kept.views() {
            held.view(view);
        }
        held.add(self.kept.bytes());
        held.total()
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.frames += 1;
        let input = match &mut self.keeper {
            Some(keeper) => Input::Operands(keeper.keep(self.format, frame)?),
            None => {
                self.kept.push(frame)?;
                Input::Frame
            }
        };
        self.hours.try_reserve(1).map_err(io::Error::other)?;
        self.hours.push(Hour::Observed {
            hour: frame.hour,
            input,
            changes: kept_changes(changes),
        });
        Ok(())
    }

    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        let Some((first, _)) = consecutive(hours, span.clone())? else {
            return Ok(());
        };
        let format = self.format;
        let reads = span.map(|position| {
            dashboard_text::metadata::slot_read(
                format,
                &hours.metrics()[position],
                O::READ,
                hours.hours()[position],
            )
        });
        match self.hours.last_mut() {
            Some(Hour::Quiet {
                first: held,
                reads: held_reads,
            }) if i64::from(*held) + held_reads.len() as i64 == i64::from(first) => {
                held_reads.extend(reads);
            }
            _ => self.hours.push(Hour::Quiet {
                first,
                reads: reads.collect(),
            }),
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other(format!("{} block has no hours", O::NAME)));
        }
        Ok(Box::new(BlockFacts::<O> {
            hours: self.hours,
            kept: self.kept,
            frames: self.frames,
        }))
    }
}

/// Fold one block's facts into the carried scan. Returns how it folded and
/// the last hour it reached.
#[cfg(any(test, feature = "test-support"))]
pub fn fold<O: Observer>(
    scan: &mut Scan<O>,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let facts = facts
        .downcast::<BlockFacts<O>>()
        .map_err(|_| io::Error::other("Pre-pass facts of another family"))?;
    #[cfg(any(test, feature = "test-support"))]
    note_frames(O::family().name, facts.frames);
    let BlockFacts { hours, kept, .. } = *facts;
    let mut kept = kept.into_frames();
    let mut reached = None;
    for hour in hours {
        check()?;
        match hour {
            Hour::Observed {
                hour,
                input,
                changes,
            } => {
                reached = Some(hour);
                #[cfg(any(test, feature = "test-support"))]
                note_observed(O::family().name);
                let result = match &input {
                    Input::Frame => {
                        let frame = kept.next().ok_or_else(|| {
                            io::Error::other("Pre-pass facts lack an observed hour's frame")
                        })??;
                        scan.prepare_changed(replay.format, &frame, &changes)
                    }
                    Input::Operands(input) => scan.prepare_kept(hour, &changes, input.as_ref()),
                };
                if !observed(hour, result)? {
                    return Ok((Folded::Fast, reached));
                }
            }
            Hour::Quiet { first, reads } => {
                for (offset, read) in reads.into_iter().enumerate() {
                    // The span's hours are consecutive i32 hours.
                    let hour = first + offset as i32;
                    if !scan.continues_to(hour) {
                        let reached =
                            observe_block(scan, replay, index, Some(hour), observed, check)?;
                        return Ok((Folded::Fallback, reached));
                    }
                    reached = Some(hour);
                    let result = scan.advance_quiet(hour, read);
                    if !observed(hour, result)? {
                        return Ok((Folded::Fast, reached));
                    }
                }
            }
        }
    }
    Ok((Folded::Fast, reached))
}

/// Observe the block hour by hour from `from` (every hour without it), with
/// the change facts of the block alone. Returns the last hour observed.
#[cfg(any(test, feature = "test-support"))]
pub fn observe_block<O: Observer>(
    scan: &mut Scan<O>,
    replay: &Replay,
    index: &BlockIndex,
    from: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    observe_hours(
        replay,
        index,
        O::TABLES,
        from,
        observed,
        check,
        |frame, facts| {
            #[cfg(any(test, feature = "test-support"))]
            note_observed(O::family().name);
            scan.prepare_changed(replay.format, frame, facts)
        },
    )
}
