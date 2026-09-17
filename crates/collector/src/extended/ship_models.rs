//! Hourly ship model operands, sharing the authenticated Ships census.
use crate::{
    sampling::{CONSISTENCY_TARGET, qpc},
    spooling::table,
    table_capture::{CaptureRows, comparison, comparison_rows, grow_capacity, grow_vec},
};
use engine::{
    Memory, cship,
    memory::{RecordField, RecordLayout, RecordSources},
    modifier_accumulator::{
        SNAPSHOT_ENTRY_CAPACITY, SNAPSHOT_NODE_CAPACITY, SnapshotArena, SnapshotCapacity, Value,
    },
    walk::Outcome,
};
use std::cell::{Cell, RefCell};

// Per-pass ceilings include acquisition and fresh terminal snapshot witnesses.
const COPY_LIMIT: usize = 128 * 1024 * 1024;
const READ_LIMIT: usize = 4_000_000;
const BUDGET_EXCEEDED: &str = "ShipModels aggregate read budget exceeded";
// A successful snapshot spends two header reads when acquired and two when
// verified; every retained entry is copied in both stages. Cache pressure above
// these useful bounds falls back to the unchanged direct reader.
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

struct BoundedMemory<'a, M> {
    inner: &'a M,
    used: &'a Cell<(usize, usize)>,
}
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

fn record_address(sources: RecordSources<'_>, row: usize, field: &RecordField) -> Option<usize> {
    sources
        .pointer(row)
        .filter(|&p| p != 0)
        .and_then(|p| p.checked_add(field.source_offset))
        .filter(|&p| p.checked_add(field.len).is_some())
}

impl<M: Memory> Memory for BoundedMemory<'_, M> {
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
        // The callback supplies CountryMemory and retains its outer direct
        // verification. This adapter never constructs or extends a roster.
        self.inner.authenticated_countries(base, state)
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
/// use the budgeted reader. No observation-recording capability is forwarded.
struct ModifierMemory<'a, M> {
    inner: &'a M,
    arena: &'a RefCell<SnapshotArena>,
}

impl<M: Memory> Memory for ModifierMemory<'_, M> {
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
use replay::{
    frame::TableView,
    rows::{ShipModelRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ShipModels];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// Exact map-census inputs for one owned ShipModels detail pass. `needed`
/// requests the independent cold pass; the first source census is always used.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureContext {
    pub base: usize,
    pub needed: [bool; N],
}

/// Detail-acquisition time only. Terminal modifier verification and row
/// comparison remain in `capture()` after the owned request is restored.
/// A terminal worker failure reports zero because its interrupted detail cost
/// is unmeasured, not because the work was free.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptureReport {
    pub ticks: [u64; N],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Seed {
    pointer: usize,
    row: cship::Row,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    modifiers: SnapshotCapacity,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 4096,
            modifiers: SnapshotCapacity {
                nodes: 4096,
                entries: 65536,
            },
        }
    }
}

