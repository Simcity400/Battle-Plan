//! Hourly ship model operands, sharing the authenticated Ships census.
//!
//! The census runs in its own job. The details, mostly the recomputed maximum
//! organisation (the game never stores it), then run
//! in up to [`MAX_PARTS`] parts on separate workers, each over a contiguous
//! range of the census in native order. The family's capture keeps the first
//! range and lends one part capture per further range; outcomes and rows join
//! in census order, so the first failure keeps its position. Each range reads
//! through [`cship::ShipFields`]: a ship's fixed fields, and its task force's,
//! location's and the game singleton's when they change, in one guarded call
//! per object ahead of the readers.
//!
//! Each capture range also holds its ships' own objects ([`cship::HeldShip`])
//! and experience and medal-store modifier arrays ([`ShipStore`]) with their
//! certificates ([`crate::ship_model_changes`]). A later hour serves a ship
//! whose handle, task force and certificate are those of its held read from
//! them, reading only its task force, location, country, weather, variants
//! and the other shared sources; the readers and their arithmetic are the
//! same, so the row is the one a fresh read makes. Every hour reads the
//! carryable ships of its sampled slice afresh and compares them with the
//! held bytes; a difference poisons the family.
//!
//! A task force's terms (country, pride ship, doctrine, terrain and location
//! weather, fleet leader with its tree certificate, HQ support) are read once
//! for its first ship in the range ([`cship::OrganisationMemo::task_terms`]);
//! a later ship of it whose own inputs are certified, whose held row was made
//! with equal terms, whose variants' bytes are unchanged and which has no
//! carrier base takes its held row without running the readers.
//! Compact records follow census order, with a pointer index for reordered
//! entries; a carried row touches no cold ship object or model text. Shared
//! terms get one identity per task force, and distinct held variant observations
//! are compared once per capture. Changed terms combine the certified own
//! organisation packet with current shared terms through the reader arithmetic.
use crate::{
    clock::qpc,
    projection::{Projection, project_table},
    sampled_verify::Check,
    ship_model_changes::{self, Certificate, Changes, Counts, NO_SLOTS, Slots},
    table_capture::{CaptureRows, grow_capacity},
};
use engine_allocation::{copy_vec, filled, owned_vec, reserve, resize_default};
#[cfg(test)]
use engine_native::memory::RecordField;
use engine_native::{
    Memory, cship, memory::RecordLayout, memory::RecordSources,
    modifier_accumulator::SNAPSHOT_ENTRY_CAPACITY, modifier_accumulator::SNAPSHOT_NODE_CAPACITY,
    modifier_accumulator::SnapshotArena, modifier_accumulator::SnapshotCapacity,
    modifier_accumulator::Value, walk::Outcome,
};
use std::{
    cell::{Cell, RefCell},
    sync::{Arc, OnceLock},
};

// Fixed-budget fixtures and the initial optional cache floor. Worker-owned
// source/detail acquisition grows from original observations without these limits.
const COPY_LIMIT: usize = 128 * 1024 * 1024;
const READ_LIMIT: usize = 4_000_000;
#[cfg(test)]
const BUDGET_EXCEEDED: &str = "ShipModels aggregate read budget exceeded";

/// Detail parts at most; each runs on its own worker.
pub const MAX_PARTS: usize = 4;
/// Ships per detail part before another part is worth its dispatch.
const SHIPS_PER_PART: usize = 512;

/// Detail parts for a census of `ships`, sized from the live count. Every
/// part requires a worker; rejected admission retains its named failure.
pub fn parts_for(ships: usize) -> usize {
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
    fn read_scalar<T: engine_native::memory::Raw>(&self, address: usize) -> Option<T> {
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
    ) -> Option<engine_native::ccountry::AuthenticatedRoster<'_>> {
        // Forward the callback's current authenticated roster without extending it.
        self.inner.authenticated_countries(base, state)
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        state: usize,
    ) -> Option<Result<engine_native::ccountry::Country, &'static str>> {
        self.inner.authenticated_reserved_country(base, state)
    }
    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        state: usize,
    ) -> Option<engine_native::ccountry::ReservedCountryObservation> {
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
    held_task: Option<(usize, &'a [[u8; 16]])>,
}

impl<M: Memory> Memory for ModifierMemory<'_, M> {
    #[inline]
    fn read_scalar<T: engine_native::memory::Raw>(&self, address: usize) -> Option<T> {
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
    ) -> Option<engine_native::ccountry::AuthenticatedRoster<'_>> {
        self.inner.authenticated_countries(base, state)
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        state: usize,
    ) -> Option<Result<engine_native::ccountry::Country, &'static str>> {
        self.inner.authenticated_reserved_country(base, state)
    }
    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        state: usize,
    ) -> Option<engine_native::ccountry::ReservedCountryObservation> {
        self.inner
            .authenticated_reserved_country_observation(base, state)
    }
    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<Value, &'static str>> {
        let mut arena = self.arena.borrow_mut();
        if let Some((held_address, entries)) = self.held_task
            && address == held_address
        {
            if let Some(result) = arena.existing_at(address, key) {
                return Some(result);
            }
            return Some(engine_native::modifier_accumulator::lookup_snapshot(
                entries.as_flattened(),
                key,
            ));
        }
        match arena.capture_at(self.inner, address, key) {
            Ok(observation) => Some(arena.value(observation)),
            Err(SNAPSHOT_NODE_CAPACITY | SNAPSHOT_ENTRY_CAPACITY) => None,
            Err(reason) => Some(Err(reason)),
        }
    }
}
use replay_data::{rows::ShipModelRow, rows::TableId, rows::tables};
use replay_frame::frame::TableInput;
#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ShipModels];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

const OPTIONAL_REFUSAL_REASONS: [&str; 15] = [
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
    "carrier capacity predicate uncertified or unreadable",
    "ship naval dominance selected design uncertified or unreadable",
];
static OPTIONAL_REFUSALS: [std::sync::atomic::AtomicU64; OPTIONAL_REFUSAL_REASONS.len()] =
    [const { std::sync::atomic::AtomicU64::new(0) }; OPTIONAL_REFUSAL_REASONS.len()];

pub fn optional_data() -> impl serde::Serialize {
    crate::status::status!({
        "optional_operands": crate::status::map(OPTIONAL_REFUSAL_REASONS.iter().zip(&OPTIONAL_REFUSALS).map(|(&reason, counter)| (reason, counter.load(std::sync::atomic::Ordering::Relaxed)))),
        "scope": "cumulative field-local ship model refusals from admitted current-hour model seeds; original model and maximum organisation remain available; concurrent reads are not an atomic totals snapshot",
    })
}

/// Exact map-census inputs for one owned ShipModels detail capture.
#[derive(Clone, Copy)]
pub struct CaptureContext {
    pub base: usize,
}

impl CaptureContext {
    pub fn new(base: usize) -> Self {
        Self { base }
    }
}

/// Detail-acquisition ticks. Interrupted work has no completed timing sample.
#[derive(Clone, Copy, Debug, Default)]
pub struct CaptureReport {
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
}

/// One ship's held read: its census handle and task force, its certificate
/// and own objects, and its two modifier arrays with their addresses
/// (address zero: none), owned so that carrying the read moves them.
#[derive(Clone)]
struct Held {
    pointer: usize,
    ship: u64,
    task_force: u64,
    certificate: Certificate,
    slots: Slots,
    fields: cship::HeldShip,
    accumulators: [(usize, Vec<[u8; 16]>); 2],
    /// The row the read made, with the task-force terms it combined.
    row: Option<(cship::ModelRow, Terms)>,
    organisation: Option<cship::OrganisationOperands>,
    terms_identity: u64,
    /// Where this capture's rows buffer holds that row: it wrote the row
    /// there the hour it held this read, and the buffer keeps its contents
    /// between hours (`usize::MAX`: nowhere).
    at: usize,
}

/// A task force's terms in one capture and its fleet leader's tree
/// certificate (the commander-class rows' source).
type Terms = (cship::TaskTerms, Option<(u64, u64)>);

// The hull variant's raw tuple and the definition database ordinal are
// independent observations; a field-local fault must recover.
fn model_carry_complete(row: &cship::ModelRow) -> bool {
    row.subunit_index.is_some()
        && if row.variant == 0 {
            row.hull_variant.is_none()
        } else {
            row.hull_variant.is_some_and(|packet| {
                packet.definition_index.is_some()
                    && packet.module_count.is_some()
                    && packet.upgrade_level_sum.is_some()
                    && packet.module_priority_sum.is_some()
                    && packet.version.is_some()
                    && packet.organisation_marker.is_some()
            })
        }
}

fn observed_differs<T: PartialEq>(kept: Option<T>, fresh: Option<T>) -> bool {
    kept.zip(fresh).is_some_and(|(kept, fresh)| kept != fresh)
}

