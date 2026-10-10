//! Naval mission icon operands: strike/invasion active counts (P2), convoy efficiencies (P3) and the
//! navies' mission ledgers. The game computes the efficiencies on demand, so this pack records
//! their inputs ([`engine::naval_mission_inputs`]); `replay::naval_mission_derive` evaluates
//! them.
//!
//! One walk of every country's fleets feeds the hourly task-force operands and names the
//! countries their roster, mission and expeditionary owners need. Each navy's live ledgers
//! retain stored order and escort presence, including unlisted entries still fading out.
//! The `naval_missions` daily sliced family (`slicing::NAVAL_MISSIONS`) reads country
//! modifiers/hubs and fleet-leader/HQ accumulators (owner decision 2026-09-25).
//! Its fingerprint uses the hour's task-force rows without further reads: a new country or
//! changed leader/commander reference refreshes within the hour; modifier changes wait for
//! the daily slice.
use crate::{
    sampling::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    spooling::table,
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine::allocation::{copy_vec, filled, owned_vec, reserve, resize_default};
use engine::{
    Memory, ccountry, cfleet, creference_registry, cstrategic_navy_manager, cweather_manager,
    modifier_accumulator::{self as accumulator, Value},
    naval_mission_inputs::{self as inputs, COUNTRY_KEYS, MISSION_KINDS, Operands},
    transport_ledger_observation as transport_native,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{
        NavalMissionCommanderRow, NavalMissionCountryRow, NavalMissionLedgerRow,
        NavalMissionTaskForceRow, TableId, tables,
    },
};
use std::mem::size_of;

pub const N: usize = 4;
pub const TABLES: [TableId; N] = [
    TableId::NavalMissionTaskForces,
    TableId::NavalMissionLedgers,
    TableId::NavalMissionCountries,
    TableId::NavalMissionCommanders,
];
/// Every table follows the task-force walk, which is charged to NavalMissionTaskForces; each
/// table's own reads are charged to it.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[(
    "naval mission task forces, ledgers, countries and commanders",
    0,
    &[0, 1, 2, 3],
)];
/// Indices of the `naval_missions` sliced family's tables in [`TABLES`].
pub(crate) const SLICED: [usize; 2] = [2, 3];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    task_forces: usize,
    headquarters: usize,
    /// Every walked task force, for the ledgers' mission pointers.
    census: usize,
    ledgers: usize,
    /// The ledgers' registrations.
    entries: usize,
    receivers: usize,
    /// Country rows and the walk's named-country scratch (up to three per task force).
    countries: usize,
    commanders: usize,
    hubs: usize,
}

impl Capacities {
    /// Native attach/cadence demand; no sampled values or retry witness.
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        use engine::{cpdx_array::Array, ctask_force};
        let countries = ccountry::Reader::new(memory, base, state)?;
        let mut tasks = 0_usize;
        let mut selected = 0_usize;
        let mut headquarters = 0_usize;
        cfleet::visit_task_forces(memory, base, &countries, |event| {
            let cfleet::Visit::TaskForce(visit, _) = event else {
                return Ok(());
            };
            tasks = tasks
                .checked_add(1)
                .ok_or("naval mission census demand overflow")?;
            if MISSION_KINDS.contains(&visit.kind) {
                selected = selected
                    .checked_add(1)
                    .ok_or("naval mission row demand overflow")?;
                // CUnit::CalcStat@140C296B0 reaches every attached HQ occurrence.
                let attached = visit
                    .task
                    .checked_add(ctask_force::ATTACHED_HEADQUARTERS)
                    .and_then(|at| Array::read(memory, at, 8))
                    .ok_or("task-force headquarters sizing header unreadable")?;
                headquarters = headquarters
                    .checked_add(attached.count as usize)
                    .ok_or("naval mission HQ demand overflow")?;
            }
            Ok(())
        })?;
        self.census = self.census.max(tasks);
        self.task_forces = self.task_forces.max(selected);
        self.headquarters = self.headquarters.max(headquarters);
        self.countries = self
            .countries
            .max(countries.count().saturating_sub(1) as usize);
        self.commanders = self.commanders.max(
            selected
                .checked_add(
                    headquarters
                        .checked_mul(4)
                        .ok_or("naval mission commander sizing overflow")?,
                )
                .ok_or("naval mission commander sizing overflow")?,
        );
        let mut ledgers = 0_usize;
        let mut entries = 0_usize;
        let mut hubs = 0_usize;
        for slot in 1..countries.count() {
            let Some(owner) = countries.at(slot)? else {
                continue;
            };
            let navy = cstrategic_navy_manager::navy(memory, base, state, owner)?;
            inputs::ledger_regions(memory, navy, |_, ledger| {
                ledgers = ledgers
                    .checked_add(1)
                    .ok_or("naval mission ledger sizing overflow")?;
                entries = entries
                    .checked_add(ledger.count as usize)
                    .ok_or("naval mission occurrence sizing overflow")?;
                Ok(())
            })?;
            let count = inputs::navy_hubs(memory, navy, &mut [])?;
            hubs = hubs
                .checked_add(count)
                .ok_or("naval mission hub sizing overflow")?;
        }
        self.ledgers = self.ledgers.max(ledgers);
        self.entries = self.entries.max(entries);
        self.receivers = self.receivers.max(entries);
        self.hubs = self.hubs.max(hubs);
        Ok(())
    }

    pub(crate) fn raise_for_transport(
        &mut self,
        demand: super::transport_mission_shared::Capacities,
    ) {
        self.ledgers = self.ledgers.max(demand.ledgers);
        self.entries = self.entries.max(demand.occurrences);
        self.receivers = self.receivers.max(demand.receivers);
    }

    /// A family row per country slot at most. Never lowers.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        self.countries = self.countries.max(crate::country_scale::rows(slots, 1));
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TaskForce {
    row: Operands,
    /// Roster country: canonical slot and native id.
    slot: u32,
    country: u32,
    headquarters: usize,
    /// [`inputs::TASK_FORCE_KEYS`] of the location region's weather.
    weather: [Value; 4],
}

