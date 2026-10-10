//! Hourly ship model operands, sharing the authenticated Ships census.
//!
//! The census runs in its own job. The details, mostly the recomputed maximum
//! organisation (the game never stores it), then run
//! in up to [`MAX_PARTS`] parts on separate workers, each over a contiguous
//! range of the census in native order. The family's capture keeps the first
//! range and lends one part capture per further range; outcomes and rows join
//! in census order, so the first failure keeps its position.
use super::transport_risk_ships::Model as TransportModel;
use crate::{
    projection::{Projection, project_table},
    sampling::qpc,
    table_capture::{CaptureRows, grow_capacity},
};
use engine::allocation::{copy_vec, filled, owned_vec, reserve, resize_default};
#[cfg(test)]
use engine::memory::RecordField;
use engine::{
    Memory, cship,
    memory::{RecordLayout, RecordSources},
    modifier_accumulator::{
        SNAPSHOT_ENTRY_CAPACITY, SNAPSHOT_NODE_CAPACITY, SnapshotArena, SnapshotCapacity, Value,
    },
    walk::Outcome,
};
use std::cell::{Cell, RefCell};

// Fixed-budget fixtures and the initial optional cache floor. Worker-owned
// source/detail acquisition grows from original observations without these limits.
const COPY_LIMIT: usize = 128 * 1024 * 1024;
const READ_LIMIT: usize = 4_000_000;
#[cfg(test)]
const BUDGET_EXCEEDED: &str = "ShipModels aggregate read budget exceeded";

/// Detail parts at most; each runs on its own worker.
pub(crate) const MAX_PARTS: usize = 4;
/// Ships per detail part before another part is worth its dispatch.
const SHIPS_PER_PART: usize = 512;

/// Detail parts for a census of `ships`, sized from the live count. Every
/// part requires a worker; rejected admission retains its named failure.
pub(crate) fn parts_for(ships: usize) -> usize {
    ships.div_ceil(SHIPS_PER_PART).clamp(1, MAX_PARTS)
}

/// Rows of a part capture for a family of `rows`: the widest range any
/// census within the family's live demand can assign to that part.
/// Parts of up to `SHIPS_PER_PART` ships come from censuses of fewer than
/// `MAX_PARTS` parts; wider ranges come from `MAX_PARTS` equal ranges.
fn part_rows(rows: usize) -> usize {
    rows.min(SHIPS_PER_PART).max(rows.div_ceil(MAX_PARTS))
}
// Fixed-reader memo storage can fall back before copying a source. Worker
// acquisition enables original-header growth before reading ship details.
const SNAPSHOT_LIMITS: SnapshotCapacity = SnapshotCapacity {
    nodes: READ_LIMIT / 4,
    entries: COPY_LIMIT / (2 * std::mem::size_of::<[u8; 16]>()),
};

fn bounded_snapshot_capacity(capacity: SnapshotCapacity) -> SnapshotCapacity {
    SnapshotCapacity {
        nodes: capacity.nodes.min(SNAPSHOT_LIMITS.nodes),
        entries: capacity.entries.min(SNAPSHOT_LIMITS.entries),
    }
}

fn grow_snapshot_capacity(capacity: &mut SnapshotCapacity, required: SnapshotCapacity) {
    let requested = bounded_snapshot_capacity(required);
    grow_capacity(&mut capacity.nodes, requested.nodes);
    grow_capacity(&mut capacity.entries, requested.entries);
    *capacity = bounded_snapshot_capacity(*capacity);
}

#[cfg(test)]
fn acquisition_failure(copied: (usize, usize), reason: &'static str) -> &'static str {
    // SnapshotArena validates successful read-ahead in source order. A bad key
    // precedes a later batch fault/refusal; budget saturation must not replace
    // that already established source error.
    if copied == (READ_LIMIT, COPY_LIMIT)
        && reason != "modifier accumulator keys not positive and increasing"
    {
        BUDGET_EXCEEDED
    } else {
        reason
    }
}

#[cfg(test)]
struct BoundedMemory<'a, M> {
    inner: &'a M,
    used: &'a Cell<(usize, usize)>,
}
#[cfg(test)]
impl<M: Memory> BoundedMemory<'_, M> {
    fn fits(&self, calls: usize, bytes: usize) -> bool {
        let used = self.used.get();
        calls <= READ_LIMIT.saturating_sub(used.0) && bytes <= COPY_LIMIT.saturating_sub(used.1)
    }

    fn charge(&self, calls: usize, bytes: usize) {
        let used = self.used.get();
        self.used.set((used.0 + calls, used.1 + bytes));
    }

    fn copy_records_individually(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let mut completed = 0;
        for row in 0..sources.len() {
            for field in layout.fields() {
                if field.len != 0 {
                    let Some(address) = record_address(sources, row, field) else {
                        return completed;
                    };
                    let start = row * layout.stride() + field.destination_offset;
                    if !self.copy(address, &mut destination[start..start + field.len]) {
                        return completed;
                    }
                }
                completed += 1;
            }
        }
        completed
    }
}

#[cfg(test)]
fn record_address(sources: RecordSources<'_>, row: usize, field: &RecordField) -> Option<usize> {
    sources
        .pointer(row)
        .filter(|&p| p != 0)
        .and_then(|p| p.checked_add(field.source_offset))
        .filter(|&p| p.checked_add(field.len).is_some())
}