fn model_observations_differ(kept: &cship::ModelRow, fresh: &cship::ModelRow) -> bool {
    kept.ship != fresh.ship
        || kept.variant != fresh.variant
        || kept.hull_sprite != fresh.hull_sprite
        || kept.fallback_sprite != fresh.fallback_sprite
        || kept.model_number != fresh.model_number
        || kept.formation_flags != fresh.formation_flags
        || kept.definition_valid != fresh.definition_valid
        || kept.carrier_air_base != fresh.carrier_air_base
        || kept.maximum_strength != fresh.maximum_strength
        || kept.anti_air_base != fresh.anti_air_base
        || kept.positive_equipment_flags != fresh.positive_equipment_flags
        || observed_differs(kept.naval_power, fresh.naval_power)
        || observed_differs(kept.subunit_index, fresh.subunit_index)
        || kept
            .hull_variant
            .zip(fresh.hull_variant)
            .is_some_and(|(kept, fresh)| {
                observed_differs(kept.definition_index, fresh.definition_index)
                    || observed_differs(kept.module_count, fresh.module_count)
                    || observed_differs(kept.upgrade_level_sum, fresh.upgrade_level_sum)
                    || observed_differs(kept.module_priority_sum, fresh.module_priority_sum)
                    || observed_differs(kept.version, fresh.version)
                    || observed_differs(kept.organisation_marker, fresh.organisation_marker)
            })
}

/// The census and certificate inputs of a carried row. The reader's objects,
/// arrays and finished row stay in its cold box until a reader needs them.
#[derive(Clone, Copy)]
struct Hot {
    pointer: usize,
    ship: u64,
    task_force: u64,
    task: usize,
    receivers: [usize; 3],
    certificate: Certificate,
    slots: Slots,
    variants: [usize; 2],
    terms_identity: u64,
    modifier_guard: u16,
    criticals: bool,
    at: usize,
}

struct Variant {
    fields: cship::HeldVariant,
    unchanged: Option<bool>,
    next: Option<usize>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Default, Debug)]
struct CarryCost {
    // Logical record accesses and explicit row clones, not hardware cache traffic.
    searches: usize,
    cold_ship_bytes: usize,
    cold_organisation_bytes: usize,
    cold_terms_bytes: usize,
    variant_compares: usize,
    terms_compares: usize,
    row_clones: usize,
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static CARRY_COST: Cell<CarryCost> = const { Cell::new(CarryCost {
        searches: 0, cold_ship_bytes: 0, cold_organisation_bytes: 0,
        cold_terms_bytes: 0, variant_compares: 0,
        terms_compares: 0, row_clones: 0,
    }) };
}

#[cfg(any(test, feature = "test-support"))]
fn carry_cost(update: impl FnOnce(&mut CarryCost)) {
    CARRY_COST.with(|cost| {
        let mut current = cost.get();
        update(&mut current);
        cost.set(current);
    });
}

/// A capture range's held reads from its latest complete hour, in census
/// order with a sorted index by ship pointer; `next` collects the hour's own.
/// Each read (kilobytes) is boxed and moves as its box: taken out of `held`
/// and pushed to `next`, and a ship read afresh reuses its held read's box.
/// A family that does not complete holds nothing in any range.
#[derive(Default)]
struct ShipStore {
    held: Vec<Option<Box<Held>>>,
    hot: Vec<Hot>,
    index: Vec<(usize, usize)>,
    tasks: Vec<HeldTask>,
    variants: Vec<Variant>,
    next: Vec<Option<Box<Held>>>,
    next_hot: Vec<Hot>,
    next_tasks: Vec<HeldTask>,
    next_variants: Vec<Variant>,
    next_variant_index: Vec<(usize, usize)>,
    quiet: Option<(u64, u64)>,
    generation: u64,
}

impl ShipStore {
    /// Census-order hit first; the index still serves a moved census entry.
    fn take(&mut self, pointer: usize, position: usize) -> Option<(Hot, Box<Held>)> {
        let at = if self
            .hot
            .get(position)
            .is_some_and(|hot| hot.pointer == pointer)
            && self.held.get(position).is_some_and(Option::is_some)
        {
            position
        } else {
            #[cfg(any(test, feature = "test-support"))]
            carry_cost(|cost| cost.searches += 1);
            let at = self
                .index
                .binary_search_by_key(&pointer, |&(held, _)| held)
                .ok()?;
            self.index[at].1
        };
        Some((self.hot[at], self.held[at].take()?))
    }

    fn remember_variant(&mut self, fields: cship::HeldVariant) -> Option<usize> {
        let pointer = fields.pointer();
        if pointer == 0 {
            return Some(usize::MAX);
        }
        let at = self
            .next_variant_index
            .partition_point(|&(held, _)| held < pointer);
        for &(held, index) in &self.next_variant_index[at..] {
            if held != pointer {
                break;
            }
            if fields.same(&self.next_variants[index].fields) {
                return Some(index);
            }
        }
        if reserve(&mut self.next_variants, 1).is_err()
            || reserve(&mut self.next_variant_index, 1).is_err()
        {
            return None;
        }
        let index = self.next_variants.len();
        self.next_variants.push(Variant {
            fields,
            unchanged: None,
            next: None,
        });
        self.next_variant_index.insert(at, (pointer, index));
        Some(index)
    }

    /// One comparison per distinct held observation this hour; ships share
    /// indices only when both its pointer and every gathered byte agree.
    fn variants_unchanged(
        &mut self,
        memory: &impl Memory,
        fields: &mut cship::ShipFields,
        hot: &mut Hot,
    ) -> bool {
        for index in &mut hot.variants {
            if *index == usize::MAX {
                continue;
            }
            let old = *index;
            let unchanged = match self.variants[old].unchanged {
                Some(unchanged) => unchanged,
                None => {
                    #[cfg(any(test, feature = "test-support"))]
                    carry_cost(|cost| cost.variant_compares += 1);
                    let unchanged =
                        fields.held_variant_unchanged(memory, &self.variants[old].fields);
                    self.variants[old].unchanged = Some(unchanged);
                    unchanged
                }
            };
            if !unchanged {
                return false;
            }
            let next = match self.variants[old].next {
                Some(next) => next,
                None => {
                    let Some(next) = self.remember_variant(self.variants[old].fields) else {
                        return false;
                    };
                    self.variants[old].next = Some(next);
                    next
                }
            };
            *index = next;
        }
        true
    }

    fn hot(&mut self, held: &Held) -> Option<Hot> {
        // Omitted own fields can hide a medal/carrier receiver or its
        // accumulator address; only a complete observation can be held.
        if !held.fields.revalidation_complete() {
            return None;
        }
        let [first, second] = held.fields.variants();
        Some(Hot {
            pointer: held.pointer,
            ship: held.ship,
            task_force: held.task_force,
            task: held.fields.task_force(),
            receivers: receivers(held.pointer, &held.fields),
            certificate: held.certificate,
            slots: held.slots,
            variants: [
                self.remember_variant(first)?,
                self.remember_variant(second)?,
            ],
            terms_identity: if held
                .row
                .as_ref()
                .is_some_and(|(row, _)| model_carry_complete(row))
                && held.fields.variants_complete()
            {
                held.terms_identity
            } else {
                0
            },
            modifier_guard: held.organisation.map_or(0, |own| own.modifier_guard()),
            criticals: held.organisation.is_some_and(|own| own.has_criticals()),
            at: held.at,
        })
    }

    fn terms_identity(&mut self, task: usize, terms: &Terms) -> u64 {
        if let Some((kept, identity)) = self.task(task).and_then(|task| task.terms) {
            #[cfg(any(test, feature = "test-support"))]
            carry_cost(|cost| cost.terms_compares += 1);
            if kept == *terms {
                return identity;
            }
        }
        let Some(generation) = self.generation.checked_add(1) else {
            return 0;
        };
        self.generation = generation;
        generation
    }

    fn task(&self, pointer: usize) -> Option<&HeldTask> {
        self.tasks
            .binary_search_by_key(&pointer, |held| held.pointer)
            .ok()
            .map(|at| &self.tasks[at])
    }

    /// Keep a task force's terms for the next hour, sharing an equal modifier
    /// array. A failed optional allocation leaves nothing held.
    fn keep_task(&mut self, mut held: HeldTask, modifiers: Option<&[[u8; 16]]>) -> bool {
        if reserve(&mut self.next_tasks, 1).is_err() {
            return false;
        }
        if let Some(array) = modifiers {
            if held.modifiers.as_deref().is_none_or(|kept| {
                (kept.len() != array.len() || kept.as_ptr() != array.as_ptr())
                    && kept.as_slice() != array
            }) {
                let Ok(entries) = copy_vec(array) else {
                    return false;
                };
                held.modifiers = Some(Arc::new(entries));
            }
        } else {
            held.modifiers = None;
        }
        self.next_tasks.push(held);
        true
    }

    /// Keep `held` for the next hour with copies of its arrays (into its own
    /// storage); `false` (and nothing kept) when storage cannot grow.
    fn keep(&mut self, mut held: Box<Held>, arrays: [&[[u8; 16]]; 2]) -> bool {
        for ((_, entries), array) in held.accumulators.iter_mut().zip(arrays) {
            entries.clear();
            if reserve(entries, array.len()).is_err() {
                return false;
            }
            entries.extend_from_slice(array);
        }
        self.keep_held(held)
    }

    /// Keep a carried ship's held read, arrays included, for the next hour.
    fn keep_held(&mut self, held: Box<Held>) -> bool {
        let Some(hot) = self.hot(&held) else {
            return false;
        };
        self.keep_hot(held, hot)
    }

    fn keep_hot(&mut self, held: Box<Held>, hot: Hot) -> bool {
        if reserve(&mut self.next, 1).is_err() || reserve(&mut self.next_hot, 1).is_err() {
            return false;
        }
        self.next.push(Some(held));
        self.next_hot.push(hot);
        true
    }

