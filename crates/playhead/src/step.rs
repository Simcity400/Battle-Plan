
use crate::{changes::Changes, playhead::Reader, state::PendingState};
use replay_data::Hour;
use std::sync::Mutex;

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
    pub(crate) hours: Vec<Changes>,
    pub(crate) states: Mutex<Vec<Rebuilt>>,
    pub(crate) built: Built,
}

#[derive(Debug)]
pub(crate) struct Rebuilt {
    pub(crate) reader: Reader,
    pub(crate) table: usize,
    pub(crate) state: PendingState,
}

impl Step {
    pub fn first(&self) -> Hour {
        self.first
    }

    pub fn last(&self) -> Hour {
        self.last
    }

    pub fn piece(&self) -> Option<usize> {
        self.piece
    }

    pub fn gap(&self) -> bool {
        self.piece.is_none()
    }

    pub fn reset(&self) -> bool {
        self.reset
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn restep(&self) -> u64 {
        self.restep
    }

    pub fn tables(&self) -> &[Changes] {
        &self.tables
    }

    pub fn table(&self, table: usize) -> Option<&Changes> {
        self.tables.binary_search_by_key(&table, Changes::table).ok().map(|at| &self.tables[at])
    }

    pub fn hours_of(&self, table: usize) -> &[Changes] {
        let from = self.hours.partition_point(|changes| changes.table() < table);
        let to = from + self.hours[from..].partition_point(|changes| changes.table() == table);
        &self.hours[from..to]
    }

    pub(crate) fn copied(&self) -> Self {
        let states = std::mem::take(&mut *self.states.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        Self { charge: Mutex::default(), epoch: self.epoch, seq: self.seq, restep: self.restep, first: self.first, last: self.last, piece: self.piece, reset: self.reset, tables: self.tables.clone(), hours: self.hours.clone(), states: Mutex::new(states), built: self.built }
    }

    pub fn built(&self) -> &Built {
        &self.built
    }

    pub fn payload(&self) -> usize {
        self.tables.iter().chain(&self.hours).map(Changes::payload).sum()
    }

    pub fn bytes_held(&self) -> usize {
        self.tables.iter().chain(&self.hours).map(Changes::bytes_held).sum::<usize>() + (self.tables.capacity() + self.hours.capacity()) * size_of::<Changes>()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Built {
    pub ns: u64,
    pub cycles: u64,
    pub threads: u32,
    pub cells: u64,
    pub rows: u64,
    pub bytes: u64,
    pub stalled: u32,
    pub short: u32,
    pub most_short: u32,
    pub fetched: Fetching,
    pub sought: Option<Seeking>,
}

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

#[derive(Clone, Copy, Debug, Default)]
pub struct Seeking {
    pub ns: u64,
    pub tables: u32,
    pub restore_ns: u64,
    pub restore_most_ns: u64,
    pub replay_ns: u64,
    pub replay_most_ns: u64,
    pub fetch_ns: u64,
    pub rebuild_ns: u64,
    pub rebuild_most_ns: u64,
    pub states: u32,
    pub state_bytes: u64,
    pub table_most_ns: u64,
    pub waited_ns: u64,
    pub ahead_ns: u64,
    pub threads: u32,
}
