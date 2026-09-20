//! Disposable country-table chunks, independent of country selection, range and
//! plot density. No complete decoded game state is retained here.

use std::{
    collections::BTreeMap,
    mem::size_of,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

use replay::{
    container::Replay,
    frame::DecodedFrame,
    rows::{CountryLinkRow, CountryRow},
};

use crate::{
    SourceKey,
    cache::{ByteBudget, Reservation},
    reduce::short_text,
    scan::country_tables,
};

pub(crate) struct ProjectedFrame {
    pub hour: i32,
    pub tables: Result<(Vec<CountryRow>, Vec<CountryLinkRow>), String>,
    _reservation: Reservation,
}

pub(crate) struct ProjectedBlock {
    pub frames: Vec<Arc<ProjectedFrame>>,
    /// Includes holes up to the query boundary, even when no frame exists there.
    pub through: i32,
}

struct Entry {
    replay: Weak<Replay>,
    block: Arc<ProjectedBlock>,
    touched: u64,
}

pub(crate) struct ProjectionCache {
    budget: Arc<ByteBudget>,
    entries: Mutex<BTreeMap<(SourceKey, usize), Entry>>,
    tick: AtomicU64,
    pub hits: AtomicU64,
    pub block_reads: AtomicU64,
}

impl ProjectionCache {
    pub fn new(bytes: usize) -> Self {
        Self {
            budget: ByteBudget::new(bytes),
            entries: Mutex::new(BTreeMap::new()),
            tick: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            block_reads: AtomicU64::new(0),
        }
    }

    pub fn bytes(&self) -> usize {
        self.budget.used()
    }

    pub fn retain_sources(&self, active: impl Fn(&SourceKey) -> bool) {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|(source, _), entry| active(source) && entry.replay.strong_count() > 0);
    }

    pub fn get(
        &self,
        source: &SourceKey,
        replay: &Arc<Replay>,
        index: usize,
        through: i32,
    ) -> Option<Arc<ProjectedBlock>> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let entry = entries.get_mut(&(source.clone(), index))?;
        // A re-registered key cannot borrow projections from another snapshot.
        if !entry.replay.ptr_eq(&Arc::downgrade(replay)) || entry.block.through < through {
            return None;
        }
        entry.touched = self.tick.fetch_add(1, Ordering::Relaxed);
        self.hits.fetch_add(1, Ordering::Relaxed);
        Some(Arc::clone(&entry.block))
    }

    pub fn insert(
        &self,
        source: SourceKey,
        replay: &Arc<Replay>,
        index: usize,
        block: ProjectedBlock,
    ) {
        if block.frames.is_empty() {
            return;
        }
        let key = (source, index);
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if entries.get(&key).is_some_and(|entry| {
            entry.replay.ptr_eq(&Arc::downgrade(replay)) && entry.block.through >= block.through
        }) {
            return;
        }
        let touched = self.tick.fetch_add(1, Ordering::Relaxed);
        entries.insert(
            key,
            Entry {
                replay: Arc::downgrade(replay),
                block: Arc::new(block),
                touched,
            },
        );
    }

    /// Cache admission is optional. Evicted chunks remain charged while a scan
    /// owns them; pressure falls back to the same authoritative streaming scan.
    pub fn capture(&self, frame: &DecodedFrame) -> Option<Arc<ProjectedFrame>> {
        let tables = country_tables(frame).map_err(|error| short_text(&error.to_string(), 512));
        let payload = match &tables {
            Ok((rows, links)) => rows
                .len()
                .checked_mul(size_of::<CountryRow>())?
                .checked_add(links.len().checked_mul(size_of::<CountryLinkRow>())?)?
                .checked_add(
                    rows.iter()
                        .map(|row| row.tag.len().saturating_add(row.name.len()))
                        .sum::<usize>(),
                )?,
            Err(reason) => reason.capacity(),
        };
        // Includes the Arc, vector slots and amortized block/cache-key ownership.
        let bytes = payload.checked_add(size_of::<ProjectedFrame>() + 1024)?;
        let reservation = loop {
            if let Some(reservation) = self.budget.reserve(bytes) {
                break reservation;
            }
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let oldest = entries
                .iter()
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(key, _)| key.clone())?;
            entries.remove(&oldest);
        };
        Some(Arc::new(ProjectedFrame {
            hour: frame.hour,
            tables: tables.map(|(rows, links)| (rows.to_vec(), links.to_vec())),
            _reservation: reservation,
        }))
    }
}