    /// The range completed: hold what it read and carried.
    fn hold(&mut self) -> bool {
        let same_order = self.hot.len() == self.next_hot.len()
            && self
                .hot
                .iter()
                .zip(&self.next_hot)
                .all(|(held, next)| held.pointer == next.pointer);
        self.next_tasks.sort_unstable_by_key(|held| held.pointer);
        std::mem::swap(&mut self.held, &mut self.next);
        std::mem::swap(&mut self.hot, &mut self.next_hot);
        std::mem::swap(&mut self.tasks, &mut self.next_tasks);
        std::mem::swap(&mut self.variants, &mut self.next_variants);
        self.next.clear();
        self.next_hot.clear();
        self.next_tasks.clear();
        self.next_variants.clear();
        self.next_variant_index.clear();
        if same_order {
            return true;
        }
        self.index.clear();
        if reserve(&mut self.index, self.held.len()).is_err() {
            self.forget();
            return false;
        }
        self.index.extend(
            self.hot
                .iter()
                .enumerate()
                .map(|(at, hot)| (hot.pointer, at)),
        );
        self.index.sort_unstable_by_key(|&(pointer, _)| pointer);
        true
    }

    /// Nothing the range read certifies a carry.
    fn forget(&mut self) {
        self.held.clear();
        self.hot.clear();
        self.index.clear();
        self.tasks.clear();
        self.variants.clear();
        self.next.clear();
        self.next_hot.clear();
        self.next_tasks.clear();
        self.next_variants.clear();
        self.next_variant_index.clear();
        self.quiet = None;
        self.generation = 0;
    }

    fn allocated_bytes(&self) -> usize {
        (self.held.capacity() + self.next.capacity()) * std::mem::size_of::<Option<Box<Held>>>()
            + self
                .held
                .iter()
                .chain(&self.next)
                .flatten()
                .map(|held| {
                    std::mem::size_of::<Held>()
                        + held
                            .accumulators
                            .iter()
                            .map(|(_, entries)| entries.capacity() * 16)
                            .sum::<usize>()
                })
                .sum::<usize>()
            + (self.hot.capacity() + self.next_hot.capacity()) * std::mem::size_of::<Hot>()
            + (self.index.capacity() + self.next_variant_index.capacity())
                * std::mem::size_of::<(usize, usize)>()
            + (self.variants.capacity() + self.next_variants.capacity())
                * std::mem::size_of::<Variant>()
            + (self.tasks.capacity() + self.next_tasks.capacity()) * std::mem::size_of::<HeldTask>()
            + self
                .tasks
                .iter()
                .chain(&self.next_tasks)
                .filter_map(|task| task.modifiers.as_ref())
                .map(|entries| size_of::<Vec<[u8; 16]>>() + entries.capacity() * 16)
                .sum::<usize>()
    }
}

/// One task force's held terms: its handle and roster owner, the country that owner resolved to,
/// its doctrine contribution with the certificate (doctrine tree writer, country tree) it was read
/// under, its mine exposure with the immutable modifier array the hourly
/// task-force modifier rebuild built from it, and its location's weather terms with their
/// certificate (region writer, country tree) and the province's applied night factor.
#[derive(Clone)]
struct HeldTask {
    pointer: usize,
    handle: u64,
    roster_owner: u32,
    country: usize,
    /// The country's modifier-tree terms with its naval tree certificate, which the tree's writers
    /// move.
    country_terms: Option<((u64, u64), [i64; 4])>,
    doctrine: Option<HeldDoctrine>,
    mine_exposure: u64,
    modifiers: Option<Arc<Vec<[u8; 16]>>>,
    weather: Option<HeldWeather>,
    terms: Option<(Terms, u64)>,
}

/// A held doctrine contribution: the two certificates it was read under and
/// its four terms.
type HeldDoctrine = ([(u64, u64); 2], [i64; 4]);
/// Held location weather: the read terms, the two certificates it was read
/// under and the province night factor it was read with.
type HeldWeather = (cship::OrganisationWeather, [(u64, u64); 2], u64);

/// The task force a range is reading: its fields, its doctrine tree's
/// sequence taken before its first ship's read, its held terms, and whether
/// the hour's slice compares them instead of serving them.
struct TaskState {
    fields: cship::TaskForceFields,
    sequence: Option<(u64, u64)>,
    held: Option<HeldTask>,
    covered: bool,
    /// The held weather terms' province night factor, as read this hour.
    night_factor: Option<u64>,
}

/// A task force's doctrine certificate: its doctrine tree's sequence and its
/// country's modifier tree's.
fn doctrine_certificate(
    changes: &dyn Changes,
    sequence: Option<(u64, u64)>,
    country: usize,
) -> Option<[(u64, u64); 2]> {
    Some([sequence?, changes.tree_sequence(country)?])
}

/// A location's weather certificate: its weather region's writer's sequence
/// and its country's modifier tree's (the weather penalty).
fn weather_certificate(
    changes: &dyn Changes,
    weather: &cship::OrganisationWeather,
    country: usize,
) -> Option<[(u64, u64); 2]> {
    Some([
        changes.sequence(weather.region())?,
        changes.tree_sequence(country)?,
    ])
}

/// A ship's certificate receivers: itself, its medal store and its carrier
/// air base.
fn receivers(pointer: usize, fields: &cship::HeldShip) -> [usize; 3] {
    [pointer, fields.medal_store(), fields.carrier_air_base()]
}

pub struct Captured {
    worker_growth: bool,
    defer_owned: bool,
    pending_owned: bool,
    pending_hold: bool,
    pending_quiet: Option<(u64, u64)>,
    copied: Cell<(usize, usize)>,
    modifiers: RefCell<SnapshotArena>,
    required_modifiers: SnapshotCapacity,
    optional_refusals: [u64; OPTIONAL_REFUSAL_REASONS.len()],
    rows: CaptureRows<cship::ModelRow>,
    row_heap_bytes: OnceLock<usize>,
    seeds: Vec<Seed>,
    context: Option<CaptureContext>,
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
    /// This range's held reads; kept across hours, unlike everything else.
    store: ShipStore,
    air_capacity: crate::air_capacity::Store,
    naval_power: cship::NavalPowerCache,
}

