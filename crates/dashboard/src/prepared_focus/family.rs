//! The focus order prepared on its own. A pre-pass worker reads a block alone
//! with the four focus tables, only at the hours a field the focus
//! observation reads changes ([`fields`]), and builds the block's focus facts
//! ([`Builder`]); the family's thread folds them in block order on the
//! carried state ([`FocusIndex::fold_block`]). The hourly walk builds the same
//! facts from every hour, so both publish the same pages. Nothing here reads
//! content or depends on the time a commit happens.
use super::{Builder, Facts, FocusIndex, TABLES};
use crate::prepared::{
    frame_changes::FrameChanges,
    prepass::{Extract, Family, Folded},
};
use replay::{
    container::{BlockIndex, QuietHours, Replay, TableFields},
    format::FormatVersion,
    frame::DecodedFrame,
    rows::TableId,
};
use std::{any::Any, io, ops::Range};

pub(crate) static FAMILY: Family = Family {
    name: "focus",
    extract: "focus.extract",
    sequential: "focus.sequential",
    fallback: "focus.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};

/// The fields the focus observation reads, so a block is read only at the
/// hours they change. Campaign is checked on every read and read by nothing
/// here.
pub(crate) fn fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // A country's identity: its id (the row key), canonical slot and tag.
        // Its balances change every hour and are not read.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        // Each raw link's slot: a country's slot check and its focus row's.
        TableFields::named(TableId::CountryLinks, &["canonical_slot"])?,
        // The order (active focus, continuous focus, pause and the completed
        // count it is checked against) and the progress the focus hour reads
        // back from the block. The focus tree is not read.
        TableFields::named(
            TableId::Focus,
            &[
                "current",
                "continuous",
                "progress",
                "paused",
                "completed_count",
            ],
        )?,
        // The completed focuses (owner and ordinal are the row key).
        TableFields::named(TableId::CompletedFocuses, &["token"])?,
    ])
}

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        builder: Builder::new(None),
    })
}

/// One block's focus facts, built on a pre-pass worker.
struct BlockExtract {
    builder: Builder,
}

impl Extract for BlockExtract {
    fn observe(&mut self, _: &Replay, frame: &DecodedFrame, _: &FrameChanges) -> io::Result<()> {
        self.builder.hour(frame)
    }

    /// Hours at which no field the focus observation reads changed: each
    /// country's observation holds through them.
    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        self.builder.quiet(&hours.hours()[span])
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        let facts = self.builder.finish();
        if facts.last_hour().is_none() {
            return Err(io::Error::other("Focus block has no hours"));
        }
        Ok(Box::new(facts))
    }
}

/// The family's observation of one hour's outcome: `Ok(true)` while it goes
/// on, `Ok(false)` once it refused the branch.
pub(crate) type Observed<'a> = dyn FnMut(i32, io::Result<()>) -> io::Result<bool> + 'a;

/// Fold one block's facts on the carried state. Its outcome is the hourly
/// walk's at the block's last hour, where that walk folds the block.
pub(crate) fn fold(
    index: &mut FocusIndex,
    facts: Box<dyn Any + Send>,
    block: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let facts = facts
        .downcast::<Facts>()
        .map_err(|_| io::Error::other("Pre-pass facts of another family"))?;
    #[cfg(test)]
    FRAMES.with(|count| count.set(count.get() + facts.frames));
    let Some(last) = facts.last_hour() else {
        return Ok((Folded::Fast, None));
    };
    let result = index.fold_block(block.branch, *facts, check);
    observed(last, result)?;
    Ok((Folded::Fast, Some(last)))
}

/// Observe the block hour by hour from its own read, as the hourly walk does.
/// Returns the last hour observed.
pub(crate) fn observe_block(
    index: &mut FocusIndex,
    replay: &Replay,
    block: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let mut reached = None;
    let mut stopped = false;
    let mut failure = None;
    let bounds = (block.first_hour, block.last_hour);
    let read = replay.read_block_alone(
        block,
        TABLES,
        |_, frame| {
            if stopped {
                return Ok(());
            }
            reached = Some(frame.hour);
            #[cfg(test)]
            FRAMES.with(|count| count.set(count.get() + 1));
            let result = index.observe_hour(block.branch, bounds, frame, &mut || Ok(()));
            match observed(frame.hour, result) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Focus observation stopped"))
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

#[cfg(test)]
thread_local! {
    static FRAMES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Frames the focus family read on this thread: each block's built frames
/// when folded from its extract, every hour of a block observed hour by
/// hour. A family prepared alone runs on the caller's thread.
#[cfg(test)]
pub(crate) fn frames() -> usize {
    FRAMES.with(std::cell::Cell::get)
}
