use crate::{
    clock::qpc,
    projection::{Projection, Witness, held_input, project_table},
    ship_model_changes::{self, Certificate, Changes, Counts, NO_SLOTS, Slots},
    table_capture::{CaptureRows, grow_capacity},
};
use engine_allocation::{copy_vec, filled, owned_vec, reserve, resize_default};
use engine_native::{
    Memory, cship, memory::RecordLayout, memory::RecordSources,
    modifier_accumulator::SnapshotArena, modifier_accumulator::SnapshotCapacity,
    modifier_accumulator::Value, walk::Outcome,
};
use std::{
    cell::RefCell,
    sync::{Arc, OnceLock},
};

pub const MAX_PARTS: usize = 4;
const SHIPS_PER_PART: usize = 512;

pub fn parts_for(ships: usize) -> usize {
    ships.div_ceil(SHIPS_PER_PART).clamp(1, MAX_PARTS)
}

fn part_rows(rows: usize) -> usize {
    rows.min(SHIPS_PER_PART).max(rows.div_ceil(MAX_PARTS))
}

fn grow_snapshot_capacity(capacity: &mut SnapshotCapacity, required: SnapshotCapacity) {
    grow_capacity(&mut capacity.nodes, required.nodes);
    grow_capacity(&mut capacity.entries, required.entries);
}

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
        Some(
            arena
                .capture_at(self.inner, address, key)
                .and_then(|observation| arena.value(observation)),
        )
    }
}
use replay_data::{rows::ShipModelRow, rows::TableId, rows::tables};
use replay_frame::frame::TableInput;
#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ShipModels];

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

#[derive(Clone, Copy)]
pub struct CaptureContext {
    pub base: usize,
}

impl CaptureContext {
    pub fn new(base: usize) -> Self {
        Self { base }
    }
}

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

collector_primitives::capacity_max!(Capacities { rows, modifiers { nodes, entries } });

#[derive(Clone)]
struct Held {
    pointer: usize,
    ship: u64,
    task_force: u64,
    certificate: Certificate,
    slots: Slots,
    fields: cship::HeldShip,
    accumulators: [(usize, Vec<[u8; 16]>); 2],
    row: Option<(cship::ModelRow, Terms)>,
    organisation: Option<cship::OrganisationOperands>,
    terms_identity: u64,
    row_version: u64,
}

type Terms = (cship::TaskTerms, Option<(u64, u64)>);

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
    roster_ordinal: u32,
    row_version: u64,
}

struct Variant {
    fields: cship::HeldVariant,
    certificate: Option<(u64, u64)>,
    unchanged: Option<bool>,
    next: Option<usize>,
    next_same_ptr: usize,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Default, Debug)]
