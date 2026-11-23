use collector_certificates::slicing::{SliceFingerprints, SliceSelection};
use collector_missions::navy_ledger_changes::{Counts as LedgerCounts, Skip as LedgerSkip};
use collector_primitives::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use collector_projection::table_conversion::table;
use engine_allocation::{copy_vec, filled, owned_vec, reserve, resize_default};
use engine_native_memory::{Memory, modifier_cache::Value, walk::Outcome};
use engine_native::{ccountry, cfleet, cweather_manager};
use engine_native_parts::{creference_registry, modifier_accumulator as accumulator};
use engine_native_naval::{
    cstrategic_navy,
    cstrategic_navy_manager,
    naval_mission_inputs::{
        self as inputs,
        COUNTRY_KEYS,
        MISSION_KINDS,
        Operands,
    },
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
pub const SLICED: [usize; 2] = [2, 3];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    task_forces: usize,
    pub headquarters: usize,
    pub census: usize,
    ledgers: usize,
    pub entries: usize,
    countries: usize,
    commanders: usize,
    pub hubs: usize,
}

collector_primitives::capacity_max!(Capacities {
    task_forces,
    headquarters,
    census,
    ledgers,
    entries,
    countries,
    commanders,
    hubs,
});

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        use engine_native_memory::cpdx_array::Array;
        use engine_native::ctask_force;
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
            let count = inputs::count_navy_hubs(memory, navy)?;
            hubs = hubs
                .checked_add(count)
                .ok_or("naval mission hub sizing overflow")?;
        }
        self.ledgers = self.ledgers.max(ledgers);
        self.entries = self.entries.max(entries);
        self.hubs = self.hubs.max(hubs);
        Ok(())
    }

    pub fn raise_for_countries(&mut self, slots: usize) {
        self.countries = self
            .countries
            .max(collector_primitives::country_slots::rows(slots, 1));
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TaskForce {
    row: Operands,
    slot: u32,
    country: u32,
    pub headquarters: usize,
    weather: [Value; 9],
    raiding_grid: Option<inputs::RaidingGrid>,
    additional_range: bool,
    patrol_context: bool,
    native_order: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Member {
    task: usize,
    id: u64,
    visit: Option<cfleet::TaskForceVisit>,
    native_order: usize,
    mission_header: Option<Result<[u8; 16], &'static str>>,
    mission_owner: Option<Result<u32, &'static str>>,
    original_selectors: inputs::OriginalSelectors,
    range_failure: Option<&'static str>,
}

#[derive(Default)]
struct HeldNavy {
    navy: usize,
    certificate: (u64, u64),
    regions: Vec<(u32, usize)>,
    pub entries: Vec<u64>,
    seen: bool,
}

impl HeldNavy {
    fn allocated_bytes(&self) -> usize {
        self.regions.capacity() * size_of::<(u32, usize)>()
            + self.entries.capacity() * size_of::<u64>()
    }
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
    pub hubs: usize,
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
    row: engine_native_naval::naval_patrol_inputs::Operands,
    local: Option<Value>,
    weather: Option<Value>,
    leader: u64,
    pub headquarters: usize,
    headquarters_count: usize,
    headquarters_known: bool,
    existing_context: bool,
    targeted_spotting: Option<Value>,
}

pub struct Captured {
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
    escort_retries: Vec<u32>,
    convoy_designs: inputs::ConvoyDesigns,
    dominance_fleets: Vec<u64>,
    weather_len: usize,
    pub headquarters: Arena<u64>,
    pub census: Arena<Member>,
    ledgers: CaptureRows<LedgerRow>,
    ledger_hold: Vec<HeldNavy>,
    leaders: Leaders,
    mines: Vec<replay_data::rows::NavalRegionalMineRow>,
    mines_outcome: Outcome,
    mine_cache: collector_missions::naval_mines::Captured,
    accidents: Vec<replay_data::rows::NavalAccidentReportRow>,
    accidents_outcome: Outcome,
    accident_cache: collector_missions::naval_accidents::Captured,
    map_ticks: [u64; 2],
    map_roster_complete: bool,
    pub entries: Arena<u64>,
    scratch: Vec<usize>,
    countries: CaptureRows<CountryRow>,
    pub hubs: Arena<u32>,
    commanders: CaptureRows<CommanderRow>,
    named: Arena<engine_native_memory::country_identity::Country>,
    keys: Vec<u64>,
    weather: Vec<(u32, [Value; 9])>,
    selection: SliceSelection,
    witness: Vec<u64>,
    ledger_counts: LedgerCounts,
    sequence: fn(usize) -> Option<(u64, u64)>,
    ledger_sequence: fn(usize) -> Option<(u64, u64)>,
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

#[derive(Default)]
struct Leaders {
    held: Vec<(u64, usize, (u64, u64))>,
    next: Vec<(u64, usize, (u64, u64))>,
    carried: u64,
    resolved: u64,
}

impl Leaders {
    fn leader(
        &mut self,
        reference: u64,
        sequence: fn(usize) -> Option<(u64, u64)>,
        resolve: inputs::Resolve<'_>,
        authenticate: impl FnOnce(usize) -> Result<(), &'static str>,
    ) -> Option<Option<(usize, (u64, u64))>> {
        if let Ok(at) = self.held.binary_search_by_key(&reference, |held| held.0) {
            let (_, leader, certificate) = self.held[at];
            if sequence(leader) == Some(certificate) {
                self.carried += 1;
                self.keep((reference, leader, certificate));
                return Some(Some((leader, certificate)));
            }
        }
        self.resolved += 1;
        let Some(leader) = resolve(reference).ok()? else {
            return Some(None);
        };
        let certificate = sequence(leader)?;
        authenticate(leader).ok()?;
        self.keep((reference, leader, certificate));
        Some(Some((leader, certificate)))
    }

    fn keep(&mut self, entry: (u64, usize, (u64, u64))) {
        if reserve(&mut self.next, 1).is_ok() {
            self.next.push(entry);
        }
    }

    fn finish(&mut self) -> [u64; 2] {
        self.next.sort_unstable_by_key(|entry| entry.0);
        self.next.dedup_by_key(|entry| entry.0);
        std::mem::swap(&mut self.held, &mut self.next);
        self.next.clear();
        [
            std::mem::take(&mut self.carried),
            std::mem::take(&mut self.resolved),
        ]
    }

    fn forget(&mut self) {
        self.held.clear();
        self.next.clear();
    }

    fn allocated_bytes(&self) -> usize {
        (self.held.capacity() + self.next.capacity()) * size_of::<(u64, usize, (u64, u64))>()
    }
}

fn hold_navy(hold: &mut Vec<HeldNavy>, navy: HeldNavy) {
    match hold.binary_search_by_key(&navy.navy, |held| held.navy) {
        Ok(at) => hold[at] = navy,
        Err(at) => {
            if reserve(hold, 1).is_ok() {
                hold.insert(at, navy);
            }
        }
    }
}

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

const NO_COUNTRY: engine_native_memory::country_identity::Country = engine_native_memory::country_identity::Country {
    pointer: 0,
    canonical_slot: 0,
    id: 0,
};

impl Captured {
    pub fn vacant() -> Self {
        Self::build(&Capacities::default(), false)
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self::build(capacity, true)
    }

    fn build(capacity: &Capacities, storage: bool) -> Self {
        let size = |count| if storage { count } else { 0 };
        Self {
            weather_pending: false,
            headquarters_pending: false,
            task_forces: CaptureRows::new(size(capacity.task_forces)),
            ranges_outcome: Outcome::Rows { len: 0 },
            patrol: Vec::new(),
            patrol_outcome: Outcome::Rows { len: 0 },
            patrol_headquarters: Vec::new(),
            mission_headquarters: Vec::new(),
            empty_headquarters: None,
            member_index: Vec::new(),
            task_index: Vec::new(),
            escort_owners: Vec::new(),
            escort_trees: Default::default(),
            escort_retries: Vec::new(),
            convoy_designs: Default::default(),
            dominance_fleets: Vec::new(),
            weather_len: 0,
            headquarters: Arena::new(size(capacity.headquarters)),
            census: Arena::new(size(capacity.census)),
            ledgers: CaptureRows::new(size(capacity.ledgers)),
            ledger_hold: Vec::new(),
            leaders: Leaders::default(),
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
            selection: SliceSelection::new(),
            witness: Vec::new(),
            ledger_counts: LedgerCounts::default(),
            sequence: collector_observers::naval_tree_changes::sequence,
            ledger_sequence: collector_missions::naval_map_changes::ledger_token,
        }
    }

    fn named_capacity(capacity: &Capacities) -> usize {
        capacity
            .countries
            .max(capacity.task_forces)
            .saturating_mul(3)
    }

    pub fn reset(&mut self) {
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
        self.escort_retries.clear();
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
            + self.ledger_hold.capacity() * size_of::<HeldNavy>()
            + self
                .ledger_hold
                .iter()
                .map(HeldNavy::allocated_bytes)
                .sum::<usize>()
            + self.leaders.allocated_bytes()
            + self.witness.capacity() * size_of::<u64>()
            + self.mines.capacity() * size_of::<replay_data::rows::NavalRegionalMineRow>()
            + self.accidents.capacity() * size_of::<replay_data::rows::NavalAccidentReportRow>()
            + self.entries.allocated_bytes()
            + self.scratch.capacity() * size_of::<usize>()
            + self.countries.allocated_bytes()
            + self.hubs.allocated_bytes()
            + self.commanders.allocated_bytes()
            + self.named.first.capacity() * size_of::<engine_native_memory::country_identity::Country>()
            + self.keys.capacity() * size_of::<u64>()
            + self.weather.capacity() * size_of::<(u32, [Value; 9])>()
            + self.dominance_fleets.capacity() * size_of::<u64>()
            + self.patrol.capacity() * size_of::<PatrolMember>()
            + (self.member_index.capacity() + self.task_index.capacity())
                * size_of::<(u64, usize)>()
            + (self.escort_owners.capacity() + self.escort_retries.capacity()) * size_of::<u32>()
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

    pub fn selection(&self) -> &SliceSelection {
        &self.selection
    }

    pub fn ledger_counts(&self) -> LedgerCounts {
        self.ledger_counts
    }

    pub fn reset_selection(&mut self, hour: i32, force_full: bool) {
        self.selection.reset_hooked(hour, force_full);
    }

    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        let outcome = |index: usize| {
            collector_extended_support::outcomes::failed_capture(
                TABLES[index],
                hour,
                force_full,
                reason,
            )
        };
        self.task_forces.fail(outcome(0));
        self.ledgers.fail(outcome(1));
        self.countries.fail(outcome(2));
        self.commanders.fail(outcome(3));
        self.mines_outcome = outcome(4);
        self.accidents_outcome = outcome(5);
        self.ranges_outcome = outcome(6);
        self.patrol_outcome = outcome(7);
        self.selection.reset_hooked(hour, force_full);
        self.ledger_hold.clear();
        self.leaders.forget();
        self.census.first.fill(Member::default());
        self.scratch.fill(0);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn acquire_with_navies(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
        holder: &mut collector_naval_movement::country_naval_movement_inputs::Captured,
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
            skip,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
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
            LedgerSkip { enabled: false },
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn capture_skipping(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        skip: LedgerSkip,
    ) -> [u64; N] {
        self.capture_with_own_navies(memory, base, game_state, hour, true, None, skip)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[allow(clippy::too_many_arguments)]
    fn capture_with_own_navies(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
        skip: LedgerSkip,
    ) -> [u64; N] {
        use collector_naval_movement::country_naval_movement_inputs as naval;
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
        holder: &mut collector_naval_movement::country_naval_movement_inputs::Captured,
        skip: LedgerSkip,
    ) -> [u64; N] {
        self.reset();
        self.empty_headquarters
            .get_or_insert_with(|| Arc::new(Vec::new()));
        self.ledger_counts = LedgerCounts::default();
        self.selection
            .reset_hooked(hour, force_full || fingerprints.is_none());
        let mut ticks = [0; N];
        let started = qpc();
        let mut census_complete = false;
        let walked = {
            let _reads = collector_memory::read_accounting::table(TABLES[0]);
            ccountry::Reader::new(memory, base, game_state).inspect(|countries| {
                self.task_forces.outcome = match self.walk(memory, base, countries) {
                    Ok(outcome) => {
                        census_complete = self.census.required <= self.census.first.len();
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
            let _reads = collector_memory::read_accounting::table(TABLES[1]);
            let mut hold = std::mem::take(&mut self.ledger_hold);
            for held in &mut hold {
                held.seen = false;
            }
            let read = self.read_ledgers(
                memory,
                base,
                &countries,
                holder,
                (skip, force_full, &mut hold),
                census_complete,
            );
            match read {
                Ok(_) => hold.retain(|held| held.seen),
                Err(_) => hold.clear(),
            }
            self.ledger_hold = hold;
            read.unwrap_or_else(|reason| Outcome::Unavailable { reason })
        };
        ticks[1] = elapsed(started)
            .saturating_sub(self.map_ticks[0])
            .saturating_sub(self.map_ticks[1]);
        ticks[4] = self.map_ticks[0];
        ticks[5] = self.map_ticks[1];
        if !self.map_roster_complete
            && let Outcome::Unavailable { reason } = self.ledgers.outcome
        {
            self.mines_outcome = Outcome::Unavailable { reason };
            self.accidents_outcome = Outcome::Unavailable { reason };
        }
        let range_started = qpc();
        {
            let _reads = collector_memory::read_accounting::table(TABLES[6]);
            self.ranges_outcome = if !matches!(self.ledgers.outcome, Outcome::Rows { .. }) {
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
            let _reads = collector_memory::read_accounting::table(TABLES[7]);
            self.patrol_outcome = if !matches!(self.ledgers.outcome, Outcome::Rows { .. }) {
                Outcome::Unavailable {
                    reason: "naval patrol source inputs require complete mission ledgers",
                }
            } else {
                match self.read_patrol_members(memory, base, game_state, &countries) {
                    Ok(len) => Outcome::Rows { len },
                    Err(reason) => Outcome::Unavailable { reason },
                }
            };
            if matches!(self.ledgers.outcome, Outcome::Rows { .. }) {
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
        let _reads = collector_memory::read_accounting::table(TABLES[2]);
        let mut receivers = None;
        if let Some(fingerprints) = fingerprints.as_deref_mut() {
            let scan = qpc();
            receivers = Some(self.scan(memory, base, game_state, fingerprints));
            self.selection.scan_ticks = elapsed(scan);
        }
        let (countries_outcome, commanders_outcome, commander_ticks) =
            self.read_family(memory, base, game_state);
        self.countries.outcome = countries_outcome;
        self.commanders.outcome = commanders_outcome;
        if let Some(fingerprints) = fingerprints.as_deref_mut() {
            self.retry_refused_escort_trees(fingerprints);
        }
        if let Some(receivers) = receivers {
            collector_observers::naval_tree_changes::missions(receivers);
        }
        ticks[2] = elapsed(started).saturating_sub(commander_ticks);
        ticks[3] = commander_ticks;
        ticks
    }

    fn retry_escort_tree(&mut self, country: u32) -> Result<(), &'static str> {
        reserve(&mut self.escort_retries, 1)
            .map_err(|_| "escort tree retry allocation failed")?;
        self.escort_retries.push(country);
        Ok(())
    }

    fn retry_refused_escort_trees(&self, fingerprints: &mut SliceFingerprints) {
        for &country in &self.escort_retries {
            fingerprints.invalidate(country);
        }
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
            engine_native_memory::walk::Buffer::Growing(&mut census.first).ensure(census.required)?;
            engine_native_memory::walk::Buffer::Growing(rows).ensure(
                len.checked_add(1)
                    .ok_or("naval mission row demand overflow")?,
            )?;

            let member_index = census.len;
            if let Some(member) = census.first.get_mut(member_index) {
                *member = Member {
                    task: visit.task,
                    id: visit.id,
                    visit: Some(visit),
                    native_order: census.required - 1,
                    mission_header: None,
                    mission_owner: None,
                    original_selectors: inputs::OriginalSelectors::default(),
                    range_failure: None,
                };
                census.len += 1;
            }
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
                {
                    engine_native_memory::walk::Buffer::GrowingFrom(
                        &mut headquarters.first,
                        headquarters.len,
                    )
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
            let owner = resolve(row.mission_owner).ok();
            let expeditionary = (row.expeditionary_owner > 0)
                .then(|| resolve(row.expeditionary_owner as u32).ok())
                .flatten();
            for country in [Some(visit.country), owner, expeditionary]
                .into_iter()
                .flatten()
            {
                named.required += 1;
                if named.required > named.first.len() {
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
            if let Some(grid) = raiding_grid.as_mut()
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

    pub fn decode_owned(&mut self) {
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

    fn add_named(&mut self, country: engine_native_memory::country_identity::Country) -> Result<(), &'static str> {
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
                engine_native_memory::walk::Buffer::Growing(&mut self.weather)
                    .ensure(self.weather_len + 1)
                    .ok()?;
                self.weather.copy_within(at..self.weather_len, at + 1);
                self.weather[at] = (region, values);
                self.weather_len += 1;
                Some(values[2])
            }
        }
    }

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
            engine_native_memory::walk::Buffer::Growing(&mut self.keys).ensure(self.entries.len)?;
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
                    engine_native_naval::naval_patrol_inputs::read_header(memory, base, visit)
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
                let row = engine_native_naval::naval_patrol_inputs::read_member_observed_stats(
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
                    packet.targeted_spotting = engine_native::ctask_force::targeted_spotting::read(
                        memory,
                        base,
                        countries,
                        visit.country,
                        known.map(|source| source.row.location),
                    )
                    .ok();
                }
                if known.is_some() {
                    packet.existing_context = true;
                } else if member.range_failure.is_none() {
                    let context = engine_native_naval::naval_patrol_inputs::read_context_with_location(
                        memory,
                        base,
                        visit,
                        member.original_selectors.location,
                        engine_native_memory::walk::Buffer::GrowingFrom(
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
                    engine_native_memory::walk::Buffer::Growing(&mut self.task_forces.rows)
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
                engine_native_naval::naval_patrol_inputs::read_header(memory, base, visit)
            });
            self.census.first[at].mission_header = Some(header);
            let owner = member.mission_owner.unwrap_or_else(|| {
                engine_native_naval::naval_patrol_inputs::read_owner(memory, base, visit, header)
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
            engine_native_memory::walk::Buffer::Growing(&mut self.keys).ensure(entries)?;
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
                    engine_native_memory::walk::Buffer::GrowingFrom(
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
                            engine_native_memory::walk::Buffer::Growing(&mut self.weather)
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
                engine_native_memory::walk::Buffer::Growing(&mut self.task_forces.rows).ensure(len + 1)?;
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

    fn read_weather<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        game_state: usize,
    ) -> Result<(), &'static str> {
        let rows = self.task_forces.outcome.len();
        engine_native_memory::walk::Buffer::Growing(&mut self.keys).ensure(rows)?;
        engine_native_memory::walk::Buffer::Growing(&mut self.weather).ensure(rows)?;

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
        self.weather_pending = true;
        Ok(())
    }

    fn read_ledgers<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        holder: &mut collector_naval_movement::country_naval_movement_inputs::Captured,
        (skip, force_full, hold): (LedgerSkip, bool, &mut Vec<HeldNavy>),
        census_complete: bool,
    ) -> Result<Outcome, &'static str> {
        let count = holder.physical_count()?;
        let lifetime = collector_lifetime::world_lifetime::token();
        collector_missions::naval_map_changes::prepare(count);
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
            let pointer = navy.ledger_navy()?;
            if pointer == 0 {
                return Err("country navy absent or unreadable");
            }
            let begin = qpc();
            if mine_ready.is_ok() {
                let _reads = collector_memory::read_accounting::table(TABLES[4]);
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
                let _reads = collector_memory::read_accounting::table(TABLES[5]);
                let certificate = collector_missions::naval_map_changes::accident_token(pointer);
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
                } else if certificate
                    != collector_missions::naval_map_changes::accident_token(pointer)
                {
                    self.accidents_outcome = Outcome::Unavailable {
                        reason: "naval accident writer changed during acquisition",
                    };
                }
            }
            self.map_ticks[1] = self.map_ticks[1].saturating_add(elapsed(begin));
            if !census_complete {
                continue;
            }
            let outer = navy.take_ledger_regions()?;
            let certificate = (self.ledger_sequence)(pointer);
            let held = hold.binary_search_by_key(&pointer, |held| held.navy);
            let mut presence = None;
            if let (Ok(at), Some(certificate), false) = (held, certificate, force_full)
                && hold[at].certificate == certificate
            {
                self.ledger_counts.carried += 1;
                hold[at].seen = true;
                let held = &hold[at];
                let mut first = 0;
                for &(region, count) in &held.regions {
                    let entries = &held.entries[first..first + count];
                    first += count;
                    engine_native_memory::walk::Buffer::Growing(&mut self.ledgers.rows).ensure(
                        len.checked_add(1)
                            .ok_or("naval mission ledger row demand overflow")?,
                    )?;
                    engine_native_memory::walk::Buffer::Growing(&mut self.entries.first).ensure(
                        self.entries
                            .len
                            .checked_add(count)
                            .ok_or("naval mission ledger demand overflow")?,
                    )?;

                    self.entries.required = self.entries.required.saturating_add(count);
                    if len == self.ledgers.rows.len()
                        || count > self.entries.first.len().saturating_sub(self.entries.len)
                    {
                        unread = unread.saturating_add(1);
                        continue;
                    }
                    self.entries.first[self.entries.len..self.entries.len + count]
                        .copy_from_slice(entries);
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
                }
                continue;
            }
            let mut fresh = HeldNavy {
                navy: pointer,
                certificate: certificate.unwrap_or_default(),
                seen: true,
                ..HeldNavy::default()
            };
            let mut holdable = certificate.is_some();
            if skip.enabled && cstrategic_navy::registered_missions(memory, pointer) == Ok(0) {
                self.ledger_counts.skipped += 1;
                if holdable {
                    hold_navy(hold, fresh);
                }
                continue;
            }
            self.ledger_counts.read += 1;
            inputs::ledger_regions_in(memory, outer, |region, ledger| {
                engine_native_memory::walk::Buffer::Growing(&mut self.scratch)
                    .ensure(ledger.count as usize)?;
                engine_native_memory::walk::Buffer::Growing(&mut self.ledgers.rows).ensure(
                    len.checked_add(1)
                        .ok_or("naval mission ledger row demand overflow")?,
                )?;
                engine_native_memory::walk::Buffer::Growing(&mut self.entries.first).ensure(
                    self.entries
                        .len
                        .checked_add(ledger.count as usize)
                        .ok_or("naval mission ledger demand overflow")?,
                )?;

                let count = inputs::ledger(memory, ledger, &mut self.scratch)?;
                self.entries.required = self.entries.required.saturating_add(count);
                if len == self.ledgers.rows.len()
                    || count > self.entries.first.len().saturating_sub(self.entries.len)
                {
                    unread = unread.saturating_add(1);
                    holdable = false;
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
                let entries = &self.entries.first[self.entries.len..self.entries.len + count];
                holdable = holdable
                    && reserve(&mut fresh.regions, 1).is_ok()
                    && reserve(&mut fresh.entries, count).is_ok();
                if holdable {
                    fresh.regions.push((region, count));
                    fresh.entries.extend_from_slice(entries);
                }
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
            if holdable {
                hold_navy(hold, fresh);
            }
        }
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

    fn owned(rows: &[TaskForce], slot: u32) -> &[TaskForce] {
        let first = rows.partition_point(|task_force| task_force.slot < slot);
        let end = rows.partition_point(|task_force| task_force.slot <= slot);
        &rows[first..end]
    }

    fn scan(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        fingerprints: &mut SliceFingerprints,
    ) -> usize {
        let resolve = |handle| creference_registry::resolve(memory, base, handle);
        let mut receivers = 0;
        let mut leaders = std::mem::take(&mut self.leaders);
        let mut witness = std::mem::take(&mut self.witness);
        for index in 0..self.named.len {
            let country = self.named.first[index];
            witness.clear();
            let stamp = self
                .witness(
                    memory,
                    base,
                    game_state,
                    country,
                    (&resolve, &mut leaders),
                    (&mut witness, &mut receivers),
                )
                .map(|()| fingerprints.witnesses.observe(country.id, &witness));
            self.selection.scan_marking(fingerprints, country.id, stamp);
        }
        fingerprints.witnesses.finish();
        collector_missions::navy_ledger_changes::record_leaders(leaders.finish());
        self.leaders = leaders;
        self.witness = witness;
        fingerprints.finish_scan();
        self.selection.finish_census(fingerprints);
        receivers
    }

    fn witness(
        &self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        country: engine_native_memory::country_identity::Country,
        (resolve, leaders): (inputs::Resolve<'_>, &mut Leaders),
        (witness, receivers): (&mut Vec<u64>, &mut usize),
    ) -> Option<()> {
        let mut push = |words: &[u64]| {
            engine_allocation::advisory(|| reserve(witness, words.len())).ok()?;
            witness.extend_from_slice(words);
            Some(())
        };
        let owned = Self::owned(Self::task_forces(&self.task_forces), country.canonical_slot);
        push(&[owned.len() as u64])?;
        for task_force in owned {
            push(&[task_force.row.leader, task_force.row.headquarters as u64])?;
            push(
                &self.headquarters.first[task_force.headquarters
                    ..task_force.headquarters + task_force.row.headquarters],
            )?;
        }
        push(&[u64::from(
            self.escort_owners.binary_search(&country.id).is_ok(),
        )])?;
        let (epoch, sequence) = (self.sequence)(country.pointer)?;
        *receivers += 1;
        push(&[country.pointer as u64, epoch, sequence])?;
        let navy = cstrategic_navy_manager::navy(memory, base, game_state, country).ok()?;
        let hubs =
            engine_native_memory::cpdx_array::Array::read(memory, navy.checked_add(inputs::HUBS)?, 4)?;
        push(&[u64::from(hubs.count)])?;
        let mut chunk = [0_u8; 256];
        let total = hubs.count as usize * 4;
        let mut done = 0;
        while done < total {
            let len = (total - done).min(chunk.len());
            if !engine_native_memory::memory::read_slice(memory, hubs.data + done, &mut chunk[..len]) {
                return None;
            }
            for word in chunk[..len].chunks(8) {
                let mut bytes = [0; 8];
                bytes[..word.len()].copy_from_slice(word);
                push(&[u64::from_le_bytes(bytes)])?;
            }
            done += len;
        }
        for task_force in owned {
            let reference = task_force.row.leader;
            if reference != 0 {
                let leader = leaders.leader(reference, self.sequence, resolve, |leader| {
                    inputs::fleet_leader(memory, base, reference, &|_| Ok(Some(leader))).map(drop)
                })?;
                match leader {
                    Some((leader, (epoch, sequence))) => {
                        *receivers += 1;
                        push(&[1, leader as u64, epoch, sequence])?;
                    }
                    None => push(&[0])?,
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
                        push(&[1, leader as u64])?;
                        let mut count = 0_u64;
                        for source in sources {
                            push(&[source as u64])?;
                            count += 1;
                        }
                        push(&[count])?;
                    }
                    None => push(&[0])?,
                }
            }
        }
        Some(())
    }

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
                let _reads = collector_memory::read_accounting::table(TABLES[3]);
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
        country: engine_native_memory::country_identity::Country,
        (len, unread): &mut (usize, u32),
    ) -> Result<(), &'static str> {
        engine_native_memory::walk::Buffer::Growing(&mut self.countries.rows).ensure(
            len.checked_add(1)
                .ok_or("naval mission country row demand overflow")?,
        )?;

        let Some(row) = self.countries.rows.get_mut(*len) else {
            *unread = unread.saturating_add(1);
            return Ok(());
        };
        let navy = cstrategic_navy_manager::navy(memory, base, game_state, country)?;
        let count = inputs::navy_hubs_buffer(
            memory,
            navy,
            engine_native_memory::walk::Buffer::GrowingFrom(&mut self.hubs.first, self.hubs.len),
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
        let demanded = self.escort_owners.binary_search(&country.id).is_ok();
        row.escort_tree = self.escort_trees.read(
            memory,
            base,
            country,
            demanded,
            (self.sequence)(country.pointer),
        );
        let refused = demanded && row.escort_tree.is_none();
        accumulator::lookup_many(memory, country.pointer, &COUNTRY_KEYS, &mut row.values)?;
        if refused {
            self.retry_escort_tree(country.id)?;
        }
        *len += 1;
        Ok(())
    }

    fn read_commanders(
        &mut self,
        memory: &impl Memory,
        base: usize,
        slot: u32,
        country: u32,
        resolve: inputs::Resolve<'_>,
        (len, unread): &mut (usize, u32),
    ) -> Result<(), &'static str> {
        let bound = self.task_forces.outcome.len().max(self.headquarters.len);
        engine_native_memory::walk::Buffer::Growing(&mut self.keys).ensure(bound)?;
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
        engine_native_memory::walk::Buffer::Growing(&mut self.commanders.rows).ensure(
            len.checked_add(additional)
                .ok_or("naval mission commander row demand overflow")?,
        )?;

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

#[cfg(any(test, feature = "test-support"))]
#[path = "naval_mission_inputs_tests.rs"]
pub mod tests;
