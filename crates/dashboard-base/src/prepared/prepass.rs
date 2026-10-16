//! The contract of a family prepared from blocks: what it reads
//! ([`Family`]), its work on one block from the block's keyframe
//! ([`Extract`]) and how its fold of a block's facts completed ([`Folded`],
//! [`Failure`]). The dashboard's block pre-pass runs the extracts on its
//! workers and its pipeline folds each family's facts in block order.

use super::frame_changes::FrameChanges;
use replay::{
    container::{QuietHours, Replay, TableFields},
    format::FormatVersion,
    frame::DecodedFrame,
    rows::TableId,
};
use std::{any::Any, io, ops::Range};

/// A family prepared from blocks: its [`Extract`] runs on a pre-pass worker
/// from each block's keyframe, with no state from earlier blocks, and its fold
/// runs on the coordinator in block order.
pub struct Family {
    /// The family's refusal and timing name.
    pub name: &'static str,
    /// Timing row of its extracts' worker time.
    pub extract: &'static str,
    /// Timing row counting the family's blocks observed hour by hour because
    /// its extract refused or failed.
    pub sequential: &'static str,
    /// Timing row counting folds that observed their block again because the
    /// carried state broke an assumption of the fast fold.
    pub fallback: &'static str,
    /// The tables its extract reads.
    pub tables: &'static [TableId],
    /// The fields of those tables its extract reads, when its blocks are read
    /// only at the hours they change; None reads every hour. Blocks are read
    /// whole when the list cannot be made (a field the schema no longer
    /// names). Read by its changes, the extract observes the block's first
    /// hour, each hour a watched field, row key or availability changes or a
    /// table watched when read is read, and each hour that follows a gap,
    /// decodes again a table watched whole or one that is incomplete, or
    /// reaches the extract's [`Extract::deadline`]; the hours between come to
    /// [`Extract::quiet`]. Every observed hour has the change facts the block
    /// read alone has there. An hour that was not read is observed on a frame
    /// built on the last one observed: its own hour, metrics and sample
    /// hours, a new allocation of each table its update rebuilt, and the
    /// last frame's rows, so its unwatched fields are that frame's.
    pub fields: Option<fn() -> io::Result<Vec<TableFields>>>,
    /// A fresh extract for one block.
    pub start: fn(FormatVersion) -> Box<dyn Extract>,
}

/// One family's work on one block, from the block's keyframe.
pub trait Extract: Send {
    /// An hour's frame, with the change facts a tracker of the block alone
    /// finds against the hour before. In a block read by its changes
    /// ([`Family::fields`]) they hold no recorded updates:
    /// [`FrameChanges::rows`] is None for every changed table, so the extract
    /// rescans a table that changed.
    fn observe(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()>;
    /// Called before the block's first hour when the block is read only at
    /// the hours its family's fields change: [`Self::observe`] then gets the
    /// hours [`Family::fields`] names, and [`Self::quiet`] the spans between.
    fn read_by_changes(&mut self) {}
    /// The next hour the extract must observe although nothing it reads
    /// changes there (a war's end, a day's boundary); None while it keeps no
    /// deadline. Asked again after every hour it observes.
    fn deadline(&self) -> Option<i32> {
        None
    }
    /// Recorded hours `span` of `hours`, after the last hour observed, at
    /// which nothing the family reads changed: its observation of each would
    /// leave its scan's result to the hour's read evidence (in `hours`). Only
    /// the extract of a family that declares its fields is told of them.
    fn quiet(&mut self, replay: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        let _ = (replay, hours, span);
        Err(io::Error::other("This block extract observes every hour"))
    }
    /// The block's facts for the family's fold.
    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>>;
}

/// How a fold completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Folded {
    /// From the extract's facts.
    Fast,
    /// By observing the block again after its first hour.
    Fallback,
}

/// A family's own observation failed at `hour` during its fold.
pub struct Failure {
    pub hour: i32,
    pub error: io::Error,
}

/// `family`'s facts for one block read alone, every hour, with the change
/// facts a tracker of the block alone finds: what the dashboard's pre-pass
/// extracts for a family that reads every hour, for the family's own tests.
#[cfg(any(test, feature = "test-support"))]
pub fn extract_alone(
    replay: &Replay,
    index: &replay::container::BlockIndex,
    family: &Family,
) -> io::Result<Box<dyn Any + Send>> {
    let mut tables = family.tables.to_vec();
    tables.sort_unstable_by_key(|table| table.index());
    tables.dedup();
    let mut extract = (family.start)(replay.format);
    let mut tracker = super::frame_changes::FrameChangeTracker::new();
    replay.read_block_alone(
        index,
        &tables,
        |raw, frame| {
            let changes = tracker.observe_updates(frame, std::sync::Arc::new(raw), replay.format);
            extract.observe(replay, frame, &changes)
        },
        || Ok(()),
    )?;
    extract.finish()
}
