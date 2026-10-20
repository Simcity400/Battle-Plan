//! A keyed family prepared block by block. A pre-pass worker reads a block
//! alone with the observer's tables, only at the hours the fields it watches
//! change ([`Observer::fields`]), and keeps those hours' frames with their
//! change facts and, for each hour between them, whether the dating table was
//! read there (its slot read, from the hour's read evidence).
//!
//! The family folds each block in order on the carried scan: each kept hour
//! is observed as an hour-by-hour walk observes it, and each hour between
//! takes the scan's unchanged-hour step ([`Scan::advance_quiet`]). When the
//! carried scan does not continue into such an hour, the rest of the block is
//! observed hour by hour.

use crate::{Observer, Scan};
use dashboard_base::prepared::kept::{Observed, consecutive, kept_changes, observe_hours};
#[cfg(any(test, feature = "test-support"))]
use dashboard_base::prepared::kept::{note_frames, note_observed};
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    prepass::{Extract, Family, Folded},
};
use replay::container::{BlockIndex, QuietHours, Replay};
use replay_data::format::FormatVersion;
use replay_frame::frame::DecodedFrame;
use std::{any::Any, io, marker::PhantomData, ops::Range};

/// The pre-pass entry of observer `O`, named `name` with its timing rows.
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

fn start<O: Observer>(format: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract::<O> {
        format,
        hours: Vec::new(),
        frames: 0,
        observer: PhantomData,
    })
}

/// One hour of a block's facts, in hour order.
#[allow(
    clippy::large_enum_variant,
    reason = "Block facts retain observed frames inline without a per-hour allocation."
)]
enum Hour {
    /// An hour the observer observes, with its frame and change facts.
    Observed {
        frame: DecodedFrame,
        changes: FrameChanges,
    },
    /// Consecutive recorded hours from `first`, after the hour before them,
    /// at which nothing the observer reads changed: whether the dating table
    /// was read at each.
    Quiet { first: i32, reads: Vec<bool> },
}

/// A block's hours as the extract kept them.
struct BlockFacts {
    hours: Vec<Hour>,
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    frames: usize,
}

struct BlockExtract<O> {
    format: FormatVersion,
    hours: Vec<Hour>,
    frames: usize,
    observer: PhantomData<fn() -> O>,
}

impl<O: Observer> Extract for BlockExtract<O> {
    fn bytes(&self) -> u64 {
        let mut held = dashboard_base::prepared::prepass::HeldBytes::default();
        held.add(self.hours.capacity() * size_of::<Hour>());
        for hour in &self.hours {
            match hour {
                Hour::Observed { frame, changes } => {
                    held.frame(frame).changes(changes);
                }
                Hour::Quiet { reads, .. } => {
                    held.add(reads.capacity());
                }
            }
        }
        held.total()
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.frames += 1;
        self.hours.push(Hour::Observed {
            frame: frame.clone(),
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
        Ok(Box::new(BlockFacts {
            hours: self.hours,
            frames: self.frames,
        }))
    }
}

/// Fold one block's facts into the carried scan. Returns how it folded and
/// the last hour it reached.
pub fn fold<O: Observer>(
    scan: &mut Scan<O>,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let facts = facts
        .downcast::<BlockFacts>()
        .map_err(|_| io::Error::other("Pre-pass facts of another family"))?;
    #[cfg(any(test, feature = "test-support"))]
    note_frames(O::family().name, facts.frames);
    let mut reached = None;
    for hour in facts.hours {
        check()?;
        match hour {
            Hour::Observed { frame, changes } => {
                reached = Some(frame.hour);
                #[cfg(any(test, feature = "test-support"))]
                note_observed(O::family().name);
                let result = scan.prepare_changed(replay.format, &frame, &changes);
                if !observed(frame.hour, result)? {
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
