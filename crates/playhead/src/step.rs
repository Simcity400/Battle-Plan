//! What readers get: one step of the playhead, its hours and every stepped table's change set as
//! owned data, the states rebuilt for its resets, and what building it took.

use crate::{changes::Changes, playhead::Reader, state::State};
use replay_data::Hour;
use std::sync::Mutex;

/// One step: game hours `first` to `last`, every stepped table's change set in table order.
#[derive(Debug, Default)]
pub struct Step {
    pub(crate) charge: Mutex<Option<crate::Charge>>,
    pub(crate) epoch: u64,
    pub(crate) seq: u64,
    pub(crate) restep: u64,
    pub(crate) first: Hour,
    pub(crate) last: Hour,
    pub(crate) piece: Option<usize>,
    pub(crate) reset: bool,
    pub(crate) tables: Vec<Changes>,
    /// The states rebuilt for the tables it resets, each until its reader takes it.
    pub(crate) states: Mutex<Vec<Rebuilt>>,
    pub(crate) built: Built,
}

#[derive(Debug)]
pub(crate) struct Rebuilt {
    pub(crate) reader: Reader,
    pub(crate) table: usize,
    pub(crate) state: State,
}

impl Step {
    pub fn first(&self) -> Hour {
        self.first
    }

    pub fn last(&self) -> Hour {
        self.last
    }

    /// The timeline's piece it is in; none for hours in no piece (a gap: nothing is recorded
    /// there, and the tables keep the rows of the piece before).
    pub fn piece(&self) -> Option<usize> {
        self.piece
    }

    pub fn gap(&self) -> bool {
        self.piece.is_none()
    }

    /// Every table's change set is a reset (a seek, or a piece's first hour).
    pub fn reset(&self) -> bool {
        self.reset
    }

    /// The order steps are built in, from the playhead's start.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// How many times, when it was built, the steps past the frames had been rebuilt for a new
    /// pace: a step lent ahead ([`crate::Consumer::ahead`]) and not taken, older than a step taken
    /// or lent since, was rebuilt and will never be taken: what was made from it is void.
    pub fn restep(&self) -> u64 {
        self.restep
    }

    pub fn tables(&self) -> &[Changes] {
        &self.tables
    }

    pub fn table(&self, table: usize) -> Option<&Changes> {
        self.tables.binary_search_by_key(&table, Changes::table).ok().map(|at| &self.tables[at])
    }

    /// A copy the ring changes in its place while a consumer that looked ahead holds this one; the
    /// states rebuilt for it move to the copy.
    pub(crate) fn copied(&self) -> Self {
        let states = std::mem::take(&mut *self.states.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        Self { charge: Mutex::default(), epoch: self.epoch, seq: self.seq, restep: self.restep, first: self.first, last: self.last, piece: self.piece, reset: self.reset, tables: self.tables.clone(), states: Mutex::new(states), built: self.built }
    }

    pub fn built(&self) -> &Built {
        &self.built
    }

    /// The bytes its change sets carry.
    pub fn payload(&self) -> usize {
        self.tables.iter().map(Changes::payload).sum()
    }

    /// The bytes it holds, buffers included.
    pub fn bytes_held(&self) -> usize {
        self.tables.iter().map(Changes::bytes_held).sum::<usize>() + self.tables.capacity() * size_of::<Changes>()
    }
}

/// What building a step took.
#[derive(Clone, Copy, Debug, Default)]
pub struct Built {
    /// The stepper's wall time: lanes stepped and change sets filled.
    pub ns: u64,
    /// Threads that stepped lanes.
    pub threads: u32,
    pub cells: u64,
    pub rows: u64,
    pub bytes: u64,
    /// Tables that stopped at a part not fetched yet.
    pub stalled: u32,
    /// Tables behind the step's last hour, and the most hours one is behind.
    pub short: u32,
    pub most_short: u32,
    /// Reads that arrived since the step before.
    pub fetched: Fetching,
    /// For the reset step of a seek: what the seek took.
    pub sought: Option<Seeking>,
}

/// Reads done on the fetchers.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fetching {
    pub jobs: u64,
    pub parts: u64,
    pub tracks: u64,
    pub stored: u64,
    pub decompressed: u64,
    pub ns: u64,
}

impl Fetching {
    pub fn add(&mut self, other: &Self) {
        self.jobs += other.jobs;
        self.parts += other.parts;
        self.tracks += other.tracks;
        self.stored += other.stored;
        self.decompressed += other.decompressed;
        self.ns += other.ns;
    }
}

/// A seek: its tables restored and replayed in parallel, each table's readers' states rebuilt
/// where it was sought.
#[derive(Clone, Copy, Debug, Default)]
pub struct Seeking {
    /// From the seek reaching the stepper to its reset step built.
    pub ns: u64,
    pub tables: u32,
    /// Summed over tables, and the most one table took.
    pub restore_ns: u64,
    pub restore_most_ns: u64,
    pub replay_ns: u64,
    pub replay_most_ns: u64,
    pub fetch_ns: u64,
    /// The readers' rebuilds, and the most one table's took.
    pub rebuild_ns: u64,
    pub rebuild_most_ns: u64,
    pub states: u32,
    pub state_bytes: u64,
    /// One table's whole preparation, the most it took.
    pub table_most_ns: u64,
    /// From the seek asked to the stepper beginning it: the step it was building finished first.
    pub waited_ns: u64,
    /// Of `fetch_ns`, the reads past the hour sought (the read-ahead).
    pub ahead_ns: u64,
    /// Threads that prepared its tables.
    pub threads: u32,
}