impl Captured {
    /// Allocation-free placeholder while acquisition owns the original buffers.
    pub fn vacant() -> Self {
        Self::part(&Capacities {
            rows: 0,
            modifiers: SnapshotCapacity::default(),
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
            defer_owned: false,
            pending_owned: false,
            pending_hold: false,
            pending_quiet: None,
            optional_refusals: [0; OPTIONAL_REFUSAL_REASONS.len()],
            rows: CaptureRows::new(capacity.rows),
            row_heap_bytes: OnceLock::new(),
            seeds: owned_vec(filled(capacity.rows, Seed::default())),
            context: None,
            seed_len: 0,
            required_rows: 0,
            parent: super::MISSING,
            len: 0,
            missing: 0,
            failure: None,
            terminal_failure: None,
            doctrine: cship::OrganisationMemo::default(),
            model: cship::ModelMemo::default(),
            naval_power: cship::NavalPowerCache::default(),
            detail_len: 0,
            split: 1,
            part_len: 0,
            oversized: false,
            extra: Vec::new(),
            store: ShipStore::default(),
            air_capacity: crate::air_capacity::Store::default(),
        }
    }
    pub fn reset(&mut self) {
        if self.pending_owned {
            self.forget_stores();
            self.release_sources();
            self.pending_owned = false;
        }
        self.defer_owned = false;
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
        self.optional_refusals = [0; OPTIONAL_REFUSAL_REASONS.len()];
        self.seed_len = 0;
        self.context = None;
        self.required_rows = 0;
        self.parent = super::MISSING;
        self.len = 0;
        self.missing = 0;
        self.failure = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + *self.row_heap_bytes.get_or_init(|| {
                self.rows
                    .rows
                    .iter()
                    .map(|row| {
                        row.hull_sprite.allocated_bytes() + row.fallback_sprite.allocated_bytes()
                    })
                    .sum::<usize>()
            })
            + self.seeds.capacity() * std::mem::size_of::<Seed>()
            + self.modifiers.borrow().allocated_bytes()
            + self.model.allocated_bytes()
            + self.store.allocated_bytes()
            + self.air_capacity.allocated_bytes()
            + self.naval_power.allocated_bytes()
            + self
                .extra
                .iter()
                .map(Captured::allocated_bytes)
                .sum::<usize>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.rows, self.required_rows);
        self.rows.grow(&mut capacity.rows);
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
        // Parts follow the family's rows and their own snapshot demand, so
        // every range of the next split fits (`part_rows`).
        let rows = part_rows(capacity.rows);
        for part in &mut self.extra {
            let mut own = Capacities {
                rows: rows.max(part.rows.rows.len()),
                modifiers: part.required_modifiers,
            };
            part.grow(&mut own);
        }
    }
    /// Start the current map-owned ship census without reading model state.
    pub fn begin_sources(&mut self) {
        if self.pending_owned {
            self.forget_stores();
            self.release_sources();
            self.pending_owned = false;
        }
        self.defer_owned = false;
        self.worker_growth = false;
        self.terminal_failure = None;
        self.context = None;
        self.release_seed_pointers();
        self.seed_len = 0;
        self.detail_len = 0;
        self.split = 1;
        self.part_len = 0;
        self.oversized = false;
        self.missing = 0;
        self.parent = super::MISSING;
    }
    /// Retain the map's native source order; only acquisition follows seed pointers.
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
            .and_then(|required| {
                engine_native::walk::Buffer::Growing(&mut self.seeds).ensure(required)
            });
        if let Err(reason) = result {
            self.terminal_failure = Some(reason);
            return;
        }
        self.visit_source(pointer, source);
    }

    pub fn finish_sources(&mut self, parent: Outcome) {
        if let Err(reason) =
            engine_native::walk::Buffer::Growing(&mut self.rows.rows).ensure(self.seed_len)
        {
            self.terminal_failure = Some(reason);
        }
        let rows = part_rows(self.seed_len);
        for part in &mut self.extra {
            for result in [
                engine_native::walk::Buffer::Growing(&mut part.seeds).ensure(rows),
                engine_native::walk::Buffer::Growing(&mut part.rows.rows).ensure(rows),
            ] {
                if let Err(reason) = result {
                    self.terminal_failure = Some(reason);
                }
            }
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
    pub fn census_len(&self) -> usize {
        self.seed_len
    }

    pub fn set_context(&mut self, context: CaptureContext) {
        self.context = Some(context);
    }

    pub fn capture_context(&self, base: usize) -> CaptureContext {
        CaptureContext::new(base)
    }

    /// Split this hour's details into `parts` contiguous census ranges. This
    /// capture keeps the first; each further range is copied into its part
    /// capture. Parts are sized so every range fits (`part_rows`); a range
    /// that does not is a broken invariant and publishes the family
    /// unavailable, never a partial census. Allocation-free. Returns the parts.
    pub fn split_details(&mut self, parts: usize) -> usize {
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
    pub fn take_part(&mut self, index: usize) -> Captured {
        std::mem::replace(&mut self.extra[index], Captured::vacant())
    }

    /// Return a part capture after its job.
    pub fn restore_part(&mut self, index: usize, part: Captured) {
        self.extra[index] = part;
    }

    /// Join the parts' outcomes in census order after every part returned.
    /// The first failure keeps its census position; an interrupted part is
    /// terminal for the family, as an interrupted unsplit job is.
    pub fn finish_split(&mut self) {
        if self.split > 1 && self.terminal_failure.is_none() {
            let parts = &self.extra[..self.split - 1];
            if let Some(reason) = parts.iter().find_map(|part| part.terminal_failure) {
                self.terminal_failure = Some(reason);
            } else {
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
        }
        if self.terminal_failure.is_some() || !matches!(self.rows.outcome, Outcome::Rows { .. }) {
            self.forget_stores();
        }
    }

    fn forget_stores(&mut self) {
        self.pending_hold = false;
        self.pending_quiet = None;
        self.store.forget();
        self.air_capacity.clear();
        for part in &mut self.extra {
            part.forget_stores();
        }
    }

    fn begin_details(&mut self) {
        self.len = 0;
        self.failure = None;
        // Clear the fixed-fixture accounting for every part.
        self.copied.set((0, 0));
        self.modifiers.get_mut().clear();
        if self.worker_growth {
            self.modifiers.get_mut().enable_worker_growth();
        }
        self.optional_refusals = [0; OPTIONAL_REFUSAL_REASONS.len()];
        self.doctrine = cship::OrganisationMemo::default();
        // Keep the worker's memo storage; every observation and pointer key
        // of the previous capture is forgotten.
        self.model.reset();
        if self.worker_growth {
            self.model.enable_worker_growth();
        }
    }
    /// Before the first ship of a task force in the range: finish the
    /// previous task force, then serve this one's held terms that still hold
    /// (outside the hour's slice): its modifier array while its mine exposure
    /// is the held one, and its doctrine contribution while its doctrine
    /// tree writer and its country's tree certificate held still.
    fn begin_task(
        &mut self,
        memory: &impl Memory,
        task: cship::TaskForceFields,
        carrying: &mut Carrying<'_>,
    ) {
        self.finish_task(memory, carrying);
        let held =
            self.store.task(task.pointer).cloned().filter(|held| {
                held.handle == task.handle && held.roster_owner == task.roster_owner
            });
        // The doctrine tree's certificate precedes the read it certifies.
        let sequence = carrying.changes.sequence(task.pointer);
        let covered = carrying.check.covers(task.handle >> 32);
        if covered {
            // The slice compares this task force's held country terms with
            // a fresh read, not with another task force's held ones.
            self.doctrine.forget_country();
        }
        // The applied night factor of the held location, which the reader's
        // province terms follow.
        let night_factor = held
            .as_ref()
            .and_then(|held| held.weather)
            .and_then(|(weather, _, _)| engine_native::read::<u64>(memory, weather.night_factor()));
        carrying.task_modifiers = None;
        if let Some(held) = held.as_ref()
            && !covered
        {
            if let Some(entries) = held.modifiers.as_ref()
                && held.mine_exposure == task.mine_exposure
            {
                carrying.task_modifiers = Some((task.modifiers, Arc::clone(entries)));
                carrying.counts.task_terms_carried += 1;
            }
            if let Some((certificate, terms)) = held.country_terms
                && carrying.changes.tree_sequence(held.country) == Some(certificate)
            {
                // Served while no tree writer touched the country.
                self.doctrine.hold_country(held.country, terms);
                carrying.counts.task_terms_carried += 1;
            }
            if let Some((certificate, contribution)) = held.doctrine
                && doctrine_certificate(carrying.changes, sequence, held.country)
                    == Some(certificate)
            {
                self.doctrine.hold_doctrine(task.pointer, contribution);
                carrying.counts.task_terms_carried += 1;
            }
            // The location terms include the task force's modifier array.
            if let Some((weather, certificate, factor)) = held.weather
                && held.mine_exposure == task.mine_exposure
                && night_factor == Some(factor)
                && weather_certificate(carrying.changes, &weather, held.country)
                    == Some(certificate)
            {
                self.doctrine.hold_weather(weather);
                carrying.counts.task_terms_carried += 1;
            }
        }
        carrying.task = Some(TaskState {
            fields: task,
            sequence,
            held,
            covered,
            night_factor,
        });
    }

    /// After a task force's last ship in the range: compare its terms with
    /// the held ones when the slice covers it, and keep them for the next
    /// hour.
    fn finish_task(&mut self, memory: &impl Memory, carrying: &mut Carrying<'_>) {
        let Some(state) = carrying.task.take() else {
            return;
        };
        let task = state.fields;
        let Some(country) = self.doctrine.country() else {
            return;
        };
        let doctrine = self.doctrine.doctrine(task.pointer);
        let modifiers = self.modifiers.borrow();
        let observed = modifiers.observed(task.modifiers).or_else(|| {
            // A current failed observation wins over a carried array too.
            if modifiers.existing_at(task.modifiers, 1).is_some() {
                return None;
            }
            carrying
                .task_modifiers
                .as_ref()
                .filter(|(address, _)| *address == task.modifiers)
                .map(|(_, entries)| entries.as_slice())
        });
        let certificate = doctrine_certificate(carrying.changes, state.sequence, country);
        let country_terms = self
            .doctrine
            .country_terms()
            .filter(|&(observed, _)| observed == country)
            .and_then(|(_, terms)| Some((carrying.changes.tree_sequence(country)?, terms)));
        let weather = self
            .doctrine
            .weather()
            .filter(|weather| weather.task() == task.pointer)
            .and_then(|weather| {
                let factor = state
                    .night_factor
                    .filter(|_| {
                        state
                            .held
                            .as_ref()
                            .and_then(|held| held.weather)
                            .is_some_and(|(held, _, _)| {
                                held.night_factor() == weather.night_factor()
                            })
                    })
                    .or_else(|| engine_native::read::<u64>(memory, weather.night_factor()))?;
                let certificate = weather_certificate(carrying.changes, &weather, country)?;
                Some((weather, certificate, factor))
            });
        if let Some(held) = state.held.as_ref().filter(|_| state.covered) {
            let modifiers_differ = held.mine_exposure == task.mine_exposure
                && held.modifiers.as_ref().map(|entries| entries.as_slice()) != observed;
            let doctrine_differs = held.country == country
                && held.doctrine.is_some_and(|(held, value)| {
                    Some(held) == certificate && Some(value) != doctrine
                });
            let weather_differs = held.mine_exposure == task.mine_exposure
                && held.weather.zip(weather).is_some_and(
                    |((kept, kept_certificate, kept_factor), (fresh, certificate, factor))| {
                        kept.same_sources(&fresh)
                            && kept_certificate == certificate
                            && kept_factor == factor
                            && kept != fresh
                    },
                );
            let country_differs = held.country == country
                && held.country_terms.zip(country_terms).is_some_and(
                    |((kept_certificate, kept), (certificate, fresh))| {
                        kept_certificate == certificate && kept != fresh
                    },
                );
            carrying.counts.verified += 1;
            if modifiers_differ || doctrine_differs || weather_differs || country_differs {
                carrying.counts.mismatches += 1;
                carrying.changes.mismatch();
            }
        }
        let held = HeldTask {
            pointer: task.pointer,
            handle: task.handle,
            roster_owner: task.roster_owner,
            country,
            country_terms,
            doctrine: certificate.zip(doctrine),
            mine_exposure: task.mine_exposure,
            modifiers: state.held.as_ref().and_then(|held| held.modifiers.clone()),
            weather,
            terms: carrying
                .terms
                .filter(|(pointer, _)| *pointer == task.pointer)
                .map(|(_, terms)| (terms, carrying.terms_identity)),
        };
        if !self.store.keep_task(held, observed) {
            carrying.counts.uncertified += 1;
        }
        carrying.task_modifiers = None;
    }

    fn set_terms(&mut self, carrying: &mut Carrying<'_>, terms: Option<(usize, Terms)>) {
        carrying.terms_identity = terms
            .as_ref()
            .map_or(0, |(task, terms)| self.store.terms_identity(*task, terms));
        carrying.terms = terms;
    }

    /// A certified ship's task-force terms, read without touching its cold
    /// objects. The terms identity is compared once for this task force.
    fn read_terms(
        &mut self,
        memory: &impl Memory,
        base: usize,
        task_pointer: usize,
        fields: &mut cship::ShipFields,
        carrying: &mut Carrying<'_>,
    ) {
        if carrying.terms.as_ref().map(|(task, _)| *task) == Some(task_pointer) {
            return;
        }
        let Some(task) = fields.task_force_at(memory, task_pointer) else {
            return;
        };
        if carrying.task.as_ref().map(|state| state.fields.pointer) != Some(task.pointer) {
            self.begin_task(memory, task, carrying);
        }
        let gathered = fields.memory(memory);
        let read = ModifierMemory {
            inner: &gathered,
            arena: &self.modifiers,
            held_task: carrying
                .task_modifiers
                .as_ref()
                .map(|(address, entries)| (*address, entries.as_slice())),
        };
        let started = qpc();
        let terms = cship::read_task_terms(&read, base, &task, &mut self.doctrine)
            .ok()
            .flatten()
            .and_then(|terms| task_terms(carrying.changes, task.pointer, terms));
        self.set_terms(carrying, terms);
        carrying.counts.terms_reads += 1;
        carrying.counts.terms_ticks += qpc().saturating_sub(started).max(0) as u64;
    }

    /// Carry a row using only its compact record. Changed shared terms use
    /// the certified own organisation operands and the shared arithmetic;
    /// critical pointees and class contributions remain current-hour reads.
    #[allow(clippy::too_many_arguments)]
    fn carry_hot(
        &mut self,
        memory: &impl Memory,
        base: usize,
        source: &cship::Row,
        fields: &mut cship::ShipFields,
        carrying: &mut Carrying<'_>,
        hot: &mut Hot,
        held: &mut Held,
    ) -> bool {
        if hot.receivers[2] != 0 || hot.terms_identity == 0 {
            return false;
        }
        self.read_terms(memory, base, hot.task, fields, carrying);
        let Some((_, terms)) = carrying
            .terms
            .as_ref()
            .filter(|(task, _)| *task == hot.task)
        else {
            return false;
        };
        if !self.store.variants_unchanged(memory, fields, hot) {
            return false;
        }
        let same_terms = hot.terms_identity == carrying.terms_identity;
        let guard_matches = if same_terms {
            match cship::organisation_modifier_guard_matches(
                memory,
                base,
                hot.modifier_guard,
                &mut self.doctrine,
            ) {
                Ok(matches) => matches,
                Err(_) => {
                    carrying.guard_fallback = true;
                    engine_native::batch_fallback::record(
                        engine_native::batch_fallback::Batch::ShipModelGuards,
                    );
                    return false;
                }
            }
        } else {
            false
        };
        let revalidated_criticals = carrying.revalidated && hot.criticals;
        let organisation = if same_terms && guard_matches && !revalidated_criticals {
            None
        } else {
            if !model_carry_enabled() {
                return false;
            }
            let Some(own) = held.organisation else {
                return false;
            };
            #[cfg(any(test, feature = "test-support"))]
            carry_cost(|cost| {
                cost.cold_organisation_bytes += std::mem::size_of::<cship::OrganisationOperands>()
            });
            let read = ModifierMemory {
                inner: memory,
                arena: &self.modifiers,
                held_task: carrying
                    .task_modifiers
                    .as_ref()
                    .map(|(address, entries)| (*address, entries.as_slice())),
            };
            let organisation = match cship::read_maximum_organisation_from_terms(
                &read,
                base,
                hot.pointer,
                &own,
                &terms.0,
                &mut self.doctrine,
            ) {
                Ok(Some(organisation)) => organisation,
                Ok(None) => return false,
                Err(reason) => {
                    self.failure.get_or_insert(reason);
                    return true;
                }
            };
            if let Some((_, kept)) = &held.row {
                #[cfg(any(test, feature = "test-support"))]
                carry_cost(|cost| cost.cold_terms_bytes += std::mem::size_of::<Terms>());
                let bits = terms.0.differences(&kept.0);
                for (bit, count) in carrying.counts.terms_changed.iter_mut().take(7).enumerate() {
                    *count += u64::from(bits >> bit & 1);
                }
                carrying.counts.terms_changed[7] += u64::from(terms.1 != kept.1);
                carrying.counts.terms_changed[9] += u64::from(same_terms && !guard_matches);
                carrying.counts.terms_changed[10] += u64::from(revalidated_criticals);
            }
            Some(organisation)
        };
        let len = self.len;
        if hot.at == len
            && self
                .rows
                .rows
                .get(len)
                .is_some_and(|row| row.ship == hot.ship)
        {
            if let Some(organisation) = organisation {
                self.rows.rows[len].maximum_organisation = organisation;
            }
            self.record_kept(source.roster_ordinal);
        } else {
            #[cfg(any(test, feature = "test-support"))]
            carry_cost(|cost| cost.row_clones += 1);
            let mut row = held.row.as_ref().expect("a carried row").0.clone();
            row.roster_ordinal = source.roster_ordinal;
            if let Some(organisation) = organisation {
                row.maximum_organisation = organisation;
            }
            self.record(|_| Ok(row));
        }
        if self.len != len {
            hot.at = len;
            if let Some(organisation) = organisation {
                let (row, kept) = held.row.as_mut().expect("a carried row");
                row.maximum_organisation = organisation;
                *kept = *terms;
                held.at = len;
                held.terms_identity = carrying.terms_identity;
                held.organisation = self
                    .doctrine
                    .organisation_operands(hot.pointer)
                    .or(held.organisation);
                hot.terms_identity = carrying.terms_identity;
                hot.modifier_guard = held.organisation.map_or(0, |own| own.modifier_guard());
                carrying.counts.carried += 1;
                carrying.counts.models_carried += 1;
            } else {
                carrying.counts.rows_carried += 1;
            }
        }
        true
    }

    fn own_observations_differ(&self, held: &Held, read: &cship::HeldShip) -> bool {
        held.fields.differs(read)
            || held.accumulators.iter().zip(read.accumulators()).any(
                |((address, entries), fresh_address)| {
                    *address == fresh_address
                        && if fresh_address == 0 {
                            !entries.is_empty()
                        } else {
                            self.modifiers
                                .borrow()
                                .observed(fresh_address)
                                .is_some_and(|fresh| entries[..] != *fresh)
                        }
                },
            )
    }

    /// Read one ship, serving its own objects and modifier arrays from its
    /// held read when its handle, task force and certificate are that read's
    /// and `check` does not cover it, and keep the hour's read for the next.
    fn visit_detail(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: Seed,
        fields: &mut cship::ShipFields,
        carrying: &mut Carrying<'_>,
    ) {
        let Seed { pointer, row } = seed;
        if self.failure.is_some() {
            return;
        }
        carrying.guard_fallback = false;
        carrying.revalidated = false;
        let mut taken = carrying
            .store
            .then(|| self.store.take(pointer, self.len))
            .flatten()
            .filter(|(hot, _)| hot.ship == row.id && hot.task_force == row.task_force);
        // The certificate precedes every read of this hour.
        let observation = taken.as_ref().and_then(|(hot, _)| {
            if carrying
                .quiet
                .is_some_and(|quiet| carrying.changes.quiet() == Some(quiet))
            {
                Some((hot.certificate, hot.slots))
            } else {
                carrying.quiet = None;
                carrying.changes.certificate_at(hot.receivers, hot.slots)
            }
        });
        let original_hot = taken.as_ref().map(|(hot, _)| *hot);
        let certificate = observation.map(|(certificate, _)| certificate);
        let covered = carrying.check.covers(row.id >> 32);
        if !covered
            && let Some((hot, held)) = taken.as_mut()
            && certificate == Some(hot.certificate)
        {
            hot.slots = observation.expect("a certified read").1;
            let len = self.len;
            if self.carry_hot(memory, base, &row, fields, carrying, hot, held) {
                if self.len != len {
                    let (hot, held) = taken.take().expect("the carried read");
                    if !self.store.keep_hot(held, hot) {
                        carrying.counts.uncertified += 1;
                    }
                }
                return;
            }
        }
        let taken = taken.map(|(hot, mut held)| {
            held.slots = observation.map_or(hot.slots, |(_, slots)| slots);
            held.at = hot.at;
            #[cfg(any(test, feature = "test-support"))]
            carry_cost(|cost| cost.cold_ship_bytes += std::mem::size_of::<cship::HeldShip>());
            held
        });
        let (held, stale) = match taken {
            Some(held) if certificate == Some(held.certificate) => (Some(held), None),
            Some(held) if !covered && certificate.is_some() => (None, Some(held)),
            _ => (None, None),
        };
        // A read the hour's slice covers is compared with a fresh one; any
        // other certified read is carried.
        let (mut carry, held) = if covered { (None, held) } else { (held, None) };
        // A held read whose certificate moved (a hooked writer ran) is
        // compared with this hour's bytes of the ship's own objects and
        // modifier arrays, each read as the readers would read it: all equal,
        // the readers' inputs are the held read's, and it carries again under
        // the new certificate; with only the arrays changed, the model part
        // below still stands.
        let mut visited = false;
        let mut stale = stale;
        if let Some(mut moved) = stale.take() {
            fields.visit(memory, pointer);
            visited = true;
            let read = fields.held();
            if read.same(&moved.fields) {
                let gathered = fields.memory(memory);
                let mut arena = self.modifiers.borrow_mut();
                let arrays_held = moved.accumulators.iter().all(|(address, entries)| {
                    *address == 0 || arena.observe(&gathered, *address) == Some(&entries[..])
                });
                drop(arena);
                if arrays_held {
                    moved.certificate = certificate.expect("a moved certificate");
                    carrying.counts.revalidated += 1;
                    carrying.revalidated = true;
                    carry = Some(moved);
                } else {
                    stale = Some(moved);
                }
            } else if model_carry_enabled()
                && certificate.is_some_and(|certificate| {
                    certificate
                        .iter()
                        .zip(moved.certificate)
                        .all(|(fresh, kept)| fresh.0 == kept.0)
                })
                && read.same_model(&moved.fields, row.class_id)
            {
                // A changed organisation input does not invalidate equal model
                // inputs. Only the model may carry: organisation and arrays are
                // read afresh, and variants still compare below.
                stale = Some(moved);
            }
        }
        if visited
            && !carrying.guard_fallback
            && let Some(mut hot) = original_hot
            && let Some(held) = carry.as_mut()
        {
            hot.certificate = held.certificate;
            hot.slots = held.slots;
            let len = self.len;
            if self.carry_hot(memory, base, &row, fields, carrying, &mut hot, held) {
                if self.len != len
                    && !self
                        .store
                        .keep_hot(carry.take().expect("the revalidated read"), hot)
                {
                    carrying.counts.uncertified += 1;
                }
                return;
            }
        }
        // A task force's first ship in the range with a carryable row: read
        // the task force's terms without the ship, as its read would keep
        // them, so that its row can carry too.
        if carrying.store
            && let Some(held) = carry.as_ref()
            && held.row.is_some()
            && held.fields.carrier_air_base() == 0
            && carrying.terms.map(|(task, _)| task) != Some(held.fields.task_force())
            && let Some(task) = fields.task_force_of(memory, &held.fields)
        {
            if carrying.task.as_ref().map(|state| state.fields.pointer) != Some(task.pointer) {
                self.begin_task(memory, task, carrying);
            }
            let gathered = fields.memory(memory);
            let read = ModifierMemory {
                inner: &gathered,
                arena: &self.modifiers,
                held_task: carrying
                    .task_modifiers
                    .as_ref()
                    .map(|(address, entries)| (*address, entries.as_slice())),
            };
            let started = qpc();
            let terms = cship::read_task_terms(&read, base, &task, &mut self.doctrine)
                .ok()
                .flatten()
                .and_then(|terms| task_terms(carrying.changes, task.pointer, terms));
            self.set_terms(carrying, terms);
            carrying.counts.terms_reads += 1;
            carrying.counts.terms_ticks += qpc().saturating_sub(started).max(0) as u64;
        }
        // The finished row, while the ship's own inputs are certified (or
        // read equal), it has no carrier base, the task force's terms this
        // hour (read for an earlier ship of it) are its row's and its
        // variants' gathered bytes held: only the variants are gathered.
        let variants_and_terms = !carrying.guard_fallback
            && carry.as_ref().is_some_and(|held| {
                held.fields.carrier_air_base() == 0
                    && held.row.as_ref().is_some_and(|(row, terms)| {
                        model_carry_complete(row)
                            && carrying.terms.is_some_and(|(task, current)| {
                                task == held.fields.task_force() && current == *terms
                            })
                    })
                    && fields.held_variants_unchanged(memory, &held.fields)
            });
        let revalidated_criticals = carrying.revalidated
            && carry
                .as_ref()
                .is_some_and(|held| held.organisation.is_some_and(|own| own.has_criticals()));
        let row_carry = variants_and_terms
            && !revalidated_criticals
            && match carry.as_ref().and_then(|held| held.organisation) {
                Some(own) => match cship::organisation_modifier_guard_matches(
                    memory,
                    base,
                    own.modifier_guard(),
                    &mut self.doctrine,
                ) {
                    Ok(matches) => matches,
                    Err(_) => {
                        carrying.guard_fallback = true;
                        engine_native::batch_fallback::record(
                            engine_native::batch_fallback::Batch::ShipModelGuards,
                        );
                        false
                    }
                },
                None => false,
            };
        // The held row's model part (everything the model reader and its
        // suffixes made) stands while the ship's own inputs are certified (or
        // read equal), its variants' gathered bytes held and it has no
        // carrier base: those readers read nothing else but the session's
        // fixed content.
        // Why a carryable row did not carry (status only).
        if !row_carry
            && let Some(held) = carry.as_ref()
            && held.fields.carrier_air_base() == 0
            && let Some((_, kept)) = held.row.as_ref()
        {
            let reasons = &mut carrying.counts.terms_changed;
            reasons[10] += u64::from(revalidated_criticals);
            match carrying
                .terms
                .filter(|(task, _)| *task == held.fields.task_force())
            {
                None => reasons[8] += 1,
                Some((_, current)) if current == *kept => {
                    reasons[9] += u64::from(!revalidated_criticals || !variants_and_terms);
                }
                Some((_, (terms, leader))) => {
                    let bits = terms.differences(&kept.0);
                    for (bit, reason) in reasons.iter_mut().take(7).enumerate() {
                        *reason += u64::from(bits >> bit & 1);
                    }
                    reasons[7] += u64::from(leader != kept.1);
                }
            }
        }
        let model_carry = row_carry
            || carry.as_ref().is_some_and(|held| {
                model_carry_enabled()
                    && held.fields.carrier_air_base() == 0
                    && held
                        .row
                        .as_ref()
                        .is_some_and(|(row, _)| model_carry_complete(row))
                    && {
                        fields.visit_held(memory, &held.fields);
                        fields.same_variants(&held.fields)
                    }
            });
        let stale_model = stale.as_ref().is_some_and(|held| {
            model_carry_enabled()
                && held.fields.carrier_air_base() == 0
                && held
                    .row
                    .as_ref()
                    .is_some_and(|(row, _)| model_carry_complete(row))
                && fields.same_variants(&held.fields)
        });
        if row_carry && let Some(mut held) = carry.take() {
            let len = self.len;
            if held.at == len
                && self
                    .rows
                    .rows
                    .get(len)
                    .is_some_and(|kept| kept.ship == held.ship)
            {
                // This buffer still holds the row from the hour it held this
                // read: only the roster position is written.
                self.record_kept(row.roster_ordinal);
            } else {
                #[cfg(any(test, feature = "test-support"))]
                carry_cost(|cost| cost.row_clones += 1);
                let mut carried = held.row.as_ref().expect("a carried row").0.clone();
                carried.roster_ordinal = row.roster_ordinal;
                self.record(|_| Ok(carried));
            }
            if self.len != len {
                carrying.counts.rows_carried += 1;
                held.at = len;
                if !self.store.keep_held(held) {
                    carrying.counts.uncertified += 1;
                }
            }
            return;
        }
        // A fresh read's certificate precedes the read it certifies (a moved
        // read's was taken before its comparison).
        let ship = match &stale {
            Some(held) => certificate.map(|certificate| (certificate[0], held.slots[0])),
            None => (carry.is_none() && carrying.store)
                .then(|| carrying.changes.sequence_at(pointer, usize::MAX))
                .flatten(),
        };
        let kept = if model_carry {
            carry.as_ref()
        } else {
            stale.as_ref().filter(|_| stale_model)
        }
        .and_then(|held| held.row.as_ref())
        .map(|(row, _)| row);
        let len = self.len;
        self.read_detail(
            memory,
            base,
            pointer,
            &row,
            fields,
            carry.as_deref(),
            model_carry || visited,
            kept,
            carrying,
        );
        if !carrying.store {
            return;
        }
        let read = fields.held();
        if self.len == len {
            if let Some(held) = &held {
                carrying.counts.verified += 1;
                if self.own_observations_differ(held, &read) {
                    carrying.counts.mismatches += 1;
                    carrying.changes.mismatch();
                }
            }
            return;
        }
        // This task force's terms, once one of its ships was read.
        if let Some(task) = fields.task_force()
            && carrying.terms.map(|(pointer, _)| pointer) != Some(task.pointer)
        {
            let terms = self
                .doctrine
                .task_terms(&task)
                .and_then(|terms| task_terms(carrying.changes, task.pointer, terms));
            self.set_terms(carrying, terms);
        }
        let made = carrying
            .terms
            .filter(|&(task, _)| task == read.task_force())
            .map(|(_, terms)| {
                #[cfg(any(test, feature = "test-support"))]
                carry_cost(|cost| cost.row_clones += 1);
                (self.rows.rows[self.len - 1].clone(), terms)
            });
        if let Some(mut held) = carry {
            carrying.counts.carried += 1;
            carrying.counts.models_carried += u64::from(model_carry);
            held.fields = read;
            held.row = made;
            held.organisation = self.doctrine.organisation_operands(pointer);
            held.terms_identity = carrying.terms_identity;
            held.at = self.len - 1;
            if !self.store.keep_held(held) {
                carrying.counts.uncertified += 1;
            }
            return;
        }
        carrying.counts.read += 1;
        carrying.counts.models_carried += u64::from(stale_model);
        // The arrays this read observed, an absent medal store's empty.
        let modifiers = self.modifiers.borrow();
        let arrays = read.accumulators().map(|address| {
            if address == 0 {
                Some(&[][..])
            } else {
                modifiers.observed(address)
            }
        });
        if let Some(held) = &held {
            carrying.counts.verified += 1;
            // Compare independent observations wherever both reads succeeded.
            let fresh = &self.rows.rows[self.len - 1];
            let row_differs = held.row.as_ref().is_some_and(|(kept, kept_terms)| {
                let model_differs =
                    fields.same_variants(&held.fields) && model_observations_differ(kept, fresh);
                let organisation_differs = carrying
                    .terms
                    .is_some_and(|(task, terms)| task == read.task_force() && terms == *kept_terms)
                    && held.organisation.map(|own| own.modifier_guard())
                        == self
                            .doctrine
                            .organisation_operands(pointer)
                            .map(|own| own.modifier_guard())
                    && kept.maximum_organisation != fresh.maximum_organisation;
                held.fields.carrier_air_base() == 0 && (model_differs || organisation_differs)
            });
            let own_differs = self.own_observations_differ(held, &read);
            if own_differs || row_differs {
                carrying.counts.mismatches += 1;
                carrying.changes.mismatch();
            }
        }
        let [Some(experience), Some(medals)] = arrays else {
            carrying.counts.uncertified += 1;
            return;
        };
        let observation = ship.and_then(|(ship, slot)| {
            let [_, store, base] = receivers(pointer, &read);
            let (mut certificate, mut slots) = carrying
                .changes
                .certificate_at([0, store, base], NO_SLOTS)?;
            certificate[0] = ship;
            slots[0] = slot;
            Some((certificate, slots))
        });
        let at = self.len - 1;
        let kept = observation.is_some_and(|(certificate, slots)| {
            let [experience_at, medals_at] = read.accumulators();
            // A spare box keeps its arrays' storage.
            let boxed = match held.or(stale) {
                Some(mut spare) => {
                    spare.pointer = pointer;
                    spare.ship = row.id;
                    spare.task_force = row.task_force;
                    spare.certificate = certificate;
                    spare.slots = slots;
                    spare.fields = read;
                    spare.accumulators[0].0 = experience_at;
                    spare.accumulators[1].0 = medals_at;
                    spare.row = made;
                    spare.organisation = self.doctrine.organisation_operands(pointer);
                    spare.terms_identity = carrying.terms_identity;
                    spare.at = at;
                    Ok(spare)
                }
                None => engine_allocation::boxed(Held {
                    pointer,
                    ship: row.id,
                    task_force: row.task_force,
                    certificate,
                    slots,
                    fields: read,
                    accumulators: [(experience_at, Vec::new()), (medals_at, Vec::new())],
                    row: made,
                    organisation: self.doctrine.organisation_operands(pointer),
                    terms_identity: carrying.terms_identity,
                    at,
                }),
            };
            boxed.is_ok_and(|held| self.store.keep(held, [experience, medals]))
        });
        if !kept {
            carrying.counts.uncertified += 1;
        }
    }

    /// Read one ship's row; `held` serves its own objects and modifier
    /// arrays (`visited`: its fields are already in `fields`), and `kept`
    /// is a held row's model part, so that only the maximum organisation is
    /// computed.
    #[allow(clippy::too_many_arguments)] // The capture context, the held store and its certificates.
    fn read_detail(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        source: &cship::Row,
        fields: &mut cship::ShipFields,
        held: Option<&Held>,
        visited: bool,
        kept: Option<&cship::ModelRow>,
        carrying: &mut Carrying<'_>,
    ) {
        self.record(|this| {
            // The ship's fixed fields, and its task force's, location's and
            // the game singleton's when they change, each in one guarded
            // call ahead of the readers, which consult them in their own
            // order (`cship::ShipFields`). A carried ship's own objects and
            // modifier arrays are served from its held read instead.
            match held {
                Some(held) => {
                    if !visited {
                        fields.visit_held(memory, &held.fields);
                    }
                    let mut modifiers = this.modifiers.borrow_mut();
                    for (address, entries) in &held.accumulators {
                        if *address != 0 {
                            modifiers.hold(*address, entries);
                        }
                    }
                }
                None if !visited => fields.visit(memory, pointer),
                None => {}
            }
            if carrying.store
                && let Some(task) = fields.task_force()
                && carrying.task.as_ref().map(|state| state.fields.pointer) != Some(task.pointer)
            {
                this.begin_task(memory, task, carrying);
            }
            let gathered = fields.memory(memory);
            let suffix_memory = &gathered;
            let (mut row, selected) = {
                // Ship modifier follows the complete current ship accumulators;
                // the original native observations have no aggregate read cap.
                let memory = ModifierMemory {
                    inner: &gathered,
                    arena: &this.modifiers,
                    held_task: carrying
                        .task_modifiers
                        .as_ref()
                        .map(|(address, entries)| (*address, entries.as_slice())),
                };
                let doctrine = &mut this.doctrine;
                let (mut row, selected) = match kept {
                    Some(kept) => {
                        debug_assert!(model_carry_complete(kept));
                        #[cfg(any(test, feature = "test-support"))]
                        carry_cost(|cost| cost.row_clones += 1);
                        (
                            cship::ModelRow {
                                roster_ordinal: source.roster_ordinal,
                                ..kept.clone()
                            },
                            None,
                        )
                    }
                    None => cship::read_model_with_selection(
                        &memory,
                        base,
                        pointer,
                        source,
                        &mut this.model,
                    )?,
                };
                row.maximum_organisation =
                    cship::read_maximum_organisation_memo(&memory, base, pointer, doctrine)?;
                row.has_criticals = doctrine
                    .organisation_operands(pointer)
                    .map(|own| u32::from(own.has_criticals()));
                (row, selected)
            };
            if kept.is_none() {
                this.capture_model_suffixes(
                    suffix_memory,
                    base,
                    pointer,
                    source,
                    selected.as_ref(),
                    &mut row,
                );
            }
            row.air_capacity_kind =
                this.air_capacity
                    .capture(suffix_memory, base, pointer, &row, carrying.changes);
            if row.carrier_air_base != 0
                && row.formation_flags & 0x80 == 0
                && row.air_capacity_kind.is_none()
            {
                this.optional_refusals[13] += 1;
            }
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
        row.naval_power = if self.worker_growth {
            selected.and_then(|source| {
                cship::read_naval_power(memory, base, source, &mut self.naval_power)
            })
        } else {
            None
        };
        if self.worker_growth && selected.is_some() && row.naval_power.is_none() {
            self.optional_refusals[14] += 1;
        }
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
    /// Record a carried row this buffer already holds at the next position,
    /// as [`Self::record`] would record it, writing only its roster position.
    fn record_kept(&mut self, roster_ordinal: u32) {
        if self.failure.is_some() {
            return;
        }
        if self.len == self.rows.rows.len() {
            self.failure = Some("ship model seed/output capacity mismatch");
            return;
        }
        self.rows.rows[self.len].roster_ordinal = roster_ordinal;
        self.len += 1;
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
                self.row_heap_bytes.take();
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
        self.model.reset();
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
        // Drain the local attempts once, including terminal release paths.
        // No per-ship atomics and no false refusal for absent native modifiers.
        for (counter, refused) in OPTIONAL_REFUSALS
            .iter()
            .zip(std::mem::take(&mut self.optional_refusals))
        {
            if refused != 0 {
                counter.fetch_add(refused, std::sync::atomic::Ordering::Relaxed);
            }
        }
        self.release_seed_pointers();
    }
    /// Consume the already-owned map seeds once; preserve native detail/error order.
    pub fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        if self.pending_owned && !self.defer_owned {
            self.forget_stores();
            self.pending_owned = false;
        }
        self.capture_carrying(
            memory,
            context,
            &ship_model_changes::Observer,
            crate::sampled_verify::current_burned_in(ship_model_changes::VERIFY),
        )
    }

    /// Native demand and certificate checks finish here; the frame retains
    /// the next store and scratch until FIFO settlement.
    pub fn capture_deferred(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        if self.pending_owned {
            self.forget_stores();
        }
        self.pending_owned = true;
        self.defer_owned = true;
        let report = self.capture_owned(memory, context);
        self.defer_owned = false;
        report
    }

    /// `modifiers`, `model`, `store`, `air_capacity` and `extra` settlement storage.
    pub fn owned_tail_allocated_bytes(&self) -> usize {
        self.modifiers.borrow().allocated_bytes()
            + self.model.allocated_bytes()
            + self.store.allocated_bytes()
            + self.air_capacity.allocated_bytes()
            + self
                .extra
                .iter()
                .map(Self::owned_tail_allocated_bytes)
                .sum::<usize>()
    }

    /// Only owned store indexes, memo storage and seed identity tokens remain.
    pub fn settle_owned(&mut self) {
        if self.pending_owned {
            if self.pending_hold && self.store.hold() {
                self.store.quiet = self.pending_quiet;
            }
            self.release_sources();
            self.doctrine = cship::OrganisationMemo::default();
            if crate::allocation::failed() {
                self.forget_stores();
            }
            self.pending_hold = false;
            self.pending_quiet = None;
            self.pending_owned = false;
        }
        for part in &mut self.extra {
            part.settle_owned();
        }
    }

    /// [`Self::capture_owned`] observing `changes`, comparing the carryable
    /// ships `check` covers.
    fn capture_carrying(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        changes: &dyn Changes,
        check: Check,
    ) -> CaptureReport {
        self.context = Some(context);
        let power_certificate = self.worker_growth.then(naval_power_certificate).flatten();
        if self.naval_power.prepare(power_certificate) {
            self.store.forget();
        }
        self.air_capacity.begin(self.detail_len, self.worker_growth);
        // One range's gathered objects; nothing outlives this capture.
        let mut fields = cship::ShipFields::new(context.base);
        if power_certificate.is_some() {
            fields.enable_naval_power();
        }
        // Taken before the first registration/read, so an unchanged token
        // proves every previously held receiver's certificate without probing.
        let quiet = changes.quiet();
        let carrying = RefCell::new(Carrying {
            store: ship_model_changes::CARRY && self.terminal_failure.is_none(),
            changes,
            check,
            counts: Counts::default(),
            task: None,
            task_modifiers: None,
            terms: None,
            terms_identity: 0,
            quiet: quiet.filter(|quiet| Some(*quiet) == self.store.quiet),
            guard_fallback: false,
            revalidated: false,
        });
        self.store.next.clear();
        self.store.next_hot.clear();
        self.store.next_tasks.clear();
        self.store.next_variants.clear();
        self.store.next_variant_index.clear();
        for variant in &mut self.store.variants {
            variant.unchanged = None;
            variant.next = None;
        }
        #[cfg(any(test, feature = "test-support"))]
        CARRY_COST.set(CarryCost::default());
        let report = self.capture_with(
            |this, seed| {
                let carrying = &mut carrying.borrow_mut();
                // Every PATH_SAMPLE-th row is timed by the path it took.
                let sampled = this.len.is_multiple_of(ship_model_changes::PATH_SAMPLE);
                let before = [
                    carrying.counts.rows_carried,
                    carrying.counts.models_carried,
                    carrying.counts.carried,
                    carrying.counts.read,
                    carrying.counts.revalidated,
                ];
                let started = if sampled { qpc() } else { 0 };
                this.visit_detail(memory, context.base, seed, &mut fields, carrying);
                if sampled {
                    let ticks = qpc().saturating_sub(started).max(0) as u64;
                    let counts = &mut carrying.counts;
                    let after = [
                        counts.rows_carried,
                        counts.models_carried,
                        counts.carried,
                        counts.read,
                        counts.revalidated,
                    ];
                    let path = if after[0] != before[0] {
                        Some(0)
                    } else if after[1] != before[1] {
                        Some(1)
                    } else if after[2] != before[2] {
                        Some(2)
                    } else if after[3] != before[3] {
                        Some(3)
                    } else {
                        None
                    };
                    for path in path.into_iter().chain((after[4] != before[4]).then_some(4)) {
                        counts.path_samples[path] += 1;
                        counts.path_ticks[path] += ticks;
                    }
                }
            },
            // The range's last task force, while its observations last.
            |this| {
                if this.failure.is_none() {
                    this.finish_task(memory, &mut carrying.borrow_mut());
                }
            },
        );
        if self.worker_growth && naval_power_certificate() != power_certificate {
            for row in &mut self.rows.rows[..self.len] {
                self.optional_refusals[14] += u64::from(row.naval_power.take().is_some());
            }
            self.naval_power.prepare(None);
            self.store.forget();
        }
        let mut carrying = carrying.into_inner();
        let complete =
            self.terminal_failure.is_none() && matches!(self.rows.outcome, Outcome::Rows { .. });
        if complete {
            self.air_capacity.finish();
            if self.defer_owned {
                self.pending_hold = true;
                self.pending_quiet = quiet;
            } else if self.store.hold() {
                self.store.quiet = quiet;
            }
        } else {
            self.forget_stores();
        }
        if carrying.store {
            // Every census ship's receivers: itself, its medal store, its
            // carrier base and its task force. The family capture's census is
            // the whole one.
            carrying.counts.receivers = self
                .seed_len
                .saturating_mul(4)
                .saturating_add(self.air_capacity.len());
            ship_model_changes::record(&carrying.counts, complete);
            if complete {
                // Ships of a task force are consecutive in the census.
                let seeds = &self.seeds[..self.seed_len.min(self.seeds.len())];
                let task_forces = seeds
                    .iter()
                    .enumerate()
                    .filter(|&(at, seed)| {
                        at == 0 || seeds[at - 1].row.task_force != seed.row.task_force
                    })
                    .count();
                crate::naval_tree_changes::ship_models(task_forces);
            }
        }
        report
    }

    /// Capture this range's details in census order, reading each ship with
    /// `visit` and then running `finish` before the range's observations
    /// expire, and publish its outcome (the unsplit table, or one range of a
    /// split for `finish_split`).
    fn capture_with(
        &mut self,
        mut visit: impl FnMut(&mut Self, Seed),
        finish: impl FnOnce(&mut Self),
    ) -> CaptureReport {
        if self.terminal_failure.is_some() {
            self.forget_stores();
            self.release_sources();
            return CaptureReport::default();
        }
        let _reads = crate::read_accounting::table(TableId::ShipModels);
        let started = qpc();
        self.begin_details();
        for index in 0..self.detail_len.min(self.seed_len) {
            visit(self, self.seeds[index]);
        }
        finish(self);
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
        if !self.defer_owned {
            self.release_sources();
        }
        CaptureReport {
            ticks: [qpc().saturating_sub(started).max(0) as u64],
        }
    }
    /// Accepted panic or stop is terminal; preserve observed demand without retrying.
    pub fn fail_owned_capture(&mut self, reason: &'static str) -> CaptureReport {
        self.terminal_failure = Some(reason);
        self.forget_stores();
        self.release_sources();
        self.pending_owned = false;
        self.defer_owned = false;
        CaptureReport::default()
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self
            .terminal_failure
            .map_or(self.rows.outcome, |reason| Outcome::Unavailable { reason })]
    }
    pub fn has_terminal_failure(&self) -> bool {
        self.terminal_failure.is_some()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection disabled"),
        })
    }
    pub fn table_inputs(&self, projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
        // A split census's rows live in its parts; join them in census order
        // here, on the spool thread.
        let joined;
        let rows = if self.split > 1 && self.terminal_failure.is_none() {
            let parts = &self.extra[..self.split - 1];
            let mut rows = Vec::new();
            if reserve(&mut rows, self.len).is_err() {
                return [
                    crate::table_conversion::unavailable("collector allocation failed").into(),
                ];
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
                    && replay_data::rows::Recorded::Value(row.has_criticals)
                        == previous.has_criticals
                    && replay_data::rows::Recorded::Value(row.air_capacity_kind)
                        == previous.air_capacity_kind
                    && previous.subunit_index
                        == replay_data::rows::Recorded::Value(row.subunit_index)
                    && previous.naval_power == replay_data::rows::Recorded::Value(row.naval_power)
                    && previous.hull_variant
                        == replay_data::rows::Recorded::Value(
                            row.hull_variant.map(hull_variant_inputs),
                        )
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
                has_criticals: r.has_criticals.into(),
                air_capacity_kind: r.air_capacity_kind.into(),
                subunit_index: replay_data::rows::Recorded::Value(r.subunit_index),
                strike: replay_data::rows::Recorded::Value(replay_data::rows::Retired),
                naval_power: replay_data::rows::Recorded::Value(r.naval_power),
                hull_variant: replay_data::rows::Recorded::Value(
                    r.hull_variant.map(hull_variant_inputs),
                ),
                counter: Some(replay_data::rows::ShipCounterInputs {
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

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Benchmarks: compare with the readers run for every ship whose row is
    /// not carried whole.
    pub static MODEL_CARRY_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn model_carry_enabled() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    return !MODEL_CARRY_OFF.get();
    #[cfg(not(any(test, feature = "test-support")))]
    true
}

/// A task force's terms with its fleet leader's tree certificate, `None`
/// when the leader cannot be observed.
fn task_terms(
    changes: &dyn Changes,
    task: usize,
    terms: cship::TaskTerms,
) -> Option<(usize, Terms)> {
    let leader = match terms.leader() {
        Some(leader) => Some(changes.tree_sequence(leader)?),
        None => None,
    };
    Some((task, (terms, leader)))
}

/// A range's carry state: whether it holds reads at all, the observation
/// domain, the hour's sampled check, its counts and the task force it is
/// reading.
struct Carrying<'a> {
    store: bool,
    changes: &'a dyn Changes,
    check: Check,
    counts: Counts,
    task: Option<TaskState>,
    task_modifiers: Option<(usize, Arc<Vec<[u8; 16]>>)>,
    /// The terms of the task force the range is reading, once one of its
    /// ships was read.
    terms: Option<(usize, Terms)>,
    terms_identity: u64,
    quiet: Option<(u64, u64)>,
    guard_fallback: bool,
    revalidated: bool,
}

fn hull_variant_inputs(value: cship::HullVariantInputs) -> replay_data::rows::HullVariantInputs {
    replay_data::rows::HullVariantInputs {
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

/// Complete equipment-reload and world certificate; no native reads.
fn naval_power_certificate() -> Option<(u64, u64, u64)> {
    use collector_model_inputs::local_subunit_changes::Changes;
    let world = collector_lifetime::world_lifetime::token()?;
    let (epoch, sequence) = collector_model_inputs::local_subunit_changes::Observer.quiet()?;
    Some((world, epoch, sequence))
}