pub struct Captured {
    copied: [Cell<(usize, usize)>; 2],
    modifiers: [RefCell<SnapshotArena>; 2],
    required_modifiers: SnapshotCapacity,
    rows: CaptureRows<cship::ModelRow>,
    seeds: [Vec<Seed>; 2],
    seed_len: [usize; 2],
    required_rows: usize,
    parent: [Outcome; 2],
    len: [usize; 2],
    missing: [u32; 2],
    failure: [Option<&'static str>; 2],
    second: Outcome,
    terminal_failure: Option<&'static str>,
}

impl Captured {
    /// Allocation-free replacement while this family's buffers are loaned to
    /// the flat acquisition pool. The original must be restored before terminal
    /// verification, table views, growth or publication.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(
            &Capacities {
                rows: 0,
                modifiers: SnapshotCapacity::default(),
            },
            &[CONSISTENCY_TARGET; N],
        )
    }

    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            copied: [Cell::new((0, 0)), Cell::new((0, 0))],
            modifiers: std::array::from_fn(|pass| {
                RefCell::new(SnapshotArena::new(
                    if pass == 0 || successful[0] < CONSISTENCY_TARGET {
                        bounded_snapshot_capacity(capacity.modifiers)
                    } else {
                        SnapshotCapacity::default()
                    },
                ))
            }),
            required_modifiers: SnapshotCapacity::default(),
            rows: CaptureRows::new(capacity.rows, successful[0]),
            seeds: [
                vec![Seed::default(); capacity.rows],
                comparison_rows(capacity.rows, successful[0]),
            ],
            seed_len: [0; 2],
            required_rows: 0,
            parent: [super::MISSING; 2],
            len: [0; 2],
            missing: [0; 2],
            failure: [None; 2],
            second: super::MISSING,
            terminal_failure: None,
        }
    }
    pub fn reset(&mut self) {
        // Raw map pointers are callback-scoped. Erase every used slot before
        // forgetting its logical length so frame recovery cannot retain one.
        self.release_seed_pointers();
        self.terminal_failure = None;
        self.rows.reset();
        for arena in &mut self.modifiers {
            arena.get_mut().clear();
        }
        self.required_modifiers = SnapshotCapacity::default();
        self.seed_len = [0; 2];
        self.required_rows = 0;
        self.parent = [super::MISSING; 2];
        self.len = [0; 2];
        self.missing = [0; 2];
        self.failure = [None; 2];
        self.second = super::MISSING;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self
                .seeds
                .iter()
                .map(|seeds| seeds.capacity() * std::mem::size_of::<Seed>())
                .sum::<usize>()
            + self
                .modifiers
                .iter()
                .map(|a| a.borrow().allocated_bytes())
                .sum::<usize>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        grow_capacity(&mut capacity.rows, self.required_rows);
        self.rows.grow(&mut capacity.rows, successful[0]);
        grow_vec(&mut self.seeds[0], capacity.rows);
        comparison(&mut self.seeds[1], capacity.rows, successful[0]);
        grow_snapshot_capacity(&mut capacity.modifiers, self.required_modifiers);
        for (pass, arena) in self.modifiers.iter_mut().enumerate() {
            if pass == 0 || successful[0] < CONSISTENCY_TARGET {
                arena.get_mut().grow(capacity.modifiers);
            } else {
                // Comparison storage is retired only on the spool thread.
                *arena.get_mut() = SnapshotArena::new(SnapshotCapacity::default());
            }
        }
    }

    /// Start one map-owned native ship census. This records no additional live
    /// model state and does not touch the pass's modifier witness arena.
    pub fn begin_sources(&mut self, second: bool) {
        let pass = usize::from(second);
        if !second {
            // A fresh complete source epoch may replace a prior terminal worker
            // failure. Terminal verification/helper re-entry alone may not.
            self.terminal_failure = None;
        }
        for seed in &mut self.seeds[pass][..self.seed_len[pass]] {
            seed.pointer = 0;
        }
        self.seed_len[pass] = 0;
        self.missing[pass] = 0;
        self.parent[pass] = super::MISSING;
    }

    /// Retain the exact pointer/row pair in native map-census order. The row is
    /// already owned; the pointer stays callback-scoped and is erased when the
    /// owned request completes or fails.
    pub fn visit_source(&mut self, pointer: usize, source: &cship::Row, second: bool) {
        let pass = usize::from(second);
        if self.seed_len[pass] == self.seeds[pass].len() {
            self.missing[pass] = self.missing[pass].saturating_add(1);
            return;
        }
        self.seeds[pass][self.seed_len[pass]] = Seed {
            pointer,
            row: *source,
        };
        self.seed_len[pass] += 1;
    }

    /// Preserve the map walk's terminal outcome for later detail acquisition.
    /// Source-derived demand survives even if the owned detail job later fails.
    pub fn finish_sources(&mut self, second: bool, parent: Outcome) {
        let pass = usize::from(second);
        self.parent[pass] = parent;
        let unread = match parent {
            Outcome::Truncated { unread_rows, .. } => unread_rows as usize,
            _ => 0,
        };
        self.required_rows = self.required_rows.max(
            self.seed_len[pass]
                .saturating_add(self.missing[pass] as usize)
                .saturating_add(unread),
        );
    }

    fn begin_details(&mut self, second: bool) {
        let pass = usize::from(second);
        self.len[pass] = 0;
        self.failure[pass] = None;
        self.copied[pass].set((0, 0));
        self.modifiers[pass].get_mut().clear();
    }

    fn visit_detail(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        source: &cship::Row,
        second: bool,
    ) {
        let pass = usize::from(second);
        if self.failure[pass].is_some() {
            return;
        }
        let output = if second {
            &mut self.rows.second
        } else {
            &mut self.rows.rows
        };
        if self.len[pass] == output.len() {
            self.failure[pass] = Some("ship model seed/output capacity mismatch");
            return;
        }
        let bounded = BoundedMemory {
            inner: memory,
            used: &self.copied[pass],
        };
        let memory = &ModifierMemory {
            inner: &bounded,
            arena: &self.modifiers[pass],
        };
        match cship::read_model(memory, base, pointer, source).and_then(|mut row| {
            row.maximum_organisation = cship::read_maximum_organisation(memory, base, pointer)?;
            Ok(row)
        }) {
            Ok(row) => {
                output[self.len[pass]] = row;
                self.len[pass] += 1;
            }
            Err(reason) => {
                self.failure[pass] = Some(acquisition_failure(self.copied[pass].get(), reason))
            }
        }
    }

    fn finish_details(&mut self, second: bool, parent: Outcome) {
        let pass = usize::from(second);
        let missing = self.missing[pass].saturating_add(match parent {
            Outcome::Truncated { unread_rows, .. } => unread_rows,
            _ => 0,
        });
        let result = match (self.failure[pass], parent) {
            (Some(reason), _) | (_, Outcome::Unavailable { reason }) => {
                Outcome::Unavailable { reason }
            }
            _ if self.len[pass].saturating_add(self.missing[pass] as usize) != parent.len() => {
                Outcome::Unavailable {
                    reason: "ship models visits differ from parent census",
                }
            }
            _ if missing != 0 => Outcome::Truncated {
                len: self.len[pass],
                unread_rows: missing,
            },
            _ => Outcome::Rows {
                len: self.len[pass],
            },
        };
        if second {
            self.second = result;
        } else {
            self.rows.outcome = result;
        }
    }

    fn release_seed_pointers(&mut self) {
        for pass in 0..2 {
            // The spool thread may retire cold comparison storage after a
            // completed frame. Dropping that vector already drops its pointers;
            // only still-retained used slots need an explicit callback scrub.
            let len = self.seed_len[pass].min(self.seeds[pass].len());
            for seed in &mut self.seeds[pass][..len] {
                seed.pointer = 0;
            }
        }
    }

    /// Consume the already-owned map seeds exactly once on an acquisition
    /// worker (or on the callback after a rejected dispatch). No map or ship
    /// census is repeated here. Native detail/error order is the seed order.
    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        if self.terminal_failure.is_some() {
            // Cleared seed pointers from a terminally failed epoch are never a
            // legal independent retry source. reset()/begin_sources(false)
            // explicitly starts the next capture epoch.
            self.release_seed_pointers();
            return CaptureReport::default();
        }
        let started = qpc();
        for pass in 0..=usize::from(context.needed[0]) {
            let second = pass != 0;
            self.begin_details(second);
            let len = self.seed_len[pass];
            for index in 0..len {
                let seed = self.seeds[pass][index];
                self.visit_detail(memory, context.base, seed.pointer, &seed.row, second);
            }
            self.finish_details(second, self.parent[pass]);
        }
        self.release_seed_pointers();
        CaptureReport {
            ticks: [qpc().saturating_sub(started).max(0) as u64],
        }
    }

    /// Accepted panic/stop is terminal: do not retry native details. Preserve
    /// seed-derived growth demand, allocations and any modifier witnesses that
    /// were captured before the failure. Their fresh verification still runs in
    /// `capture()`. Raw ship pointers are no longer retained after restoration.
    pub(crate) fn fail_owned_capture(
        &mut self,
        context: CaptureContext,
        reason: &'static str,
    ) -> CaptureReport {
        self.terminal_failure = Some(reason);
        self.rows.checked = context.needed[0];
        self.release_seed_pointers();
        CaptureReport::default()
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        _base: usize,
        _state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        let started = qpc();
        let mut verified = [true; 2];
        for (pass, valid) in verified.iter_mut().enumerate() {
            let arena = self.modifiers[pass].borrow();
            let required = arena.required_capacity();
            self.required_modifiers.nodes = self.required_modifiers.nodes.max(required.nodes);
            self.required_modifiers.entries = self.required_modifiers.entries.max(required.entries);
            // This reader bypasses the modifier hook. Every retained successful
            // observation is checked even after another source/pass failed.
            *valid = arena.verify(&BoundedMemory {
                inner: memory,
                used: &self.copied[pass],
            });
            if !*valid {
                let reason = if self.copied[pass].get() == (READ_LIMIT, COPY_LIMIT) {
                    BUDGET_EXCEEDED
                } else {
                    "ship model modifier snapshot changed"
                };
                self.failure[pass].get_or_insert(reason);
                let outcome = if pass == 0 {
                    &mut self.rows.outcome
                } else {
                    &mut self.second
                };
                if !matches!(outcome, Outcome::Unavailable { .. }) {
                    *outcome = Outcome::Unavailable { reason };
                }
            }
        }
        self.rows.checked = needed[0];
        let matched = if needed[0] {
            let same_sources = self.modifiers[0]
                .borrow()
                .same_observation(&self.modifiers[1].borrow());
            let rows = self.rows.compare(self.second);
            let failures = verified.iter().filter(|&&v| !v).count()
                + usize::from(!same_sources)
                + usize::from(self.terminal_failure.is_some());
            self.rows.mismatches += failures;
            rows && failures == 0
        } else {
            false
        };
        // No native addresses or snapshot epochs remain available after the
        // callback. Save demand above for off-thread growth before clearing.
        for arena in &mut self.modifiers {
            arena.get_mut().clear();
        }
        ([matched], [qpc().saturating_sub(started).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if let Some(reason) = self.terminal_failure {
            Outcome::Unavailable { reason }
        } else if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "ship models changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }
    pub(crate) fn has_terminal_failure(&self) -> bool {
        self.terminal_failure.is_some()
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::ShipModels, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| ShipModelRow {
                ship: r.ship,
                variant: r.variant,
                hull_sprite: r.hull_sprite.as_bytes().to_vec(),
                fallback_sprite: r.fallback_sprite.as_bytes().to_vec(),
                model_number: r.model_number,
                formation_flags: r.formation_flags,
                counter: Some(replay::rows::ShipCounterInputs {
                    roster_ordinal: r.roster_ordinal,
                    definition_valid: r.definition_valid,
                    carrier_air_base: r.carrier_air_base,
                    maximum_strength: r.maximum_strength,
                    maximum_organisation: r.maximum_organisation,
                    positive_equipment_flags: r.positive_equipment_flags,
                }),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::memory::SliceMemory;
    #[test]
    fn source_faults_capacity_and_missing_comparison_remain_explicit() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capacity = Capacities {
            rows: 0,
            modifiers: SnapshotCapacity::default(),
        };
        let mut capture = Captured::with_capacities(&capacity, &[0]);
        capture.begin_sources(false);
        capture.visit_source(0, &cship::Row::default(), false);
        capture.finish_sources(
            false,
            Outcome::Truncated {
                len: 1,
                unread_rows: 3,
            },
        );
        capture.capture_owned(
            &memory,
            CaptureContext {
                base: 0,
                needed: [false],
            },
        );
        assert_eq!(
            capture.outcomes(),
            [Outcome::Truncated {
                len: 0,
                unread_rows: 4
            }]
        );
        capture.grow(&mut capacity, &[0]);
        assert!(capacity.rows >= 4);
        capture.reset();
        capture.begin_sources(false);
        capture.visit_source(0, &cship::Row::default(), false);
        capture.finish_sources(false, Outcome::Rows { len: 1 });
        capture.capture_owned(
            &memory,
            CaptureContext {
                base: 0,
                needed: [false],
            },
        );
        assert!(matches!(capture.outcomes()[0], Outcome::Unavailable { .. }));
        capture.reset();
        capture.begin_sources(false);
        capture.finish_sources(false, Outcome::Rows { len: 0 });
        capture.capture_owned(
            &memory,
            CaptureContext {
                base: 0,
                needed: [true],
            },
        );
        assert!(!capture.capture(&memory, 0, 0, &[true]).0[0]);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.grow(&mut capacity, &[CONSISTENCY_TARGET]);
        assert!(capture.rows.second.is_empty());
    }

    #[test]
    fn owned_seed_handoff_preserves_native_order_growth_and_clears_raw_pointers_on_failure() {
        let mut capacity = Capacities {
            rows: 2,
            modifiers: SnapshotCapacity::default(),
        };
        let mut capture = Captured::with_capacities(&capacity, &[0]);
        capture.begin_sources(false);
        for (pointer, id) in [(0x1100, 11_u64), (0x2200, 22), (0x3300, 33)] {
            capture.visit_source(
                pointer,
                &cship::Row {
                    id,
                    ..Default::default()
                },
                false,
            );
        }
        capture.finish_sources(false, Outcome::Rows { len: 3 });
        capture.begin_sources(true);
        for (pointer, id) in [(0x4400, 44_u64), (0x5500, 55)] {
            capture.visit_source(
                pointer,
                &cship::Row {
                    id,
                    ..Default::default()
                },
                true,
            );
        }
        capture.finish_sources(true, Outcome::Rows { len: 2 });
        assert_eq!(capture.seed_len, [2, 2]);
        assert_eq!(
            capture.seeds[0][..2]
                .iter()
                .map(|seed| (seed.pointer, seed.row.id))
                .collect::<Vec<_>>(),
            [(0x1100, 11), (0x2200, 22)]
        );
        assert_eq!(capture.required_rows, 3);
        let allocations = capture.allocated_bytes();
        let report = capture.fail_owned_capture(
            CaptureContext {
                base: 0,
                needed: [true],
            },
            "owned ShipModels job failed",
        );
        assert_eq!(report.ticks, [0]);
        assert!(capture.seeds.iter().enumerate().all(|(pass, seeds)| {
            seeds[..capture.seed_len[pass]]
                .iter()
                .all(|seed| seed.pointer == 0)
        }));
        assert_eq!(
            capture.seeds[0][..2]
                .iter()
                .map(|seed| seed.row.id)
                .collect::<Vec<_>>(),
            [11, 22],
            "owned row seeds keep their original native order"
        );
        assert!(matches!(
            capture.outcomes()[0],
            Outcome::Unavailable {
                reason: "owned ShipModels job failed"
            }
        ));
        capture.grow(&mut capacity, &[0]);
        assert!(capacity.rows >= 3);
        assert!(capture.allocated_bytes() >= allocations);
        capture.reset();
        assert!(capture.terminal_failure.is_none());
        assert_eq!(capture.seed_len, [0; 2]);
        assert_eq!(capture.required_rows, 0);
        capture.begin_sources(false);
        capture.visit_source(0x6600, &cship::Row::default(), false);
        assert_eq!(capture.seeds[0][0].pointer, 0x6600);
        capture.reset();
        assert_eq!(capture.seed_len, [0; 2]);
        assert_eq!(
            capture.seeds[0][0].pointer, 0,
            "reset erases a used callback pointer before dropping its logical length"
        );
    }

    #[test]
    fn model_window_compares_every_operand_and_conversion_preserves_bytes() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(
            &Capacities {
                rows: 1,
                modifiers: SnapshotCapacity::default(),
            },
            &[0],
        );
        capture.rows.rows[0].ship = 51 | (1 << 32);
        capture.rows.rows[0].variant = 70 | (2 << 32);
        capture.rows.rows[0].hull_sprite.bytes[..3].copy_from_slice(&[65, 0, 255]);
        capture.rows.rows[0].hull_sprite.len = 3;
        capture.rows.rows[0].model_number = i32::MIN;
        capture.rows.rows[0].formation_flags = 1 << 39;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.second = capture.rows.outcome;
        capture.rows.second[0] = capture.rows.rows[0];
        assert!(capture.capture(&memory, 0, 0, &[true]).0[0]);
        let [TableView::Complete(replay::rows::TableRows::ShipModels(rows))] = capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].hull_sprite, [65, 0, 255]);
        assert_eq!(rows[0].model_number, i32::MIN);
        for field in 0..5 {
            capture.rows.second[0] = capture.rows.rows[0];
            let row = &mut capture.rows.second[0];
            match field {
                0 => row.variant ^= 1 << 32,
                1 => row.hull_sprite.bytes[0] ^= 1,
                2 => {
                    row.fallback_sprite.len = 1;
                    row.fallback_sprite.bytes[0] = 42;
                }
                3 => row.model_number ^= 1,
                _ => row.formation_flags ^= 1 << 7,
            }
            assert!(!capture.capture(&memory, 0, 0, &[true]).0[0]);
            assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        }
    }
}