/// A walked task force, for mapping a ledger's mission pointers to handles.
#[derive(Clone, Copy, Debug, Default)]
struct Member {
    task: usize,
    id: u64,
    mission_header: Option<Result<[u8; 16], &'static str>>,
    mission_owner: Option<Result<u32, &'static str>>,
}

#[derive(Clone, Copy, Debug, Default)]
struct LedgerRow {
    owner: u32,
    region: u32,
    first: usize,
    count: usize,
    ring: bool,
    counted: inputs::Presence,
}

#[derive(Clone, Copy, Debug, Default)]
struct CountryRow {
    country: u32,
    values: [Value; 7],
    hubs: usize,
    hub_count: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct CommanderRow {
    country: u32,
    reference: u64,
    source: u8,
    values: [Value; 4],
}

pub struct Captured {
    growing: bool,
    task_forces: CaptureRows<TaskForce>,
    headquarters: Arena<u64>,
    census: Arena<Member>,
    ledgers: CaptureRows<LedgerRow>,
    entries: Arena<u64>,
    /// One ledger's mission pointers; as long as the registration arena.
    scratch: Vec<usize>,
    nullable: Vec<u64>,
    pub(crate) transport: super::transport_receivers::Captured,
    countries: CaptureRows<CountryRow>,
    hubs: Arena<u32>,
    commanders: CaptureRows<CommanderRow>,
    /// The countries the walk named, sorted by canonical slot and deduplicated once the walk
    /// is complete.
    named: Arena<ccountry::Country>,
    /// Sort scratch for located regions and distinct commander references.
    keys: Vec<u64>,
    /// The distinct located regions' weather, in region order.
    weather: Vec<(u32, [Value; 4])>,
    /// The countries this hour's family read covers.
    selection: SliceSelection,
}

fn elapsed(started: i64) -> u64 {
    qpc().saturating_sub(started).max(0) as u64
}

fn finish(len: usize, unread: u32) -> Outcome {
    if unread == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated {
            len,
            unread_rows: unread,
        }
    }
}

