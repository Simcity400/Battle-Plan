//! Hourly ship model operands, sharing the authenticated Ships census.
use crate::{
    projection::{Projection, project_table},
    sampling::qpc,
    table_capture::{CaptureRows, grow_capacity, grow_vec},
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

// Acquisition ceilings bound the complete current source walk.
const COPY_LIMIT: usize = 128 * 1024 * 1024;
const READ_LIMIT: usize = 4_000_000;
const BUDGET_EXCEEDED: &str = "ShipModels aggregate read budget exceeded";
// Keep snapshot storage bounded; capacity pressure falls back to the direct reader.
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
#[cfg(test)]
use replay::frame::TableView;
use replay::{
    frame::TableInput,
    rows::{ShipModelRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ShipModels];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// Exact map-census inputs for one owned ShipModels detail capture.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureContext {
    pub base: usize,
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
    copied: Cell<(usize, usize)>,
    modifiers: RefCell<SnapshotArena>,
    required_modifiers: SnapshotCapacity,
    rows: CaptureRows<cship::ModelRow>,
    seeds: Vec<Seed>,
    seed_len: usize,
    required_rows: usize,
    parent: Outcome,
    len: usize,
    missing: u32,
    failure: Option<&'static str>,
    terminal_failure: Option<&'static str>,
    doctrine: cship::OrganisationMemo,
    model: cship::ModelMemo,
}

impl Captured {
    /// Allocation-free placeholder while acquisition owns the original buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            modifiers: SnapshotCapacity::default(),
        })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            copied: Cell::new((0, 0)),
            modifiers: RefCell::new(SnapshotArena::new(bounded_snapshot_capacity(
                capacity.modifiers,
            ))),
            required_modifiers: SnapshotCapacity::default(),
            rows: CaptureRows::new(capacity.rows),
            seeds: vec![Seed::default(); capacity.rows],
            seed_len: 0,
            required_rows: 0,
            parent: super::MISSING,
            len: 0,
            missing: 0,
            failure: None,
            terminal_failure: None,
            doctrine: cship::OrganisationMemo::default(),
            model: cship::ModelMemo::default(),
        }
    }
    pub fn reset(&mut self) {
        self.release_seed_pointers();
        self.terminal_failure = None;
        self.rows.reset();
        self.modifiers.get_mut().clear();
        self.required_modifiers = SnapshotCapacity::default();
        self.seed_len = 0;
        self.required_rows = 0;
        self.parent = super::MISSING;
        self.len = 0;
        self.missing = 0;
        self.failure = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.seeds.capacity() * std::mem::size_of::<Seed>()
            + self.modifiers.borrow().allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.rows, self.required_rows);
        self.rows.grow(&mut capacity.rows);
        grow_vec(&mut self.seeds, capacity.rows);
        grow_snapshot_capacity(&mut capacity.modifiers, self.required_modifiers);
        self.modifiers.get_mut().grow(capacity.modifiers);
    }
    /// Start the current map-owned ship census without reading model state.
    pub fn begin_sources(&mut self) {
        self.terminal_failure = None;
        self.release_seed_pointers();
        self.seed_len = 0;
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
    pub fn finish_sources(&mut self, parent: Outcome) {
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
    }
    fn begin_details(&mut self) {
        self.len = 0;
        self.failure = None;
        self.copied.set((0, 0));
        self.modifiers.get_mut().clear();
        self.doctrine = cship::OrganisationMemo::default();
        self.model = cship::ModelMemo::default();
    }
    fn visit_detail(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        source: &cship::Row,
    ) {
        if self.failure.is_some() {
            return;
        }
        if self.len == self.rows.rows.len() {
            self.failure = Some("ship model seed/output capacity mismatch");
            return;
        }
        let bounded = BoundedMemory {
            inner: memory,
            used: &self.copied,
        };
        let memory = ModifierMemory {
            inner: &bounded,
            arena: &self.modifiers,
        };
        let doctrine = &mut self.doctrine;
        match cship::read_model(&memory, base, pointer, source, &mut self.model).and_then(
            |mut row| {
                row.maximum_organisation =
                    cship::read_maximum_organisation_memo(&memory, base, pointer, doctrine)?;
                Ok(row)
            },
        ) {
            Ok(row) => {
                self.rows.rows[self.len] = row;
                self.len += 1;
            }
            Err(reason) => self.failure = Some(acquisition_failure(self.copied.get(), reason)),
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
        self.release_seed_pointers();
    }
    /// Consume the already-owned map seeds once; preserve native detail/error order.
    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        if self.terminal_failure.is_some() {
            self.release_sources();
            return CaptureReport::default();
        }
        let _reads = crate::read_accounting::table(TableId::ShipModels);
        let started = qpc();
        self.begin_details();
        for index in 0..self.seed_len {
            let seed = self.seeds[index];
            self.visit_detail(memory, context.base, seed.pointer, &seed.row);
        }
        self.finish_details(self.parent);
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
    #[cfg(test)]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection disabled"),
        })
    }
    pub(crate) fn table_inputs(&self, projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
        [project_table::<tables::ShipModels, _>(
            projection,
            self.outcomes()[0],
            &self.rows.rows,
            |row| u128::from(row.ship),
            |row, previous| {
                row.ship == previous.ship
                    && row.variant == previous.variant
                    && row.hull_sprite.as_bytes() == previous.hull_sprite
                    && row.fallback_sprite.as_bytes() == previous.fallback_sprite
                    && row.model_number == previous.model_number
                    && row.formation_flags == previous.formation_flags
                    && previous.counter.as_ref().is_some_and(|counter| {
                        row.roster_ordinal == counter.roster_ordinal
                            && row.definition_valid == counter.definition_valid
                            && row.carrier_air_base == counter.carrier_air_base
                            && row.maximum_strength == counter.maximum_strength
                            && row.maximum_organisation == counter.maximum_organisation
                            && row.positive_equipment_flags == counter.positive_equipment_flags
                    })
            },
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
    use engine::memory::SliceMemory;

    #[test]
    fn projection_compares_every_model_field_and_preserves_encoded_boundaries() {
        use crate::projection::ProjectionScratch;
        use replay::{
            frame::{FrameEncoder, FrameInput, Metrics},
            rows::TableRows,
        };

        let fixture = || {
            let mut capture = Captured::with_capacities(&Capacities {
                rows: 2,
                modifiers: SnapshotCapacity::default(),
            });
            let mut row = cship::ModelRow {
                ship: u64::MAX,
                variant: u64::MAX,
                model_number: i32::MIN,
                formation_flags: 1 << 63,
                ..Default::default()
            };
            row.hull_sprite.bytes[..3].copy_from_slice(&[65, 0, 255]);
            row.hull_sprite.len = 3;
            row.fallback_sprite.bytes[..2].copy_from_slice(&[255, 0]);
            row.fallback_sprite.len = 2;
            capture.rows.rows[0] = row;
            row.ship = 51 | (1 << 32);
            capture.rows.rows[1] = row;
            capture.rows.outcome = Outcome::Rows { len: 2 };
            capture
        };
        let frame = |branch, hour, [table]: [TableInput; N]| {
            let mut frame = FrameInput {
                branch,
                hour,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|_| {
                    crate::spooling::unavailable("fixture table").into()
                }),
            };
            frame.tables[TABLES[0].index()] = table;
            frame
        };
        for change in 0..27 {
            let mut current = fixture();
            match change {
                0 => {}
                1 => current.rows.rows[0].variant ^= 1,
                2 => current.rows.rows[0].hull_sprite.bytes[1] ^= 1,
                3 => current.rows.rows[0].fallback_sprite.bytes[0] ^= 1,
                4 => current.rows.rows[0].model_number ^= 1,
                5 => current.rows.rows[0].formation_flags ^= 1,
                6 => current.rows.rows[0].roster_ordinal = u32::MAX,
                7 => current.rows.rows[0].definition_valid = u8::MAX,
                8 => current.rows.rows[0].carrier_air_base = u64::MAX,
                9 => current.rows.rows[0].maximum_strength = i64::MIN,
                10 => current.rows.rows[0].maximum_organisation = i64::MAX,
                11 => current.rows.rows[0].positive_equipment_flags = 1 << 63,
                12 => current.rows.rows[0].hull_sprite.len = 0,
                13 => current.rows.rows[0].fallback_sprite.len = 0,
                14 => current.rows.rows.reverse(),
                15 => current.rows.rows[0].ship -= 1,
                16 => current.rows.rows[0].ship = current.rows.rows[1].ship,
                17 => current.rows.outcome = Outcome::Rows { len: 0 },
                18 => {
                    current.rows.outcome = Outcome::Truncated {
                        len: 1,
                        unread_rows: 1,
                    }
                }
                19 => {
                    current.rows.outcome = Outcome::Unavailable {
                        reason: "native model fault",
                    }
                }
                20 => {
                    current.fail_owned_capture("owned model request failed");
                }
                21 => current.rows.outcome = Outcome::Rows { len: 3 },
                22 => {
                    current.rows.outcome = Outcome::Truncated {
                        len: 1,
                        unread_rows: 0,
                    }
                }
                23..=26 => {}
                _ => unreachable!(),
            }
            let mut baseline = frame(1, 100, fixture().table_inputs(None));
            if change == 23 {
                let TableInput::Sampled(TableView::Complete(TableRows::ShipModels(rows))) =
                    &mut baseline.tables[TABLES[0].index()]
                else {
                    panic!("complete fixture")
                };
                // Legacy absence is distinct from all current counter fields
                // being present and zero; the native fixture has zero counters.
                rows[1].counter = None;
            }
            let force = change == 24;
            let branch = if change == 25 { 2 } else { 1 };
            let hour = if change == 26 { 105 } else { 101 };
            let mut projected = FrameEncoder::new();
            let mut reference = FrameEncoder::new();
            projected.encode_input(baseline.clone(), false).unwrap();
            reference.encode_input(baseline, false).unwrap();
            let mut scratch = ProjectionScratch::default();
            let mut projection = Projection::new(&projected, branch, hour, force, &mut scratch);
            fixture().table_inputs(Some(&mut projection));
            let inputs = current.table_inputs(Some(&mut projection));
            if matches!(change, 0 | 14) {
                assert!(matches!(inputs[0], TableInput::ComparedEqual(_)));
            }
            if (1..=13).contains(&change) || change == 23 {
                let TableInput::ComparedChanges { indices, .. } = &inputs[0] else {
                    panic!("model field {change} must replace its row");
                };
                assert_eq!(indices, &[1]);
            }
            if matches!(change, 16 | 19..=22) {
                assert!(matches!(
                    inputs[0],
                    TableInput::Sampled(TableView::Unavailable { .. })
                ));
            }
            assert_eq!(
                projected
                    .encode_input(frame(branch, hour, inputs), force)
                    .unwrap(),
                reference
                    .encode_input(frame(branch, hour, current.table_inputs(None)), force)
                    .unwrap(),
                "change {change}",
            );
        }
    }

    #[test]
    fn source_faults_capacity_and_empty_primary_census_remain_explicit() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capacity = Capacities {
            rows: 0,
            modifiers: SnapshotCapacity::default(),
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.begin_sources();
        capture.visit_source(0, &cship::Row::default());
        capture.finish_sources(Outcome::Truncated {
            len: 1,
            unread_rows: 3,
        });
        capture.capture_owned(&memory, CaptureContext { base: 0 });
        assert_eq!(
            capture.outcomes(),
            [Outcome::Truncated {
                len: 0,
                unread_rows: 4
            }]
        );
        capture.grow(&mut capacity);
        assert!(capacity.rows >= 4);
        capture.reset();
        capture.begin_sources();
        capture.visit_source(0, &cship::Row::default());
        capture.finish_sources(Outcome::Rows { len: 1 });
        capture.capture_owned(&memory, CaptureContext { base: 0 });
        assert!(matches!(capture.outcomes()[0], Outcome::Unavailable { .. }));
        capture.reset();
        capture.begin_sources();
        capture.finish_sources(Outcome::Rows { len: 0 });
        capture.capture_owned(&memory, CaptureContext { base: 0 });
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: 0 }]);
        assert!(matches!(capture.tables()[0], TableView::Complete(_)));
    }
    #[test]
    fn owned_seed_handoff_preserves_order_demand_and_clears_callback_pointers() {
        let mut capacity = Capacities {
            rows: 2,
            modifiers: SnapshotCapacity::default(),
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.begin_sources();
        for (pointer, id) in [(0x1100, 11_u64), (0x2200, 22), (0x3300, 33)] {
            capture.visit_source(
                pointer,
                &cship::Row {
                    id,
                    ..Default::default()
                },
            );
        }
        capture.finish_sources(Outcome::Rows { len: 3 });
        assert_eq!(capture.seed_len, 2);
        assert_eq!(
            capture.seeds[..2]
                .iter()
                .map(|s| (s.pointer, s.row.id))
                .collect::<Vec<_>>(),
            [(0x1100, 11), (0x2200, 22)]
        );
        assert_eq!(capture.required_rows, 3);
        let allocated = capture.allocated_bytes();
        assert_eq!(
            capture
                .fail_owned_capture("owned ShipModels job failed")
                .ticks,
            [0]
        );
        assert!(
            capture.seeds[..capture.seed_len]
                .iter()
                .all(|s| s.pointer == 0)
        );
        assert_eq!(
            capture.seeds[..2]
                .iter()
                .map(|s| s.row.id)
                .collect::<Vec<_>>(),
            [11, 22]
        );
        struct NoRead;
        impl Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("accepted failure must not retry cleared sources")
            }
        }
        capture.capture_owned(&NoRead, CaptureContext { base: 0 });
        assert_eq!(
            capture.outcomes(),
            [Outcome::Unavailable {
                reason: "owned ShipModels job failed"
            }]
        );
        capture.grow(&mut capacity);
        assert!(capacity.rows >= 3);
        assert!(capture.allocated_bytes() >= allocated);
        capture.reset();
        assert_eq!(capture.seed_len, 0);
        assert_eq!(capture.required_rows, 0);
        capture.begin_sources();
        capture.visit_source(0x6600, &cship::Row::default());
        assert_eq!(capture.seeds[0].pointer, 0x6600);
        capture.reset();
        assert_eq!(capture.seeds[0].pointer, 0);
    }
    #[test]
    fn model_conversion_preserves_primary_handles_text_and_counter_operands() {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            modifiers: SnapshotCapacity::default(),
        });
        let row = &mut capture.rows.rows[0];
        row.ship = 51 | (1 << 32);
        row.variant = 70 | (2 << 32);
        row.hull_sprite.bytes[..3].copy_from_slice(&[65, 0, 255]);
        row.hull_sprite.len = 3;
        row.fallback_sprite.bytes[..2].copy_from_slice(&[0xff, 0]);
        row.fallback_sprite.len = 2;
        row.model_number = i32::MIN;
        row.formation_flags = 1 << 39;
        row.roster_ordinal = u32::MAX;
        row.definition_valid = u8::MAX;
        row.carrier_air_base = u64::MAX;
        row.maximum_strength = i64::MIN;
        row.maximum_organisation = i64::MAX;
        row.positive_equipment_flags = u64::MAX;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(replay::rows::TableRows::ShipModels(rows))] = capture.tables()
        else {
            panic!("complete")
        };
        assert_eq!(
            (rows[0].ship, rows[0].variant),
            (51 | (1 << 32), 70 | (2 << 32))
        );
        assert_eq!(rows[0].hull_sprite, [65, 0, 255]);
        assert_eq!(rows[0].fallback_sprite, [255, 0]);
        assert_eq!(
            (rows[0].model_number, rows[0].formation_flags),
            (i32::MIN, 1 << 39)
        );
        let counter = rows[0].counter.as_ref().unwrap();
        assert_eq!(
            (
                counter.roster_ordinal,
                counter.definition_valid,
                counter.carrier_air_base
            ),
            (u32::MAX, u8::MAX, u64::MAX)
        );
        assert_eq!(
            (
                counter.maximum_strength,
                counter.maximum_organisation,
                counter.positive_equipment_flags
            ),
            (i64::MIN, i64::MAX, u64::MAX)
        );
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
        scalars: Cell<usize>,
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
                scalars: Cell::new(0),
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
        fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
            self.scalars.set(self.scalars.get() + 1);
            Scalar(self).read_scalar::<T>(address)
        }

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

    fn acquire(capture: &mut Captured, source: &Source) -> Result<(), &'static str> {
        capture.begin_details();
        let result = {
            let bounded = BoundedMemory {
                inner: source,
                used: &capture.copied,
            };
            let memory = ModifierMemory {
                inner: &bounded,
                arena: &capture.modifiers,
            };
            modifier_accumulator::lookup_at(&memory, BASE, 1)
        };
        match result {
            Ok(value) => {
                capture.rows.rows[0].maximum_organisation = value.raw;
                capture.len = 1;
            }
            Err(reason) => {
                capture.failure = Some(acquisition_failure(capture.copied.get(), reason))
            }
        }
        capture.finish_details(Outcome::Rows { len: 1 });
        result.map(|_| ())
    }

    #[test]
    fn modifier_memo_reuses_current_bytes_and_release_performs_no_reads() {
        let source = Source::new(2);
        let mut capture = Captured::with_capacities(&capacities(2, 4));
        let allocated = capture.allocated_bytes();
        acquire(&mut capture, &source).unwrap();
        let before = source.reads.borrow().len();
        let observation = capture
            .modifiers
            .borrow_mut()
            .capture_at(&source, BASE, 2)
            .unwrap();
        assert_eq!(source.reads.borrow().len(), before);
        assert_eq!(
            capture.modifiers.borrow().value(observation).unwrap().raw,
            -13
        );
        let copied = capture.copied.get();
        assert_eq!(copied.0, before);
        assert_eq!(
            copied.1,
            source
                .reads
                .borrow()
                .iter()
                .map(|&(_, len)| len)
                .sum::<usize>()
        );
        capture.release_sources();
        assert_eq!(source.reads.borrow().len(), before);
        assert_eq!(capture.copied.get(), copied);
        assert_eq!(
            capture.required_modifiers,
            SnapshotCapacity {
                nodes: 1,
                entries: 2
            }
        );
        assert_eq!(
            capture.modifiers.borrow().value(observation),
            Err("modifier snapshot observation expired")
        );
        assert_eq!(
            capture.modifiers.borrow().required_capacity(),
            SnapshotCapacity::default()
        );
        assert_eq!(capture.allocated_bytes(), allocated);
    }

    #[test]
    fn failed_snapshot_prefix_keeps_first_fault_and_releases_without_retry() {
        let source = Source::new(70);
        let failed = ENTRIES + 66 * 16;
        source.fault.set(Some(failed));
        let mut capture = Captured::with_capacities(&capacities(2, 72));
        assert_eq!(
            acquire(&mut capture, &source),
            Err("modifier accumulator entry unreadable")
        );
        let before = source.reads.borrow().len();
        assert_eq!(
            source
                .reads
                .borrow()
                .iter()
                .filter(|&&(p, _)| p == failed)
                .count(),
            1
        );
        capture.release_sources();
        assert_eq!(source.reads.borrow().len(), before);
        assert_eq!(
            capture.failure,
            Some("modifier accumulator entry unreadable")
        );
        assert_eq!(
            capture.outcomes(),
            [Outcome::Unavailable {
                reason: "modifier accumulator entry unreadable"
            }]
        );
        assert_eq!(capture.required_modifiers.entries, 70);
    }

    #[test]
    fn modifier_capacity_fallback_and_primary_growth_preserve_current_values() {
        let source = Source::new(3);
        let mut capacity = capacities(0, 0);
        let mut capture = Captured::with_capacities(&capacity);
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
            acquire(&mut capture, &source).unwrap();
            assert_eq!(capture.rows.rows[0].maximum_organisation, 0);
            let reads = source.reads.borrow().len();
            capture.release_sources();
            assert_eq!(source.reads.borrow().len(), reads);
            assert_eq!(capture.required_modifiers, expected);
            assert_eq!(capture.allocated_bytes(), allocated);
            capture.grow(&mut capacity);
            assert!(
                capacity.modifiers.nodes >= expected.nodes
                    && capacity.modifiers.entries >= expected.entries
            );
            assert_eq!(capture.modifiers.borrow().capacity(), capacity.modifiers);
            capture.reset();
        }
        acquire(&mut capture, &source).unwrap();
        capture.release_sources();
        assert_eq!(
            capture.allocated_bytes(),
            capture.rows.allocated_bytes()
                + capture.seeds.capacity() * std::mem::size_of::<Seed>()
                + capture.modifiers.borrow().allocated_bytes()
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
    fn only_current_country_authentication_is_forwarded_for_matching_context() {
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
            used.set(value);
            assert_eq!(memory.read_scalar::<u32>(1), None);
            assert_eq!(used.get(), (READ_LIMIT, COPY_LIMIT));
        }
        for initial in [(0, 0), (READ_LIMIT - 1, 0), (0, COPY_LIMIT - 4)] {
            for failed in [None, Some(ENTRIES)] {
                let source = Source::new(1);
                source.fault.set(failed);
                let used = Cell::new(initial);
                let bounded = BoundedMemory {
                    inner: &source,
                    used: &used,
                };
                let arena = RefCell::new(SnapshotArena::new(SnapshotCapacity::default()));
                let memory = ModifierMemory {
                    inner: &bounded,
                    arena: &arena,
                };
                let expected = engine::read::<u32>(&Scalar(&memory), ENTRIES);
                let expected_used = used.replace(initial);
                let expected_reads = std::mem::take(&mut *source.reads.borrow_mut());
                assert_eq!(source.scalars.get(), 0);
                assert_eq!(engine::read::<u32>(&memory, ENTRIES), expected);
                assert_eq!(used.get(), expected_used);
                assert_eq!(*source.reads.borrow(), expected_reads);
                assert_eq!(source.scalars.get(), 1);
                assert_eq!(used.get(), (initial.0 + 1, initial.1 + 4));
                if initial != (0, 0) {
                    assert_eq!(engine::read::<u32>(&memory, ENTRIES), None);
                    assert_eq!(used.get(), (READ_LIMIT, COPY_LIMIT));
                    assert_eq!(source.scalars.get(), 1);
                    assert_eq!(*source.reads.borrow(), expected_reads);
                } else {
                    assert_eq!(engine::read::<u64>(&memory, BASE), Some(ENTRIES as u64));
                    assert_eq!(used.get(), (2, 12));
                    assert_eq!(source.scalars.get(), 1);
                }
            }
        }
        let source = Source::new(1);
        let used = Cell::new((0, COPY_LIMIT));
        let memory = BoundedMemory {
            inner: &source,
            used: &used,
        };
        assert_eq!(engine::read::<[u8; 0]>(&memory, 0), Some([]));
        assert_eq!(used.get(), (0, COPY_LIMIT));
        assert_eq!(source.scalars.get(), 0);
        assert_eq!(memory.read_scalar::<[u8; 0]>(BASE), Some([]));
        assert_eq!(used.get(), (1, COPY_LIMIT));
        assert_eq!(source.scalars.get(), 1);
        assert_eq!(*source.reads.borrow(), [(BASE, 0)]);
    }
}