#[cfg(test)]
mod acquisition_tests {
    use super::*;
    use engine::{memory::SliceMemory, modifier_accumulator};

    const BASE: usize = 0x1000;
    const ENTRIES: usize = 0x1100;

    struct Source {
        base: usize,
        bytes: RefCell<Vec<u8>>,
        reads: RefCell<Vec<(usize, usize)>>,
        fault: Cell<Option<usize>>,
        scatters: Cell<usize>,
        records: Cell<usize>,
    }
    impl Source {
        fn from_bytes(base: usize, bytes: Vec<u8>) -> Self {
            Self {
                base,
                bytes: RefCell::new(bytes),
                reads: RefCell::new(Vec::new()),
                fault: Cell::new(None),
                scatters: Cell::new(0),
                records: Cell::new(0),
            }
        }
        fn new(count: u32) -> Self {
            let source = Self::from_bytes(BASE, vec![0; 0x400 + count as usize * 16]);
            source.array(BASE, ENTRIES, count);
            for index in 0..count {
                source.put(ENTRIES + index as usize * 16, &(index + 1).to_le_bytes());
                source.put(
                    ENTRIES + index as usize * 16 + 8,
                    &(i64::from(index) * -13).to_le_bytes(),
                );
            }
            source
        }
        fn put(&self, address: usize, value: &[u8]) {
            let offset = address - self.base;
            self.bytes.borrow_mut()[offset..offset + value.len()].copy_from_slice(value);
        }
        fn array(&self, address: usize, data: usize, count: u32) {
            self.put(address, &(data as u64).to_le_bytes());
            self.put(address + 8, &count.to_le_bytes());
            self.put(address + 12, &count.to_le_bytes());
        }
    }