/// A leader and commander fingerprint of one country's task forces (FNV-1a over their
/// references, in walk order).
fn fingerprint(task_forces: &[TaskForce], headquarters: &[u64]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut mix = |value: u64| {
        for byte in value.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    for task_force in task_forces {
        mix(task_force.row.leader);
        mix(task_force.row.headquarters as u64);
        for &character in &headquarters
            [task_force.headquarters..task_force.headquarters + task_force.row.headquarters]
        {
            mix(character);
        }
    }
    hash
}

/// Sorts and deduplicates `keys` in place; returns the distinct count.
fn distinct<T: Copy, K: Ord>(keys: &mut [T], key: impl Fn(T) -> K) -> usize {
    keys.sort_unstable_by_key(|value| key(*value));
    let mut distinct = 0;
    for index in 0..keys.len() {
        if distinct == 0 || key(keys[index]) != key(keys[distinct - 1]) {
            keys[distinct] = keys[index];
            distinct += 1;
        }
    }
    distinct
}

const NO_COUNTRY: ccountry::Country = ccountry::Country {
    pointer: 0,
    canonical_slot: 0,
    id: 0,
};

impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::build(&Capacities::default(), false)
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self::build(capacity, true)
    }

    fn build(capacity: &Capacities, storage: bool) -> Self {
        let size = |count| if storage { count } else { 0 };
        Self {
            growing: false,
            task_forces: CaptureRows::new(size(capacity.task_forces)),
            headquarters: Arena::new(size(capacity.headquarters)),
            census: Arena::new(size(capacity.census)),
            ledgers: CaptureRows::new(size(capacity.ledgers)),
            entries: Arena::new(size(capacity.entries)),
            scratch: owned_vec(filled(size(capacity.entries), 0)),
            nullable: owned_vec(filled(size(capacity.entries), 0)),
            transport: super::transport_receivers::Captured::new(size(capacity.receivers)),
            countries: CaptureRows::new(size(capacity.countries)),
            hubs: Arena::new(size(capacity.hubs)),
            commanders: CaptureRows::new(size(capacity.commanders)),
            named: Arena {
                first: owned_vec(filled(size(Self::named_capacity(capacity)), NO_COUNTRY)),
                len: 0,
                required: 0,
            },
            keys: owned_vec(filled(
                size(capacity.task_forces).max(size(capacity.headquarters)),
                0,
            )),
            weather: owned_vec(filled(
                size(capacity.task_forces),
                (0, [Value::default(); 4]),
            )),
            selection: SliceSelection::new(storage),
        }
    }

    /// The walk names up to three countries per recorded task force.
    fn named_capacity(capacity: &Capacities) -> usize {
        capacity
            .countries
            .max(capacity.task_forces)
            .saturating_mul(3)
    }

    pub fn reset(&mut self) {
        self.task_forces.reset();
        self.headquarters.reset();
        self.census.reset();
        self.transport.reset();
        self.ledgers.reset();
        self.entries.reset();
        self.countries.reset();
        self.hubs.reset();
        self.commanders.reset();
        self.named.len = 0;
        self.named.required = 0;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.task_forces.allocated_bytes()
            + self.headquarters.allocated_bytes()
            + self.census.allocated_bytes()
            + self.ledgers.allocated_bytes()
            + self.entries.allocated_bytes()
            + self.scratch.capacity() * size_of::<usize>()
            + self.nullable.capacity() * size_of::<u64>()
            + self.transport.allocated_bytes()
            + self.countries.allocated_bytes()
            + self.hubs.allocated_bytes()
            + self.commanders.allocated_bytes()
            + self.named.first.capacity() * size_of::<ccountry::Country>()
            + self.keys.capacity() * size_of::<u64>()
            + self.weather.capacity() * size_of::<(u32, [Value; 4])>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        if let Outcome::Truncated { len, unread_rows } = self.task_forces.outcome {
            // The family did not run: raise it to the bounds the walk's demand implies (a
            // leader and four commander sources per task force and entry).
            let task_forces = len.saturating_add(unread_rows as usize);
            grow_capacity(
                &mut capacity.commanders,
                task_forces.saturating_add(self.headquarters.required.saturating_mul(4)),
            );
        }
        self.task_forces.grow(&mut capacity.task_forces);
        self.headquarters.grow(&mut capacity.headquarters);
        self.census.grow(&mut capacity.census);
        self.ledgers.grow(&mut capacity.ledgers);
        self.entries.grow(&mut capacity.entries);
        if self.scratch.len() < capacity.entries
            && resize_default(&mut self.scratch, capacity.entries).is_err()
        {
            return;
        }
        if self.nullable.len() < capacity.entries
            && resize_default(&mut self.nullable, capacity.entries).is_err()
        {
            return;
        }
        // A short memo sees only the first missing receiver in each ledger.
        // The original registration count bounds every mission and receiver.
        grow_capacity(&mut capacity.receivers, self.entries.required);
        self.transport.grow(&mut capacity.receivers);
        self.countries.grow(&mut capacity.countries);
        self.hubs.grow(&mut capacity.hubs);
        self.commanders.grow(&mut capacity.commanders);
        let mut named = self.named.first.len();
        grow_capacity(&mut named, self.named.required);
        grow_capacity(&mut named, Self::named_capacity(capacity));
        if self.named.first.len() < named {
            let extra = named - self.named.first.len();
            if reserve(&mut self.named.first, extra).is_err() {
                return;
            }
            self.named.first.resize(named, NO_COUNTRY);
        }
        let keys = capacity.task_forces.max(capacity.headquarters);
        if self.keys.len() < keys && resize_default(&mut self.keys, keys).is_err() {
            return;
        }
        if self.weather.len() < capacity.task_forces {
            let _ = resize_default(&mut self.weather, capacity.task_forces);
        }
    }

    /// The countries whose family rows this frame's read replaced.
    pub(crate) fn selection(&self) -> &SliceSelection {
        &self.selection
    }

    /// Start a frame's family read at its hour; the capture resets it again.
    pub(crate) fn reset_selection(&mut self, hour: i32, force_full: bool) {
        self.selection.reset(hour, force_full);
    }

    /// Terminal completion after an accepted worker failure: nothing is read or published.
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        let outcome = |index: usize| super::failed_capture(TABLES[index], hour, force_full, reason);
        self.task_forces.fail(outcome(0));
        self.ledgers.fail(outcome(1));
        self.countries.fail(outcome(2));
        self.commanders.fail(outcome(3));
        self.selection.reset(hour, force_full);
        self.transport.retire();
        self.census.first.fill(Member::default());
        self.scratch.fill(0);
    }

    /// One hour's read. Every table is refreshed every hour; the sliced tables read the
    /// countries the hour and, with a store, the changed fingerprints select.
    pub fn capture_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
    ) -> [u64; N] {
        self.capture_impl(
            memory,
            base,
            game_state,
            hour,
            force_full,
            fingerprints,
            None,
            None,
            true,
        )
    }

    #[cfg(test)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
    ) -> [u64; N] {
        self.capture_impl(
            memory,
            base,
            game_state,
            hour,
            force_full,
            fingerprints,
            None,
            None,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture_with_transport_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
        holder: &mut super::country_naval_movement_inputs::Captured,
        shared: &mut super::transport_mission_shared::Shared,
    ) -> [u64; N] {
        shared.begin_growing(hour, holder.physical_count());
        let started = qpc();
        // One original manager selection belongs to this entire mission loan.
        // Failed selectors/headers cannot be retried by the strict borrower.
        let weather;
        {
            let _reads = crate::read_accounting::table(TableId::ReservedCountryNaming);
            weather = cweather_manager::Reader::new(memory, base, game_state);
            shared.capture_naval_search_with_weather(
                memory,
                base,
                game_state,
                weather.as_ref().map_err(|reason| *reason),
            );
        }
        let graph_ticks = elapsed(started);
        let mut ticks = self.capture_impl(
            memory,
            base,
            game_state,
            hour,
            force_full,
            fingerprints,
            Some((holder, &mut *shared)),
            Some(weather.as_ref().map_err(|reason| *reason)),
            true,
        );
        shared.finish_weather_union();
        ticks[1] += graph_ticks;
        // Receiver certificates alone continue into the original counter pass.
        // Ledger pointers and original headers have no later consumer.
        self.census.first.fill(Member::default());
        self.scratch.fill(0);
        ticks
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture_with_transport(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
        holder: &mut super::country_naval_movement_inputs::Captured,
        shared: &mut super::transport_mission_shared::Shared,
    ) -> [u64; N] {
        shared.begin(hour, holder.physical_count());
        let started = qpc();
        // One original manager selection belongs to this entire mission loan.
        // Failed selectors/headers cannot be retried by the strict borrower.
        let weather;
        {
            let _reads = crate::read_accounting::table(TableId::ReservedCountryNaming);
            weather = cweather_manager::Reader::new(memory, base, game_state);
            shared.capture_naval_search_with_weather(
                memory,
                base,
                game_state,
                weather.as_ref().map_err(|reason| *reason),
            );
        }
        let graph_ticks = elapsed(started);
        let mut ticks = self.capture_impl(
            memory,
            base,
            game_state,
            hour,
            force_full,
            fingerprints,
            Some((holder, &mut *shared)),
            Some(weather.as_ref().map_err(|reason| *reason)),
            false,
        );
        shared.finish_weather_union();
        ticks[1] += graph_ticks;
        // Receiver certificates alone continue into the original counter pass.
        // Ledger pointers and original headers have no later consumer.
        self.census.first.fill(Member::default());
        self.scratch.fill(0);
        ticks
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_impl<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
        mut transport: Option<(
            &mut super::country_naval_movement_inputs::Captured,
            &mut super::transport_mission_shared::Shared,
        )>,
        weather: Option<Result<&cweather_manager::Reader<'_, M>, &'static str>>,
        growing: bool,
    ) -> [u64; N] {
        self.reset();
        self.growing = growing;
        self.selection.reset(hour, force_full);
        let mut ticks = [0; N];
        let started = qpc();
        let mut original_failure = None;
        let walked = {
            let _reads = crate::read_accounting::table(TABLES[0]);
            ccountry::Reader::new(memory, base, game_state).and_then(|countries| {
                let result = self.walk(memory, base, &countries).and_then(|outcome| {
                    self.task_forces.outcome = outcome;
                    self.read_weather(
                        memory,
                        base,
                        game_state,
                        transport.as_mut().map(|(_, shared)| {
                            (
                                &mut **shared,
                                weather.unwrap_or(Err("transport original weather reader missing")),
                            )
                        }),
                    )
                });
                if let Err(reason) = result {
                    if transport.is_none() {
                        return Err(reason);
                    }
                    original_failure = Some(reason);
                    self.fail_all(reason);
                }
                Ok(countries)
            })
        };
        ticks[0] = elapsed(started);
        let countries = match walked {
            Ok(countries) => countries,
            Err(reason) => {
                self.fail_all(reason);
                if let Some((_, shared)) = transport.as_mut() {
                    shared.fail(reason);
                }
                return ticks;
            }
        };
        let started = qpc();
        self.ledgers.outcome = {
            let _reads = crate::read_accounting::table(TABLES[1]);
            if let Some((holder, shared)) = transport {
                self.read_transport_ledgers(
                    memory,
                    base,
                    &countries,
                    holder,
                    shared,
                    original_failure,
                )
                .unwrap_or_else(|reason| {
                    shared.fail(reason);
                    Outcome::Unavailable { reason }
                })
            } else if self.census.required > self.census.first.len() {
                // The growth follows the walk's demand.
                Outcome::Unavailable {
                    reason: "naval mission ledger census truncated",
                }
            } else {
                self.read_ledgers(memory, base, game_state, &countries)
                    .unwrap_or_else(|reason| Outcome::Unavailable { reason })
            }
        };
        ticks[1] = elapsed(started);
        if original_failure.is_some() {
            return ticks;
        }
        if !matches!(self.task_forces.outcome, Outcome::Rows { .. })
            || self.named.required > self.named.first.len()
        {
            // The family's census and fingerprints need every task force: nothing more is
            // published, the restore reads the family's owners again and the growth follows
            // the walk's demand.
            let outcome = Outcome::Unavailable {
                reason: "naval mission task-force walk truncated",
            };
            self.countries.outcome = outcome;
            self.commanders.outcome = outcome;
            return ticks;
        }
        let named = &mut self.named.first[..self.named.len];
        self.named.len = distinct(named, |country| country.canonical_slot);
        let started = qpc();
        let _reads = crate::read_accounting::table(TABLES[2]);
        if let Some(fingerprints) = fingerprints {
            let scan = qpc();
            self.scan(fingerprints);
            self.selection.scan_ticks = elapsed(scan);
        }
        let (countries_outcome, commanders_outcome, commander_ticks) =
            self.read_family(memory, base, game_state);
        self.countries.outcome = countries_outcome;
        self.commanders.outcome = commanders_outcome;
        ticks[2] = elapsed(started).saturating_sub(commander_ticks);
        ticks[3] = commander_ticks;
        ticks
    }

    fn fail_all(&mut self, reason: &'static str) {
        let outcome = Outcome::Unavailable { reason };
        self.task_forces.outcome = outcome;
        self.ledgers.outcome = outcome;
        self.countries.outcome = outcome;
        self.commanders.outcome = outcome;
    }

    /// Every task force into the authenticated census, and the mission-input
    /// and escort task forces in walk order (so each roster country's rows are contiguous and
    /// in slot order) with the countries they name. A row or list that does not fit is
    /// counted and its demand kept; the walk goes on so the whole demand is known.
    fn walk<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
    ) -> Result<Outcome, &'static str> {
        let mut unread = 0_u32;
        let rows = &mut self.task_forces.rows;
        let headquarters = &mut self.headquarters;
        let census = &mut self.census;
        let named = &mut self.named;
        let mut len = 0_usize;
        cfleet::visit_task_forces(memory, base, countries, |event| {
            let cfleet::Visit::TaskForce(visit, _) = event else {
                return Ok(());
            };
            census.required += 1;
            if self.growing {
                engine::walk::Buffer::Growing(&mut census.first).ensure(census.required)?;
                engine::walk::Buffer::Growing(rows).ensure(
                    len.checked_add(1)
                        .ok_or("naval mission row demand overflow")?,
                )?;
            }
            if let Some(member) = census.first.get_mut(census.len) {
                *member = Member {
                    task: visit.task,
                    id: visit.id,
                    mission_header: None,
                    mission_owner: None,
                };
                census.len += 1;
            }
            // Raw references that name the roster country itself need no lookup.
            let resolve = |raw: u32| {
                if raw == visit.country.id {
                    Ok(visit.country)
                } else {
                    countries.resolve(raw)
                }
            };
            if !MISSION_KINDS.contains(&visit.kind) {
                return Ok(());
            }
            let fits = rows.len() > len;
            let mut original_header = None;
            let mut original_owner = None;
            let observed = inputs::read_task_force_transport_observed_buffer(
                memory,
                base,
                visit,
                if self.growing {
                    engine::walk::Buffer::GrowingFrom(&mut headquarters.first, headquarters.len)
                } else if fits {
                    engine::walk::Buffer::Fixed(&mut headquarters.first[headquarters.len..])
                } else {
                    engine::walk::Buffer::Fixed(&mut [])
                },
                &mut original_header,
                &mut original_owner,
            );
            if let Some(member) = census.first.get_mut(census.required - 1) {
                member.mission_header = original_header;
                member.mission_owner = original_owner;
            }
            let row = observed?;
            headquarters.required = headquarters.required.saturating_add(row.headquarters);
            // Preserve raw owners even when not linked; only the family's country census
            // needs a resolved reference. Playback names the missing link when it uses it.
            let owner = resolve(row.mission_owner).ok();
            let expeditionary = (row.expeditionary_owner > 0)
                .then(|| resolve(row.expeditionary_owner as u32).ok())
                .flatten();
            for country in [Some(visit.country), owner, expeditionary]
                .into_iter()
                .flatten()
            {
                named.required += 1;
                if self.growing && named.required > named.first.len() {
                    {
                        let additional = named.required - named.first.len();
                        reserve(&mut named.first, additional)
                    }
                    .map_err(|_| "capture buffer allocation failed")?;
                    named.first.resize(named.required, NO_COUNTRY);
                }
                if let Some(entry) = named.first.get_mut(named.len) {
                    *entry = country;
                    named.len += 1;
                }
            }
            if !fits || headquarters.len + row.headquarters > headquarters.first.len() {
                unread = unread.saturating_add(1);
                return Ok(());
            }
            rows[len] = TaskForce {
                row,
                slot: visit.country.canonical_slot,
                country: visit.country.id,
                headquarters: headquarters.len,
                weather: [Value::default(); 4],
            };
            headquarters.len += row.headquarters;
            len += 1;
            Ok(())
        })?;
        Ok(finish(len, unread))
    }

    fn task_forces(rows: &CaptureRows<TaskForce>) -> &[TaskForce] {
        &rows.rows[..rows.outcome.len()]
    }

    /// The weather modifiers of every region a recorded task force is located in, once each.
    fn read_weather<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        game_state: usize,
        mut original: Option<(
            &mut super::transport_mission_shared::Shared,
            Result<&cweather_manager::Reader<'_, M>, &'static str>,
        )>,
    ) -> Result<(), &'static str> {
        let rows = self.task_forces.outcome.len();
        if self.growing {
            engine::walk::Buffer::Growing(&mut self.keys).ensure(rows)?;
            engine::walk::Buffer::Growing(&mut self.weather).ensure(rows)?;
        }
        let mut located = 0;
        for task_force in &self.task_forces.rows[..rows] {
            if task_force.row.region != 0 {
                self.keys[located] = u64::from(task_force.row.region);
                located += 1;
            }
        }
        if located == 0 {
            return Ok(());
        }
        let regions = distinct(&mut self.keys[..located], |key| key);
        let standalone;
        let weather = if let Some((_, weather)) = &original {
            *weather
        } else {
            standalone = cweather_manager::Reader::new(memory, base, game_state);
            standalone.as_ref().map_err(|reason| *reason)
        };
        for index in 0..regions {
            let region = self.keys[index] as u32;
            let mut values = [Value::default(); 4];
            if let Some((shared, original_weather)) = original.as_mut() {
                values = shared.original_strict_weather(memory, base, region, *original_weather)?;
            } else {
                inputs::region_weather(memory, weather?, region, &mut values)?;
            }
            self.weather[index] = (region, values);
        }
        let known = &self.weather[..regions];
        for task_force in &mut self.task_forces.rows[..rows] {
            let region = task_force.row.region;
            if region != 0 {
                let at = known
                    .binary_search_by_key(&region, |(region, _)| *region)
                    .expect("every located region was read");
                task_force.weather = known[at].1;
            }
        }
        Ok(())
    }

    /// Every nonempty live (navy, region) ledger: its registrations in stored
    /// order and, the navy's presence counts for the region (also read by unlisted escort records).
    fn read_ledgers<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        game_state: usize,
        countries: &ccountry::Reader<'_, M>,
    ) -> Result<Outcome, &'static str> {
        let census = &mut self.census.first[..self.census.len];
        census.sort_unstable_by_key(|member| member.task);
        let mut len = 0_usize;
        let mut unread = 0_u32;
        for slot in 1..countries.count() {
            let Some(owner) = countries.at(slot)? else {
                continue;
            };
            let pointer = cstrategic_navy_manager::navy(memory, base, game_state, owner)?;
            let mut map = None;
            inputs::ledger_regions(memory, pointer, |region, ledger| {
                if self.growing {
                    engine::walk::Buffer::Growing(&mut self.scratch)
                        .ensure(ledger.count as usize)?;
                    engine::walk::Buffer::Growing(&mut self.ledgers.rows).ensure(
                        len.checked_add(1)
                            .ok_or("naval mission ledger row demand overflow")?,
                    )?;
                    engine::walk::Buffer::Growing(&mut self.entries.first).ensure(
                        self.entries
                            .len
                            .checked_add(ledger.count as usize)
                            .ok_or("naval mission ledger demand overflow")?,
                    )?;
                }
                let count = inputs::ledger(memory, ledger, &mut self.scratch)?;
                self.entries.required = self.entries.required.saturating_add(count);
                if len == self.ledgers.rows.len()
                    || self.entries.len + count > self.entries.first.len()
                {
                    unread = unread.saturating_add(1);
                    return Ok(());
                }
                for (at, &mission) in self.scratch[..count].iter().enumerate() {
                    let member = mission
                        .checked_sub(engine::ctask_force::NAVAL_MISSION)
                        .and_then(|task| {
                            census
                                .binary_search_by_key(&task, |member| member.task)
                                .ok()
                        })
                        .map(|index| census[index])
                        .ok_or("navy ledger names a task force outside the fleet census")?;
                    self.entries.first[self.entries.len + at] = member.id;
                }
                let mut row = LedgerRow {
                    owner: owner.id,
                    region,
                    first: self.entries.len,
                    count,
                    ..LedgerRow::default()
                };
                {
                    // 1418ED090 updates every record, including an unlisted escort
                    // animating out. Its alert still reads this region's presence.
                    let map = match map {
                        Some(map) => map,
                        None => {
                            let read = inputs::PresenceMap::read(memory, pointer)?;
                            map = Some(read);
                            read
                        }
                    };
                    match map.presence(memory, region)? {
                        None => row.ring = false,
                        Some(counted) => {
                            row.ring = true;
                            row.counted = counted;
                        }
                    }
                }
                self.entries.len += count;
                self.ledgers.rows[len] = row;
                len += 1;
                Ok(())
            })?;
        }
        Ok(finish(len, unread))
    }

    /// The SAME original ledger pass serves the strict icon rows and native
    /// nullable transport occurrences. Physical manager order and controls are
    /// borrowed from country_counters; no owner/navy getter or second pass.
    fn read_transport_ledgers<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        holder: &mut super::country_naval_movement_inputs::Captured,
        shared: &mut super::transport_mission_shared::Shared,
        original_failure: Option<&'static str>,
    ) -> Result<Outcome, &'static str> {
        let count = holder.physical_count()?;
        let original_attempts_complete = self.census.required <= self.census.first.len();
        let census = &mut self.census.first[..self.census.len];
        census.sort_unstable_by_key(|member| member.task);
        let mut strict_error = original_failure
            .or((!original_attempts_complete).then_some("naval mission ledger census truncated"));
        let mut len = 0_usize;
        let mut unread = 0_u32;
        let (mut observer, receiver_error) = match if self.growing {
            self.transport.observer_growing(memory, base)
        } else {
            self.transport.observer(memory, base)
        } {
            Ok(observer) => (Some(observer), None),
            Err(reason) => {
                shared.refuse_receivers(reason);
                (None, Some(reason))
            }
        };
        for index in 0..count {
            let slot = u32::try_from(index).map_err(|_| "transport physical navy slot overflow")?;
            let owner = if strict_error.is_some() || slot == 0 || slot >= countries.count() {
                None
            } else {
                match countries.at(slot) {
                    Ok(owner) => owner,
                    Err(reason) => {
                        strict_error.get_or_insert(reason);
                        None
                    }
                }
            };
            let original = holder.borrow_navy(slot).and_then(|inputs| {
                let strict_owner = owner.map(|owner| {
                    inputs.observe_owner(memory).and_then(|raw| {
                        if countries.resolve(raw)?.pointer != owner.pointer {
                            return Err("navy identity or owner mismatch");
                        }
                        Ok(())
                    })
                });
                Ok((
                    inputs.present,
                    inputs.ledger_navy()?,
                    inputs.take_ledger_regions(),
                    inputs.take_access_modes(),
                    strict_owner,
                ))
            });
            let (present, pointer, outer, access_modes, strict_owner) = match original {
                Ok(original) => original,
                Err(reason) => {
                    shared.navy(slot, false, Err(reason));
                    if owner.is_some() {
                        strict_error.get_or_insert(reason);
                    }
                    continue;
                }
            };
            if let Some(Err(reason)) = strict_owner {
                strict_error.get_or_insert(reason);
            }
            shared.navy(slot, present, outer.map(|outer| outer.count));
            shared.access_modes(memory, access_modes);
            let outer = match outer {
                Ok(outer) => outer,
                Err(reason) => {
                    shared.refuse_ledgers(reason);
                    if owner.is_some() {
                        strict_error.get_or_insert(reason);
                    }
                    continue;
                }
            };
            let mut presence = None;
            let result = inputs::ledger_regions_in(memory, outer, |region, ledger| {
                if self.growing {
                    engine::walk::Buffer::Growing(&mut self.scratch)
                        .ensure(ledger.count as usize)?;
                    engine::walk::Buffer::Growing(&mut self.nullable)
                        .ensure(ledger.count as usize)?;
                    engine::walk::Buffer::Growing(&mut self.ledgers.rows).ensure(
                        len.checked_add(1)
                            .ok_or("naval mission ledger row demand overflow")?,
                    )?;
                    engine::walk::Buffer::Growing(&mut self.entries.first).ensure(
                        self.entries
                            .len
                            .checked_add(ledger.count as usize)
                            .ok_or("naval mission ledger demand overflow")?,
                    )?;
                }
                let observation =
                    transport_native::observe_ledger(memory, ledger, &mut self.scratch);
                let required = observation.required;
                self.entries.required = self.entries.required.saturating_add(required);
                let original_nullable = observation.nullable.and_then(|count| {
                    let output = self
                        .nullable
                        .get_mut(..count)
                        .ok_or("transport occurrence scratch arena exhausted")?;
                    for (at, &mission) in self.scratch[..count].iter().enumerate() {
                        let original_header = mission
                            .checked_sub(engine::ctask_force::NAVAL_MISSION)
                            .and_then(|task| {
                                census
                                    .binary_search_by_key(&task, |member| member.task)
                                    .ok()
                            })
                            .map(|index| census[index].mission_header)
                            .unwrap_or_else(|| {
                                (!original_attempts_complete).then_some(Err(
                                    "transport original mission header attempt coverage exhausted",
                                ))
                            });
                        let original_owner = mission
                            .checked_sub(engine::ctask_force::NAVAL_MISSION)
                            .and_then(|task| {
                                census
                                    .binary_search_by_key(&task, |member| member.task)
                                    .ok()
                            })
                            .map(|index| census[index].mission_owner)
                            .unwrap_or_else(|| {
                                (!original_attempts_complete).then_some(Err(
                                    "transport original mission owner attempt coverage exhausted",
                                ))
                            });
                        output[at] = match observer.as_mut() {
                            Some(observer) => observer.observe_transport(
                                mission,
                                original_header,
                                original_owner,
                            )?,
                            None if mission == 0 => 0,
                            None => {
                                return Err(receiver_error
                                    .unwrap_or("transport receiver observer unavailable"));
                            }
                        };
                    }
                    Ok(count)
                });
                shared.ledger(
                    region,
                    required,
                    original_nullable.map(|count| &self.nullable[..count]),
                );
                // An independent nullable refusal cannot change the original
                // strict icon result, its row capacities or first failure.
                if strict_error.is_none()
                    && let Some(owner) = owner
                {
                    match observation.strict {
                        Err(reason) => {
                            strict_error = Some(reason);
                        }
                        Ok(count) => {
                            if len == self.ledgers.rows.len()
                                || count > self.entries.first.len().saturating_sub(self.entries.len)
                            {
                                unread = unread.saturating_add(1);
                            } else {
                                for (at, &mission) in self.scratch[..count].iter().enumerate() {
                                    let member = mission
                                        .checked_sub(engine::ctask_force::NAVAL_MISSION)
                                        .and_then(|task| {
                                            census
                                                .binary_search_by_key(&task, |member| member.task)
                                                .ok()
                                        })
                                        .map(|index| census[index]);
                                    let Some(member) = member else {
                                        strict_error = Some(
                                            "navy ledger names a task force outside the fleet census",
                                        );
                                        break;
                                    };
                                    self.entries.first[self.entries.len + at] = member.id;
                                }
                                if strict_error.is_none() {
                                    let map = match presence {
                                        Some(map) => Ok(map),
                                        None => inputs::PresenceMap::read(memory, pointer)
                                            .inspect(|map| presence = Some(*map)),
                                    };
                                    let counted = map.and_then(|map| map.presence(memory, region));
                                    match counted {
                                        Ok(counted) => {
                                            self.ledgers.rows[len] = LedgerRow {
                                                owner: owner.id,
                                                region,
                                                first: self.entries.len,
                                                count,
                                                ring: counted.is_some(),
                                                counted: counted.unwrap_or_default(),
                                            };
                                            self.entries.len += count;
                                            len += 1;
                                        }
                                        Err(reason) => strict_error = Some(reason),
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(())
            });
            if let Err(reason) = result {
                shared.refuse_ledgers(reason);
                if owner.is_some() {
                    strict_error.get_or_insert(reason);
                }
            }
        }
        shared.retain_access_demand(holder.known_access_modes());
        // The original icon pass required a navy for every positive canonical
        // country. Preserve its first failure before checking missing tail
        // slots; native null/empty managers remain valid for transport alone.
        if strict_error.is_none() {
            for slot in (u32::try_from(count).unwrap_or(u32::MAX).max(1))..countries.count() {
                match countries.at(slot) {
                    Ok(Some(_)) => {
                        strict_error = Some("navy manager country outside array");
                        break;
                    }
                    Err(reason) => {
                        strict_error = Some(reason);
                        break;
                    }
                    Ok(None) => {}
                }
            }
        }
        let required = observer.as_ref().map_or(0, |observer| observer.required());
        drop(observer);
        self.transport.finish_identities(required);
        match strict_error {
            Some(reason) => Ok(Outcome::Unavailable { reason }),
            None => Ok(finish(len, unread)),
        }
    }

    /// The rows of one roster country, contiguous in walk order.
    fn owned(rows: &[TaskForce], slot: u32) -> &[TaskForce] {
        let first = rows.partition_point(|task_force| task_force.slot < slot);
        let end = rows.partition_point(|task_force| task_force.slot <= slot);
        &rows[first..end]
    }

    /// Scan every named country's fingerprint; the walk saw every mission task force, so the
    /// named set is the family's complete census.
    fn scan(&mut self, fingerprints: &mut SliceFingerprints) {
        for index in 0..self.named.len {
            let country = self.named.first[index];
            let value = fingerprint(
                Self::owned(Self::task_forces(&self.task_forces), country.canonical_slot),
                &self.headquarters.first,
            );
            self.selection.scan(fingerprints, country.id, Some(value));
        }
        fingerprints.finish_scan();
        self.selection.finish_census(fingerprints);
    }

    /// The selected countries' rows. Returns the two outcomes and the commander read ticks.
    fn read_family(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
    ) -> (Outcome, Outcome, u64) {
        let resolve = |handle| creference_registry::resolve(memory, base, handle);
        let mut country_rows = (0, 0_u32);
        let mut commander_rows = (0, 0_u32);
        let mut country_failure = None;
        let mut commander_failure = None;
        let mut commander_ticks = 0;
        for index in 0..self.named.len {
            let country = self.named.first[index];
            if !self.selection.contains(country.id) {
                continue;
            }
            if country_failure.is_none()
                && let Err(reason) =
                    self.read_country(memory, base, game_state, country, &mut country_rows)
            {
                country_failure = Some(reason);
            }
            if commander_failure.is_none() {
                let _reads = crate::read_accounting::table(TABLES[3]);
                let started = qpc();
                if let Err(reason) = self.read_commanders(
                    memory,
                    base,
                    country.canonical_slot,
                    country.id,
                    &resolve,
                    &mut commander_rows,
                ) {
                    commander_failure = Some(reason);
                }
                commander_ticks += elapsed(started);
            }
        }
        let outcome = |failure: Option<&'static str>, (len, unread)| {
            failure.map_or_else(
                || finish(len, unread),
                |reason| Outcome::Unavailable { reason },
            )
        };
        (
            outcome(country_failure, country_rows),
            outcome(commander_failure, commander_rows),
            commander_ticks,
        )
    }

    fn read_country(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        country: ccountry::Country,
        (len, unread): &mut (usize, u32),
    ) -> Result<(), &'static str> {
        if self.growing {
            engine::walk::Buffer::Growing(&mut self.countries.rows).ensure(
                len.checked_add(1)
                    .ok_or("naval mission country row demand overflow")?,
            )?;
        }
        let Some(row) = self.countries.rows.get_mut(*len) else {
            *unread = unread.saturating_add(1);
            return Ok(());
        };
        let navy = cstrategic_navy_manager::navy(memory, base, game_state, country)?;
        let count = inputs::navy_hubs_buffer(
            memory,
            navy,
            if self.growing {
                engine::walk::Buffer::GrowingFrom(&mut self.hubs.first, self.hubs.len)
            } else {
                engine::walk::Buffer::Fixed(&mut self.hubs.first[self.hubs.len..])
            },
        )?;
        self.hubs.required = self.hubs.required.saturating_add(count);
        if self.hubs.len + count > self.hubs.first.len() {
            *unread = unread.saturating_add(1);
            return Ok(());
        }
        row.hubs = self.hubs.len;
        row.hub_count = count;
        self.hubs.len += count;
        row.country = country.id;
        accumulator::lookup_many(memory, country.pointer, &COUNTRY_KEYS, &mut row.values)?;
        *len += 1;
        Ok(())
    }

    /// A country's distinct fleet leaders (source 0) and headquarters commanders (sources 1
    /// to 4) across its task forces.
    fn read_commanders(
        &mut self,
        memory: &impl Memory,
        base: usize,
        slot: u32,
        country: u32,
        resolve: inputs::Resolve<'_>,
        (len, unread): &mut (usize, u32),
    ) -> Result<(), &'static str> {
        if self.growing {
            let bound = self.task_forces.outcome.len().max(self.headquarters.len);
            engine::walk::Buffer::Growing(&mut self.keys).ensure(bound)?;
            let additional = self
                .task_forces
                .outcome
                .len()
                .checked_add(
                    self.headquarters
                        .len
                        .checked_mul(4)
                        .ok_or("naval mission commander demand overflow")?,
                )
                .ok_or("naval mission commander demand overflow")?;
            engine::walk::Buffer::Growing(&mut self.commanders.rows).ensure(
                len.checked_add(additional)
                    .ok_or("naval mission commander row demand overflow")?,
            )?;
        }
        let owned = Self::owned(Self::task_forces(&self.task_forces), slot);
        let headquarters = &self.headquarters.first;
        let characters = |task_force: &TaskForce| {
            &headquarters
                [task_force.headquarters..task_force.headquarters + task_force.row.headquarters]
        };
        let commanders = &mut self.commanders.rows;
        let mut push = |reference, source, values| match commanders.get_mut(*len) {
            Some(row) => {
                *row = CommanderRow {
                    country,
                    reference,
                    source,
                    values,
                };
                *len += 1;
            }
            None => *unread = unread.saturating_add(1),
        };
        // One sort per source kind avoids a scan of every earlier task force/HQ.
        let mut used = 0;
        for task_force in owned {
            debug_assert_eq!(task_force.country, country);
            if task_force.row.leader != 0 {
                self.keys[used] = task_force.row.leader;
                used += 1;
            }
        }
        let used = distinct(&mut self.keys[..used], |key| key);
        for &leader in &self.keys[..used] {
            let mut values = [Value::default(); 4];
            inputs::leader_modifiers(memory, base, leader, resolve, &mut values)?;
            push(leader, 0, values);
        }
        let mut used = 0;
        for task_force in owned {
            for &character in characters(task_force) {
                if character != 0 {
                    self.keys[used] = character;
                    used += 1;
                }
            }
        }
        let used = distinct(&mut self.keys[..used], |key| key);
        for &character in &self.keys[..used] {
            let mut sources = [[Value::default(); 4]; 4];
            inputs::headquarters_modifiers(memory, base, character, resolve, &mut sources)?;
            for (source, values) in (1..).zip(sources) {
                push(character, source, values);
            }
        }
        Ok(())
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.task_forces.outcome,
            self.ledgers.outcome,
            self.countries.outcome,
            self.commanders.outcome,
        ]
    }

    pub fn tables(&self) -> [TableView; N] {
        let value = |value: Value| value.present.then_some(value.raw);
        let task_forces = {
            let rows = Self::task_forces(&self.task_forces);
            let arena = &self.headquarters.first[..self.headquarters.len];
            table::<tables::NavalMissionTaskForces, _>(self.task_forces.outcome, rows, |t| {
                let row = &t.row;
                let [
                    local_coordination,
                    local_range,
                    local_range_factor,
                    local_raiding,
                ] = row.local.map(value);
                let [
                    weather_coordination,
                    weather_range,
                    weather_range_factor,
                    weather_raiding,
                ] = t.weather.map(value);
                NavalMissionTaskForceRow {
                    task_force: row.id,
                    mission_owner: row.mission_owner,
                    raiding_factor: row.raiding_factor,
                    range_base: row.range_base,
                    fuel: row.fuel[0],
                    fuel_requested: row.fuel[1],
                    underway: row.underway,
                    replenishment: row.replenishment[0],
                    replenishment_limit: row.replenishment[1],
                    local_range,
                    local_range_factor,
                    weather_range,
                    weather_range_factor,
                    local_coordination,
                    local_raiding,
                    weather_coordination,
                    weather_raiding,
                    leader: row.leader,
                    headquarters: owned_vec(copy_vec(
                        &arena[t.headquarters..t.headquarters + row.headquarters],
                    )),
                    range_fleet: row.range_fleet,
                    range_begin: row.range[0],
                    range_end: row.range[1],
                    convoys: row.convoys,
                }
            })
        };
        let ledgers = {
            let rows = &self.ledgers.rows[..self.ledgers.outcome.len()];
            let arena = &self.entries.first[..self.entries.len];
            table::<tables::NavalMissionLedgers, _>(self.ledgers.outcome, rows, |row| {
                NavalMissionLedgerRow {
                    owner: row.owner,
                    region: row.region,
                    task_forces: owned_vec(copy_vec(&arena[row.first..row.first + row.count])),
                    ring: row.ring,
                    positive: row.counted.positive,
                    total: row.counted.total,
                }
            })
        };
        [
            task_forces,
            ledgers,
            table::<tables::NavalMissionCountries, _>(
                self.countries.outcome,
                &self.countries.rows,
                |row| {
                    let [
                        coordination,
                        range,
                        range_factor,
                        raiding,
                        escort,
                        weather_reduction,
                        underway,
                    ] = row.values.map(value);
                    NavalMissionCountryRow {
                        country: row.country,
                        coordination,
                        raiding,
                        escort,
                        weather_reduction,
                        range,
                        range_factor,
                        underway,
                        hubs: owned_vec(copy_vec(
                            &self.hubs.first[row.hubs..row.hubs + row.hub_count],
                        )),
                    }
                },
            ),
            table::<tables::NavalMissionCommanders, _>(
                self.commanders.outcome,
                &self.commanders.rows,
                |row| {
                    let [coordination, range, range_factor, raiding] = row.values.map(value);
                    NavalMissionCommanderRow {
                        country: row.country,
                        reference: row.reference,
                        source: row.source,
                        range,
                        range_factor,
                        coordination,
                        raiding,
                    }
                },
            ),
        ]
    }
}

#[cfg(test)]
#[path = "naval_mission_inputs_tests.rs"]
mod tests;