#[cfg(test)]
impl<M: Memory> Memory for BoundedMemory<'_, M> {
    #[inline]
    fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
        let (calls, bytes) = self.used.get();
        let len = std::mem::size_of::<T>();
        if calls >= READ_LIMIT || len > COPY_LIMIT.saturating_sub(bytes) {
            self.used.set((READ_LIMIT, COPY_LIMIT));
            return None;
        }
        self.used.set((calls + 1, bytes + len));
        self.inner.read_scalar::<T>(address)
    }

    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let (calls, bytes) = self.used.get();
        if calls >= READ_LIMIT || output.len() > COPY_LIMIT.saturating_sub(bytes) {
            self.used.set((READ_LIMIT, COPY_LIMIT));
            return false;
        }
        self.used.set((calls + 1, bytes + output.len()));
        self.inner.copy(address, output)
    }

    fn authenticated_countries(
        &self,
        base: usize,
        state: usize,
    ) -> Option<engine::ccountry::AuthenticatedRoster<'_>> {
        // Forward the callback's current authenticated roster without extending it.
        self.inner.authenticated_countries(base, state)
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        state: usize,
    ) -> Option<Result<engine::ccountry::Country, &'static str>> {
        self.inner.authenticated_reserved_country(base, state)
    }
    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        state: usize,
    ) -> Option<engine::ccountry::ReservedCountryObservation> {
        self.inner
            .authenticated_reserved_country_observation(base, state)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let calls = destinations.iter().filter(|d| !d.is_empty()).count();
        let bytes = destinations
            .iter()
            .fold(0usize, |n, d| n.saturating_add(d.len()));
        if !self.fits(calls, bytes) {
            // Metadata admission observes no source bytes. At a budget edge,
            // retain scalar order: an earlier source fault wins over a later
            // budget refusal, and empty/invalid fields consume no copy budget.
            for (index, (address, destination)) in addresses
                .into_iter()
                .zip(destinations.iter_mut())
                .enumerate()
            {
                if !destination.is_empty()
                    && (address == 0
                        || address.checked_add(destination.len()).is_none()
                        || !self.copy(address, destination))
                {
                    return index;
                }
            }
            return N;
        }
        let completed = self.inner.copy_scattered(addresses, destinations);
        let attempted = completed
            + usize::from(
                completed < N
                    && !destinations[completed].is_empty()
                    && addresses[completed] != 0
                    && addresses[completed]
                        .checked_add(destinations[completed].len())
                        .is_some(),
            );
        // Charge a partially failed copy in full, exactly like copy(). Source
        // arithmetic rejection and later fields did not attempt a copy.
        self.charge(
            destinations[..attempted]
                .iter()
                .filter(|d| !d.is_empty())
                .count(),
            destinations[..attempted].iter().map(|d| d.len()).sum(),
        );
        completed
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let fields = layout.fields();
        let Some(count) = sources.len().checked_mul(fields.len()) else {
            return 0;
        };
        if sources
            .len()
            .checked_mul(layout.stride())
            .is_none_or(|n| n > destination.len())
        {
            return 0;
        }
        let nonempty = fields.iter().filter(|f| f.len != 0).count();
        if count == 0 || nonempty == 0 {
            return count;
        }
        if !self.fits(
            sources.len().saturating_mul(nonempty),
            layout.completed_bytes(count),
        ) {
            return self.copy_records_individually(sources, layout, destination);
        }
        let completed = self.inner.copy_records(sources, layout, destination);
        let attempted = completed
            + usize::from(
                completed < count
                    && fields[completed % fields.len()].len != 0
                    && record_address(
                        sources,
                        completed / fields.len(),
                        &fields[completed % fields.len()],
                    )
                    .is_some(),
            );
        self.charge(
            attempted / fields.len() * nonempty
                + fields[..attempted % fields.len()]
                    .iter()
                    .filter(|f| f.len != 0)
                    .count(),
            layout.completed_bytes(attempted),
        );
        completed
    }
}

/// Only raw modifier arrays are shared; metadata and source graph checks still
/// use the original reader. No observation-recording capability is forwarded.
struct ModifierMemory<'a, M> {
    inner: &'a M,
    arena: &'a RefCell<SnapshotArena>,
}

impl<M: Memory> Memory for ModifierMemory<'_, M> {
    #[inline]
    fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
        self.inner.read_scalar::<T>(address)
    }

    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.inner.copy(address, output)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.inner.copy_scattered(addresses, destinations)
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.inner.copy_records(sources, layout, destination)
    }
    fn authenticated_countries(
        &self,
        base: usize,
        state: usize,
    ) -> Option<engine::ccountry::AuthenticatedRoster<'_>> {
        self.inner.authenticated_countries(base, state)
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        state: usize,
    ) -> Option<Result<engine::ccountry::Country, &'static str>> {
        self.inner.authenticated_reserved_country(base, state)
    }
    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        state: usize,
    ) -> Option<engine::ccountry::ReservedCountryObservation> {
        self.inner
            .authenticated_reserved_country_observation(base, state)
    }
    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<Value, &'static str>> {
        let mut arena = self.arena.borrow_mut();
        match arena.capture_at(self.inner, address, key) {
            Ok(observation) => Some(arena.value(observation)),
            Err(SNAPSHOT_NODE_CAPACITY | SNAPSHOT_ENTRY_CAPACITY) => None,
            Err(reason) => Some(Err(reason)),
        }
    }
}
struct StrikeMemory<'a, M> {
    inner: &'a M,
    core: &'a RefCell<SnapshotArena>,
    suffix: &'a RefCell<SnapshotArena>,
}

