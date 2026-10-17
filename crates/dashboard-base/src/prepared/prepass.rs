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

/// A family's reads for one run, chosen from the content the run binds: the
/// tables and fields its extract reads in place of [`Family::tables`] and
/// [`Family::fields`], and the extract that reads them. A family whose
/// content decides what its observation can read (a trigger kit that only
/// some content evaluates) names them per run, so each block's read holds
/// only what this run's observation can reach. Its fold binds the same
/// content, and its facts say what its extract read.
pub struct Bound {
    /// The tables its extract reads.
    pub tables: Vec<TableId>,
    /// The fields of those tables its extract reads, as [`Family::fields`]
    /// gives them; None reads every hour.
    pub fields: Option<Vec<TableFields>>,
    /// A fresh extract for one block of this run.
    pub start: Box<dyn Fn(FormatVersion) -> Box<dyn Extract> + Send + Sync>,
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
    /// [`Self::observe`] of an hour of a block read by its changes whose
    /// read watches exactly the family's fields, with the watched tables
    /// the block reader reports changed since the hour before (its
    /// `BlockChange::Changed` tables, in `TableId` order: each whose watched
    /// fields, row keys or availability changed, or that is watched when
    /// read and was read; every watched table at the block's first hour;
    /// none at an hour built in a quiet span). An extract that would find
    /// them again from the rows takes them instead.
    fn observe_reported(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        reported: &[TableId],
    ) -> io::Result<()> {
        let _ = reported;
        self.observe(replay, frame, changes)
    }
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
    /// The bytes the facts [`Self::finish`] returns hold, about: what a block
    /// extracted ahead holds until its family takes it, which the pre-pass
    /// counts against its read-ahead budget beside the block's recorded raw
    /// bytes. An extract whose facts stay small (a few counts, spans or
    /// keys) reports none; one keeping rows or frames counts them, never
    /// much less than they hold.
    fn bytes(&self) -> u64 {
        0
    }
    /// The block's facts for the family's fold.
    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>>;
}

/// What a block's facts hold, counted as an extract keeps them, for
/// [`Extract::bytes`]: each table view once however many kept frames share
/// it, at the capacity of its rows and every value they own (as the frame
/// cache counts it); each hour's recorded updates once; and whatever else
/// the extract adds: every value it keeps beside them, its maps, sets,
/// strings and serialized rows, through [`Held`].
#[derive(Default)]
pub struct HeldBytes {
    /// The views, recorded updates and shared values counted, by allocation.
    seen: std::collections::HashSet<usize>,
    total: u64,
}

mod held;
pub use held::Held;

impl HeldBytes {
    /// A kept frame: its every table view not yet counted.
    pub fn frame(&mut self, frame: &DecodedFrame) -> &mut Self {
        self.value(frame)
    }

    /// A kept table view, once per allocation.
    pub fn view(&mut self, view: &replay::frame::SharedView) -> &mut Self {
        let table: &replay::frame::TableView = view;
        if self
            .seen
            .insert(std::ptr::from_ref(table) as *const () as usize)
        {
            self.add(super::query::frames::view_bytes(table));
        }
        self
    }

    /// Kept change facts: their flags, and their recorded updates once.
    pub fn changes(&mut self, changes: &FrameChanges) -> &mut Self {
        self.value(changes)
    }

    /// Change facts' recorded updates, once however many keep them.
    fn recorded(&mut self, changes: &FrameChanges) -> &mut Self {
        if let Some(frame) = changes.recorded()
            && self.once(std::sync::Arc::as_ptr(frame))
        {
            self.total = self.total.saturating_add(recorded_bytes(frame));
        }
        self
    }

    /// `bytes` held by the allocation at `shared`, once however many kept
    /// values share it.
    pub fn shared<T: ?Sized>(&mut self, shared: *const T, bytes: usize) -> &mut Self {
        if self.once(shared) {
            self.add(bytes);
        }
        self
    }

    /// `bytes` more.
    pub fn add(&mut self, bytes: usize) -> &mut Self {
        self.total = self.total.saturating_add(bytes as u64);
        self
    }

    /// Everything counted.
    pub fn total(&self) -> u64 {
        self.total
    }
}

/// What one hour's recorded updates keep alive: the frame and every
/// update's rows, values and keys at capacity, as a block's facts count
/// them ([`HeldBytes`]).
pub fn recorded_bytes(frame: &replay::frame::Frame) -> u64 {
    let bytes = size_of::<replay::frame::Frame>()
        + frame
            .tables
            .iter()
            .map(super::query::frames::update_bytes)
            .sum::<usize>();
    bytes as u64
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

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static COUNTS: std::cell::RefCell<std::collections::BTreeMap<&'static str, usize>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

/// Count `name` on this thread: a fallback or hourly block of a family's
/// fold, or another step a family's tests count. The dashboard's pre-pass
/// carries the counts to the thread that asks for them.
pub fn note(name: &'static str) {
    #[cfg(any(test, feature = "test-support"))]
    COUNTS.with(|counts| *counts.borrow_mut().entry(name).or_default() += 1);
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = name;
}

/// How often `name` was noted on this thread.
#[cfg(any(test, feature = "test-support"))]
pub fn noted(name: &str) -> usize {
    COUNTS.with(|counts| counts.borrow().get(name).copied().unwrap_or(0))
}

/// Every count noted on this thread, to carry to another.
#[cfg(any(test, feature = "test-support"))]
pub fn notes() -> std::collections::BTreeMap<&'static str, usize> {
    COUNTS.with(|counts| counts.borrow().clone())
}

/// Add counts noted on another thread to this thread's.
#[cfg(any(test, feature = "test-support"))]
pub fn add_notes(notes: &std::collections::BTreeMap<&'static str, usize>) {
    COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        for (&name, &count) in notes {
            *counts.entry(name).or_default() += count;
        }
    });
}
