//! Naval mission icon operands: strike/invasion active counts (P2), convoy efficiencies (P3) and the
//! navies' mission ledgers. The game computes the efficiencies on demand, so this pack records
//! their inputs ([`engine_native::naval_mission_inputs`]); `replay_derive::naval_mission_derive` evaluates
//! them.
//!
//! One walk of every country's fleets feeds the hourly task-force operands and names the
//! countries their roster, mission and expeditionary owners need. Each navy's live ledgers
//! retain stored order and escort presence, including unlisted entries still fading out;
//! they are read through the hour's physical navy pass
//! ([`super::country_naval_movement_inputs`]), which authenticated every navy once.
//! The `naval_missions` sliced family (`slicing::NAVAL_MISSIONS`) reads country
//! modifiers/hubs and fleet-leader/HQ accumulators. Its fingerprint is a certificate of
//! everything a country's rows read ([`Captured::certificate`]): its task forces' leader and
//! commander references, the naval tree family's certificates of its accumulator and its fleet
//! leaders' ([`crate::naval_tree_changes`]), its navy's stored hubs and its headquarters
//! commanders' skill objects. A country whose certificate moved is read within the hour, so its
//! rows equal the game's every hour; the daily slice reads every other country and compares it
//! with its previous read, and a difference disables every certificate. A navy whose
//! registered-mission set is empty has empty ledgers, so its region headers are not read
//! ([`crate::navy_ledger_changes`]).
use crate::{
    navy_ledger_changes::{Counts as LedgerCounts, Skip as LedgerSkip},
    sampling::qpc,
    slicing::{Digest, SliceFingerprints, SliceSelection},
    spooling::table,
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine_allocation::{copy_vec, filled, owned_vec, reserve, resize_default};
use engine_native::{
    Memory, ccountry, cfleet, creference_registry, cstrategic_navy, cstrategic_navy_manager,
    cweather_manager, modifier_accumulator as accumulator, modifier_accumulator::Value,
    naval_mission_inputs as inputs, naval_mission_inputs::COUNTRY_KEYS,
    naval_mission_inputs::MISSION_KINDS, naval_mission_inputs::Operands, walk::Outcome,
};
use replay_data::{
    rows::NavalMissionCommanderRow, rows::NavalMissionCountryRow, rows::NavalMissionLedgerRow,
    rows::NavalMissionTaskForceRow, rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;
use std::{mem::size_of, sync::Arc};

pub const N: usize = 8;
pub const TABLES: [TableId; N] = [
    TableId::NavalMissionTaskForces,
    TableId::NavalMissionLedgers,
    TableId::NavalMissionCountries,
    TableId::NavalMissionCommanders,
    TableId::NavalRegionalMines,
    TableId::NavalAccidentReports,
    TableId::NavalDominanceTaskForceRanges,
    TableId::NavalPatrolMissionInputs,
];
/// Every table follows the task-force walk, which is charged to NavalMissionTaskForces; each
/// table's own reads are charged to it.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[(
    "naval mission task forces, ledgers, countries and commanders",
    0,
    &[0, 1, 2, 3, 4, 5, 6, 7],
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
        use engine_native::{cpdx_array::Array, ctask_force};
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
                // Reach every attached HQ occurrence.
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
        self.hubs = self.hubs.max(hubs);
        Ok(())
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
    weather: [Value; 9],
    raiding_grid: Option<inputs::RaidingGrid>,
    additional_range: bool,
    patrol_context: bool,
    native_order: usize,
}

/// A walked task force, for mapping a ledger's mission pointers to handles.
#[derive(Clone, Copy, Debug, Default)]
struct Member {
    task: usize,
    id: u64,
    visit: Option<cfleet::TaskForceVisit>,
    native_order: usize,
    mission_header: Option<Result<[u8; 16], &'static str>>,
    mission_owner: Option<Result<u32, &'static str>>,
    original_selectors: inputs::OriginalSelectors,
    range_failure: Option<&'static str>,
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
    values: [Value; 12],
    escort_tree: Option<[usize; 2]>,
    hubs: usize,
    hub_count: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct CommanderRow {
    country: u32,
    reference: u64,
    source: u8,
    values: [Value; 9],
}

#[derive(Clone, Copy, Debug, Default)]
struct PatrolMember {
    id: u64,
    owner: Option<u32>,
    row: engine_native::naval_patrol_inputs::Operands,
    local: Option<Value>,
    weather: Option<Value>,
    leader: u64,
    headquarters: usize,
    headquarters_count: usize,
    headquarters_known: bool,
    existing_context: bool,
    targeted_spotting: Option<Value>,
}

pub struct Captured {
    growing: bool,
    weather_pending: bool,
    headquarters_pending: bool,
    task_forces: CaptureRows<TaskForce>,
    ranges_outcome: Outcome,
    patrol: Vec<PatrolMember>,
    patrol_outcome: Outcome,
    patrol_headquarters: Vec<(u64, Arc<Vec<u64>>)>,
    mission_headquarters: Vec<(u64, Arc<Vec<u64>>)>,
    empty_headquarters: Option<Arc<Vec<u64>>>,
    member_index: Vec<(u64, usize)>,
    task_index: Vec<(u64, usize)>,
    escort_owners: Vec<u32>,
    escort_trees: super::naval_escort_trees::Trees,
    convoy_designs: inputs::ConvoyDesigns,
    dominance_fleets: Vec<u64>,
    weather_len: usize,
    headquarters: Arena<u64>,
    census: Arena<Member>,
    ledgers: CaptureRows<LedgerRow>,
    mines: Vec<replay_data::rows::NavalRegionalMineRow>,
    mines_outcome: Outcome,
    mine_cache: crate::naval_mines::Captured,
    accidents: Vec<replay_data::rows::NavalAccidentReportRow>,
    accidents_outcome: Outcome,
    accident_cache: crate::naval_accidents::Captured,
    map_ticks: [u64; 2],
    map_roster_complete: bool,
    entries: Arena<u64>,
    /// One ledger's mission pointers; as long as the registration arena.
    scratch: Vec<usize>,
    countries: CaptureRows<CountryRow>,
    hubs: Arena<u32>,
    commanders: CaptureRows<CommanderRow>,
    /// The countries the walk named, sorted by canonical slot and deduplicated once the walk
    /// is complete.
    named: Arena<ccountry::Country>,
    /// Sort scratch for located regions and distinct commander references.
    keys: Vec<u64>,
    /// The distinct located regions' weather, in region order.
    weather: Vec<(u32, [Value; 9])>,
    /// The countries this hour's family read covers.
    selection: SliceSelection,
    /// This hour's ledger read of navies without registered missions.
    ledger_counts: LedgerCounts,
    /// A CCountry's or CUnitLeader's certificate ([`crate::naval_tree_changes::sequence`]).
    sequence: fn(usize) -> Option<(u64, u64)>,
    /// Burn-in (`naval_tree_verify`): every country every hour, each one whose certificate
    /// held still compared.
    verify: bool,
}

#[cfg(not(test))]
fn default_sequence() -> fn(usize) -> Option<(u64, u64)> {
    crate::naval_tree_changes::sequence
}

/// Tests: every receiver is observed and no writer runs, unless a test says otherwise.
#[cfg(test)]
fn default_sequence() -> fn(usize) -> Option<(u64, u64)> {
    |_| Some((1, 0))
}

/// Tests read slices unless a test asks for the burn-in.
const DEFAULT_VERIFY: bool = cfg!(not(test)) && crate::naval_tree_changes::VERIFY;

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
            weather_pending: false,
            headquarters_pending: false,
            task_forces: CaptureRows::new(size(capacity.task_forces)),
            ranges_outcome: Outcome::Rows { len: 0 },
            patrol: Vec::new(),
            patrol_outcome: Outcome::Rows { len: 0 },
            patrol_headquarters: Vec::new(),
            mission_headquarters: Vec::new(),
            empty_headquarters: storage.then(|| Arc::new(Vec::new())),
            member_index: Vec::new(),
            task_index: Vec::new(),
            escort_owners: Vec::new(),
            escort_trees: Default::default(),
            convoy_designs: Default::default(),
            dominance_fleets: Vec::new(),
            weather_len: 0,
            headquarters: Arena::new(size(capacity.headquarters)),
            census: Arena::new(size(capacity.census)),
            ledgers: CaptureRows::new(size(capacity.ledgers)),
            mines: Vec::new(),
            mines_outcome: Outcome::Rows { len: 0 },
            mine_cache: Default::default(),
            accidents: Vec::new(),
            accidents_outcome: Outcome::Rows { len: 0 },
            accident_cache: Default::default(),
            map_ticks: [0; 2],
            map_roster_complete: false,
            entries: Arena::new(size(capacity.entries)),
            scratch: owned_vec(filled(size(capacity.entries), 0)),
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
                (0, [Value::default(); 9]),
            )),
            selection: SliceSelection::new(storage),
            ledger_counts: LedgerCounts::default(),
            sequence: default_sequence(),
            verify: DEFAULT_VERIFY,
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
        // A discarded cut must not leave a partially maintained cache as a baseline.
        if self.headquarters_pending {
            self.mission_headquarters.clear();
        }
        self.weather_pending = false;
        self.headquarters_pending = false;
        self.task_forces.reset();
        self.ranges_outcome = Outcome::Rows { len: 0 };
        self.patrol.clear();
        self.patrol_outcome = Outcome::Rows { len: 0 };
        self.member_index.clear();
        self.task_index.clear();
        self.escort_owners.clear();
        self.escort_trees.reset();
        self.convoy_designs.clear();
        self.dominance_fleets.clear();
        self.weather_len = 0;
        self.headquarters.reset();
        self.census.reset();
        self.ledgers.reset();
        self.mines.clear();
        self.mines_outcome = Outcome::Rows { len: 0 };
        self.accidents.clear();
        self.accidents_outcome = Outcome::Rows { len: 0 };
        self.map_ticks = [0; 2];
        self.map_roster_complete = false;
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
            + self.mines.capacity() * size_of::<replay_data::rows::NavalRegionalMineRow>()
            + self.accidents.capacity() * size_of::<replay_data::rows::NavalAccidentReportRow>()
            + self.entries.allocated_bytes()
            + self.scratch.capacity() * size_of::<usize>()
            + self.countries.allocated_bytes()
            + self.hubs.allocated_bytes()
            + self.commanders.allocated_bytes()
            + self.named.first.capacity() * size_of::<ccountry::Country>()
            + self.keys.capacity() * size_of::<u64>()
            + self.weather.capacity() * size_of::<(u32, [Value; 9])>()
            + self.dominance_fleets.capacity() * size_of::<u64>()
            + self.patrol.capacity() * size_of::<PatrolMember>()
            + (self.member_index.capacity() + self.task_index.capacity())
                * size_of::<(u64, usize)>()
            + self.escort_owners.capacity() * size_of::<u32>()
            + self.escort_trees.allocated_bytes()
            + self.convoy_designs.allocated_bytes()
            + self.patrol_headquarters.capacity() * size_of::<(u64, Arc<Vec<u64>>)>()
            + self
                .patrol_headquarters
                .iter()
                .map(|(_, refs)| refs.capacity() * size_of::<u64>())
                .sum::<usize>()
            + self.mission_headquarters.capacity() * size_of::<(u64, Arc<Vec<u64>>)>()
            + self
                .mission_headquarters
                .iter()
                .map(|(_, refs)| refs.capacity() * size_of::<u64>())
                .sum::<usize>()
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

    /// The latest ledger read's skipped, read and compared navies.
    pub(crate) fn ledger_counts(&self) -> LedgerCounts {
        self.ledger_counts
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
        self.mines_outcome = outcome(4);
        self.accidents_outcome = outcome(5);
        self.ranges_outcome = outcome(6);
        self.patrol_outcome = outcome(7);
        self.selection.reset(hour, force_full);
        self.census.first.fill(Member::default());
        self.scratch.fill(0);
    }

    /// One hour's read beside the physical navy pass `holder` began for it.
    /// Every table is refreshed every hour; the ledgers borrow the holder's
    /// authenticated navies and skip, as `skip` allows, those without
    /// registered missions; the sliced tables read the countries the hour
    /// and, with a store, the changed fingerprints select.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn acquire_with_navies_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
        holder: &mut super::country_naval_movement_inputs::Captured,
        skip: LedgerSkip,
    ) -> [u64; N] {
        self.capture_impl(
            memory,
            base,
            game_state,
            hour,
            force_full,
            fingerprints,
            holder,
            true,
            skip,
        )
    }

    /// One hour's read with its own physical navy pass, as the Ordinary job
    /// runs it, into growing storage.
    #[cfg(test)]
    pub fn capture_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
    ) -> [u64; N] {
        self.capture_with_own_navies(
            memory,
            base,
            game_state,
            hour,
            force_full,
            fingerprints,
            true,
            LedgerSkip::NEVER,
        )
    }

    /// One hour's read with its own physical navy pass, as the Ordinary job
    /// runs it, into fixed storage.
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
        self.capture_with_own_navies(
            memory,
            base,
            game_state,
            hour,
            force_full,
            fingerprints,
            false,
            LedgerSkip::NEVER,
        )
    }

    /// A full-hour read into fixed storage with its own physical navy pass,
    /// whose ledgers skip navies without registered missions as `skip` allows.
    #[cfg(test)]
    pub(crate) fn capture_skipping(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        skip: LedgerSkip,
    ) -> [u64; N] {
        self.capture_with_own_navies(memory, base, game_state, hour, true, None, false, skip)
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn capture_with_own_navies(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
        growing: bool,
        skip: LedgerSkip,
    ) -> [u64; N] {
        use super::country_naval_movement_inputs as naval;
        let mut holder = naval::Captured::new(naval::Capacities::default());
        let mut launch = naval::Launch::new(naval::LaunchCapacities::default());
        let mut reader = holder.begin(memory, base, game_state, Some(hour), &mut launch);
        let ticks = self.capture_impl(
            memory,
            base,
            game_state,
            hour,
            force_full,
            fingerprints,
            &mut holder,
            growing,
            skip,
        );
        reader.clear();
        holder.finish();
        self.decode_owned();
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
        mut fingerprints: Option<&mut SliceFingerprints>,
        holder: &mut super::country_naval_movement_inputs::Captured,
        growing: bool,
        skip: LedgerSkip,
    ) -> [u64; N] {
        self.reset();
        self.growing = growing;
        self.empty_headquarters
            .get_or_insert_with(|| Arc::new(Vec::new()));
        self.ledger_counts = LedgerCounts::default();
        self.selection.reset(hour, force_full);
        let mut ticks = [0; N];
        let started = qpc();
        let mut census_complete = false;
        let walked = {
            let _reads = crate::read_accounting::table(TABLES[0]);
            ccountry::Reader::new(memory, base, game_state).inspect(|countries| {
                self.task_forces.outcome = match self.walk(memory, base, countries) {
                    Ok(outcome) => {
                        census_complete = self.census.required <= self.census.first.len();
                        // Weather consumes this freshly acquired row prefix. Its
                        // failure does not erase the independent roster witness.
                        self.task_forces.outcome = outcome;
                        self.read_weather(memory, base, game_state)
                            .map(|()| outcome)
                            .unwrap_or_else(|reason| Outcome::Unavailable { reason })
                    }
                    Err(reason) => Outcome::Unavailable { reason },
                };
            })
        };
        ticks[0] = elapsed(started);
        let countries = match walked {
            Ok(countries) => countries,
            Err(reason) => {
                self.fail_all(reason);
                return ticks;
            }
        };
        let started = qpc();
        self.ledgers.outcome = {
            let _reads = crate::read_accounting::table(TABLES[1]);
            self.read_ledgers(memory, base, &countries, holder, skip, census_complete)
                .unwrap_or_else(|reason| Outcome::Unavailable { reason })
        };
        ticks[1] = elapsed(started)
            .saturating_sub(self.map_ticks[0])
            .saturating_sub(self.map_ticks[1]);
        ticks[4] = self.map_ticks[0];
        ticks[5] = self.map_ticks[1];
        if !self.map_roster_complete
            && let Outcome::Unavailable { reason } = self.ledgers.outcome
        {
            // A complete country roster is required to prove omitted zero mines
            // and empty report lists. Preserve the other mission tables.
            self.mines_outcome = Outcome::Unavailable { reason };
            self.accidents_outcome = Outcome::Unavailable { reason };
        }
        let range_started = qpc();
        {
            let _reads = crate::read_accounting::table(TABLES[6]);
            self.ranges_outcome = if !self.growing {
                Outcome::Unavailable {
                    reason: "naval dominance sibling ranges require asynchronous census",
                }
            } else if !matches!(self.ledgers.outcome, Outcome::Rows { .. }) {
                Outcome::Unavailable {
                    reason: "naval dominance sibling ranges require complete mission ledgers",
                }
            } else {
                match self.read_dominance_ranges(memory, base, game_state, &countries) {
                    Ok(len) => Outcome::Rows { len },
                    Err(reason) => Outcome::Unavailable { reason },
                }
            };
        }
        ticks[6] = elapsed(range_started);
        let patrol_started = qpc();
        {
            let _reads = crate::read_accounting::table(TABLES[7]);
            self.patrol_outcome = if !self.growing {
                Outcome::Unavailable {
                    reason: "naval patrol source inputs require asynchronous census",
                }
            } else if !matches!(self.ledgers.outcome, Outcome::Rows { .. }) {
                Outcome::Unavailable {
                    reason: "naval patrol source inputs require complete mission ledgers",
                }
            } else {
                match self.read_patrol_members(memory, base, game_state, &countries) {
                    Ok(len) => Outcome::Rows { len },
                    Err(reason) => Outcome::Unavailable { reason },
                }
            };
            if self.growing && matches!(self.ledgers.outcome, Outcome::Rows { .. }) {
                // Tree acquisition remains within the existing certified owner slice.
                // A missing first-owner observation leaves that tree unavailable.
                if self.read_escort_demand(memory, base, &countries).is_err() {
                    self.escort_owners.clear();
                }
            }
        }
        ticks[7] = elapsed(patrol_started);
        if let Err(reason) = self.share_mission_headquarters() {
            self.task_forces.outcome = Outcome::Unavailable { reason };
            self.ranges_outcome = Outcome::Unavailable { reason };
        }
        if !matches!(self.task_forces.outcome, Outcome::Rows { .. })
            || self.named.required > self.named.first.len()
        {
            // The family's census and fingerprints need every task force: nothing more is
            // published, the restore reads the family's owners again and the growth follows
            // the walk's demand.
            let outcome = match self.task_forces.outcome {
                Outcome::Unavailable { reason } => Outcome::Unavailable { reason },
                _ => Outcome::Unavailable {
                    reason: "naval mission task-force walk truncated",
                },
            };
            self.countries.outcome = outcome;
            self.commanders.outcome = outcome;
            return ticks;
        }
        let named = &mut self.named.first[..self.named.len];
        self.named.len = distinct(named, |country| country.canonical_slot);
        let started = qpc();
        let _reads = crate::read_accounting::table(TABLES[2]);
        let mut receivers = None;
        if let Some(fingerprints) = fingerprints.as_deref_mut() {
            let scan = qpc();
            receivers = Some(self.scan(memory, base, game_state, fingerprints));
            self.selection.scan_ticks = elapsed(scan);
            if self.verify {
                self.selection.force_full = true;
            }
        }
        let (countries_outcome, commanders_outcome, commander_ticks) =
            self.read_family(memory, base, game_state);
        self.countries.outcome = countries_outcome;
        self.commanders.outcome = commanders_outcome;
        if let (Some(fingerprints), Some(receivers)) = (fingerprints, receivers) {
            let hashes = self.content_hashes();
            let (verified, mismatches) = self.selection.compare(fingerprints, hashes.as_deref());
            crate::naval_tree_changes::missions(receivers, verified, mismatches);
        }
        ticks[2] = elapsed(started).saturating_sub(commander_ticks);
        ticks[3] = commander_ticks;
        ticks
    }

    fn fail_all(&mut self, reason: &'static str) {
        let outcome = Outcome::Unavailable { reason };
        self.task_forces.outcome = outcome;
        self.ranges_outcome = outcome;
        self.patrol_outcome = outcome;
        self.ledgers.outcome = outcome;
        self.countries.outcome = outcome;
        self.commanders.outcome = outcome;
        self.mines_outcome = outcome;
        self.accidents_outcome = outcome;
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
        let mut raid_resolver = creference_registry::ScopedResolver::new(base);
        cfleet::visit_task_forces(memory, base, countries, |event| {
            let cfleet::Visit::TaskForce(visit, _) = event else {
                return Ok(());
            };
            census.required += 1;
            if self.growing {
                engine_native::walk::Buffer::Growing(&mut census.first).ensure(census.required)?;
                engine_native::walk::Buffer::Growing(rows).ensure(
                    len.checked_add(1)
                        .ok_or("naval mission row demand overflow")?,
                )?;
            }
            let member_index = census.len;
            if let Some(member) = census.first.get_mut(member_index) {
                *member = Member {
                    task: visit.task,
                    id: visit.id,
                    visit: self.growing.then_some(visit),
                    native_order: census.required - 1,
                    mission_header: None,
                    mission_owner: None,
                    original_selectors: inputs::OriginalSelectors::default(),
                    range_failure: None,
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
            let mut mission_header = None;
            let mut mission_owner = None;
            let mut original_selectors = inputs::OriginalSelectors::default();
            let attempt = inputs::read_task_force_transport_observed_buffer_with_stats(
                memory,
                base,
                visit,
                if self.growing {
                    engine_native::walk::Buffer::GrowingFrom(
                        &mut headquarters.first,
                        headquarters.len,
                    )
                } else if fits {
                    engine_native::walk::Buffer::Fixed(&mut headquarters.first[headquarters.len..])
                } else {
                    engine_native::walk::Buffer::Fixed(&mut [])
                },
                &mut mission_header,
                &mut mission_owner,
                &mut original_selectors,
            );
            if let Some(member) = census.first.get_mut(member_index) {
                member.mission_header = mission_header;
                member.mission_owner = mission_owner;
                member.original_selectors = original_selectors;
                member.range_failure = attempt.as_ref().err().copied();
            }
            let row = attempt?;
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
            let mut raiding_grid =
                inputs::read_raiding_grid(memory, base, visit, &mut raid_resolver);
            if self.growing
                && let Some(grid) = raiding_grid.as_mut()
                && let Some(owner) = grid.target.country
            {
                grid.design = self.convoy_designs.read(memory, base, countries, owner);
                let live = grid.targets.is_some_and(|targets| {
                    targets[0] == Some(true)
                        || targets[0] == Some(false) && targets[1] == Some(true)
                });
                if live && grid.design.variant != Some(0) {
                    grid.source_speed = inputs::read_source_speed(memory, Some(row.stat_block));
                }
            }
            rows[len] = TaskForce {
                row,
                slot: visit.country.canonical_slot,
                country: visit.country.id,
                headquarters: headquarters.len,
                weather: [Value::default(); 9],
                raiding_grid,
                additional_range: false,
                patrol_context: false,
                native_order: census.required - 1,
            };
            headquarters.len += row.headquarters;
            len += 1;
            Ok(())
        })?;
        Ok(finish(len, unread))
    }

    fn index_members(&mut self) -> Result<(), &'static str> {
        self.member_index.clear();
        self.task_index.clear();
        reserve(&mut self.member_index, self.census.len)
            .map_err(|_| "naval member index allocation failed")?;
        self.member_index.extend(
            self.census.first[..self.census.len]
                .iter()
                .enumerate()
                .map(|(index, member)| (member.id, index)),
        );
        self.member_index.sort_unstable_by_key(|entry| entry.0);
        let len = self.task_forces.outcome.len();
        reserve(&mut self.task_index, len).map_err(|_| "naval source index allocation failed")?;
        self.task_index.extend(
            self.task_forces.rows[..len]
                .iter()
                .enumerate()
                .map(|(index, row)| (row.row.id, index)),
        );
        self.task_index.sort_unstable_by_key(|entry| entry.0);
        Ok(())
    }

    /// The existing mission row and its demanded sibling ranges share each
    /// identity's immutable headquarters list until its observed values change.
    fn share_mission_headquarters(&mut self) -> Result<(), &'static str> {
        if matches!(self.task_forces.outcome, Outcome::Unavailable { .. }) {
            return Ok(());
        }
        self.task_index.clear();
        for row in &self.task_forces.rows[..self.task_forces.outcome.len()] {
            if row.patrol_context {
                continue;
            }
            let id = row.row.id;
            reserve(&mut self.task_index, 1)
                .map_err(|_| "naval mission headquarters identity allocation failed")?;
            self.task_index.push((id, 0));
            let refs =
                &self.headquarters.first[row.headquarters..row.headquarters + row.row.headquarters];
            match self
                .mission_headquarters
                .binary_search_by_key(&id, |entry| entry.0)
            {
                Ok(at) => {
                    if self.mission_headquarters[at].1.as_slice() != refs {
                        self.mission_headquarters[at].1 = if refs.is_empty() {
                            Arc::clone(
                                self.empty_headquarters
                                    .as_ref()
                                    .expect("headquarters storage initialized"),
                            )
                        } else {
                            Arc::new(
                                copy_vec(refs)
                                    .map_err(|_| "naval mission headquarters allocation failed")?,
                            )
                        };
                    }
                }
                Err(at) => {
                    reserve(&mut self.mission_headquarters, 1)
                        .map_err(|_| "naval mission headquarters cache allocation failed")?;
                    let refs = if refs.is_empty() {
                        Arc::clone(
                            self.empty_headquarters
                                .as_ref()
                                .expect("headquarters storage initialized"),
                        )
                    } else {
                        Arc::new(
                            copy_vec(refs)
                                .map_err(|_| "naval mission headquarters allocation failed")?,
                        )
                    };
                    self.mission_headquarters.insert(at, (id, refs));
                }
            }
        }
        self.headquarters_pending = true;
        Ok(())
    }

    /// Only the initial mission rows need weather fan-out. Sibling ranges
    /// already retain their copied weather and may reorder those initial rows.
    /// No native capability or writer decision reaches this FIFO tail.
    pub(crate) fn decode_owned(&mut self) {
        if std::mem::take(&mut self.weather_pending) {
            let known = &self.weather[..self.weather_len];
            for row in &mut self.task_forces.rows[..self.task_forces.outcome.len()] {
                if row.additional_range || row.patrol_context || row.row.region == 0 {
                    continue;
                }
                let at = known
                    .binary_search_by_key(&row.row.region, |(region, _)| *region)
                    .expect("every located mission region was read");
                row.weather = known[at].1;
            }
        }
        if std::mem::take(&mut self.headquarters_pending) {
            self.task_index.sort_unstable_by_key(|entry| entry.0);
            self.mission_headquarters.retain(|(id, _)| {
                self.task_index
                    .binary_search_by_key(id, |entry| entry.0)
                    .is_ok()
            });
        }
    }

    fn add_named(&mut self, country: ccountry::Country) -> Result<(), &'static str> {
        self.named.required += 1;
        if self.named.required > self.named.first.len() {
            let additional = self.named.required - self.named.first.len();
            reserve(&mut self.named.first, additional)
                .map_err(|_| "naval source country allocation failed")?;
            self.named.first.resize(self.named.required, NO_COUNTRY);
        }
        self.named.first[self.named.len] = country;
        self.named.len += 1;
        Ok(())
    }

    fn patrol_weather<'m, M: Memory>(
        &mut self,
        memory: &'m M,
        base: usize,
        game_state: usize,
        region: Option<u32>,
        weather: &mut Option<Result<cweather_manager::Reader<'m, M>, &'static str>>,
    ) -> Option<Value> {
        let region = region?;
        if region == 0 {
            return Some(Value::default());
        }
        match self.weather[..self.weather_len].binary_search_by_key(&region, |entry| entry.0) {
            Ok(at) => Some(self.weather[at].1[2]),
            Err(at) => {
                let reader = weather
                    .get_or_insert_with(|| cweather_manager::Reader::new(memory, base, game_state));
                let mut values = [Value::default(); 9];
                inputs::region_weather(memory, reader.as_ref().ok()?, region, &mut values).ok()?;
                engine_native::walk::Buffer::Growing(&mut self.weather)
                    .ensure(self.weather_len + 1)
                    .ok()?;
                self.weather.copy_within(at..self.weather_len, at + 1);
                self.weather[at] = (region, values);
                self.weather_len += 1;
                Some(values[2])
            }
        }
    }

    /// Distinct existing ledger members, retaining their full ordered source
    /// context in flat arenas. Optional failure never replaces mission output.
    fn read_patrol_members<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        game_state: usize,
        countries: &ccountry::Reader<'_, M>,
    ) -> Result<usize, &'static str> {
        if !matches!(self.task_forces.outcome, Outcome::Rows { .. })
            || self.census.required > self.census.first.len()
        {
            return Err("naval patrol source census incomplete");
        }
        self.index_members()?;
        let original = self.task_forces.outcome;
        let original_hq = self.headquarters.len;
        let original_named = (self.named.len, self.named.required);
        let result = (|| {
            engine_native::walk::Buffer::Growing(&mut self.keys).ensure(self.entries.len)?;
            self.keys[..self.entries.len].copy_from_slice(&self.entries.first[..self.entries.len]);
            let count = distinct(&mut self.keys[..self.entries.len], |key| key);
            reserve(&mut self.patrol, count)
                .map_err(|_| "naval patrol source allocation failed")?;
            let mut len = original.len();
            let mut resolver = creference_registry::ScopedResolver::new(base);
            let mut weather = None;
            for ordinal in 0..count {
                let id = self.keys[ordinal];
                let member_index = self
                    .member_index
                    .binary_search_by_key(&id, |entry| entry.0)
                    .ok()
                    .map(|index| self.member_index[index].1)
                    .ok_or("naval patrol member outside authenticated census")?;
                let member = self.census.first[member_index];
                let visit = member
                    .visit
                    .ok_or("naval patrol member lacks asynchronous witness")?;
                let header = member.mission_header.unwrap_or_else(|| {
                    engine_native::naval_patrol_inputs::read_header(memory, base, visit)
                });
                self.census.first[member_index].mission_header = Some(header);
                let known = self
                    .task_index
                    .binary_search_by_key(&id, |entry| entry.0)
                    .ok()
                    .map(|index| self.task_forces.rows[self.task_index[index].1]);
                let convoy_demand = visit.kind == inputs::RAIDING
                    && known
                        .and_then(|row| row.raiding_grid)
                        .and_then(|grid| grid.targets)
                        .is_some_and(|targets| {
                            targets[0] == Some(true)
                                || targets[0] == Some(false) && targets[1] == Some(true)
                        });
                let row = engine_native::naval_patrol_inputs::read_member_observed_stats(
                    memory,
                    base,
                    visit,
                    Some(header),
                    member
                        .original_selectors
                        .stats
                        .or_else(|| known.map(|source| Ok(source.row.stat_block))),
                    convoy_demand,
                    &mut resolver,
                );
                let mut packet = PatrolMember {
                    id,
                    row,
                    owner: member.mission_owner.and_then(Result::ok),
                    ..PatrolMember::default()
                };
                if convoy_demand {
                    packet.targeted_spotting = inputs::targeted_spotting::read(
                        memory,
                        base,
                        countries,
                        visit.country,
                        known.map(|source| source.row.location),
                    )
                    .ok();
                }
                if known.is_some() {
                    // Consumers obtain this context from its existing mission/range row.
                    packet.existing_context = true;
                } else if member.range_failure.is_none() {
                    let context = engine_native::naval_patrol_inputs::read_context_with_location(
                        memory,
                        base,
                        visit,
                        member.original_selectors.location,
                        engine_native::walk::Buffer::GrowingFrom(
                            &mut self.headquarters.first,
                            self.headquarters.len,
                        ),
                    );
                    packet.local = context.local_detection;
                    packet.weather =
                        self.patrol_weather(memory, base, game_state, context.region, &mut weather);
                    packet.leader = context.leader;
                    packet.headquarters = self.headquarters.len;
                    if let Ok(count) = context.headquarters {
                        packet.headquarters_count = count;
                        packet.headquarters_known = true;
                        self.headquarters.len += count;
                        self.headquarters.required =
                            self.headquarters.required.max(self.headquarters.len);
                    }
                    self.add_named(visit.country)?;
                    engine_native::walk::Buffer::Growing(&mut self.task_forces.rows)
                        .ensure(len + 1)?;
                    self.task_forces.rows[len] = TaskForce {
                        row: Operands {
                            id: visit.id,
                            kind: visit.kind,
                            mission_owner: visit.country.id,
                            leader: context.leader,
                            headquarters: packet.headquarters_count,
                            ..Operands::default()
                        },
                        slot: visit.country.canonical_slot,
                        country: visit.country.id,
                        headquarters: packet.headquarters,
                        additional_range: false,
                        patrol_context: true,
                        native_order: member.native_order,
                        ..TaskForce::default()
                    };
                    len += 1;
                }
                if !packet.existing_context && packet.headquarters_known {
                    let refs = &self.headquarters.first
                        [packet.headquarters..packet.headquarters + packet.headquarters_count];
                    match self
                        .patrol_headquarters
                        .binary_search_by_key(&id, |entry| entry.0)
                    {
                        Ok(at) => {
                            if self.patrol_headquarters[at].1.as_slice() != refs {
                                self.patrol_headquarters[at].1 = if refs.is_empty() {
                                    Arc::clone(
                                        self.empty_headquarters
                                            .as_ref()
                                            .expect("headquarters storage initialized"),
                                    )
                                } else {
                                    Arc::new(owned_vec(copy_vec(refs)))
                                };
                            }
                        }
                        Err(at) => {
                            reserve(&mut self.patrol_headquarters, 1)
                                .map_err(|_| "naval patrol headquarters cache allocation failed")?;
                            let held = if refs.is_empty() {
                                Arc::clone(
                                    self.empty_headquarters
                                        .as_ref()
                                        .expect("headquarters storage initialized"),
                                )
                            } else {
                                Arc::new(owned_vec(copy_vec(refs)))
                            };
                            self.patrol_headquarters.insert(at, (id, held));
                        }
                    }
                }
                self.patrol.push(packet);
            }
            self.task_forces.rows[..len].sort_unstable_by_key(|row| (row.slot, row.native_order));
            self.task_forces.outcome = Outcome::Rows { len };
            self.patrol_headquarters.retain(|(id, _)| {
                self.patrol
                    .binary_search_by_key(id, |packet| packet.id)
                    .is_ok_and(|at| {
                        !self.patrol[at].existing_context && self.patrol[at].headquarters_known
                    })
            });
            Ok(self.patrol.len())
        })();
        if result.is_err() {
            self.patrol.clear();
            self.task_forces.outcome = original;
            self.headquarters.len = original_hq;
            self.named.len = original_named.0;
            self.named.required = original_named.1;
        }
        result
    }

    /// Escort text resolves the first registration's owner before filtering
    /// mission kinds. An additional context never substitutes a roster owner.
    fn read_escort_demand<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
    ) -> Result<(), &'static str> {
        self.index_members()?;
        for index in 0..self.ledgers.outcome.len() {
            let ledger = self.ledgers.rows[index];
            let handles = &self.entries.first[ledger.first..ledger.first + ledger.count];
            let escort = handles.iter().any(|handle| {
                self.member_index
                    .binary_search_by_key(handle, |entry| entry.0)
                    .ok()
                    .and_then(|at| self.census.first[self.member_index[at].1].visit)
                    .is_some_and(|visit| visit.kind == inputs::ESCORT)
            });
            if !escort {
                continue;
            }
            let Some(&first) = handles.first() else {
                continue;
            };
            let at = self
                .member_index
                .binary_search_by_key(&first, |entry| entry.0)
                .ok()
                .map(|at| self.member_index[at].1)
                .ok_or("escort source first registration outside authenticated census")?;
            let member = self.census.first[at];
            let visit = member
                .visit
                .ok_or("escort source lacks asynchronous witness")?;
            let header = member.mission_header.unwrap_or_else(|| {
                engine_native::naval_patrol_inputs::read_header(memory, base, visit)
            });
            self.census.first[at].mission_header = Some(header);
            let owner = member.mission_owner.unwrap_or_else(|| {
                engine_native::naval_patrol_inputs::read_owner(memory, base, visit, header)
            });
            self.census.first[at].mission_owner = Some(owner);
            if let Ok(packet) = self.patrol.binary_search_by_key(&first, |packet| packet.id) {
                self.patrol[packet].owner = owner.ok();
            }
            let Ok(owner) = owner else {
                continue;
            };
            let Ok(country) = countries.resolve(owner) else {
                continue;
            };
            reserve(&mut self.escort_owners, 1)
                .map_err(|_| "escort source owner demand allocation failed")?;
            self.escort_owners.push(country.id);
            self.add_named(country)?;
        }
        self.escort_owners.sort_unstable();
        self.escort_owners.dedup();
        Ok(())
    }

    /// Additional range demand comes only from registered strike fleets. The
    /// original authenticated visits remain valid in this same frozen pass.
    /// Failure rolls back the optional suffix, preserving the mission family.
    fn read_dominance_ranges<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        game_state: usize,
        countries: &ccountry::Reader<'_, M>,
    ) -> Result<usize, &'static str> {
        let original = self.task_forces.outcome;
        if !matches!(original, Outcome::Rows { .. })
            || self.census.required > self.census.first.len()
        {
            return Err("naval dominance sibling range census incomplete");
        }
        let original_len = original.len();
        let original_hq = self.headquarters.len;
        let original_named = self.named.len;
        let original_named_required = self.named.required;
        let result = (|| {
            let entries = self.entries.len;
            engine_native::walk::Buffer::Growing(&mut self.keys).ensure(entries)?;
            self.keys[..entries].copy_from_slice(&self.entries.first[..entries]);
            self.keys[..entries].sort_unstable();
            for member in &self.census.first[..self.census.len] {
                let Some(visit) = member.visit else {
                    continue;
                };
                if visit.kind == 2 && self.keys[..entries].binary_search(&visit.id).is_ok() {
                    reserve(&mut self.dominance_fleets, 1)
                        .map_err(|_| "naval dominance fleet demand allocation failed")?;
                    self.dominance_fleets.push(visit.fleet_handle);
                }
            }
            self.dominance_fleets.sort_unstable();
            self.dominance_fleets.dedup();
            let mut len = original_len;
            let mut weather = None;
            for index in 0..self.census.len {
                let Some(visit) = self.census.first[index].visit else {
                    continue;
                };
                if MISSION_KINDS.contains(&visit.kind)
                    || self
                        .dominance_fleets
                        .binary_search(&visit.fleet_handle)
                        .is_err()
                {
                    continue;
                }
                let mut mission_header = None;
                let mut mission_owner = None;
                let mut original_selectors = inputs::OriginalSelectors::default();
                let attempt = inputs::read_task_force_transport_observed_buffer_with_stats(
                    memory,
                    base,
                    visit,
                    engine_native::walk::Buffer::GrowingFrom(
                        &mut self.headquarters.first,
                        self.headquarters.len,
                    ),
                    &mut mission_header,
                    &mut mission_owner,
                    &mut original_selectors,
                );
                self.census.first[index].mission_header = mission_header;
                self.census.first[index].mission_owner = mission_owner;
                self.census.first[index].original_selectors = original_selectors;
                self.census.first[index].range_failure = attempt.as_ref().err().copied();
                let row = attempt?;
                let mut values = [Value::default(); 9];
                if row.region != 0 {
                    match self.weather[..self.weather_len]
                        .binary_search_by_key(&row.region, |entry| entry.0)
                    {
                        Ok(at) => values = self.weather[at].1,
                        Err(at) => {
                            let reader = weather.get_or_insert_with(|| {
                                cweather_manager::Reader::new(memory, base, game_state)
                            });
                            inputs::region_weather(
                                memory,
                                reader.as_ref().map_err(|reason| *reason)?,
                                row.region,
                                &mut values,
                            )?;
                            engine_native::walk::Buffer::Growing(&mut self.weather)
                                .ensure(self.weather_len + 1)?;
                            self.weather.copy_within(at..self.weather_len, at + 1);
                            self.weather[at] = (row.region, values);
                            self.weather_len += 1;
                        }
                    }
                }
                let resolve = |raw| {
                    if raw == visit.country.id {
                        Ok(visit.country)
                    } else {
                        countries.resolve(raw)
                    }
                };
                let owner = resolve(row.mission_owner).ok();
                let expeditionary = (row.expeditionary_owner > 0)
                    .then(|| resolve(row.expeditionary_owner as u32).ok())
                    .flatten();
                for country in [Some(visit.country), owner, expeditionary]
                    .into_iter()
                    .flatten()
                {
                    self.named.required += 1;
                    if self.named.required > self.named.first.len() {
                        let additional = self.named.required - self.named.first.len();
                        reserve(&mut self.named.first, additional)
                            .map_err(|_| "naval dominance country demand allocation failed")?;
                        self.named.first.resize(self.named.required, NO_COUNTRY);
                    }
                    self.named.first[self.named.len] = country;
                    self.named.len += 1;
                }
                engine_native::walk::Buffer::Growing(&mut self.task_forces.rows).ensure(len + 1)?;
                self.task_forces.rows[len] = TaskForce {
                    row,
                    slot: visit.country.canonical_slot,
                    country: visit.country.id,
                    headquarters: self.headquarters.len,
                    weather: values,
                    raiding_grid: None,
                    additional_range: true,
                    patrol_context: false,
                    native_order: self.census.first[index].native_order,
                };
                self.headquarters.len += row.headquarters;
                self.headquarters.required = self.headquarters.required.max(self.headquarters.len);
                len += 1;
            }
            self.task_forces.rows[..len].sort_unstable_by_key(|row| (row.slot, row.native_order));
            self.task_forces.outcome = Outcome::Rows { len };
            Ok(len - original_len)
        })();
        if result.is_err() {
            self.task_forces.outcome = original;
            self.headquarters.len = original_hq;
            self.named.len = original_named;
            self.named.required = original_named_required;
        }
        result
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
    ) -> Result<(), &'static str> {
        let rows = self.task_forces.outcome.len();
        if self.growing {
            engine_native::walk::Buffer::Growing(&mut self.keys).ensure(rows)?;
            engine_native::walk::Buffer::Growing(&mut self.weather).ensure(rows)?;
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
        self.weather_len = regions;
        let weather = cweather_manager::Reader::new(memory, base, game_state);
        let weather = weather.as_ref().map_err(|reason| *reason);
        for index in 0..regions {
            let region = self.keys[index] as u32;
            let mut values = [Value::default(); 9];
            inputs::region_weather(memory, weather?, region, &mut values)?;
            self.weather[index] = (region, values);
        }
        // The map remains available to later native-demand decisions. Copying
        // its values into mission rows has no bearing on any of those reads.
        self.weather_pending = true;
        Ok(())
    }

    /// Every nonempty live (navy, region) ledger of each positive country's
    /// navy, borrowed from the hour's physical navy pass in manager order: its
    /// registrations in stored order and the navy's presence counts for the
    /// region (also read by unlisted escort records). The navy's owner is
    /// authenticated against the country at its slot, and the original icon
    /// pass requires a navy for every positive canonical country. A navy
    /// whose registered-mission set is empty
    /// ([`cstrategic_navy::REGISTERED_MISSIONS`]) has every ledger empty: as
    /// `skip` allows, its region headers are not read unless `skip.check`
    /// covers its slot, when a nonempty ledger is a mismatch.
    fn read_ledgers<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        holder: &mut super::country_naval_movement_inputs::Captured,
        skip: LedgerSkip,
        census_complete: bool,
    ) -> Result<Outcome, &'static str> {
        let count = holder.physical_count()?;
        let lifetime = crate::world_lifetime::token();
        crate::naval_map_changes::prepare(count);
        let mine_ready = self.mine_cache.prepare(lifetime, count);
        let accident_ready = self.accident_cache.prepare(lifetime, count);
        if let Err(reason) = mine_ready {
            self.mines_outcome = Outcome::Unavailable { reason };
        }
        if let Err(reason) = accident_ready {
            self.accidents_outcome = Outcome::Unavailable { reason };
        }
        let census = &mut self.census.first[..self.census.len];
        census.sort_unstable_by_key(|member| member.task);
        let mut len = 0_usize;
        let mut unread = 0_u32;
        for index in 0..count {
            let slot = u32::try_from(index).map_err(|_| "physical navy slot overflow")?;
            if slot == 0 || slot >= countries.count() {
                continue;
            }
            let Some(owner) = countries.at(slot)? else {
                continue;
            };
            let navy = holder.borrow_navy(slot)?;
            let raw = navy.observe_owner(memory)?;
            if countries.resolve(raw)?.pointer != owner.pointer {
                return Err("navy identity or owner mismatch");
            }
            let pointer = navy.ledger_navy()?;
            // Map facts are independent of mission registration. The physical
            // pass already authenticated this navy and its current country.
            let begin = qpc();
            if mine_ready.is_ok() {
                let _reads = crate::read_accounting::table(TABLES[4]);
                let rows = &mut self.mines;
                let result = self.mine_cache.country(memory, pointer, owner.id, |row| {
                    if rows.len() == rows.capacity() {
                        reserve(rows, 1).map_err(|_| "naval mine output allocation failed")?;
                    }
                    rows.push(row);
                    Ok(())
                });
                if let Err(reason) = result {
                    self.mines_outcome = Outcome::Unavailable { reason };
                }
            }
            self.map_ticks[0] = self.map_ticks[0].saturating_add(elapsed(begin));
            let begin = qpc();
            if accident_ready.is_ok() {
                let _reads = crate::read_accounting::table(TABLES[5]);
                let certificate = crate::naval_map_changes::accident_token(pointer);
                let rows = &mut self.accidents;
                let result = self.accident_cache.country(
                    memory,
                    base,
                    pointer,
                    owner.id,
                    certificate,
                    |row| {
                        if rows.len() == rows.capacity() {
                            reserve(rows, 1)
                                .map_err(|_| "naval accident output allocation failed")?;
                        }
                        rows.push(row);
                        Ok(())
                    },
                );
                if let Err(reason) = result {
                    self.accidents_outcome = Outcome::Unavailable { reason };
                } else if certificate != crate::naval_map_changes::accident_token(pointer) {
                    self.accidents_outcome = Outcome::Unavailable {
                        reason: "naval accident writer changed during acquisition",
                    };
                }
            }
            self.map_ticks[1] = self.map_ticks[1].saturating_add(elapsed(begin));
            if !census_complete {
                // Mine amounts and incident reports have no task-force dependency.
                // Preserve them even if the mission census cannot join a ledger.
                continue;
            }
            let outer = navy.take_ledger_regions()?;
            // Append, remove and reset write a
            // ledger and the set together: no registered mission, no entry.
            let empty =
                skip.enabled && cstrategic_navy::registered_missions(memory, pointer) == Ok(0);
            if empty && !skip.check.covers(u64::from(slot)) {
                self.ledger_counts.skipped += 1;
                continue;
            }
            self.ledger_counts.read += 1;
            let mut filed = false;
            let mut presence = None;
            inputs::ledger_regions_in(memory, outer, |region, ledger| {
                filed = true;
                if self.growing {
                    engine_native::walk::Buffer::Growing(&mut self.scratch)
                        .ensure(ledger.count as usize)?;
                    engine_native::walk::Buffer::Growing(&mut self.ledgers.rows).ensure(
                        len.checked_add(1)
                            .ok_or("naval mission ledger row demand overflow")?,
                    )?;
                    engine_native::walk::Buffer::Growing(&mut self.entries.first).ensure(
                        self.entries
                            .len
                            .checked_add(ledger.count as usize)
                            .ok_or("naval mission ledger demand overflow")?,
                    )?;
                }
                let count = inputs::ledger(memory, ledger, &mut self.scratch)?;
                self.entries.required = self.entries.required.saturating_add(count);
                if len == self.ledgers.rows.len()
                    || count > self.entries.first.len().saturating_sub(self.entries.len)
                {
                    unread = unread.saturating_add(1);
                    return Ok(());
                }
                for (at, &mission) in self.scratch[..count].iter().enumerate() {
                    let member = mission
                        .checked_sub(engine_native::ctask_force::NAVAL_MISSION)
                        .and_then(|task| {
                            census
                                .binary_search_by_key(&task, |member| member.task)
                                .ok()
                        })
                        .map(|index| census[index])
                        .ok_or("navy ledger names a task force outside the fleet census")?;
                    self.entries.first[self.entries.len + at] = member.id;
                }
                // The naval mission updater updates every record, including an unlisted escort
                // animating out. Its alert still reads this region's presence.
                let map = match presence {
                    Some(map) => map,
                    None => {
                        let read = inputs::PresenceMap::read(memory, pointer)?;
                        presence = Some(read);
                        read
                    }
                };
                let counted = map.presence(memory, region)?;
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
                Ok(())
            })?;
            if empty {
                self.ledger_counts.verified += 1;
                self.ledger_counts.mismatches += u64::from(filed);
            }
        }
        // The original icon pass required a navy for every positive
        // canonical country beyond the manager's physical slots too.
        for slot in (u32::try_from(count).unwrap_or(u32::MAX).max(1))..countries.count() {
            if countries.at(slot)?.is_some() {
                return Err("navy manager country outside array");
            }
        }
        if matches!(self.mines_outcome, Outcome::Rows { .. }) {
            self.mines_outcome = Outcome::Rows {
                len: self.mines.len(),
            };
        }
        if matches!(self.accidents_outcome, Outcome::Rows { .. }) {
            self.accidents_outcome = Outcome::Rows {
                len: self.accidents.len(),
            };
        }
        self.map_roster_complete = true;
        Ok(if census_complete {
            finish(len, unread)
        } else {
            Outcome::Unavailable {
                reason: "naval mission ledger census truncated",
            }
        })
    }

    /// The rows of one roster country, contiguous in walk order.
    fn owned(rows: &[TaskForce], slot: u32) -> &[TaskForce] {
        let first = rows.partition_point(|task_force| task_force.slot < slot);
        let end = rows.partition_point(|task_force| task_force.slot <= slot);
        &rows[first..end]
    }

    /// Scan every named country's certificate; the walk saw every mission task force, so the
    /// named set is the family's complete census. Returns the receivers the certificates key.
    fn scan(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        fingerprints: &mut SliceFingerprints,
    ) -> usize {
        let resolve = |handle| creference_registry::resolve(memory, base, handle);
        let mut receivers = 0;
        for index in 0..self.named.len {
            let country = self.named.first[index];
            let certificate =
                self.certificate(memory, base, game_state, country, &resolve, &mut receivers);
            self.selection
                .scan_marking(fingerprints, country.id, certificate);
        }
        fingerprints.finish_scan();
        self.selection.finish_census(fingerprints);
        receivers
    }

    /// What `country`'s family rows read, beside the accumulator entries a hooked writer certifies:
    /// task-force leader and commander references; the country's Tree certificate  (modifier
    /// recalculation and appearance reset rebuild its accumulator); the navy's stored hubs (a short
    /// array refreshed by reset and append); fleet leaders' tree certificates (recalculation, HQ
    /// scaling, construction and destruction rebuild the leader's accumulator); and each HQ
    /// commander's unit leader and four skill objects, selected by four component setters. Their
    /// source accumulators are content. An unreadable or unobserved part returns `None`, so the
    /// country is read every hour.
    fn certificate(
        &self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        country: ccountry::Country,
        resolve: inputs::Resolve<'_>,
        receivers: &mut usize,
    ) -> Option<u64> {
        let owned = Self::owned(Self::task_forces(&self.task_forces), country.canonical_slot);
        let mut digest = Digest::new();
        digest.u64(fingerprint(owned, &self.headquarters.first));
        digest.u64(u64::from(
            self.escort_owners.binary_search(&country.id).is_ok(),
        ));
        let mut certify = |digest: &mut Digest, receiver: usize| {
            let (epoch, sequence) = (self.sequence)(receiver)?;
            *receivers += 1;
            digest.u64(receiver as u64).u64(epoch).u64(sequence);
            Some(())
        };
        certify(&mut digest, country.pointer)?;
        let navy = cstrategic_navy_manager::navy(memory, base, game_state, country).ok()?;
        let hubs =
            engine_native::cpdx_array::Array::read(memory, navy.checked_add(inputs::HUBS)?, 4)?;
        digest.u64(u64::from(hubs.count));
        let mut chunk = [0_u8; 256];
        let total = hubs.count as usize * 4;
        let mut done = 0;
        while done < total {
            let len = (total - done).min(chunk.len());
            if !engine_native::memory::read_slice(memory, hubs.data + done, &mut chunk[..len]) {
                return None;
            }
            digest.bytes(&chunk[..len]);
            done += len;
        }
        for task_force in owned {
            if task_force.row.leader != 0 {
                match inputs::fleet_leader(memory, base, task_force.row.leader, resolve).ok()? {
                    Some(leader) => certify(&mut digest, leader)?,
                    None => {
                        digest.u64(0);
                    }
                }
            }
            for &character in &self.headquarters.first
                [task_force.headquarters..task_force.headquarters + task_force.row.headquarters]
            {
                if character == 0 {
                    continue;
                }
                match inputs::headquarters_sources(memory, base, character, resolve).ok()? {
                    Some((leader, sources)) => {
                        digest.u64(leader as u64);
                        for source in sources {
                            digest.u64(source as u64);
                        }
                    }
                    None => {
                        digest.u64(0);
                    }
                }
            }
        }
        Some(digest.finish())
    }

    /// Each country this hour's complete family read covers, ascending, with a hash of its
    /// rows; `None` when either table is incomplete.
    fn content_hashes(&self) -> Option<Vec<(u32, u64)>> {
        let (Outcome::Rows { len: countries }, Outcome::Rows { len: commanders }) =
            (self.countries.outcome, self.commanders.outcome)
        else {
            return None;
        };
        let value = |digest: &mut Digest, value: Value| {
            digest.i64(value.raw).u64(u64::from(value.present));
        };
        let mut hashes = engine_allocation::vec(countries).ok()?;
        // Each country's row and then its commanders were appended in turn.
        let mut commander_rows = self.commanders.rows[..commanders].iter().peekable();
        for row in &self.countries.rows[..countries] {
            let mut digest = Digest::new();
            for entry in row.values {
                value(&mut digest, entry);
            }
            digest.u64(self.escort_trees.hash(row.escort_tree));
            for &hub in &self.hubs.first[row.hubs..row.hubs + row.hub_count] {
                digest.u64(u64::from(hub));
            }
            while let Some(commander) =
                commander_rows.next_if(|commander| commander.country == row.country)
            {
                digest
                    .u64(commander.reference)
                    .u64(u64::from(commander.source));
                for entry in commander.values {
                    value(&mut digest, entry);
                }
            }
            hashes.push((row.country, digest.finish()));
        }
        hashes.sort_unstable_by_key(|&(country, _)| country);
        Some(hashes)
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
            engine_native::walk::Buffer::Growing(&mut self.countries.rows).ensure(
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
                engine_native::walk::Buffer::GrowingFrom(&mut self.hubs.first, self.hubs.len)
            } else {
                engine_native::walk::Buffer::Fixed(&mut self.hubs.first[self.hubs.len..])
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
        row.escort_tree = self.escort_trees.read(
            memory,
            base,
            country,
            self.escort_owners.binary_search(&country.id).is_ok(),
            (self.sequence)(country.pointer),
        );
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
            engine_native::walk::Buffer::Growing(&mut self.keys).ensure(bound)?;
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
            engine_native::walk::Buffer::Growing(&mut self.commanders.rows).ensure(
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
            let mut values = [Value::default(); 9];
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
            let mut sources = [[Value::default(); 9]; 4];
            inputs::headquarters_modifiers(memory, base, character, resolve, &mut sources)?;
            for (source, values) in (1..).zip(sources) {
                push(character, source, values);
            }
        }
        Ok(())
    }

    fn original_mission_outcome(&self) -> Outcome {
        match self.task_forces.outcome {
            Outcome::Rows { .. } => Outcome::Rows {
                len: Self::task_forces(&self.task_forces)
                    .iter()
                    .filter(|row| !row.additional_range && !row.patrol_context)
                    .count(),
            },
            other => other,
        }
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.original_mission_outcome(),
            self.ledgers.outcome,
            self.countries.outcome,
            self.commanders.outcome,
            self.mines_outcome,
            self.accidents_outcome,
            self.ranges_outcome,
            self.patrol_outcome,
        ]
    }

    pub fn tables(&self) -> [TableView; N] {
        assert!(
            !self.weather_pending && !self.headquarters_pending,
            "naval mission operands not decoded"
        );
        let value = |value: Value| value.present.then_some(value.raw);
        let convert = |t: &TaskForce| {
            let row = &t.row;
            let [
                local_coordination,
                local_speed,
                local_detection,
                local_spotting,
                local_range,
                local_range_factor,
                local_raiding,
                local_night_spotting,
                local_visibility,
            ] = row.local.map(value);
            let [
                weather_coordination,
                weather_speed,
                weather_detection,
                weather_spotting,
                weather_range,
                weather_range_factor,
                weather_raiding,
                weather_night_spotting,
                weather_visibility,
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
                local_speed,
                local_detection,
                local_spotting,
                local_night_spotting,
                local_visibility,
                weather_coordination,
                weather_raiding,
                weather_speed,
                weather_detection,
                weather_spotting,
                weather_night_spotting,
                weather_visibility,
                leader: row.leader,
                headquarters: self
                    .mission_headquarters
                    .binary_search_by_key(&row.id, |entry| entry.0)
                    .ok()
                    .map_or_else(
                        || {
                            Arc::clone(
                                self.empty_headquarters
                                    .as_ref()
                                    .expect("headquarters storage initialized"),
                            )
                        },
                        |at| Arc::clone(&self.mission_headquarters[at].1),
                    ),
                range_fleet: row.range_fleet,
                range_begin: row.range[0],
                range_end: row.range[1],
                convoys: row.convoys,
                raiding_spotting: t.raiding_grid.and_then(|grid| grid.spotting),
                raiding_region: t.raiding_grid.and_then(|grid| grid.region),
                raiding_targets: t.raiding_grid.and_then(|grid| grid.targets),
                raiding_target_country: t.raiding_grid.and_then(|grid| grid.target.country),
                raiding_target_convoys: t.raiding_grid.and_then(|grid| grid.target.convoys),
                raiding_transport_invasion: t.raiding_grid.and_then(|grid| grid.target.invasion),
                raiding_convoy_variant: t.raiding_grid.and_then(|grid| grid.design.variant),
                raiding_convoy_surface_detection: t
                    .raiding_grid
                    .and_then(|grid| grid.design.surface_detection),
                raiding_convoy_sub_detection: t
                    .raiding_grid
                    .and_then(|grid| grid.design.sub_detection),
                raiding_convoy_visibility: t.raiding_grid.and_then(|grid| grid.design.visibility),
                raiding_convoy_maximum_speed: t
                    .raiding_grid
                    .and_then(|grid| grid.design.maximum_speed),
                raiding_source_speed: t.raiding_grid.and_then(|grid| grid.source_speed.base),
                raiding_source_terrain_class: t
                    .raiding_grid
                    .and_then(|grid| grid.source_speed.terrain_class),
            }
        };
        let old_rows: Vec<_> = Self::task_forces(&self.task_forces)
            .iter()
            .filter(|row| !row.additional_range && !row.patrol_context)
            .collect();
        let range_rows: Vec<_> = Self::task_forces(&self.task_forces)
            .iter()
            .filter(|row| row.additional_range)
            .collect();
        let task_forces = table::<tables::NavalMissionTaskForces, _>(
            self.original_mission_outcome(),
            &old_rows,
            |row| convert(row),
        );
        let ranges = table::<tables::NavalDominanceTaskForceRanges, _>(
            self.ranges_outcome,
            &range_rows,
            |row| convert(row),
        );
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
                        speed,
                        detection,
                        spotting,
                        range,
                        range_factor,
                        raiding,
                        night_spotting,
                        escort,
                        visibility,
                        weather_reduction,
                        underway,
                    ] = row.values.map(value);
                    NavalMissionCountryRow {
                        country: row.country,
                        escort_tree: self.escort_trees.project(row.escort_tree),
                        coordination,
                        speed: speed.into(),
                        raiding,
                        escort,
                        weather_reduction,
                        range,
                        range_factor,
                        underway,
                        detection: detection.into(),
                        spotting: spotting.into(),
                        night_spotting: night_spotting.into(),
                        visibility: visibility.into(),
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
                    let [
                        coordination,
                        speed,
                        detection,
                        spotting,
                        range,
                        range_factor,
                        raiding,
                        night_spotting,
                        visibility,
                    ] = row.values.map(value);
                    NavalMissionCommanderRow {
                        country: row.country,
                        reference: row.reference,
                        source: row.source,
                        range,
                        range_factor,
                        coordination,
                        speed: speed.into(),
                        raiding,
                        detection: detection.into(),
                        spotting: spotting.into(),
                        night_spotting: night_spotting.into(),
                        visibility: visibility.into(),
                    }
                },
            ),
            table::<tables::NavalRegionalMines, _>(self.mines_outcome, &self.mines, Clone::clone),
            table::<tables::NavalAccidentReports, _>(
                self.accidents_outcome,
                &self.accidents,
                Clone::clone,
            ),
            ranges,
            table::<tables::NavalPatrolMissionInputs, _>(
                self.patrol_outcome,
                &self.patrol,
                |row| {
                    let value = |value: Option<Value>| {
                        value.and_then(|value| value.present.then_some(value.raw))
                    };
                    replay_data::rows::NavalPatrolMissionInputRow {
                        task_force: row.id,
                        target: row.row.target,
                        target_live: row.row.target_live,
                        progress: row.row.progress,
                        chance: row.row.chance,
                        surface_detection: row.row.surface_detection,
                        carrier_detection: row.row.carrier_detection,
                        local_detection_known: row.local.is_some(),
                        local_detection: value(row.local),
                        weather_detection_known: row.weather.is_some(),
                        weather_detection: value(row.weather),
                        leader: row.leader,
                        headquarters_known: row.headquarters_known,
                        headquarters: self
                            .patrol_headquarters
                            .binary_search_by_key(&row.id, |entry| entry.0)
                            .ok()
                            .map_or_else(
                                || {
                                    Arc::clone(
                                        self.empty_headquarters
                                            .as_ref()
                                            .expect("headquarters storage initialized"),
                                    )
                                },
                                |at| Arc::clone(&self.patrol_headquarters[at].1),
                            ),
                        surface_visibility: row.row.surface_visibility,
                        submarine_visibility: row.row.submarine_visibility,
                        formation_class: row.row.formation_class,
                        radar_cache: row.row.radar_cache,
                        air_cache: row.row.air_cache,
                        submarine_detection: row.row.submarine_detection,
                        carrier_submarine_detection: row.row.carrier_submarine_detection,
                        mission_owner: row.owner,
                        targeted_spotting_known: row.targeted_spotting.is_some(),
                        targeted_spotting: value(row.targeted_spotting),
                    }
                },
            ),
        ]
    }
}

#[cfg(test)]
#[path = "naval_mission_inputs_tests.rs"]
mod tests;