impl<M: Memory> Memory for StrikeMemory<'_, M> {
    fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
        self.inner.read_scalar::<T>(address)
    }

    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.inner.copy(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.inner.copy_scattered(addresses, destinations)
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.inner.copy_records(sources, layout, destination)
    }

    fn authenticated_countries(
        &self,
        base: usize,
        state: usize,
    ) -> Option<engine::ccountry::AuthenticatedRoster<'_>> {
        self.inner.authenticated_countries(base, state)
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        state: usize,
    ) -> Option<Result<engine::ccountry::Country, &'static str>> {
        self.inner.authenticated_reserved_country(base, state)
    }
    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        state: usize,
    ) -> Option<engine::ccountry::ReservedCountryObservation> {
        self.inner
            .authenticated_reserved_country_observation(base, state)
    }

    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<Value, &'static str>> {
        // Core is borrowed immutably. It never gains a new address, demand,
        // failure, metadata observation or generation from this suffix lane.
        let existing = self.core.borrow().existing_at(address, key);
        match existing {
            // These are storage admission refusals, not failed native entry
            // reads. The original reader may already have used its established
            // direct-success fallback. A separately prepared suffix can acquire
            // that successful source; this never retries a stored source fault.
            Some(Err(SNAPSHOT_NODE_CAPACITY | SNAPSHOT_ENTRY_CAPACITY)) | None => (),
            Some(result) => return Some(result),
        }
        let mut suffix = self.suffix.borrow_mut();
        // Worker storage grows from this original header/entry acquisition.
        // Always return its result: None would restart a failed native census.
        Some(
            suffix
                .capture_at(self.inner, address, key)
                .and_then(|observation| suffix.value(observation)),
        )
    }
}

#[cfg(test)]
use replay::frame::TableView;
use replay::{
    frame::TableInput,
    rows::{ShipModelRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ShipModels];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

static STRIKE_REFUSALS: [std::sync::atomic::AtomicU64; 5] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 5];
const OPTIONAL_REFUSAL_REASONS: [&str; 13] = [
    "ship subunit address overflow",
    "ship subunit identity unreadable",
    "ship subunit module overflow",
    "ship subunit identity differs from census",
    "ship subunit index address overflow",
    "ship subunit index unreadable",
    "ship current hull selected source unreadable or changed",
    "ship current hull definition index unreadable or invalid",
    "ship current hull module count unreadable",
    "ship current hull upgrade level sum unreadable",
    "ship current hull module priority sum unreadable",
    "ship current hull version unreadable",
    "ship current hull organisation marker unreadable",
];
static OPTIONAL_REFUSALS: [std::sync::atomic::AtomicU64; OPTIONAL_REFUSAL_REASONS.len()] =
    [const { std::sync::atomic::AtomicU64::new(0) }; OPTIONAL_REFUSAL_REASONS.len()];

pub(crate) fn strike_data() -> impl serde::Serialize {
    crate::status::status!({
        "light_attack_refused": STRIKE_REFUSALS[0].load(std::sync::atomic::Ordering::Relaxed),
        "heavy_attack_refused": STRIKE_REFUSALS[1].load(std::sync::atomic::Ordering::Relaxed),
        "light_piercing_refused": STRIKE_REFUSALS[2].load(std::sync::atomic::Ordering::Relaxed),
        "torpedo_attack_refused": STRIKE_REFUSALS[3].load(std::sync::atomic::Ordering::Relaxed),
        "torpedo_cooldown_modifier_refused": STRIKE_REFUSALS[4].load(std::sync::atomic::Ordering::Relaxed),
        "optional_operands": crate::status::map(OPTIONAL_REFUSAL_REASONS.iter().zip(&OPTIONAL_REFUSALS).map(|(&reason, counter)| (reason, counter.load(std::sync::atomic::Ordering::Relaxed)))),
        "scope": "cumulative field-local ship readiness refusals from admitted current-hour model seeds; original model and maximum organisation remain available; concurrent reads are not an atomic totals snapshot",
    })
}

/// Exact map-census inputs for one owned ShipModels detail capture.
#[derive(Clone, Copy)]
pub(crate) struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    transport_admission: Option<crate::transport_admission::Admission>,
}

impl CaptureContext {
    /// Copy the callback verdict before this context moves to a detail worker.
    pub(crate) fn current(base: usize, game_state: usize, hour: i32) -> Self {
        Self {
            base,
            game_state,
            hour,
            transport_admission: Some(crate::transport_admission::current(base, game_state, hour)),
        }
    }

    /// The late legacy fallback has no original callback world/hour witness.
    pub(crate) fn legacy(base: usize) -> Self {
        Self {
            base,
            game_state: 0,
            hour: 0,
            transport_admission: None,
        }
    }

    fn check_transport(self) -> Result<(), &'static str> {
        self.transport_admission
            .ok_or("transport original ship-model callback witness unavailable")?
            .check(self.base, self.game_state, self.hour)
    }
}

/// Detail-acquisition ticks. Interrupted work has no completed timing sample.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptureReport {
    pub ticks: [u64; N],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Seed {
    pointer: usize,
    row: cship::Row,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    modifiers: SnapshotCapacity,
    strike_modifiers: SnapshotCapacity,
    strike_metadata: usize,
}

impl Capacities {
    /// Native validated count is storage only, never present-key admission.
    pub(crate) fn raise_for_strike_metadata(&mut self, count: usize) {
        self.strike_metadata = self.strike_metadata.max(count);
    }
}