struct CarryCost {
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
    static CARRY_COST: std::cell::Cell<CarryCost> = const { std::cell::Cell::new(CarryCost {
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
    next_variant_head: std::collections::HashMap<usize, usize>,
    quiet: Option<(u64, u64)>,
    variant_quiet: Option<(u64, u64)>,
    naval_power: Option<(u64, u64, u64)>,
    generation: u64,
    rows: Option<u64>,
}

impl ShipStore {
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
        self.remember_variant_with_cert(fields, None)
    }

    fn remember_variant_with_cert(
        &mut self,
        fields: cship::HeldVariant,
        certificate: Option<(u64, u64)>,
    ) -> Option<usize> {
        let pointer = fields.pointer();
        if pointer == 0 {
            return Some(usize::MAX);
        }
        let prev_head = self
            .next_variant_head
            .get(&pointer)
            .copied()
            .unwrap_or(usize::MAX);
        let mut cursor = prev_head;
        while cursor != usize::MAX {
            if fields.same(&self.next_variants[cursor].fields) {
                if self.next_variants[cursor].certificate.is_none() {
                    self.next_variants[cursor].certificate = certificate;
                }
                return Some(cursor);
            }
            cursor = self.next_variants[cursor].next_same_ptr;
        }
        if reserve(&mut self.next_variants, 1).is_err()
            || engine_allocation::reserve_map(&mut self.next_variant_head, 1).is_err()
        {
            return None;
        }
        let index = self.next_variants.len();
        self.next_variants.push(Variant {
            fields,
            certificate,
            unchanged: None,
            next: None,
            next_same_ptr: prev_head,
        });
        self.next_variant_head.insert(pointer, index);
        Some(index)
    }

    fn certified(
        &mut self,
        changes: &dyn Changes,
        variant_quiet: &mut Option<(u64, u64)>,
        old: usize,
        counts: &mut Counts,
    ) -> bool {
        let variant = &self.variants[old];
        if variant.unchanged == Some(true) {
            return true;
        }
        let certified = if variant.unchanged.is_none() && variant.certificate.is_some() {
            if variant_quiet.is_some_and(|quiet| changes.variant_quiet() == Some(quiet)) {
                true
            } else {
                *variant_quiet = None;
                changes.variant_sequence(variant.fields.pointer()) == variant.certificate
            }
        } else {
            false
        };
        if certified {
            self.variants[old].unchanged = Some(true);
            counts.variants_carried += 1;
        }
        certified
    }

    fn variants_certified(
        &mut self,
        changes: &dyn Changes,
        variant_quiet: &mut Option<(u64, u64)>,
        hot: &Hot,
        counts: &mut Counts,
    ) -> bool {
        hot.variants
            .iter()
            .all(|&old| old == usize::MAX || self.certified(changes, variant_quiet, old, counts))
    }

    fn certify_variants(
        &mut self,
        changes: &dyn Changes,
        quiet: Option<(u64, u64)>,
        quiet_held: bool,
    ) {
        let reuse = quiet_held && quiet.is_some() && changes.variant_quiet() == quiet;
        for variant in &mut self.next_variants {
            if reuse && variant.certificate.is_some() {
                continue;
            }
            variant.certificate = variant
                .fields
                .complete()
                .then(|| changes.variant_sequence(variant.fields.pointer()))
                .flatten();
        }
        if quiet.is_none() || changes.variant_quiet() != quiet {
            for variant in &mut self.next_variants {
                variant.certificate = None;
            }
            self.variant_quiet = None;
        } else {
            self.variant_quiet = quiet;
        }
    }

    fn variants_unchanged(
        &mut self,
        memory: &impl Memory,
        fields: &mut cship::ShipFields,
        hot: &mut Hot,
        changes: &dyn Changes,
        variant_quiet: &mut Option<(u64, u64)>,
        counts: &mut Counts,
    ) -> bool {
        for index in &mut hot.variants {
            if *index == usize::MAX {
                continue;
            }
            let old = *index;
            let known = self.variants[old].unchanged;
            let unchanged = match known {
                Some(unchanged) => unchanged,
                None if self.certified(changes, variant_quiet, old, counts) => true,
                None => {
                    #[cfg(any(test, feature = "test-support"))]
                    carry_cost(|cost| cost.variant_compares += 1);
                    counts.variants_read += 1;
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
                    let carried_cert = variant_quiet.and(self.variants[old].certificate);
                    let Some(next) = self.remember_variant_with_cert(
                        self.variants[old].fields,
                        carried_cert,
                    ) else {
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
            roster_ordinal: held.row.as_ref().map_or(0, |(row, _)| row.roster_ordinal),
            row_version: held.row_version,
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

    fn task_index(&self, pointer: usize) -> Option<usize> {
        self.tasks
            .binary_search_by_key(&pointer, |held| held.pointer)
            .ok()
    }

    fn task(&self, pointer: usize) -> Option<&HeldTask> {
        self.task_index(pointer).map(|at| &self.tasks[at])
    }

    fn keep_task_direct(&mut self, held: HeldTask) -> bool {
        if reserve(&mut self.next_tasks, 1).is_err() {
            return false;
        }
        self.next_tasks.push(held);
        true
    }

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
        self.next_variant_head.clear();
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
        self.next_variant_head.clear();
        self.quiet = None;
        self.variant_quiet = None;
        self.naval_power = None;
        self.generation = 0;
        self.rows = None;
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
            + (self.index.capacity() + self.next_variant_head.capacity())
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

#[derive(Clone)]
struct HeldTask {
    pointer: usize,
    handle: u64,
    roster_owner: u32,
    country: usize,
    country_terms: Option<((u64, u64), [i64; 4])>,
    doctrine: Option<HeldDoctrine>,
    mine_exposure: u64,
    modifiers: Option<Arc<Vec<[u8; 16]>>>,
    weather: Option<HeldWeather>,
    location: Option<(u64, usize, usize, usize)>,
    fleet: Option<HeldFleet>,
    terms: Option<(Terms, u64)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct HeldFleet {
    fleet: usize,
    leader: Option<usize>,
    terms: [i64; 4],
    certificate: FleetCertificate,
}

type FleetCertificate = ((u64, u64), Option<(u64, u64)>);

fn fleet_certificate(
    changes: &dyn Changes,
    sequence: Option<(u64, u64)>,
    leader: Option<usize>,
) -> Option<FleetCertificate> {
    let tree = match leader {
        Some(leader) => Some(changes.tree_sequence(leader)?),
        None => None,
    };
    Some((sequence?, tree))
}

type HeldDoctrine = ([(u64, u64); 2], [i64; 4]);
type HeldWeather = (cship::OrganisationWeather, [(u64, u64); 2], u64);

struct TaskState {
    fields: cship::TaskForceFields,
    sequence: Option<(u64, u64)>,
    fleet_sequence: Option<(u64, u64)>,
    held: Option<usize>,
    night_factor: Option<u64>,
    all_carried: bool,
}

fn doctrine_certificate(
    changes: &dyn Changes,
    sequence: Option<(u64, u64)>,
    country: usize,
) -> Option<[(u64, u64); 2]> {
    Some([sequence?, changes.tree_sequence(country)?])
}

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

fn receivers(pointer: usize, fields: &cship::HeldShip) -> [usize; 3] {
    [pointer, fields.medal_store(), fields.carrier_air_base()]
}

pub struct Captured {
    defer_owned: bool,
    pending_owned: bool,
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
    detail_len: usize,
    split: usize,
    part_len: usize,
    oversized: bool,
    extra: Vec<Captured>,
    store: ShipStore,
    air_capacity: crate::air_capacity::Store,
    naval_power: cship::NavalPowerCache,
    range: usize,
    shared: bool,
    rows_serial: u64,
    row_versions: Vec<u64>,
    unchanged_rows: usize,
    witness: Option<u64>,
}

#[derive(Default)]
struct Shared {
    store: ShipStore,
    air_capacity: crate::air_capacity::Store,
}

type SharedSlots = [std::sync::Mutex<Option<Shared>>; MAX_PARTS];

#[cfg(not(test))]
static SHARED: SharedSlots = [const { std::sync::Mutex::new(None) }; MAX_PARTS];
#[cfg(test)]
thread_local! {
    static SHARED: SharedSlots = const { [const { std::sync::Mutex::new(None) }; MAX_PARTS] };
}

fn shared_slot<R>(range: usize, visit: impl FnOnce(&mut Option<Shared>) -> R) -> R {
    let with = |slots: &SharedSlots| {
        visit(
            &mut slots[range]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    };
    #[cfg(not(test))]
    return with(&SHARED);
    #[cfg(test)]
    SHARED.with(with)
}

static SERIALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl Captured {
    pub fn vacant() -> Self {
        Self::part(&Capacities {
            rows: 0,
            modifiers: SnapshotCapacity::default(),
        })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        let share = Capacities {
            rows: part_rows(capacity.rows),
            modifiers: SnapshotCapacity {
                nodes: capacity.modifiers.nodes.div_ceil(2),
                entries: capacity.modifiers.entries.div_ceil(2),
            },
        };
        Self {
            extra: owned_vec(engine_allocation::collect_vec((1..MAX_PARTS).map(
                |range| Self {
                    range,
                    ..Self::part(&share)
                },
            ))),
            ..Self::part(capacity)
        }
    }
    fn part(capacity: &Capacities) -> Self {
        Self {
            modifiers: RefCell::new(
                SnapshotArena::try_new(capacity.modifiers)
                    .map_err(crate::allocation::io)
                    .unwrap_or_else(|_| SnapshotArena::vacant()),
            ),
            required_modifiers: SnapshotCapacity::default(),
            defer_owned: false,
            pending_owned: false,
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
            range: 0,
            shared: false,
            rows_serial: 0,
            row_versions: owned_vec(filled(capacity.rows, 0)),
            unchanged_rows: 0,
            witness: None,
        }
    }
    pub fn reset(&mut self) {
        if self.pending_owned {
            self.forget_stores();
            self.release_sources();
            self.pending_owned = false;
        }
        self.witness = None;
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
            + self.row_versions.capacity() * std::mem::size_of::<u64>()
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
        if self.row_versions.len() < capacity.rows
            && resize_default(&mut self.row_versions, capacity.rows).is_err()
        {
            return;
        }
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
        let rows = part_rows(capacity.rows);
        for part in &mut self.extra {
            let mut own = Capacities {
                rows: rows.max(part.rows.rows.len()),
                modifiers: part.required_modifiers,
            };
            part.grow(&mut own);
        }
    }
    pub fn begin_sources(&mut self) {
        if self.pending_owned {
            self.forget_stores();
            self.release_sources();
            self.pending_owned = false;
        }
        self.witness = None;
        self.defer_owned = false;
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
    pub fn visit_source(&mut self, pointer: usize, source: &cship::Row) {
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
        self.seeds[self.seed_len] = Seed {
            pointer,
            row: *source,
        };
        self.seed_len += 1;
    }

    pub fn finish_sources(&mut self, parent: Outcome) {
        if let Err(reason) =
            engine_native::walk::Buffer::Growing(&mut self.rows.rows).ensure(self.seed_len)
        {
            self.terminal_failure = Some(reason);
        }
        if self.row_versions.len() < self.rows.rows.len()
            && resize_default(&mut self.row_versions, self.rows.rows.len()).is_err()
        {
            self.terminal_failure = Some("ship model version allocation failed");
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
            if part.row_versions.len() < part.rows.rows.len()
                && resize_default(&mut part.row_versions, part.rows.rows.len()).is_err()
            {
                self.terminal_failure = Some("ship model version allocation failed");
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

    pub fn census_len(&self) -> usize {
        self.seed_len
    }

    pub fn set_context(&mut self, context: CaptureContext) {
        self.context = Some(context);
    }

    pub fn capture_context(&self, base: usize) -> CaptureContext {
        CaptureContext::new(base)
    }

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

    pub fn take_part(&mut self, index: usize) -> Captured {
        std::mem::replace(&mut self.extra[index], Captured::vacant())
    }

    pub fn restore_part(&mut self, index: usize, part: Captured) {
        self.extra[index] = part;
    }

    pub fn finish_split(&mut self) {
        if self.split > 1 && self.terminal_failure.is_none() {
            let parts = &self.extra[..self.split - 1];
            if let Some(reason) = parts.iter().find_map(|part| part.terminal_failure) {
                self.terminal_failure = Some(reason);
            } else {
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
        self.store.forget();
        self.air_capacity.clear();
        shared_slot(self.range, |slot| *slot = None);
        for part in &mut self.extra {
            part.forget_stores();
        }
    }

    fn begin_details(&mut self) {
        self.len = 0;
        self.unchanged_rows = 0;
        self.failure = None;
        self.modifiers.get_mut().clear();
        self.optional_refusals = [0; OPTIONAL_REFUSAL_REASONS.len()];
        self.doctrine.reset_hour();
        self.model.reset();
    }
    fn begin_task(
        &mut self,
        memory: &impl Memory,
        task: cship::TaskForceFields,
        carrying: &mut Carrying<'_>,
    ) {
        self.finish_task(memory, carrying);
        let held_index = self.store.task_index(task.pointer).filter(|&at| {
            let held = &self.store.tasks[at];
            held.handle == task.handle && held.roster_owner == task.roster_owner
        });
        let held = held_index.map(|at| &self.store.tasks[at]);
        let sequence = carrying.changes.sequence(task.pointer);
        let fleet_sequence = (task.fleet != 0)
            .then(|| carrying.changes.sequence(task.fleet))
            .flatten();
        let night_factor = held
            .and_then(|held| held.weather)
            .and_then(|(weather, _, _)| engine_native::read::<u64>(memory, weather.night_factor()));
        carrying.task_modifiers = None;
        let mut modifiers_ok = false;
        let mut country_ok = false;
        let mut doctrine_ok = false;
        let mut weather_ok = task.location == 0;
        let mut location_ok = task.location == 0;
        let mut fleet_ok = false;
        if let Some(held) = held {
            if let Some(entries) = held.modifiers.as_ref()
                && held.mine_exposure == task.mine_exposure
            {
                carrying.task_modifiers = Some((task.modifiers, Arc::clone(entries)));
                carrying.counts.task_terms_carried += 1;
                modifiers_ok = true;
            }
            if let Some((certificate, terms)) = held.country_terms
                && carrying.changes.tree_sequence(held.country) == Some(certificate)
            {
                self.doctrine.hold_country(held.country, terms);
                carrying.counts.task_terms_carried += 1;
                country_ok = true;
            }
            if let Some((certificate, contribution)) = held.doctrine
                && doctrine_certificate(carrying.changes, sequence, held.country)
                    == Some(certificate)
            {
                self.doctrine.hold_doctrine(task.pointer, contribution);
                carrying.counts.task_terms_carried += 1;
                doctrine_ok = true;
            }
            if let Some((weather, certificate, factor)) = held.weather
                && held.mine_exposure == task.mine_exposure
                && night_factor == Some(factor)
                && weather_certificate(carrying.changes, &weather, held.country)
                    == Some(certificate)
            {
                self.doctrine.hold_weather(weather);
                carrying.counts.task_terms_carried += 1;
                weather_ok = task.location != 0;
            }
            if let Some((epoch, location, definition, terrain)) = held.location
                && location == task.location
                && sequence.is_some_and(|(current, _)| current == epoch)
            {
                self.doctrine.hold_location(location, definition, terrain);
                carrying.counts.task_terms_carried += 1;
                location_ok = task.location != 0;
            }
            if let Some(fleet) = held.fleet
                && fleet.fleet == task.fleet
                && fleet_certificate(carrying.changes, fleet_sequence, fleet.leader)
                    == Some(fleet.certificate)
            {
                self.doctrine
                    .hold_fleet(fleet.fleet, fleet.leader, fleet.terms);
                carrying.counts.task_terms_carried += 1;
                fleet_ok = true;
            }
        }
        let all_carried = modifiers_ok
            && country_ok
            && doctrine_ok
            && weather_ok
            && location_ok
            && fleet_ok
            && (task.location != 0
                || held.is_some_and(|held| held.weather.is_none() && held.location.is_none()));
        carrying.task = Some(TaskState {
            fields: task,
            sequence,
            fleet_sequence,
            held: held_index,
            night_factor,
            all_carried,
        });
    }

    fn finish_task(&mut self, memory: &impl Memory, carrying: &mut Carrying<'_>) {
        let Some(state) = carrying.task.take() else {
            return;
        };
        let task = state.fields;
        let Some(country) = self.doctrine.country() else {
            return;
        };
        if state.all_carried
            && let Some(held_index) = state.held
        {
            let modifiers = self.modifiers.borrow();
            if modifiers.observed(task.modifiers).is_none()
                && modifiers.existing_at(task.modifiers, 1).is_none()
                && self.store.tasks[held_index].country == country
            {
                drop(modifiers);
                let mut held = self.store.tasks[held_index].clone();
                held.terms = carrying
                    .terms
                    .filter(|(pointer, _)| *pointer == task.pointer)
                    .map(|(_, terms)| (terms, carrying.terms_identity));
                if !self.store.keep_task_direct(held) {
                    carrying.counts.uncertified += 1;
                }
                carrying.task_modifiers = None;
                return;
            }
        }
        let doctrine = self.doctrine.doctrine(task.pointer);
        let modifiers = self.modifiers.borrow();
        let observed = modifiers.observed(task.modifiers).or_else(|| {
            if modifiers.existing_at(task.modifiers, 1).is_some() {
                return None;
            }
            carrying
                .task_modifiers
                .as_ref()
                .filter(|(address, _)| *address == task.modifiers)
                .map(|(_, entries)| entries.as_slice())
        });
        let held_prev = state.held.map(|at| &self.store.tasks[at]);
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
                        held_prev
                            .and_then(|held| held.weather)
                            .is_some_and(|(held, _, _)| {
                                held.night_factor() == weather.night_factor()
                            })
                    })
                    .or_else(|| engine_native::read::<u64>(memory, weather.night_factor()))?;
                let certificate = weather_certificate(carrying.changes, &weather, country)?;
                Some((weather, certificate, factor))
            });
        let location = state
            .sequence
            .zip(self.doctrine.location_source(task.location))
            .map(|((epoch, _), (definition, terrain))| (epoch, task.location, definition, terrain));
        let fleet = self
            .doctrine
            .fleet_source(task.fleet)
            .and_then(|(leader, terms)| {
                Some(HeldFleet {
                    fleet: task.fleet,
                    leader,
                    terms,
                    certificate: fleet_certificate(carrying.changes, state.fleet_sequence, leader)?,
                })
            });
        let held = HeldTask {
            pointer: task.pointer,
            handle: task.handle,
            roster_owner: task.roster_owner,
            country,
            country_terms,
            doctrine: certificate.zip(doctrine),
            mine_exposure: task.mine_exposure,
            modifiers: held_prev.and_then(|held| held.modifiers.clone()),
            weather,
            location,
            fleet,
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
        if let Some(state) = carrying.task.as_ref()
            && state.all_carried
            && task.influences_count == Some(0)
            && engine_sites::at(base, engine_native::ctask_force::VTABLE_RVA)
                .is_some_and(|vtable| task.vtable == vtable as u64)
            && let Some(held) = state.held.map(|at| &self.store.tasks[at])
            && let Some((terms, identity)) = held.terms
            && fields.game_state() == Some(terms.0.owner().0)
            && terms.0.owner().1 == task.roster_owner
            && terms.0.owner().2 == held.country
            && terms.0.support() == [0; 4]
            && self.doctrine.read_pride(memory, held.country) == Some(terms.0.pride())
        {
            let leader = match terms.0.leader() {
                Some(leader) => carrying.changes.tree_sequence(leader).map(Some),
                None => Some(None),
            };
            if leader == Some(terms.1) {
                self.doctrine.hold_owner(terms.0.owner());
                self.doctrine.hold_support(task.pointer, [0; 4]);
                if self.doctrine.task_terms(&task) == Some(terms.0) {
                    carrying.terms = Some((task.pointer, terms));
                    carrying.terms_identity = identity;
                    return;
                }
            }
        }
        if self.doctrine.location_source(task.location).is_none() {
            fields.visit_location(memory);
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
        if hot.terms_identity == 0 {
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
        if !self.store.variants_unchanged(
            memory,
            fields,
            hot,
            carrying.changes,
            &mut carrying.variant_quiet,
            &mut carrying.counts,
        ) {
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
        let air_capacity = if hot.receivers[2] != 0 {
            let held_row = &held.row.as_ref().expect("a carried row").0;
            let kind = self.air_capacity.capture(
                memory,
                base,
                hot.pointer,
                held_row,
                carrying.changes,
            );
            if held_row.carrier_air_base != 0
                && held_row.formation_flags & 0x80 == 0
                && kind.is_none()
            {
                self.optional_refusals[13] += 1;
            }
            (kind != held_row.air_capacity_kind).then_some(kind)
        } else {
            None
        };
        let len = self.len;
        let old_version = hot.row_version;
        let next_version = if organisation.is_some() || air_capacity.is_some() {
            self.rows_serial
        } else {
            old_version
        };
        if old_version != 0
            && self.row_versions.get(len).copied() == Some(old_version)
            && self
                .rows
                .rows
                .get(len)
                .is_some_and(|row| row.ship == hot.ship)
        {
            if let Some(organisation) = organisation {
                self.rows.rows[len].maximum_organisation = organisation;
            }
            if let Some(kind) = air_capacity {
                self.rows.rows[len].air_capacity_kind = kind;
            }
            self.row_versions[len] = next_version;
            self.record_kept(source.roster_ordinal);
        } else {
            #[cfg(any(test, feature = "test-support"))]
            carry_cost(|cost| cost.row_clones += 1);
            let mut row = held.row.as_ref().expect("a carried row").0.clone();
            row.roster_ordinal = source.roster_ordinal;
            if let Some(organisation) = organisation {
                row.maximum_organisation = organisation;
            }
            if let Some(kind) = air_capacity {
                row.air_capacity_kind = kind;
            }
            self.record(|_| Ok(row));
            if self.len != len && len < self.row_versions.len() {
                self.row_versions[len] = next_version;
            }
        }
        if self.len != len {
            let unchanged =
                hot.roster_ordinal == source.roster_ordinal && air_capacity.is_none();
            hot.row_version = next_version;
            held.row_version = next_version;
            hot.roster_ordinal = source.roster_ordinal;
            if let Some(kind) = air_capacity {
                held.row.as_mut().expect("a carried row").0.air_capacity_kind = kind;
            }
            if let Some(organisation) = organisation {
                let (row, kept) = held.row.as_mut().expect("a carried row");
                row.maximum_organisation = organisation;
                *kept = *terms;
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
                self.unchanged_rows += usize::from(unchanged);
            }
        }
        true
    }

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
        if let Some((hot, held)) = taken.as_mut()
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
            held.row_version = hot.row_version;
            #[cfg(any(test, feature = "test-support"))]
            carry_cost(|cost| cost.cold_ship_bytes += std::mem::size_of::<cship::HeldShip>());
            held
        });
        let (mut carry, mut stale) = match taken {
            Some(held) if certificate == Some(held.certificate) => (Some(held), None),
            Some(held) if certificate.is_some() => (None, Some(held)),
            _ => (None, None),
        };
        let mut visited = false;
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
        if carrying.store
            && let Some(held) = carry.as_ref()
            && held.row.is_some()
            && carrying.terms.map(|(task, _)| task) != Some(held.fields.task_force())
        {
            self.read_terms(memory, base, held.fields.task_force(), fields, carrying);
        }
        let certified = carry
            .as_ref()
            .is_some_and(|held| held.fields.variants_complete())
            && original_hot.is_some_and(|hot| {
                self.store.variants_certified(
                    carrying.changes,
                    &mut carrying.variant_quiet,
                    &hot,
                    &mut carrying.counts,
                )
            });
        let variants_and_terms = !carrying.guard_fallback
            && carry.as_ref().is_some_and(|held| {
                held.row.as_ref().is_some_and(|(row, terms)| {
                    model_carry_complete(row)
                        && carrying.terms.is_some_and(|(task, current)| {
                            task == held.fields.task_force() && current == *terms
                        })
                }) && (certified || {
                    carrying.counts.variants_read += 1;
                    fields.held_variants_unchanged(memory, &held.fields)
                })
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
        if !row_carry
            && let Some(held) = carry.as_ref()
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
                    && held
                        .row
                        .as_ref()
                        .is_some_and(|(row, _)| model_carry_complete(row))
                    && if certified {
                        fields.visit_held_certified(memory, &held.fields);
                        true
                    } else {
                        carrying.counts.variants_read += 1;
                        fields.visit_held(memory, &held.fields);
                        fields.same_variants(&held.fields)
                    }
            });
        let stale_model = stale.as_ref().is_some_and(|held| {
            model_carry_enabled()
                && held
                    .row
                    .as_ref()
                    .is_some_and(|(row, _)| model_carry_complete(row))
                && fields.same_variants(&held.fields)
        });
        if row_carry && let Some(mut held) = carry.take() {
            let air_capacity = if held.fields.carrier_air_base() != 0 {
                let held_row = &held.row.as_ref().expect("a carried row").0;
                let kind = self.air_capacity.capture(
                    memory,
                    base,
                    held.pointer,
                    held_row,
                    carrying.changes,
                );
                if held_row.carrier_air_base != 0
                    && held_row.formation_flags & 0x80 == 0
                    && kind.is_none()
                {
                    self.optional_refusals[13] += 1;
                }
                (kind != held_row.air_capacity_kind).then_some(kind)
            } else {
                None
            };
            let len = self.len;
            let old_version = held.row_version;
            let next_version = if air_capacity.is_some() {
                self.rows_serial
            } else {
                old_version
            };
            if old_version != 0
                && self.row_versions.get(len).copied() == Some(old_version)
                && self
                    .rows
                    .rows
                    .get(len)
                    .is_some_and(|kept| kept.ship == held.ship)
            {
                if let Some(kind) = air_capacity {
                    self.rows.rows[len].air_capacity_kind = kind;
                }
                self.row_versions[len] = next_version;
                self.record_kept(row.roster_ordinal);
            } else {
                #[cfg(any(test, feature = "test-support"))]
                carry_cost(|cost| cost.row_clones += 1);
                let mut carried = held.row.as_ref().expect("a carried row").0.clone();
                carried.roster_ordinal = row.roster_ordinal;
                if let Some(kind) = air_capacity {
                    carried.air_capacity_kind = kind;
                }
                self.record(|_| Ok(carried));
                if self.len != len && len < self.row_versions.len() {
                    self.row_versions[len] = next_version;
                }
            }
            if self.len != len {
                carrying.counts.rows_carried += 1;
                self.unchanged_rows += usize::from(
                    air_capacity.is_none()
                        && original_hot.is_some_and(|hot| hot.roster_ordinal == row.roster_ordinal),
                );
                if let Some((kept, _)) = held.row.as_mut() {
                    kept.roster_ordinal = row.roster_ordinal;
                    if let Some(kind) = air_capacity {
                        kept.air_capacity_kind = kind;
                    }
                }
                held.row_version = next_version;
                if !self.store.keep_held(held) {
                    carrying.counts.uncertified += 1;
                }
            }
            return;
        }
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
        if !carrying.store || self.len == len {
            return;
        }
        let read = fields.held();
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
        let row_version = self.rows_serial;
        if self.len - 1 < self.row_versions.len() {
            self.row_versions[self.len - 1] = row_version;
        }
        if let Some(mut held) = carry {
            carrying.counts.carried += 1;
            carrying.counts.models_carried += u64::from(model_carry);
            held.fields = read;
            held.row = made;
            held.organisation = self.doctrine.organisation_operands(pointer);
            held.terms_identity = carrying.terms_identity;
            held.row_version = row_version;
            if !self.store.keep_held(held) {
                carrying.counts.uncertified += 1;
            }
            return;
        }
        carrying.counts.read += 1;
        carrying.counts.models_carried += u64::from(stale_model);
        let modifiers = self.modifiers.borrow();
        let arrays = read.accumulators().map(|address| {
            if address == 0 {
                Some(&[][..])
            } else {
                modifiers.observed(address)
            }
        });
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
        let kept = observation.is_some_and(|(certificate, slots)| {
            let [experience_at, medals_at] = read.accumulators();
            let boxed = match stale {
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
                    spare.row_version = row_version;
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
                    row_version,
                }),
            };
            boxed.is_ok_and(|held| self.store.keep(held, [experience, medals]))
        });
        if !kept {
            carrying.counts.uncertified += 1;
        }
    }

    #[allow(clippy::too_many_arguments)]
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
        row.naval_power = selected.and_then(|source| {
            cship::read_naval_power(memory, base, source, &mut self.naval_power)
        });
        if selected.is_some() && row.naval_power.is_none() {
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
                if self.len < self.row_versions.len() {
                    self.row_versions[self.len] = 0;
                }
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
    fn release_sources(&mut self) {
        let arena = self.modifiers.get_mut();
        let required = arena.required_capacity();
        self.required_modifiers.nodes = self.required_modifiers.nodes.max(required.nodes);
        self.required_modifiers.entries = self.required_modifiers.entries.max(required.entries);
        arena.clear();
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
    pub fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        if self.pending_owned && !self.defer_owned {
            self.forget_stores();
            self.pending_owned = false;
        }
        self.capture_carrying(memory, context, &ship_model_changes::Observer)
    }

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

    pub fn settle_owned(&mut self) {
        if self.pending_owned {
            self.release_sources();
            self.doctrine.reset_hour();
            if crate::allocation::failed() {
                self.forget_stores();
            }
            self.pending_owned = false;
        }
        for part in &mut self.extra {
            part.settle_owned();
        }
    }

    fn capture_carrying(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        changes: &dyn Changes,
    ) -> CaptureReport {
        self.context = Some(context);
        self.rows_serial = SERIALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        self.shared = ship_model_changes::CARRY && changes.shared_store();
        if self.shared {
            let held = shared_slot(self.range, Option::take).unwrap_or_default();
            self.store = held.store;
            self.air_capacity = held.air_capacity;
        }
        let power_certificate = naval_power_certificate();
        self.naval_power.prepare(power_certificate);
        if self.store.naval_power != power_certificate {
            self.store.forget();
        }
        self.store.naval_power = power_certificate;
        self.witness = None;
        let held_rows = self
            .store
            .rows
            .map(|generation| (generation, self.store.hot.len()));
        self.air_capacity.begin(self.detail_len);
        let mut fields = cship::ShipFields::new(context.base);
        if power_certificate.is_some() {
            fields.enable_naval_power();
        }
        let quiet = changes.quiet();
        let variant_quiet = changes.variant_quiet();
        let carrying = RefCell::new(Carrying {
            store: ship_model_changes::CARRY && self.terminal_failure.is_none(),
            changes,
            counts: Counts::default(),
            task: None,
            task_modifiers: None,
            terms: None,
            terms_identity: 0,
            quiet: quiet.filter(|quiet| Some(*quiet) == self.store.quiet),
            variant_quiet: variant_quiet.filter(|quiet| Some(*quiet) == self.store.variant_quiet),
            guard_fallback: false,
            revalidated: false,
        });
        self.store.next.clear();
        self.store.next_hot.clear();
        self.store.next_tasks.clear();
        self.store.next_variants.clear();
        self.store.next_variant_head.clear();
        for variant in &mut self.store.variants {
            variant.unchanged = None;
            variant.next = None;
        }
        #[cfg(any(test, feature = "test-support"))]
        CARRY_COST.set(CarryCost::default());
        let report = self.capture_with(
            |this, seed| {
                let carrying = &mut carrying.borrow_mut();
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
            |this| {
                if this.failure.is_none() {
                    this.finish_task(memory, &mut carrying.borrow_mut());
                }
            },
        );
        if naval_power_certificate() != power_certificate {
            for row in &mut self.rows.rows[..self.len] {
                self.optional_refusals[14] += u64::from(row.naval_power.take().is_some());
            }
            self.naval_power.prepare(None);
            self.store.forget();
        }
        let mut carrying = carrying.into_inner();
        let complete =
            self.terminal_failure.is_none() && matches!(self.rows.outcome, Outcome::Rows { .. });
        let mut carriers = 0;
        if complete {
            self.air_capacity.finish();
            carriers = self.air_capacity.len();
            self.store
                .certify_variants(changes, variant_quiet, carrying.variant_quiet.is_some());
            if self.store.hold() {
                self.store.quiet = quiet;
            }
            self.store.rows = (self.store.held.len() == self.len).then(|| match held_rows {
                Some((generation, held)) if held == self.len && self.unchanged_rows == self.len => {
                    generation
                }
                _ => crate::projection::generation(),
            });
            self.witness = self.store.rows;
            if self.shared {
                let held = Shared {
                    store: std::mem::take(&mut self.store),
                    air_capacity: std::mem::take(&mut self.air_capacity),
                };
                shared_slot(self.range, |slot| *slot = Some(held));
            }
        } else {
            self.forget_stores();
        }
        if carrying.store {
            carrying.counts.receivers = self.seed_len.saturating_mul(5).saturating_add(carriers);
            ship_model_changes::record(&carrying.counts, complete);
            if complete {
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
        [held_input::<tables::ShipModels>(
            projection,
            self.held_witness(),
            |projection| self.project(projection),
        )]
    }

    fn held_witness(&self) -> Option<Witness> {
        if !matches!(self.outcomes()[0], Outcome::Rows { .. }) {
            return None;
        }
        let mut words = [0; MAX_PARTS + 1];
        words[0] = self.split as u64;
        words[1] = self.witness?;
        for (word, part) in words[2..].iter_mut().zip(self.extra.get(..self.split - 1)?) {
            *word = part.witness?;
        }
        Some(Witness::new(words))
    }

    fn project(&self, projection: Option<&mut Projection<'_>>) -> TableInput {
        if self.split > 1 && self.terminal_failure.is_none() {
            let parts = &self.extra[..self.split - 1];
            let mut refs: Vec<&cship::ModelRow> = Vec::new();
            if reserve(&mut refs, self.len).is_err() {
                return crate::table_conversion::unavailable("collector allocation failed").into();
            }
            refs.extend(self.rows.rows[..self.part_len.min(self.rows.rows.len())].iter());
            for part in parts {
                refs.extend(part.rows.rows[..part.part_len.min(part.rows.rows.len())].iter());
            }
            Self::project_slice(projection, self.outcomes()[0], &refs)
        } else {
            Self::project_slice(projection, self.outcomes()[0], &self.rows.rows[..])
        }
    }

    fn project_slice<R: std::borrow::Borrow<cship::ModelRow>>(
        projection: Option<&mut Projection<'_>>,
        outcome: Outcome,
        rows: &[R],
    ) -> TableInput {
        project_table::<tables::ShipModels, _>(
            projection,
            outcome,
            rows,
            |row| u128::from(row.borrow().ship),
            |row, previous| {
                let row = row.borrow();
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
            |r| {
                let r = r.borrow();
                ShipModelRow {
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
                }
            },
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static MODEL_CARRY_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn model_carry_enabled() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    return !MODEL_CARRY_OFF.get();
    #[cfg(not(any(test, feature = "test-support")))]
    true
}

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

struct Carrying<'a> {
    store: bool,
    changes: &'a dyn Changes,
    counts: Counts,
    task: Option<TaskState>,
    task_modifiers: Option<(usize, Arc<Vec<[u8; 16]>>)>,
    terms: Option<(usize, Terms)>,
    terms_identity: u64,
    quiet: Option<(u64, u64)>,
    variant_quiet: Option<(u64, u64)>,
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

fn naval_power_certificate() -> Option<(u64, u64, u64)> {
    use collector_model_inputs::local_subunit_changes::Changes;
    let world = collector_lifetime::world_lifetime::token()?;
    let (epoch, sequence) = collector_model_inputs::local_subunit_changes::Observer.quiet()?;
    Some((world, epoch, sequence))
}
