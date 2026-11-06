//! A recording's changes held key-major, so any hour's tables are read without walking.
//!
//! Each indexed table's slots fall in blocks of [`Layout::block`] slots. A strip is one column
//! of one block, or the block's rows ([`ROWS`]: which slots are live, and their keys). A strip's
//! pages each hold its cells at the page's first ordinal and then every change after it, in
//! ordinal order, through the hour before the next page's first. A segment's walk
//! ([`Builder`]) opens a strip's page at the strip's first change in the segment, from the
//! walker's cells once that hour applied, and closes it once it holds [`Layout::runs`]
//! changes, so pages are written once, in parallel, each segment's to its own [`Disk`], and
//! their directories stitched in segment order ([`Index::append`]). One value written to most of a block's rows is one change, naming the
//! live rows it leaves. Per table the index keeps each change of its status, sample hour, slot
//! count and truncated view, and per ordinal its hour and metrics.
//!
//! A [`Cursor`] holds a walker's [`State`] of the indexed tables and moves it to any ordinal of
//! a run: a strip whose page and changes are the same at both ordinals is left; any other
//! applies its page's changes from where it stood, or decodes its page. The state then names
//! what differs from where it stood, as the walker's changes of one hour do.
//!
//! [`State`]: crate::walk::State

use crate::{
    invalid,
    manifest::Status,
    walk::{Chunks, Packed},
};
use std::{io, sync::Arc};

mod build;
mod cell;
mod cursor;
mod index;
#[cfg(test)]
mod tests;

pub use build::{Builder, Closed, Piece};
pub use cursor::{Cursor, Keyed, beyond_memory, rows_at};
pub use index::{Chunk, Index, RowsPage, RunIndex, Strips};

/// A strip's column for a block's rows: which slots are live, and their keys.
pub const ROWS: u32 = u32::MAX;

/// The raw bytes a page holds at most: a page closes once it holds a quarter of them, and one
/// whose cells at its first ordinal alone pass them is refused, as is a page claiming more.
pub const PAGE_BYTES: u32 = 1 << 28;

/// How tables are cut into strips and strips into pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Slots in a block.
    pub block: u32,
    /// Changes a page holds before it closes.
    pub runs: u32,
}

impl Layout {
    /// Most tables in one block, and a page whose changes outweigh its cells four times.
    pub const RECORDING: Self = Self {
        block: 1024,
        runs: 4096,
    };

    pub(crate) fn check(self) -> io::Result<Self> {
        if self.block == 0 || self.block > 1 << 20 || self.runs == 0 {
            return Err(invalid("A run index's layout holds no slots or changes"));
        }
        Ok(self)
    }

    /// The blocks `slots` slots take.
    pub fn blocks(self, slots: usize) -> usize {
        slots.div_ceil(self.block as usize)
    }
}

/// Where a stored page lies: its stored bytes from `offset`, and its raw length. A page stored
/// as raw bytes has `stored == raw`; any other was packed smaller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Location {
    pub offset: u64,
    pub stored: u32,
    pub raw: u32,
}

/// A stored page of a strip: its cells at `first`, and changes through `last`, its last
/// change's ordinal (`first` when it holds none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    pub first: u32,
    pub last: u32,
    pub location: Location,
}

/// A table's sample hour from an ordinal on: one hour, or each ordinal's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub enum Sample {
    At(Option<i32>),
    Each,
}

impl Sample {
    /// The sample hour at an ordinal of `hour`.
    pub fn at(self, hour: i32) -> Option<i32> {
        match self {
            Self::At(sampled) => sampled,
            Self::Each => Some(hour),
        }
    }
}

/// What an indexed table held over a run, each from its ordinal on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Series {
    pub statuses: Vec<(u32, Status)>,
    pub samples: Vec<(u32, Sample)>,
    pub slots: Vec<(u32, u32)>,
    /// A truncated table's view of rows read, as slots ([`crate::leaf::Writer::slots`]).
    pub views: Vec<(u32, Location)>,
}

/// The entry of `series` in force at `ordinal`.
pub(crate) fn at<T>(series: &[(u32, T)], ordinal: u32) -> Option<(usize, &T)> {
    let at = series.partition_point(|&(first, _)| first <= ordinal);
    let at = at.checked_sub(1)?;
    Some((at, &series[at].1))
}

/// One hour's metrics as its manifest keeps them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Metrics {
    pub total_ns: u64,
    pub pool_bytes: u64,
    pub dropped_frames: u64,
}

/// Packs each page's raw bytes for its store; [`Pages`] reads them back.
pub trait Pack: Send {
    /// `raw`'s stored bytes into `out`, emptied first: packed only when smaller.
    fn pack(&mut self, raw: &[u8], out: &mut Vec<u8>) -> io::Result<()>;
}

/// Pages stored as their raw bytes.
pub struct Raw;

impl Pack for Raw {
    fn pack(&mut self, raw: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        engine_allocation::reserve(out, raw.len())?;
        out.extend_from_slice(raw);
        Ok(())
    }
}

/// Where stitched pages are kept.
pub trait Store {
    /// Keeps `stored`, a page of `raw` raw bytes as [`Pack`] stored it.
    fn put(&mut self, stored: &[u8], raw: u32) -> io::Result<Location>;
}

/// What a segment's walk writes to: its pages, truncated views and dictionary chunks, as
/// [`Store`] keeps them, each where it lies among the segment's bytes; and its open pages'
/// bytes, kept apart until their pages close.
pub trait Disk: Store + Send {
    /// Keeps `bytes`: where they lie among those it kept.
    fn keep(&mut self, bytes: &[u8]) -> io::Result<u64>;

    /// Fills `into` with the bytes it kept from `at`.
    fn kept(&mut self, at: u64, into: &mut [u8]) -> io::Result<()>;
}

/// Where a cursor reads pages.
pub trait Pages: Sync {
    /// The raw bytes of the page at `location`, into `into`, emptied first.
    fn read(&self, location: Location, into: &mut Vec<u8>) -> io::Result<()>;

    /// `chunk`'s strings, read whole: a store shared by many cursors keeps them.
    fn strings(&self, chunk: &Chunk) -> io::Result<Arc<Packed>> {
        let mut raw = Vec::new();
        self.read(chunk.location, &mut raw)?;
        let strings = Packed::read(chunk.first as usize, raw)?;
        if strings.len() != chunk.count as usize {
            return Err(invalid("A dictionary chunk holds other than its strings"));
        }
        Ok(Arc::new(strings))
    }

    /// A run's dictionary chunks, `chunks`, as a run's dictionary reads them when a cell names
    /// one of their strings ([`crate::walk::Dictionary::run`]): a store that is not shared
    /// reads them all now.
    fn chunks(&self, chunks: &[Chunk]) -> io::Result<Arc<dyn Chunks>> {
        let mut loaded = engine_allocation::vec(chunks.len())?;
        for chunk in chunks {
            loaded.push((*chunk, self.strings(chunk)?));
        }
        Ok(Arc::new(Loaded(loaded)))
    }
}

/// A run's chunks, each with its strings.
struct Loaded(Vec<(Chunk, Arc<Packed>)>);

impl Chunks for Loaded {
    fn chunk(&self, id: u32) -> io::Result<Arc<Packed>> {
        let at = self.0.partition_point(|(chunk, _)| chunk.first <= id);
        at.checked_sub(1)
            .map(|at| &self.0[at])
            .filter(|(chunk, _)| id - chunk.first < chunk.count)
            .map(|(_, strings)| Arc::clone(strings))
            .ok_or_else(|| invalid(format!("No dictionary chunk holds string {id}")))
    }
}