pub struct Captured {
    worker_growth: bool,
    copied: Cell<(usize, usize)>,
    modifiers: RefCell<SnapshotArena>,
    required_modifiers: SnapshotCapacity,
    strike_refusals: [u64; 5],
    optional_refusals: [u64; OPTIONAL_REFUSAL_REASONS.len()],
    strike_modifiers: RefCell<SnapshotArena>,
    required_strike_modifiers: SnapshotCapacity,
    strike_metadata: cship::TorpedoCooldownMemo,
    rows: CaptureRows<cship::ModelRow>,
    seeds: Vec<Seed>,
    transport_models: crate::table_capture::Arena<TransportModel>,
    context: Option<CaptureContext>,
    transport_models_admitted: bool,
    seed_len: usize,
    required_rows: usize,
    parent: Outcome,
    len: usize,
    missing: u32,
    failure: Option<&'static str>,
    terminal_failure: Option<&'static str>,
    doctrine: cship::OrganisationMemo,
    model: cship::ModelMemo,
    /// Seeds this capture details: its census range when split, else all.
    detail_len: usize,
    /// Parts this hour's details were split into (1: unsplit).
    split: usize,
    /// Rows of this capture's own range when split.
    part_len: usize,
    /// A split range this part capture could not hold (a broken sizing
    /// invariant); the range is published unavailable, never partial.
    oversized: bool,
    /// Part captures for the further ranges; empty in a part itself.
    extra: Vec<Captured>,
}