    // Force Memory's original scalar batch defaults without forwarding any
    // capability. This supplies an independent budget/failure-prefix reference.
    struct Scalar<'a, M>(&'a M);
    impl<M: Memory> Memory for Scalar<'_, M> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.0.copy(address, out)
        }
    }
    impl Memory for Source {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, out.len()));
            if self.fault.get() == Some(address) {
                for byte in out.iter_mut().take(2) {
                    *byte = 0xe1;
                }
                return false;
            }
            SliceMemory {
                base: self.base,
                bytes: &self.bytes.borrow(),
            }
            .copy(address, out)
        }
        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.scatters.set(self.scatters.get() + 1);
            Scalar(self).copy_scattered(addresses, destinations)
        }
        fn copy_records(
            &self,
            sources: RecordSources<'_>,
            layout: &RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.records.set(self.records.get() + 1);
            Scalar(self).copy_records(sources, layout, destination)
        }
    }

    fn capacities(nodes: usize, entries: usize) -> Capacities {
        Capacities {
            rows: 1,
            modifiers: SnapshotCapacity { nodes, entries },
        }
    }

    fn acquire(capture: &mut Captured, source: &Source, second: bool) -> Result<(), &'static str> {
        capture.begin_details(second);
        let pass = usize::from(second);
        let result = {
            let bounded = BoundedMemory {
                inner: source,
                used: &capture.copied[pass],
            };
            let memory = ModifierMemory {
                inner: &bounded,
                arena: &capture.modifiers[pass],
            };
            modifier_accumulator::lookup_at(&memory, BASE, 1)
        };
        match result {
            Ok(value) => {
                let output = if second {
                    &mut capture.rows.second
                } else {
                    &mut capture.rows.rows
                };
                output[0].maximum_organisation = value.raw;
                capture.len[pass] = 1;
            }
            Err(reason) => {
                capture.failure[pass] =
                    Some(acquisition_failure(capture.copied[pass].get(), reason))
            }
        }
        capture.finish_details(second, Outcome::Rows { len: 1 });
        result.map(|_| ())
    }

    #[test]
    fn snapshots_are_independent_budgeted_verified_and_released_before_publication() {
        let source = Source::new(2);
        let mut capture = Captured::with_capacities(&capacities(2, 4), &[0]);
        let allocated = capture.allocated_bytes();
        acquire(&mut capture, &source, false).unwrap();
        let observation = capture.modifiers[0]
            .borrow_mut()
            .capture_at(&source, BASE, 2)
            .unwrap();
        assert_eq!(source.reads.borrow().len(), 4);
        acquire(&mut capture, &source, true).unwrap();
        assert_eq!(
            source.reads.borrow().len(),
            8,
            "second pass performs its own complete capture"
        );
        assert!(
            capture.modifiers[0].borrow().value(observation).is_ok(),
            "second begin leaves the first arena intact"
        );
        assert_eq!(capture.copied.each_ref().map(|v| v.get()), [(4, 64); 2]);
        assert_eq!(capture.capture(&source, 0, 0, &[true]).0, [true]);
        assert_eq!(capture.copied.each_ref().map(|v| v.get()), [(8, 128); 2]);
        assert_eq!(source.reads.borrow().len(), 16);
        assert_eq!(
            capture.required_modifiers,
            SnapshotCapacity {
                nodes: 1,
                entries: 2
            }
        );
        assert_eq!(
            capture.modifiers[0].borrow().value(observation),
            Err("modifier snapshot observation expired")
        );
        assert!(
            capture
                .modifiers
                .iter()
                .all(|a| a.borrow().required_capacity() == SnapshotCapacity::default())
        );
        assert_eq!(
            capture.allocated_bytes(),
            allocated,
            "acquisition/verification never grow or retire storage"
        );
    }

    #[test]
    fn terminal_snapshot_failure_rejects_unchanged_rows_in_checked_and_warm_passes() {
        for checked in [false, true] {
            let source = Source::new(2);
            let successful = if checked { 0 } else { CONSISTENCY_TARGET };
            let mut capture = Captured::with_capacities(&capacities(2, 4), &[successful]);
            acquire(&mut capture, &source, false).unwrap();
            // Unselected entry padding cannot affect the projected maximum,
            // but still belongs to the complete raw source observation.
            source.put(ENTRIES + 16 + 4, &[0x77]);
            if checked {
                acquire(&mut capture, &source, true).unwrap();
            }
            let before = capture.copied.each_ref().map(|v| v.get());
            assert_eq!(capture.capture(&source, 0, 0, &[checked]).0, [false]);
            assert!(matches!(capture.outcomes()[0], Outcome::Unavailable { .. }));
            assert_eq!(
                capture.failure[0],
                Some("ship model modifier snapshot changed")
            );
            assert!(capture.copied[0].get().0 > before[0].0);
            if checked {
                assert!(
                    capture.copied[1].get().0 > before[1].0,
                    "first failure never skips second verification"
                );
                assert!(capture.checked_mismatches()[0] > 0);
            }
        }
    }

    #[test]
    fn failed_snapshot_prefixes_are_verified_without_retrying_the_failed_field() {
        let source = Source::new(70);
        let failed = ENTRIES + 66 * 16;
        source.fault.set(Some(failed));
        let mut capture = Captured::with_capacities(&capacities(2, 72), &[0]);
        for second in [false, true] {
            assert_eq!(
                acquire(&mut capture, &source, second),
                Err("modifier accumulator entry unreadable")
            );
        }
        source.put(ENTRIES + 65 * 16 + 8, &[0x77]);
        source.reads.borrow_mut().clear();
        assert_eq!(capture.capture(&source, 0, 0, &[true]).0, [false]);
        assert!(capture.checked_mismatches()[0] > 0);
        assert_eq!(
            capture.failure,
            [Some("modifier accumulator entry unreadable"); 2]
        );
        let reads = source.reads.borrow();
        assert!(reads.contains(&(ENTRIES + 65 * 16, 16)));
        assert!(!reads.iter().any(|&(p, _)| p == failed));
        assert_eq!(
            reads.iter().filter(|&&(p, _)| p == BASE).count(),
            2,
            "both failed arenas retain their initial header witnesses"
        );
    }

    #[test]
    fn terminal_entry_faults_continue_both_passes_and_charge_every_attempt() {
        let source = Source::new(70);
        let mut capture = Captured::with_capacities(&capacities(2, 72), &[0]);
        acquire(&mut capture, &source, false).unwrap();
        acquire(&mut capture, &source, true).unwrap();
        source.fault.set(Some(ENTRIES + 31 * 16));
        source.reads.borrow_mut().clear();
        assert_eq!(capture.capture(&source, 0, 0, &[true]).0, [false]);
        assert_eq!(capture.copied.each_ref().map(|v| v.get()), [(144, 2304); 2]);
        assert_eq!(
            capture.failure,
            [Some("ship model modifier snapshot changed"); 2]
        );
        assert!(matches!(capture.outcomes()[0], Outcome::Unavailable { .. }));
        assert!(capture.checked_mismatches()[0] > 0);
        let mut expected = Vec::new();
        for _ in 0..2 {
            expected.push((BASE, 16));
            expected.extend((0..70).map(|index| (ENTRIES + index * 16, 16)));
            expected.push((BASE, 16));
        }
        assert_eq!(*source.reads.borrow(), expected);
        assert_eq!(
            capture.required_modifiers,
            SnapshotCapacity {
                nodes: 1,
                entries: 70
            }
        );
        assert!(
            capture
                .modifiers
                .iter()
                .all(|a| a.borrow().required_capacity() == SnapshotCapacity::default())
        );
    }

    #[test]
    fn snapshot_verification_uses_the_same_budget_and_never_skips_the_other_pass() {
        let source = Source::new(2);
        let mut capture = Captured::with_capacities(&capacities(2, 4), &[0]);
        acquire(&mut capture, &source, false).unwrap();
        acquire(&mut capture, &source, true).unwrap();
        capture.copied[0].set((READ_LIMIT, 0));
        source.reads.borrow_mut().clear();
        assert_eq!(capture.capture(&source, 0, 0, &[true]).0, [false]);
        assert_eq!(capture.copied[0].get(), (READ_LIMIT, COPY_LIMIT));
        assert_eq!(capture.copied[1].get(), (8, 128));
        assert_eq!(capture.failure, [Some(BUDGET_EXCEEDED), None]);
        assert_eq!(
            source.reads.borrow().len(),
            4,
            "the second terminal pass remains fresh"
        );
    }

    #[test]
    fn capacity_fallback_is_complete_and_growth_and_retirement_are_charged_off_thread() {
        let source = Source::new(3);
        let mut capacity = capacities(0, 0);
        let mut capture = Captured::with_capacities(&capacity, &[0]);
        for expected in [
            SnapshotCapacity {
                nodes: 1,
                entries: 0,
            },
            SnapshotCapacity {
                nodes: 1,
                entries: 3,
            },
        ] {
            let allocated = capture.allocated_bytes();
            acquire(&mut capture, &source, false).unwrap();
            acquire(&mut capture, &source, true).unwrap();
            assert_eq!(capture.capture(&source, 0, 0, &[true]).0, [true]);
            assert_eq!(capture.required_modifiers, expected);
            assert_eq!(capture.allocated_bytes(), allocated);
            capture.grow(&mut capacity, &[0]);
            assert!(
                capacity.modifiers.nodes >= expected.nodes
                    && capacity.modifiers.entries >= expected.entries
            );
            assert_eq!(capture.modifiers[0].borrow().capacity(), capacity.modifiers);
            assert_eq!(capture.modifiers[1].borrow().capacity(), capacity.modifiers);
            capture.reset();
        }
        acquire(&mut capture, &source, false).unwrap();
        acquire(&mut capture, &source, true).unwrap();
        assert_eq!(capture.capture(&source, 0, 0, &[true]).0, [true]);
        let allocated = capture.allocated_bytes();
        assert_eq!(
            allocated,
            capture.rows.allocated_bytes()
                + capture
                    .seeds
                    .iter()
                    .map(|seeds| seeds.capacity() * std::mem::size_of::<Seed>())
                    .sum::<usize>()
                + capture
                    .modifiers
                    .iter()
                    .map(|a| a.borrow().allocated_bytes())
                    .sum::<usize>()
        );
        capture.grow(&mut capacity, &[CONSISTENCY_TARGET]);
        assert!(capture.rows.second.is_empty());
        assert_eq!(capture.modifiers[1].borrow().allocated_bytes(), 0);
        assert_eq!(
            capture.allocated_bytes(),
            capture.rows.allocated_bytes()
                + capture.seeds[0].capacity() * std::mem::size_of::<Seed>()
                + capture.modifiers[0].borrow().allocated_bytes()
        );
        assert!(capture.allocated_bytes() < allocated);
    }

    #[test]
    fn terminal_failure_keeps_the_larger_second_pass_capacity_demand() {
        let source = Source::new(6);
        source.array(BASE, ENTRIES, 2);
        let mut capacity = capacities(1, 1);
        let mut capture = Captured::with_capacities(&capacity, &[0]);
        acquire(&mut capture, &source, false).unwrap();
        source.array(BASE, ENTRIES, 6);
        acquire(&mut capture, &source, true).unwrap();
        capture.capture(&source, 0, 0, &[true]);
        assert_eq!(capture.required_modifiers.entries, 6);
        capture.grow(&mut capacity, &[0]);
        assert!(capacity.modifiers.entries >= 6);
        assert_eq!(
            capture.modifiers[0].borrow().capacity(),
            capture.modifiers[1].borrow().capacity()
        );
    }

    #[test]
    fn huge_cache_demand_is_bounded_without_allocating_or_clamping_reported_demand() {
        let required = SnapshotCapacity {
            nodes: usize::MAX,
            entries: usize::MAX,
        };
        let mut capacity = SnapshotCapacity {
            nodes: 1,
            entries: 1,
        };
        grow_snapshot_capacity(&mut capacity, required);
        assert_eq!(capacity, SNAPSHOT_LIMITS);
        assert_eq!(required.nodes, usize::MAX);
        assert_eq!(required.entries, usize::MAX);
        grow_snapshot_capacity(&mut capacity, SnapshotCapacity::default());
        assert_eq!(capacity, SNAPSHOT_LIMITS);
    }

    #[test]
    fn only_existing_country_authentication_is_forwarded_and_outer_verification_remains_required() {
        const STATE: usize = 0x1000;
        let source = Source::from_bytes(0, vec![0; 0x8000]);
        source.array(STATE + engine::cgame_state::COUNTRIES, 0x5000, 2);
        source.array(STATE + engine::cgame_state::COUNTRY_LINKS, 0x5100, 2);
        let mut cache = engine::country_memory::Cache::with_capacity(64);
        let cached = cache.prepare(&source, 0, STATE);
        assert_eq!(
            cached.seed_status(),
            engine::country_memory::SeedStatus::Ready
        );
        let used = Cell::new((0, 0));
        let bounded = BoundedMemory {
            inner: &cached,
            used: &used,
        };
        let arena = RefCell::new(SnapshotArena::new(SnapshotCapacity::default()));
        let memory = ModifierMemory {
            inner: &bounded,
            arena: &arena,
        };
        assert!(memory.authenticated_countries(0, STATE).is_some());
        assert!(memory.authenticated_countries(1, STATE).is_none());
        assert!(memory.authenticated_countries(0, STATE + 1).is_none());
        assert_eq!(
            used.get(),
            (0, 0),
            "forwarding supplies no new observations"
        );
        assert!(
            BoundedMemory {
                inner: &source,
                used: &used
            }
            .authenticated_countries(0, STATE)
            .is_none()
        );
        assert!(cached.verify().is_ok());
        source.put(
            STATE + engine::cgame_state::COUNTRIES,
            &0x5200u64.to_le_bytes(),
        );
        assert!(
            cache.view(&source).verify().is_err(),
            "the callback still owns final country authentication"
        );
    }

    fn scattered(memory: &impl Memory, addresses: [usize; 4]) -> (usize, [[u8; 8]; 4]) {
        let mut output = [[0xa5; 8]; 4];
        let [a, b, c, d] = &mut output;
        let completed = memory.copy_scattered(
            addresses,
            &mut [&mut a[..4], &mut b[..0], &mut c[..8], &mut d[..3]],
        );
        (completed, output)
    }

    #[test]
    fn scatter_forwarding_matches_scalar_prefixes_bytes_and_budgets_at_every_edge() {
        for initial in [
            (0, 0),
            (READ_LIMIT - 1, 0),
            (0, COPY_LIMIT - 5),
            (READ_LIMIT, 0),
            (0, COPY_LIMIT),
        ] {
            for failed in [None, Some(ENTRIES), Some(ENTRIES + 16), Some(ENTRIES + 32)] {
                for invalid in [None, Some(0), Some(usize::MAX)] {
                    let source = Source::new(4);
                    source.fault.set(failed);
                    let used = Cell::new(initial);
                    let bounded = BoundedMemory {
                        inner: &source,
                        used: &used,
                    };
                    let addresses = [
                        ENTRIES,
                        usize::MAX,
                        invalid.unwrap_or(ENTRIES + 16),
                        ENTRIES + 32,
                    ];
                    let expected = scattered(&Scalar(&bounded), addresses);
                    let expected_used = used.replace(initial);
                    let expected_reads = std::mem::take(&mut *source.reads.borrow_mut());
                    assert_eq!(scattered(&bounded, addresses), expected);
                    assert_eq!(used.get(), expected_used);
                    assert_eq!(*source.reads.borrow(), expected_reads);
                    if initial == (0, 0) {
                        assert_eq!(source.scatters.get(), 1);
                    }
                }
            }
        }
    }

    #[test]
    fn record_forwarding_matches_scalar_prefixes_partial_rows_and_budget_precedence() {
        let fields = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 4,
            },
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 4,
                len: 0,
            },
            RecordField {
                source_offset: 8,
                destination_offset: 8,
                len: 8,
            },
        ];
        let layout = RecordLayout::new(&fields, 16).unwrap();
        for initial in [
            (0, 0),
            (READ_LIMIT - 2, 0),
            (0, COPY_LIMIT - 5),
            (READ_LIMIT, 0),
            (0, COPY_LIMIT),
        ] {
            for failed in [None, Some(ENTRIES), Some(ENTRIES + 8), Some(ENTRIES + 32)] {
                for kind in 0..5 {
                    let source = Source::new(4);
                    source.fault.set(failed);
                    let used = Cell::new(initial);
                    let bounded = BoundedMemory {
                        inner: &source,
                        used: &used,
                    };
                    let pointers = [
                        ENTRIES,
                        match kind {
                            2 => 0,
                            3 => usize::MAX,
                            _ => ENTRIES + 32,
                        },
                    ];
                    let sources = match kind {
                        1 => RecordSources::Strided {
                            base: ENTRIES,
                            stride: 32,
                            count: 2,
                        },
                        4 => RecordSources::Strided {
                            base: usize::MAX - 16,
                            stride: 32,
                            count: 2,
                        },
                        _ => RecordSources::Pointers(&pointers),
                    };
                    let mut expected = [0xa5; 32];
                    let expected_count =
                        Scalar(&bounded).copy_records(sources, &layout, &mut expected);
                    let expected_used = used.replace(initial);
                    let expected_reads = std::mem::take(&mut *source.reads.borrow_mut());
                    let mut actual = [0xa5; 32];
                    assert_eq!(
                        bounded.copy_records(sources, &layout, &mut actual),
                        expected_count
                    );
                    assert_eq!(actual, expected);
                    assert_eq!(used.get(), expected_used);
                    assert_eq!(*source.reads.borrow(), expected_reads);
                    if initial == (0, 0) {
                        assert_eq!(source.records.get(), 1);
                    }
                }
            }
        }
    }

    #[test]
    fn malformed_snapshot_prefix_precedes_a_later_batch_budget_refusal() {
        let source = Source::new(3);
        source.put(ENTRIES, &0u32.to_le_bytes());
        let used = Cell::new((READ_LIMIT - 2, 0));
        let bounded = BoundedMemory {
            inner: &source,
            used: &used,
        };
        let arena = RefCell::new(SnapshotArena::new(SnapshotCapacity {
            nodes: 1,
            entries: 3,
        }));
        let memory = ModifierMemory {
            inner: &bounded,
            arena: &arena,
        };
        let reason = modifier_accumulator::lookup_at(&memory, BASE, 1).unwrap_err();
        assert_eq!(used.get(), (READ_LIMIT, COPY_LIMIT));
        assert_eq!(
            acquisition_failure(used.get(), reason),
            "modifier accumulator keys not positive and increasing"
        );
        assert_eq!(
            source.reads.borrow().len(),
            2,
            "no source access after the admitted prefix"
        );
    }

    #[test]
    fn empty_fields_and_invalid_destination_geometry_never_spend_budget() {
        let source = Source::new(0);
        let used = Cell::new((READ_LIMIT, COPY_LIMIT));
        let bounded = BoundedMemory {
            inner: &source,
            used: &used,
        };
        assert_eq!(
            bounded.copy_scattered([0, usize::MAX], &mut [&mut [], &mut []]),
            2
        );
        let empty = [RecordField {
            source_offset: usize::MAX,
            destination_offset: 0,
            len: 0,
        }];
        let layout = RecordLayout::new(&empty, 0).unwrap();
        assert_eq!(
            bounded.copy_records(RecordSources::Pointers(&[0, usize::MAX]), &layout, &mut []),
            2
        );
        let nonempty = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 4,
        }];
        let layout = RecordLayout::new(&nonempty, 4).unwrap();
        assert_eq!(
            bounded.copy_records(RecordSources::Pointers(&[ENTRIES]), &layout, &mut [0; 3]),
            0
        );
        assert_eq!(used.get(), (READ_LIMIT, COPY_LIMIT));
        assert!(source.reads.borrow().is_empty());
    }

    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("copy budget must stop before inner memory access")
        }
    }
    #[test]
    fn whole_census_budget_refuses_bytes_and_calls_before_memory_access() {
        for value in [(READ_LIMIT, 0), (0, COPY_LIMIT)] {
            let used = Cell::new(value);
            let memory = BoundedMemory {
                inner: &NoReads,
                used: &used,
            };
            assert!(!memory.copy(1, &mut [0]));
            assert_eq!(used.get(), (READ_LIMIT, COPY_LIMIT));
        }
    }
}