impl Captured {
    /// Allocation-free placeholder while acquisition owns the original buffers.
    pub(crate) fn vacant() -> Self {
        Self::part(&Capacities {
            rows: 0,
            modifiers: SnapshotCapacity::default(),
            strike_modifiers: SnapshotCapacity::default(),
            strike_metadata: 0,
        })
    }
    /// The family's capture with its part captures, each sized for its share.
    pub fn with_capacities(capacity: &Capacities) -> Self {
        let share = Capacities {
            rows: part_rows(capacity.rows),
            modifiers: SnapshotCapacity {
                nodes: capacity.modifiers.nodes.div_ceil(2),
                entries: capacity.modifiers.entries.div_ceil(2),
            },
            strike_modifiers: SnapshotCapacity {
                nodes: capacity.strike_modifiers.nodes.div_ceil(2),
                entries: capacity.strike_modifiers.entries.div_ceil(2),
            },
            // Each part can reach every dynamic key; never divide this floor.
            strike_metadata: capacity.strike_metadata,
        };
        Self {
            extra: (1..MAX_PARTS).map(|_| Self::part(&share)).collect(),
            ..Self::part(capacity)
        }
    }
    fn part(capacity: &Capacities) -> Self {
        Self {
            copied: Cell::new((0, 0)),
            modifiers: RefCell::new(
                SnapshotArena::try_new(bounded_snapshot_capacity(capacity.modifiers))
                    .map_err(crate::allocation::io)
                    .unwrap_or_else(|_| SnapshotArena::vacant()),
            ),
            required_modifiers: SnapshotCapacity::default(),
            worker_growth: false,
            strike_refusals: [0; 5],
            optional_refusals: [0; OPTIONAL_REFUSAL_REASONS.len()],
            strike_modifiers: RefCell::new(
                SnapshotArena::try_new(capacity.strike_modifiers)
                    .map_err(crate::allocation::io)
                    .unwrap_or_else(|_| SnapshotArena::vacant()),
            ),
            required_strike_modifiers: SnapshotCapacity::default(),
            strike_metadata: cship::TorpedoCooldownMemo::try_from_validated_count(
                capacity.strike_metadata,
            )
            .map_err(crate::allocation::io)
            .unwrap_or_else(|_| cship::TorpedoCooldownMemo::vacant(0)),
            rows: CaptureRows::new(capacity.rows),
            seeds: owned_vec(filled(capacity.rows, Seed::default())),
            transport_models: crate::table_capture::Arena::new(capacity.rows),
            context: None,
            transport_models_admitted: false,
            seed_len: 0,
            required_rows: 0,
            parent: super::MISSING,
            len: 0,
            missing: 0,
            failure: None,
            terminal_failure: None,
            doctrine: cship::OrganisationMemo::default(),
            model: cship::ModelMemo::default(),
            detail_len: 0,
            split: 1,
            part_len: 0,
            oversized: false,
            extra: Vec::new(),
        }
    }
    pub fn reset(&mut self) {
        for part in &mut self.extra {
            part.reset();
        }
        self.detail_len = 0;
        self.split = 1;
        self.part_len = 0;
        self.oversized = false;
        self.release_seed_pointers();
        self.terminal_failure = None;
        self.rows.reset();
        self.modifiers.get_mut().clear();
        self.required_modifiers = SnapshotCapacity::default();
        self.strike_refusals = [0; 5];
        self.optional_refusals = [0; OPTIONAL_REFUSAL_REASONS.len()];
        self.strike_modifiers.get_mut().clear();
        self.required_strike_modifiers = SnapshotCapacity::default();
        self.strike_metadata.clear();
        self.seed_len = 0;
        self.transport_models.reset();
        self.context = None;
        self.transport_models_admitted = false;
        self.required_rows = 0;
        self.parent = super::MISSING;
        self.len = 0;
        self.missing = 0;
        self.failure = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self
                .rows
                .rows
                .iter()
                .map(|row| {
                    row.hull_sprite.allocated_bytes() + row.fallback_sprite.allocated_bytes()
                })
                .sum::<usize>()
            + self.transport_models.allocated_bytes()
            + self.seeds.capacity() * std::mem::size_of::<Seed>()
            + self.modifiers.borrow().allocated_bytes()
            + self.strike_modifiers.borrow().allocated_bytes()
            + self.strike_metadata.allocated_bytes()
            + self
                .extra
                .iter()
                .map(Captured::allocated_bytes)
                .sum::<usize>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.rows, self.required_rows);
        self.rows.grow(&mut capacity.rows);
        self.transport_models.grow(&mut capacity.rows);
        if self.seeds.len() < capacity.rows
            && resize_default(&mut self.seeds, capacity.rows).is_err()
        {
            return;
        }
        grow_snapshot_capacity(&mut capacity.modifiers, self.required_modifiers);
        if self
            .modifiers
            .get_mut()
            .try_grow(capacity.modifiers)
            .map_err(crate::allocation::io)
            .is_err()
        {
            return;
        }
        grow_capacity(
            &mut capacity.strike_modifiers.nodes,
            self.required_strike_modifiers.nodes,
        );
        grow_capacity(
            &mut capacity.strike_modifiers.entries,
            self.required_strike_modifiers.entries,
        );
        if self
            .strike_modifiers
            .get_mut()
            .try_grow(capacity.strike_modifiers)
            .map_err(crate::allocation::io)
            .is_err()
            || self
                .strike_metadata
                .try_grow_from_validated_count(capacity.strike_metadata)
                .map_err(crate::allocation::io)
                .is_err()
        {
            return;
        }
        // Parts follow the family's rows and their own snapshot demand, so
        // every range of the next split fits (`part_rows`).
        let rows = part_rows(capacity.rows);
        for part in &mut self.extra {
            let mut own = Capacities {
                rows: rows.max(part.rows.rows.len()),
                modifiers: part.required_modifiers,
                strike_modifiers: part.required_strike_modifiers,
                strike_metadata: capacity.strike_metadata,
            };
            part.grow(&mut own);
        }
    }
    /// Start the current map-owned ship census without reading model state.
    pub fn begin_sources(&mut self) {
        self.worker_growth = false;
        self.terminal_failure = None;
        self.context = None;
        self.transport_models_admitted = false;
        self.transport_models.reset();
        self.release_seed_pointers();
        self.seed_len = 0;
        self.detail_len = 0;
        self.split = 1;
        self.part_len = 0;
        self.oversized = false;
        self.missing = 0;
        self.parent = super::MISSING;
    }
    /// Retain the map's native source order; pointers live only during capture.
    pub fn visit_source(&mut self, pointer: usize, source: &cship::Row) {
        if self.seed_len == self.seeds.len() {
            self.missing = self.missing.saturating_add(1);
            return;
        }
        self.seeds[self.seed_len] = Seed {
            pointer,
            row: *source,
        };
        self.seed_len += 1;
    }
    /// The worker retains every original census occurrence, including a
    /// population increase between storage hints. No native read is restarted.
    pub fn visit_source_growing(&mut self, pointer: usize, source: &cship::Row) {
        self.worker_growth = true;
        if self.terminal_failure.is_some() {
            return;
        }
        let result = self
            .seed_len
            .checked_add(1)
            .ok_or("ship model seed count overflow")
            .and_then(|required| engine::walk::Buffer::Growing(&mut self.seeds).ensure(required));
        if let Err(reason) = result {
            self.terminal_failure = Some(reason);
            return;
        }
        self.visit_source(pointer, source);
    }

    pub fn finish_sources(&mut self, parent: Outcome) {
        if let Err(reason) =
            engine::walk::Buffer::Growing(&mut self.rows.rows).ensure(self.seed_len)
        {
            self.terminal_failure = Some(reason);
        }
        let rows = part_rows(self.seed_len);
        for part in &mut self.extra {
            for result in [
                engine::walk::Buffer::Growing(&mut part.seeds).ensure(rows),
                engine::walk::Buffer::Growing(&mut part.rows.rows).ensure(rows),
            ] {
                if let Err(reason) = result {
                    self.terminal_failure = Some(reason);
                }
            }
        }
        // ShipModels runs on a worker. The prior ownership loan returned an
        // idle journal; prepare it before any original detail callback runs.
        crate::table_capture::grow_vec(&mut self.transport_models.first, self.seeds.len());
        for part in &mut self.extra {
            crate::table_capture::grow_vec(&mut part.transport_models.first, part.rows.rows.len());
        }
        self.parent = parent;
        let unread = match parent {
            Outcome::Truncated { unread_rows, .. } => unread_rows as usize,
            _ => 0,
        };
        self.required_rows = self.required_rows.max(
            self.seed_len
                .saturating_add(self.missing as usize)
                .saturating_add(unread),
        );
        self.detail_len = self.seed_len;
    }

    /// Ships in this hour's census.
    pub(crate) fn census_len(&self) -> usize {
        self.seed_len
    }

    /// Restore only the original census request's copied callback verdict.
    pub(crate) fn set_context(&mut self, context: CaptureContext) {
        self.context = Some(context);
    }

    pub(crate) fn capture_context(&self, base: usize) -> CaptureContext {
        self.context.map_or_else(
            || CaptureContext::legacy(base),
            |context| {
                // A different caller base cannot reuse the old numerical verdict.
                CaptureContext { base, ..context }
            },
        )
    }

    /// Split this hour's details into `parts` contiguous census ranges. This
    /// capture keeps the first; each further range is copied into its part
    /// capture. Parts are sized so every range fits (`part_rows`); a range
    /// that does not is a broken invariant and publishes the family
    /// unavailable, never a partial census. Allocation-free. Returns the parts.
    pub(crate) fn split_details(&mut self, parts: usize) -> usize {
        let parts = parts.clamp(1, 1 + self.extra.len());
        let ships = self.seed_len;
        let bound = |part: usize| ships * part / parts;
        self.split = parts;
        self.detail_len = bound(1);
        for part in 1..parts {
            let (first, end) = (bound(part), bound(part + 1));
            let capture = &mut self.extra[part - 1];
            capture.begin_sources();
            capture.worker_growth = self.worker_growth;
            capture.context = self.context;
            capture.split = parts;
            capture.required_rows = capture.required_rows.max(end - first);
            if end - first > capture.seeds.len().min(capture.rows.rows.len()) {
                capture.oversized = true;
                continue;
            }
            capture.seeds[..end - first].copy_from_slice(&self.seeds[first..end]);
            capture.seed_len = end - first;
            capture.detail_len = end - first;
            capture.parent = Outcome::Rows { len: end - first };
        }
        parts
    }

    /// Lend part capture `index` (range `index + 1`) to its detail job.
    pub(crate) fn take_part(&mut self, index: usize) -> Captured {
        std::mem::replace(&mut self.extra[index], Captured::vacant())
    }

    /// Return a part capture after its job.
    pub(crate) fn restore_part(&mut self, index: usize, part: Captured) {
        self.extra[index] = part;
    }

    /// Join the parts' outcomes in census order after every part returned.
    /// The first failure keeps its census position; an interrupted part is
    /// terminal for the family, as an interrupted unsplit job is.
    pub(crate) fn finish_split(&mut self) {
        if self.split <= 1 || self.terminal_failure.is_some() {
            return;
        }
        let parts = &self.extra[..self.split - 1];
        if let Some(reason) = parts.iter().find_map(|part| part.terminal_failure) {
            self.terminal_failure = Some(reason);
            return;
        }
        // Each range publishes Rows or Unavailable; the census's own overflow
        // and truncation are the family's, applied once below.
        let failure = std::iter::once(self.rows.outcome)
            .chain(parts.iter().map(|part| part.rows.outcome))
            .find_map(|outcome| match outcome {
                Outcome::Unavailable { reason } => Some(reason),
                _ => None,
            });
        self.len = self.part_len + parts.iter().map(|part| part.part_len).sum::<usize>();
        self.failure = failure;
        self.finish_details(self.parent);
    }

    fn begin_details(&mut self, base: usize) {
        self.len = 0;
        self.failure = None;
        // Clear the fixed-fixture accounting for every part.
        self.copied.set((0, 0));
        self.modifiers.get_mut().clear();
        if self.worker_growth {
            self.modifiers.get_mut().enable_worker_growth();
        }
        self.strike_refusals = [0; 5];
        self.optional_refusals = [0; OPTIONAL_REFUSAL_REASONS.len()];
        self.strike_modifiers.get_mut().clear();
        // Read the live ship/medal/critical/support accumulators and their
        // complete array extents without an aggregate ceiling.
        self.strike_modifiers.get_mut().enable_worker_growth();
        self.strike_metadata.begin_capture(base);
        if self.worker_growth {
            self.strike_metadata.enable_worker_growth();
        }
        self.doctrine = cship::OrganisationMemo::default();
        self.model = cship::ModelMemo::default();
        if self.worker_growth {
            self.model.enable_worker_growth();
        }
        self.transport_models.reset();
        self.transport_models_admitted = false;
    }
    fn visit_detail(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        source: &cship::Row,
    ) {
        self.record(|this| {
            let suffix_memory = memory;
            let (mut row, selected, torpedo_cooldown_modifier) = {
                // Read the complete current ship accumulators;
                // the original native observations have no aggregate read cap.
                let memory = ModifierMemory {
                    inner: memory,
                    arena: &this.modifiers,
                };
                let doctrine = &mut this.doctrine;
                let (mut row, selected) = if this.transport_models_admitted {
                    let mut observation = cship::TransportModelObservation::default();
                    let original = cship::read_model_with_selection_transport_observed(
                        &memory,
                        suffix_memory,
                        base,
                        pointer,
                        source,
                        &mut this.model,
                        &mut observation,
                    );
                    this.transport_models.required =
                        this.transport_models.required.saturating_add(1);
                    if let Some(target) = this
                        .transport_models
                        .first
                        .get_mut(this.transport_models.len)
                    {
                        *target = TransportModel {
                            ship: source.id,
                            observation,
                        };
                        this.transport_models.len += 1;
                    }
                    original
                } else {
                    // Preserve the complete legacy reader and source/error order.
                    cship::read_model_with_selection(
                        &memory,
                        base,
                        pointer,
                        source,
                        &mut this.model,
                    )
                }?;
                let strike_memory = StrikeMemory {
                    inner: suffix_memory,
                    core: &this.modifiers,
                    suffix: &this.strike_modifiers,
                };
                let observed = cship::read_maximum_organisation_and_torpedo_cooldown(
                    &memory,
                    &strike_memory,
                    base,
                    pointer,
                    doctrine,
                    &mut this.strike_metadata,
                )?;
                row.maximum_organisation = observed.maximum_organisation;
                (row, selected, observed.torpedo_cooldown_modifier)
            };
            // Exactly four guarded 8-byte reads per admitted seed.
            let [light_attack, heavy_attack, light_piercing, torpedo_attack] =
                cship::read_strike_bases(suffix_memory, pointer);
            for (counter, refused) in this.strike_refusals.iter_mut().zip([
                light_attack.is_none(),
                heavy_attack.is_none(),
                light_piercing.is_none(),
                torpedo_attack.is_none(),
                torpedo_cooldown_modifier.is_err(),
            ]) {
                *counter += u64::from(refused);
            }
            row.strike = Some(cship::ShipStrikeInputs {
                light_attack,
                heavy_attack,
                light_piercing,
                torpedo_attack,
                torpedo_cooldown_modifier: torpedo_cooldown_modifier.ok(),
            });
            this.capture_model_suffixes(
                suffix_memory,
                base,
                pointer,
                source,
                selected.as_ref(),
                &mut row,
            );
            Ok(row)
        });
    }
    /// Fixed current-source operands are independent of the core model attempt.
    /// The ship's selected hull may change; read its current terms.
    fn capture_model_suffixes(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        source: &cship::Row,
        selected: Option<&cship::SelectedModelVariant>,
        row: &mut cship::ModelRow,
    ) {
        row.subunit_index = match cship::read_model_subunit_index(memory, base, pointer, source) {
            Ok(index) => Some(index),
            Err(reason) => {
                let index = OPTIONAL_REFUSAL_REASONS[..6]
                    .iter()
                    .position(|&named| named == reason)
                    .expect("subunit refusal names cover the engine reader");
                self.optional_refusals[index] += 1;
                None
            }
        };
        row.hull_variant = selected.and_then(|selected| {
            let packet = cship::read_hull_variant(memory, base, selected, &mut self.model);
            match packet {
                None => self.optional_refusals[6] += 1,
                Some(packet) => {
                    for (counter, refused) in self.optional_refusals[7..].iter_mut().zip([
                        packet.definition_index.is_none(),
                        packet.module_count.is_none(),
                        packet.upgrade_level_sum.is_none(),
                        packet.module_priority_sum.is_none(),
                        packet.version.is_none(),
                        packet.organisation_marker.is_none(),
                    ]) {
                        *counter += u64::from(refused);
                    }
                }
            }
            packet
        });
    }
    /// Record one detail read in census order; the first failure stops the range.
    fn record(&mut self, read: impl FnOnce(&mut Self) -> Result<cship::ModelRow, &'static str>) {
        if self.failure.is_some() {
            return;
        }
        if self.len == self.rows.rows.len() {
            self.failure = Some("ship model seed/output capacity mismatch");
            return;
        }
        match read(self) {
            Ok(row) => {
                self.rows.rows[self.len] = row;
                self.len += 1;
            }
            Err(reason) => self.failure = Some(reason),
        }
    }
    fn finish_details(&mut self, parent: Outcome) {
        let missing = self.missing.saturating_add(match parent {
            Outcome::Truncated { unread_rows, .. } => unread_rows,
            _ => 0,
        });
        self.rows.outcome = match (self.failure, parent) {
            (Some(reason), _) | (_, Outcome::Unavailable { reason }) => {
                Outcome::Unavailable { reason }
            }
            _ if self.len.saturating_add(self.missing as usize) != parent.len() => {
                Outcome::Unavailable {
                    reason: "ship models visits differ from parent census",
                }
            }
            _ if missing != 0 => Outcome::Truncated {
                len: self.len,
                unread_rows: missing,
            },
            _ => Outcome::Rows { len: self.len },
        };
    }
    fn release_seed_pointers(&mut self) {
        self.model = cship::ModelMemo::default();
        let len = self.seed_len.min(self.seeds.len());
        for seed in &mut self.seeds[..len] {
            seed.pointer = 0;
        }
    }
    /// Retain growth demand and expire callback-scoped snapshots without reads.
    fn release_sources(&mut self) {
        let arena = self.modifiers.get_mut();
        let required = arena.required_capacity();
        self.required_modifiers.nodes = self.required_modifiers.nodes.max(required.nodes);
        self.required_modifiers.entries = self.required_modifiers.entries.max(required.entries);
        arena.clear();
        let strike = self.strike_modifiers.get_mut();
        let required = strike.required_capacity();
        self.required_strike_modifiers.nodes =
            self.required_strike_modifiers.nodes.max(required.nodes);
        self.required_strike_modifiers.entries =
            self.required_strike_modifiers.entries.max(required.entries);
        strike.clear();
        self.strike_metadata.clear();
        // Drain the local attempts once, including terminal release paths.
        // No per-ship atomics and no false refusal for absent native modifiers.
        for (counter, refused) in STRIKE_REFUSALS
            .iter()
            .zip(std::mem::take(&mut self.strike_refusals))
            .chain(
                OPTIONAL_REFUSALS
                    .iter()
                    .zip(std::mem::take(&mut self.optional_refusals)),
            )
        {
            if refused != 0 {
                counter.fetch_add(refused, std::sync::atomic::Ordering::Relaxed);
            }
        }
        self.release_seed_pointers();
    }
    /// Consume the already-owned map seeds once; preserve native detail/error order.
    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        self.context = Some(context);
        self.capture_with(context.base, |this, seed| {
            this.visit_detail(memory, context.base, seed.pointer, &seed.row)
        })
    }
    /// Capture this range's details in census order, reading each ship with
    /// `visit`, and publish its outcome (the unsplit table, or one range of a
    /// split for `finish_split`).
    fn capture_with(
        &mut self,
        base: usize,
        mut visit: impl FnMut(&mut Self, Seed),
    ) -> CaptureReport {
        if self.terminal_failure.is_some() {
            self.release_sources();
            return CaptureReport::default();
        }
        let _reads = crate::read_accounting::table(TableId::ShipModels);
        let started = qpc();
        self.begin_details(base);
        self.transport_models_admitted = self
            .context
            .is_some_and(|context| context.check_transport().is_ok());
        for index in 0..self.detail_len.min(self.seed_len) {
            visit(self, self.seeds[index]);
        }
        if self.split <= 1 {
            self.finish_details(self.parent);
        } else {
            // One range of a split census; `finish_split` joins the parts.
            self.part_len = self.len;
            self.rows.outcome = match self.failure {
                Some(reason) => Outcome::Unavailable { reason },
                None if self.oversized => Outcome::Unavailable {
                    reason: "ship model part capacity below its census range",
                },
                None => Outcome::Rows { len: self.len },
            };
        }
        self.release_sources();
        CaptureReport {
            ticks: [qpc().saturating_sub(started).max(0) as u64],
        }
    }
    /// Accepted panic or stop is terminal; preserve observed demand without retrying.
    pub(crate) fn fail_owned_capture(&mut self, reason: &'static str) -> CaptureReport {
        self.terminal_failure = Some(reason);
        self.release_sources();
        CaptureReport::default()
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self
            .terminal_failure
            .map_or(self.rows.outcome, |reason| Outcome::Unavailable { reason })]
    }
    pub(crate) fn has_terminal_failure(&self) -> bool {
        self.terminal_failure.is_some()
    }
    /// Lend only original numerical model attempts after every detail part has
    /// joined. Unattempted census suffixes have no entry and may be acquired
    /// once by an authenticated original risk SHIPS callback.
    pub(crate) fn lend_transport_models(
        &mut self,
        target: &mut super::transport_risk_ships::Captured,
        base: usize,
        game_state: usize,
        hour: i32,
    ) -> Result<(), &'static str> {
        target.retain_model_demand(
            self.required_rows.max(
                std::iter::once(&*self)
                    .chain(self.extra.iter().take(self.split.saturating_sub(1)))
                    .fold(0_usize, |count, part| {
                        count.saturating_add(part.transport_models.required)
                    }),
            ),
        );
        if self.terminal_failure.is_some()
            || self
                .extra
                .iter()
                .take(self.split.saturating_sub(1))
                .any(|part| part.terminal_failure.is_some())
        {
            return Err("transport original ship-model job failed");
        }
        for part in
            std::iter::once(&*self).chain(self.extra.iter().take(self.split.saturating_sub(1)))
        {
            if !part.transport_models_admitted {
                return Err("transport original ship-model callback journal not admitted");
            }
            if !part.context.is_some_and(|context| {
                context.base == base && context.game_state == game_state && context.hour == hour
            }) {
                return Err(
                    "transport original ship-model callback journal base world or hour mismatch",
                );
            }
        }
        target.loan_model_part(0, &mut self.transport_models);
        for (index, part) in self
            .extra
            .iter_mut()
            .take(self.split.saturating_sub(1))
            .enumerate()
        {
            target.loan_model_part(index + 1, &mut part.transport_models);
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection disabled"),
        })
    }
    pub(crate) fn table_inputs(&self, projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
        // A split census's rows live in its parts; join them in census order
        // here, on the spool thread.
        let joined;
        let rows = if self.split > 1 && self.terminal_failure.is_none() {
            let parts = &self.extra[..self.split - 1];
            let mut rows = Vec::new();
            if reserve(&mut rows, self.len).is_err() {
                return [crate::spooling::unavailable("collector allocation failed").into()];
            }
            rows.extend_from_slice(&self.rows.rows[..self.part_len.min(self.rows.rows.len())]);
            for part in parts {
                rows.extend_from_slice(&part.rows.rows[..part.part_len.min(part.rows.rows.len())]);
            }
            joined = rows;
            &joined[..]
        } else {
            &self.rows.rows[..]
        };
        [project_table::<tables::ShipModels, _>(
            projection,
            self.outcomes()[0],
            rows,
            |row| u128::from(row.ship),
            |row, previous| {
                row.ship == previous.ship
                    && row.variant == previous.variant
                    && row.hull_sprite.as_bytes() == previous.hull_sprite
                    && row.fallback_sprite.as_bytes() == previous.fallback_sprite
                    && row.model_number == previous.model_number
                    && row.formation_flags == previous.formation_flags
                    && previous.subunit_index == replay::rows::Recorded::Value(row.subunit_index)
                    && previous.strike
                        == replay::rows::Recorded::Value(row.strike.map(strike_inputs))
                    && previous.hull_variant
                        == replay::rows::Recorded::Value(row.hull_variant.map(hull_variant_inputs))
                    && previous.counter.as_ref().is_some_and(|counter| {
                        row.roster_ordinal == counter.roster_ordinal
                            && row.definition_valid == counter.definition_valid
                            && row.carrier_air_base == counter.carrier_air_base
                            && row.maximum_strength == counter.maximum_strength
                            && row.anti_air_base == counter.anti_air_base
                            && row.maximum_organisation == counter.maximum_organisation
                            && row.positive_equipment_flags == counter.positive_equipment_flags
                    })
            },
            |r| ShipModelRow {
                ship: r.ship,
                variant: r.variant,
                hull_sprite: owned_vec(copy_vec(r.hull_sprite.as_bytes())),
                fallback_sprite: owned_vec(copy_vec(r.fallback_sprite.as_bytes())),
                model_number: r.model_number,
                formation_flags: r.formation_flags,
                subunit_index: replay::rows::Recorded::Value(r.subunit_index),
                strike: replay::rows::Recorded::Value(r.strike.map(strike_inputs)),
                hull_variant: replay::rows::Recorded::Value(
                    r.hull_variant.map(hull_variant_inputs),
                ),
                counter: Some(replay::rows::ShipCounterInputs {
                    roster_ordinal: r.roster_ordinal,
                    definition_valid: r.definition_valid,
                    carrier_air_base: r.carrier_air_base,
                    maximum_strength: r.maximum_strength,
                    anti_air_base: r.anti_air_base,
                    maximum_organisation: r.maximum_organisation,
                    positive_equipment_flags: r.positive_equipment_flags,
                }),
            },
        )]
    }
}

fn strike_inputs(value: cship::ShipStrikeInputs) -> replay::rows::ShipStrikeInputs {
    replay::rows::ShipStrikeInputs {
        light_attack: value.light_attack,
        heavy_attack: value.heavy_attack,
        light_piercing: value.light_piercing,
        torpedo_attack: value.torpedo_attack,
        torpedo_cooldown_modifier: value.torpedo_cooldown_modifier,
    }
}

fn hull_variant_inputs(value: cship::HullVariantInputs) -> replay::rows::HullVariantInputs {
    replay::rows::HullVariantInputs {
        definition_index: value.definition_index,
        module_count: value.module_count,
        upgrade_level_sum: value.upgrade_level_sum,
        module_priority_sum: value.module_priority_sum,
        version: value.version,
        organisation_marker: value.organisation_marker,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod acquisition_tests;
