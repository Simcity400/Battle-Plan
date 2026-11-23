use collector_divisions::orders::Orders;
use collector_primitives::clock::qpc;
use collector_primitives::table_capture::{CaptureRows, grow_capacity, grow_vec};
use collector_raw::raw::RawLog;
use engine_native::{
    Memory, cair_wing, cfleet, chuman, cland_combat, cnaval_combat, cnaval_combatant, cship,
    ctask_force, walk::Outcome,
};
#[cfg(test)]
use engine_native::{corder_instance, corders_group};
use replay_data::rows::TableId;
use std::mem::size_of;

pub const MAP_TABLES: usize = 10;

mod combat;
pub use combat::{CombatReport, CombatRequest};

fn ship_name_result<T>(result: Result<Option<T>, &'static str>) -> Option<T> {
    result.ok().flatten()
}

pub trait MapSources {
    fn begin_land(&mut self) -> u64 {
        0
    }
    fn land(&mut self, _source: cland_combat::Source) -> u64 {
        0
    }
    fn finish_land(&mut self, _outcome: Outcome) -> u64 {
        0
    }
}

#[cfg(test)]
struct NoSources;

#[cfg(test)]
impl MapSources for NoSources {}

const AIR_NOT_ACQUIRED: Outcome = Outcome::Unavailable {
    reason: "air wing payload not acquired this hour",
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MapCapacities {
    rows: [usize; MAP_TABLES],
    player_names: usize,
    air_wing_names: usize,
    land_participants: usize,
    order_geometry: usize,
    order_groups: usize,
    order_visits: usize,
    naval_groups: usize,
}

collector_primitives::capacity_max!(MapCapacities {
    rows,
    player_names,
    air_wing_names,
    land_participants,
    order_geometry,
    order_groups,
    order_visits,
    naval_groups,
});

impl Default for MapCapacities {
    fn default() -> Self {
        Self {
            rows: [0; MAP_TABLES],
            player_names: 0,
            air_wing_names: collector_air::sizing::air_wing_name_bytes(),
            land_participants: 0,
            order_geometry: 0,
            order_groups: 0,
            order_visits: 0,
            naval_groups: 0,
        }
    }
}

impl MapCapacities {
    /// # Safety
    /// Only the active session's sizing owner or its designated allocator may use capacities
    /// measured for that session. A test fixture may use its own capacities in a process without
    /// a live attachment. Allocation failure can stop the recording.
    pub unsafe fn orders(&self) -> Orders {
        // SAFETY: Caller upholds the active-session/fixture capacity sizing contract.
        unsafe {
            Orders::new(
                self.rows[9],
                self.order_geometry,
                self.order_groups,
                self.order_visits,
            )
        }
    }

    pub fn presize_combats(
        &mut self,
        memory: &impl Memory,
        base: usize,
    ) -> Result<(), &'static str> {
        let state = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
            .and_then(|address| engine_native::read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("map combat sizing state unreadable")?;
        let land = cland_combat::core_demand(memory, base, state);
        let naval = cnaval_combat::storage_demand(memory, base, state);
        let mut failure = None;
        match land {
            Ok(demand) => {
                grow_capacity(&mut self.rows[2], demand.rows);
                grow_capacity(&mut self.land_participants, demand.participants);
            }
            Err(reason) => failure = Some(reason),
        }
        for (index, outcome) in [(6, naval.combats), (7, naval.participants)] {
            match outcome {
                Outcome::Rows { len } => grow_capacity(&mut self.rows[index], len),
                Outcome::Truncated { len, unread_rows } => grow_capacity(
                    &mut self.rows[index],
                    len.saturating_add(unread_rows as usize),
                ),
                Outcome::Unavailable { reason } => {
                    failure.get_or_insert(reason);
                }
            }
        }
        grow_capacity(&mut self.naval_groups, naval.required_group_capacity);
        failure.map_or(Ok(()), Err)
    }

    pub fn raise_air_wings(&mut self, count: usize) {
        collector_primitives::country_slots::raise(
            &mut self.rows[8],
            collector_primitives::country_slots::rows(count, 1),
        );
    }

    pub fn raise_naval(&mut self, index: usize, count: usize) {
        collector_primitives::country_slots::raise(
            &mut self.rows[index + 3],
            collector_primitives::country_slots::rows(count, 1),
        );
    }

    /// # Safety
    /// Use only the active sizing pass's measured name capacity or an isolated fixture. An invalid
    /// capacity can invoke the process-wide allocation failure handler.
    pub unsafe fn raise_for_air_wing_names(&mut self, bytes: usize) {
        collector_primitives::country_slots::raise(&mut self.air_wing_names, bytes);
    }

    pub fn raise_for_countries(&mut self, slots: usize) {
        collector_primitives::country_slots::raise(
            &mut self.rows[1],
            collector_primitives::country_slots::rows(slots, 1),
        );
    }
}

pub struct ShipCensus {
    pub management: collector_naval_management::forces::Captured,
    management_owned: Option<collector_naval_management::forces::Owned>,
    pub fleets: CaptureRows<cfleet::Row>,
    pub fleet_names: Vec<u8>,
    pub ship_names: Vec<u8>,
    pub ship_overrides: Vec<u8>,
    pub task_forces: CaptureRows<ctask_force::Row>,
    pub ships: CaptureRows<cship::Row>,
    pub ticks: [u64; 3],
}

impl ShipCensus {
    fn new(rows: &[usize]) -> Self {
        Self {
            management: Default::default(),
            management_owned: None,
            fleets: CaptureRows::new(rows[0]),
            fleet_names: Vec::new(),
            ship_names: Vec::new(),
            ship_overrides: Vec::new(),
            task_forces: CaptureRows::new(rows[1]),
            ships: CaptureRows::new(rows[2]),
            ticks: [0; 3],
        }
    }
    pub fn vacant() -> Self {
        Self::new(&[0; 3])
    }
    fn reset(&mut self) {
        // SAFETY: This frame's owner is returning its outstanding naval loan before reuse.
        unsafe { self.discard_owned() };
        self.management = Default::default();
        self.fleets.reset();
        self.fleet_names.clear();
        self.ship_names.clear();
        self.ship_overrides.clear();
        self.task_forces.reset();
        self.ships.reset();
        self.ticks = [0; 3];
    }
    fn allocated_bytes(&self) -> usize {
        self.management
            .tables
            .iter()
            .filter_map(|table| table.as_ref().ok())
            .map(|rows| rows.allocated_bytes())
            .sum::<usize>()
            + self.ship_names.capacity()
            + self.ship_overrides.capacity()
            + self.fleet_names.capacity()
            + self.fleets.allocated_bytes()
            + self.task_forces.allocated_bytes()
            + self.ships.allocated_bytes()
    }
    /// # Safety
    /// Only the admitted naval capture may walk the current game and take its process-wide
    /// management loan. Settle or discard that loan for this frame before reusing the slot or
    /// detaching.
    pub unsafe fn walk(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        models: &mut collector_ship_models::ship_models::Captured,
    ) {
        let tables = [TableId::Fleets, TableId::TaskForces, TableId::Ships];
        let mut reads = collector_memory::read_accounting::table(tables[0]);
        let begin = qpc();
        // SAFETY: This frame's owner is returning its outstanding naval loan before reuse.
        unsafe { self.discard_owned() };
        models.begin_sources();
        // SAFETY: `ShipCensus::walk` runs in the admitted naval capture against the session image
        // and settles or discards `self.management_owned` before frame reuse or detach.
        let management = std::cell::RefCell::new(unsafe {
            collector_naval_management::forces::Session::begin_owned(
                memory,
                base,
                state,
                collector_workers::panic_failure::owned_failed,
            )
        });
        let (mut name_len, mut name_required) = (0, 0);
        let (mut override_len, mut override_required) = (0, 0);
        let naval = cfleet::walk_timed_with_management(
            memory,
            base,
            state,
            &mut self.fleets.rows,
            &mut self.task_forces.rows,
            &mut self.ships.rows,
            |phase| {
                reads.select(tables[phase]);
                qpc() as u64
            },
            |pointer, row| {
                row.name = ship_name_result(cship::cached_name_growing(
                    memory,
                    base,
                    pointer,
                    row.id,
                    &mut self.ship_names,
                    &mut name_len,
                    &mut name_required,
                ));
                if let Some((_, len)) = row.name {
                    name_len += len;
                }
                row.naming = ship_name_result(cship::naming(
                    memory,
                    base,
                    pointer,
                    row.id,
                    &mut self.ship_overrides,
                    &mut override_len,
                    &mut override_required,
                ));
                if let Some(naming) = row.naming {
                    override_len += naming.override_name.1;
                }
                let crew = collector_air::crew::ship(
                    memory,
                    pointer,
                    collector_observers::crew_changes::quiet,
                );
                row.acquired_manpower = crew.map(|value| value[0]);
                row.required_manpower = crew.map(|value| value[1]);
                models.visit_source(pointer, row);
                management.borrow_mut().ship(pointer, row);
            },
            &mut self.fleet_names,
            |visit| management.borrow_mut().visit(visit),
        );
        self.ship_names.truncate(name_len);
        self.ship_overrides.truncate(override_len);
        (self.management, self.management_owned) =
            management.into_inner().finish_owned(naval.capture);
        models.finish_sources(naval.capture.ships);
        self.fleets.outcome = naval.capture.fleets;
        self.task_forces.outcome = naval.capture.task_forces;
        self.ships.outcome = naval.capture.ships;
        self.ticks = naval.read_ticks;
        self.ticks[2] = elapsed(begin)
            .saturating_sub(self.ticks[0])
            .saturating_sub(self.ticks[1]);
    }
    pub fn fail(&mut self, reason: &'static str) {
        // SAFETY: This frame's owner is returning its outstanding naval loan before reuse.
        unsafe { self.discard_owned() };
        self.ship_names.clear();
        self.ship_overrides.clear();
        self.management.tables = std::array::from_fn(|_| Err(reason));
        let outcome = Outcome::Unavailable { reason };
        self.fleets.outcome = outcome;
        self.task_forces.outcome = outcome;
        self.ships.outcome = outcome;
        self.ticks = [0; 3];
    }

    /// # Safety
    /// Settle only this frame's naval management loan, once, before the originating session
    /// releases its management storage.
    pub unsafe fn decode_owned(&mut self) {
        if let Some(owned) = self.management_owned.take() {
            self.management = owned.decode();
        }
    }

    /// # Safety
    /// Settle only this frame's naval management loan, once, before the originating session
    /// releases its management storage.
    pub unsafe fn discard_owned(&mut self) {
        self.management_owned = None;
    }
}

pub struct MapCaptured {
    pub players: CaptureRows<chuman::Row>,
    pub human_controllers: CaptureRows<chuman::ControllerRow>,
    pub land_combats: CaptureRows<cland_combat::Row>,
    pub naval: ShipCensus,
    pub naval_combats: CaptureRows<cnaval_combat::Row>,
    pub naval_combat_participants: CaptureRows<cnaval_combatant::Row>,
    pub air_wings: CaptureRows<cair_wing::Row>,
    pub air_wing_names: Vec<u8>,
    pub air_wing_names_len: usize,
    air_wing_names_required: usize,
    pub(crate) air_wing_name_failures: usize,
    pub orders: Orders,
    players_raw_mode: bool,
    owned_air_captured: bool,
    air_capture_failure: Option<&'static str>,
    pub player_names: Vec<u8>,
    pub player_names_len: usize,
    player_names_required: usize,
    pub land_participants: Vec<u64>,
    pub land_classes: Vec<u32>,
    pub land_participants_len: usize,
    land_participants_required: usize,
    naval_groups: Vec<usize>,
    naval_groups_required: usize,
    combat_ticks: Option<[u64; MAP_TABLES]>,
}

impl MapCaptured {
    /// # Safety
    /// Only the admitted frame owner may set this flag, after the matching player job or its
    /// explicit failure has been installed for the same hour.
    pub unsafe fn players_raw_mode_mut(&mut self) -> &mut bool {
        &mut self.players_raw_mode
    }
    /// # Safety
    /// Install once into the originating frame, using the completed job that owns the matching wing
    /// and name buffers. Do not reuse the job without refreshing its lengths.
    pub unsafe fn install_owned_air(
        &mut self,
        job: &mut collector_air::extended::air::AirWingJob,
    ) -> u64 {
        if self.air_capture_failure.is_some() {
            return 0;
        }
        let begin = qpc();
        let len = job.map_outcome.len();
        if len > self.air_wings.rows.len() {
            std::mem::swap(&mut self.air_wings.rows, &mut job.map_rows);
        } else {
            self.air_wings.rows[..len].copy_from_slice(&job.map_rows[..len]);
        }
        let names_swapped = job.names_len > self.air_wing_names.len();
        if names_swapped {
            std::mem::swap(&mut self.air_wing_names, &mut job.names);
            self.air_wing_names_len = job.names_len;
        }
        let names_fit = job.names_len <= self.air_wing_names.len();
        if names_fit {
            if !names_swapped {
                self.air_wing_names[..job.names_len].copy_from_slice(&job.names[..job.names_len]);
            }
            self.air_wing_names_len = job.names_len;
        } else {
            self.air_wing_names_len = 0;
            for row in &mut self.air_wings.rows[..len] {
                row.name = None;
            }
        }
        self.air_wing_names_required = job.names_required.max(job.names_len);
        self.air_wing_name_failures = job.name_failures + if names_fit { 0 } else { len };
        self.air_wings.outcome = job.map_outcome;
        self.owned_air_captured = true;
        qpc().saturating_sub(begin).max(0) as u64
    }

    pub fn fail_air_capture(&mut self, reason: &'static str) {
        self.air_capture_failure = Some(reason);
        self.owned_air_captured = false;
        self.air_wings.outcome = Outcome::Unavailable { reason };
    }

    #[cfg(any(test, feature = "test-support"))]
    /// # Safety
    /// Only an isolated collector fixture may allocate this test map. No live recording may share
    /// its allocation-failure state.
    pub unsafe fn new() -> Self {
        // SAFETY: These are session-measured map capacities or capacities owned by an isolated
        // fixture.
        unsafe { Self::with_capacities(&MapCapacities::default()) }
    }

    /// # Safety
    /// Allocate only from the attaching or admitted session's measured map capacities or in an
    /// isolated fixture that owns its allocation-failure state.
    pub unsafe fn with_capacities(capacity: &MapCapacities) -> Self {
        Self {
            players: CaptureRows::new(capacity.rows[0]),
            human_controllers: CaptureRows::new(capacity.rows[1]),
            land_combats: CaptureRows::new(capacity.rows[2]),
            naval: ShipCensus::new(&capacity.rows[3..6]),
            naval_combats: CaptureRows::new(capacity.rows[6]),
            naval_combat_participants: CaptureRows::new(capacity.rows[7]),
            air_wings: CaptureRows::new(capacity.rows[8]),
            air_wing_names: engine_allocation::owned_vec(engine_allocation::default_vec(
                capacity.air_wing_names,
            )),
            air_wing_names_len: 0,
            air_wing_names_required: 0,
            air_wing_name_failures: 0,
            owned_air_captured: false,
            air_capture_failure: None,
            // SAFETY: The constructor requires measured capacities from the session sizing owner
            // or capacities owned by an isolated fixture.
            orders: unsafe { capacity.orders() },
            player_names: engine_allocation::owned_vec(engine_allocation::default_vec(
                capacity.player_names,
            )),
            player_names_len: 0,
            player_names_required: 0,
            land_participants: engine_allocation::owned_vec(engine_allocation::default_vec(
                capacity.land_participants,
            )),
            land_classes: engine_allocation::owned_vec(engine_allocation::default_vec(
                capacity.land_participants,
            )),
            land_participants_len: 0,
            land_participants_required: 0,
            naval_groups: engine_allocation::owned_vec(engine_allocation::default_vec(
                capacity.naval_groups,
            )),
            naval_groups_required: 0,
            players_raw_mode: false,
            combat_ticks: None,
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.players.allocated_bytes()
            + self.human_controllers.allocated_bytes()
            + self.land_combats.allocated_bytes()
            + self.naval.allocated_bytes()
            + self.naval_combats.allocated_bytes()
            + self.naval_combat_participants.allocated_bytes()
            + self.air_wings.allocated_bytes()
            + self.air_wing_names.capacity()
            + self.orders.allocated_bytes()
            + self.player_names.capacity()
            + self.land_participants.capacity() * size_of::<u64>()
            + self.land_classes.capacity() * size_of::<u32>()
            + self.naval_groups.capacity() * size_of::<usize>()
    }

    pub fn reset(&mut self) {
        self.players.reset();
        self.human_controllers.reset();
        self.land_combats.reset();
        self.naval.reset();
        self.naval_combats.reset();
        self.naval_combat_participants.reset();
        self.air_wings.reset();
        self.air_wing_names_len = 0;
        self.air_wing_names_required = 0;
        self.air_wing_name_failures = 0;
        self.owned_air_captured = false;
        self.air_capture_failure = None;
        self.orders.reset();
        self.player_names_len = 0;
        self.player_names_required = 0;
        self.land_participants_len = 0;
        self.land_participants_required = 0;
        self.naval_groups_required = 0;
        self.players_raw_mode = false;
        self.combat_ticks = None;
    }

    pub fn outcomes(&self) -> [Outcome; MAP_TABLES] {
        [
            self.players.outcome,
            self.human_controllers.outcome,
            self.land_combats.outcome,
            self.naval.fleets.outcome,
            self.naval.task_forces.outcome,
            self.naval.ships.outcome,
            self.naval_combats.outcome,
            self.naval_combat_participants.outcome,
            self.air_wings.outcome,
            self.orders.rows.outcome,
        ]
    }

    /// # Safety
    /// Only the session's frame sizing owner or isolated fixture may grow this map, using validated
    /// demands and with no conflicting teardown.
    pub unsafe fn grow(&mut self, capacities: &mut MapCapacities) {
        self.players.grow(&mut capacities.rows[0]);
        self.human_controllers.grow(&mut capacities.rows[1]);
        self.land_combats.grow(&mut capacities.rows[2]);
        capacities.rows[3] = capacities.rows[3].max(self.naval.fleets.rows.len());
        self.naval.fleets.grow(&mut capacities.rows[3]);
        capacities.rows[4] = capacities.rows[4].max(self.naval.task_forces.rows.len());
        self.naval.task_forces.grow(&mut capacities.rows[4]);
        capacities.rows[5] = capacities.rows[5].max(self.naval.ships.rows.len());
        self.naval.ships.grow(&mut capacities.rows[5]);
        self.naval_combats.grow(&mut capacities.rows[6]);
        self.naval_combat_participants.grow(&mut capacities.rows[7]);
        capacities.rows[8] = capacities.rows[8].max(self.air_wings.rows.len());
        self.air_wings.grow(&mut capacities.rows[8]);
        grow_capacity(
            &mut capacities.air_wing_names,
            self.air_wing_names_required
                .max(collector_air::sizing::air_wing_name_bytes()),
        );
        grow_vec(&mut self.air_wing_names, capacities.air_wing_names);
        // SAFETY: Caller holds exclusive `&mut` access to this map frame at the frame-growth boundary.
        unsafe {
            self.orders.grow(
                &mut capacities.rows[9],
                &mut capacities.order_geometry,
                &mut capacities.order_groups,
                &mut capacities.order_visits,
            );
        }
        grow_capacity(&mut capacities.player_names, self.player_names_required);
        grow_capacity(
            &mut capacities.land_participants,
            self.land_participants_required,
        );
        grow_capacity(&mut capacities.naval_groups, self.naval_groups_required);
        grow_vec(&mut self.player_names, capacities.player_names);
        grow_vec(&mut self.land_participants, capacities.land_participants);
        grow_vec(&mut self.land_classes, capacities.land_participants);
        grow_vec(&mut self.naval_groups, capacities.naval_groups);
    }

    /// # Safety
    /// Only the admitted hour may sample its map with the active game's located image. A deferred
    /// player or combat walk must belong to and settle into this frame.
    pub unsafe fn sample(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: &mut impl MapSources,
    ) -> [u64; MAP_TABLES] {
        let mut ticks = self.combat_ticks.unwrap_or([0; MAP_TABLES]);
        if !self.players_raw_mode {
            let mut reads = collector_memory::read_accounting::table(TableId::Players);
            let begin = qpc();
            let names = chuman::walk(
                memory,
                base,
                state,
                &mut self.players.rows,
                &mut self.player_names,
            );
            self.players.outcome = names.outcome;
            self.player_names_len = names.bytes_len;
            self.player_names_required = names.required_bytes;
            ticks[0] = (qpc() - begin).max(0) as u64;

            reads.select(TableId::HumanControllers);
            let begin = qpc();
            self.human_controllers.outcome =
                chuman::walk_controllers(memory, base, state, &mut self.human_controllers.rows);
            ticks[1] = (qpc() - begin).max(0) as u64;
        }

        let mut reads = collector_memory::read_accounting::table(TableId::LandCombats);
        if self.combat_ticks.is_none() {
            let begin = qpc();
            let mut nested = sources.begin_land();
            let land = cland_combat::walk_with(
                memory,
                base,
                state,
                &mut self.land_combats.rows,
                &mut self.land_participants,
                &mut self.land_classes,
                |source| nested = nested.saturating_add(sources.land(source)),
            );
            nested = nested.saturating_add(sources.finish_land(land.outcome));
            self.land_combats.outcome = land.outcome;
            self.land_participants_len = land.participants_len;
            self.land_participants_required = land.required_participants;
            ticks[2] = ((qpc() - begin).max(0) as u64).saturating_sub(nested);

            let naval_tables = [TableId::NavalCombats, TableId::NavalCombatParticipants];
            reads.select(naval_tables[0]);
            let naval = cnaval_combat::walk_timed(
                memory,
                base,
                state,
                &mut self.naval_combats.rows,
                &mut self.naval_combat_participants.rows,
                &mut self.naval_groups,
                |phase| {
                    reads.select(naval_tables[phase]);
                    qpc() as u64
                },
            );
            self.naval_combats.outcome = naval.capture.combats;
            self.naval_combat_participants.outcome = naval.capture.participants;
            self.naval_groups_required = naval.capture.required_group_capacity;
            ticks[6..8].copy_from_slice(&naval.read_ticks);
        }
        if let Some(reason) = self.air_capture_failure {
            self.air_wings.outcome = Outcome::Unavailable { reason };
        } else if !self.owned_air_captured {
            self.air_wings.outcome = AIR_NOT_ACQUIRED;
        }

        ticks
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

const P_PLAYERS: u32 = 0x01;
const P_CONTROLLERS: u32 = 0x02;

#[derive(Debug, Default)]
pub struct PlayersJob {
    pub log: RawLog<2>,
    pub game_state: usize,
}

impl PlayersJob {
    pub fn new() -> Self {
        Self {
            log: RawLog::new(0, 0),
            game_state: 0,
        }
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PlayersCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; 2],
}

#[derive(Debug, Clone, Copy)]
pub struct PlayersCopyContext {
    pub base: usize,
    pub game_state: usize,
}

fn ppool_u64(log: &RawLog<2>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn pspan(
    log: &mut RawLog<2>,
    memory: &impl engine_native::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take_bulk(memory, addr?, len, mask)
}

fn copy_player(
    log: &mut RawLog<2>,
    memory: &impl engine_native::Memory,
    pointer: usize,
    vtable: usize,
    keys: &mut Vec<i32>,
) -> bool {
    use engine_native::{chuman, msvc_string};
    let Some(key_address) = pointer.checked_add(chuman::PLAYER_KEY) else {
        return false;
    };
    let Some(country_address) = pointer.checked_add(chuman::COUNTRY_TAG) else {
        return false;
    };
    let Some(flags_address) = pointer.checked_add(chuman::FLAGS) else {
        return false;
    };
    let Some(strings_address) = pointer.checked_add(chuman::FIRST_STRING) else {
        return false;
    };
    let Some(identity) = pspan(log, memory, Some(pointer), 8, P_PLAYERS) else {
        return false;
    };
    let Some(key) = pspan(log, memory, Some(key_address), 4, P_PLAYERS) else {
        return false;
    };
    if pspan(log, memory, Some(country_address), 4, P_PLAYERS).is_none() {
        return false;
    }
    if pspan(log, memory, Some(flags_address), 1, P_PLAYERS).is_none() {
        return false;
    }
    let Some(strings) = pspan(log, memory, Some(strings_address), 64, P_PLAYERS) else {
        return false;
    };
    let key = i32::from_le_bytes(log.bytes[key..key + 4].try_into().unwrap());
    if ppool_u64(log, identity) != vtable as u64 || key == -1 || keys.contains(&key) {
        return false;
    }
    keys.push(key);
    let mut headers = [0_u8; 64];
    headers.copy_from_slice(&log.bytes[strings..strings + 64]);
    let mut payloads = [None; 2];
    for (index, chunk) in headers.chunks_exact(32).enumerate() {
        let len = u64::from_le_bytes(chunk[16..24].try_into().unwrap());
        let capacity = u64::from_le_bytes(chunk[24..32].try_into().unwrap());
        if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
            return false;
        }
        let Ok(len) = usize::try_from(len) else {
            return false;
        };
        if capacity == msvc_string::INLINE_CAPACITY {
            if chunk[len] != 0 {
                return false;
            }
            continue;
        }
        let Some(source) = usize::try_from(u64::from_le_bytes(chunk[..8].try_into().unwrap()))
            .ok()
            .filter(|&source| source != 0)
        else {
            return false;
        };
        let Some(terminator_address) = source.checked_add(len) else {
            return false;
        };
        let Some(terminator) = pspan(log, memory, Some(terminator_address), 1, P_PLAYERS) else {
            return false;
        };
        if log.bytes[terminator] != 0 {
            return false;
        }
        payloads[index] = Some((source, len));
    }
    for (source, len) in payloads.into_iter().flatten() {
        if pspan(log, memory, Some(source), len, P_PLAYERS).is_none() {
            return false;
        }
    }
    true
}

fn copy_players_pass(
    log: &mut RawLog<2>,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
) {
    copy_player_roster(log, memory, base, game_state);
    if !log.overflowed {
        copy_controller_pass(log, memory, base, game_state);
    }
}

fn copy_player_roster(
    log: &mut RawLog<2>,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
) {
    use engine_native::{cgame_state, chuman, cpdx_array::Array};
    log.has_bulk = true;
    let roster_start = match pspan(
        log,
        memory,
        game_state.checked_add(cgame_state::PLAYER_RECORDS),
        16,
        P_PLAYERS,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[roster_start..roster_start + 16]);
    let roster = match Array::from_header(raw, chuman::STRIDE) {
        Some(a) => a,
        None => return,
    };
    let Some(vtable) = engine_sites::at(base, chuman::VTABLE_RVA) else {
        return;
    };
    let Ok(mut keys) = engine_allocation::vec(roster.count as usize) else {
        return;
    };
    for index in 0..roster.count {
        let Some(address) = roster.element(index, chuman::STRIDE) else {
            return;
        };
        if !copy_player(log, memory, address, vtable, &mut keys) {
            return;
        }
    }
}

fn copy_controller_pass(
    log: &mut RawLog<2>,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
) {
    use std::cell::RefCell;
    struct ControllerCopy<'a, M> {
        memory: &'a M,
        log: RefCell<&'a mut RawLog<2>>,
    }
    impl<M: engine_native::Memory> engine_native::Memory for ControllerCopy<'_, M> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let mut log = self.log.borrow_mut();
            let Some(start) = log.take(self.memory, address, output.len(), P_CONTROLLERS) else {
                return false;
            };
            output.copy_from_slice(&log.bytes[start..start + output.len()]);
            true
        }
    }
    let _ = chuman::count_controllers(
        &ControllerCopy {
            memory,
            log: RefCell::new(log),
        },
        base,
        game_state,
    );
}

/// # Safety
/// Only the admitted player acquisition may read against the active game's located image, into the
/// pending job for that hour, before decode takes it.
pub unsafe fn copy_players_refresh(
    memory: &impl engine_native::Memory,
    ctx: PlayersCopyContext,
    job: &mut PlayersJob,
) -> PlayersCopyReport {
    let _reads = collector_memory::read_accounting::table(TableId::Players);
    let started = qpc();
    job.clear();
    job.game_state = ctx.game_state;
    copy_players_pass(&mut job.log, memory, ctx.base, ctx.game_state);
    PlayersCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: [elapsed(started), 0],
    }
}

impl MapCaptured {
    pub fn fail_players_capture(&mut self, reason: &'static str) {
        self.players.outcome = Outcome::Unavailable { reason };
        self.human_controllers.outcome = Outcome::Unavailable { reason };
        self.player_names_len = 0;
        self.players_raw_mode = false;
    }

    /// # Safety
    /// Decode the completed player job into its originating hour, using its active image while that
    /// session's roster accounting remains live.
    pub unsafe fn decode_players_payload(
        &mut self,
        base: usize,
        job: &PlayersJob,
        decode_ticks: &mut [u64; 2],
    ) {
        let recorded = match collector_raw::raw::Recorded::build(&job.log) {
            Ok(recorded) => recorded,
            Err(reason) => {
                self.fail_players_capture(reason);
                return;
            }
        };
        let started = qpc();
        let walked = chuman::walk(
            &recorded,
            base,
            job.game_state,
            &mut self.players.rows,
            &mut self.player_names,
        );
        self.players.outcome = walked.outcome;
        self.player_names_len = walked.bytes_len;
        self.player_names_required = walked.required_bytes;
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        self.human_controllers.outcome = chuman::walk_controllers(
            &recorded,
            base,
            job.game_state,
            &mut self.human_controllers.rows,
        );
        decode_ticks[1] += elapsed(started);
        self.players_raw_mode = false;
    }
}

#[cfg(test)]
mod players_sizing_tests;

#[cfg(test)]
mod players_split_tests {
    use super::NoSources;
    use super::{
        MapCapacities, MapCaptured, PlayersCopyContext, PlayersCopyReport, PlayersJob,
        copy_players_refresh,
    };
    use engine_native::{ccountry, cgame_state, chuman, memory::SliceMemory, walk::Outcome};

    const BASE: usize = 0;
    const GS: usize = 0x10000;
    const ROSTER: usize = 0x20000;
    const COUNTRY_PTR: usize = 0x11000;
    const LINKS: usize = 0x12000;
    const COUNTRY: usize = 0x13000;
    const COUNTS: usize = 0x14000;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - GS..address - GS + value.len()].copy_from_slice(value);
    }

    fn array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
        put(bytes, header, &(data as u64).to_le_bytes());
        put(bytes, header + 8, &count.to_le_bytes());
        put(bytes, header + 12, &count.to_le_bytes());
    }

    fn inline_string(bytes: &mut [u8], at: usize, value: &[u8]) {
        assert!(value.len() <= 15);
        let mut header = [0u8; 32];
        header[..value.len()].copy_from_slice(value);
        header[value.len()] = 0;
        header[16..24].copy_from_slice(&(value.len() as u64).to_le_bytes());
        header[24..32].copy_from_slice(&15_u64.to_le_bytes());
        put(bytes, at, &header);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = engine_allocation::owned_vec(engine_allocation::default_vec(0x20000));
        array(&mut bytes, GS + cgame_state::COUNTRIES, COUNTRY_PTR, 2);
        put(&mut bytes, COUNTRY_PTR + 8, &(COUNTRY as u64).to_le_bytes());
        array(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS, 2);
        put(&mut bytes, LINKS + 4, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            COUNTRY,
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
        array(&mut bytes, GS + cgame_state::HUMAN_STATUS, COUNTS, 2);
        put(&mut bytes, COUNTS + 1, &[7]);
        array(&mut bytes, GS + cgame_state::PLAYER_RECORDS, ROSTER, 1);
        put(
            &mut bytes,
            ROSTER,
            &(chuman::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            ROSTER + chuman::PLAYER_KEY,
            &42_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            ROSTER + chuman::COUNTRY_TAG,
            &1_u32.to_le_bytes(),
        );
        put(&mut bytes, ROSTER + chuman::FLAGS, &[3]);
        inline_string(&mut bytes, ROSTER + chuman::FIRST_STRING, b"ab");
        inline_string(&mut bytes, ROSTER + chuman::NAME, b"c");
        bytes
    }

    fn capacities() -> MapCapacities {
        MapCapacities::default()
    }

    fn direct(memory: &SliceMemory<'_>) -> MapCaptured {
        let mut captured =
        // SAFETY: These are session-measured map capacities or capacities owned by an isolated
        // fixture.
        unsafe { MapCaptured::with_capacities(&capacities()) };
        // SAFETY: The admitted hour or isolated fixture owns this map and the corresponding game
        // image.
        unsafe { captured.sample(memory, BASE, GS, &mut NoSources) };
        captured
    }

    fn split(memory: &SliceMemory<'_>) -> (MapCaptured, PlayersCopyReport) {
        let mut captured =
        // SAFETY: These are session-measured map capacities or capacities owned by an isolated
        // fixture.
        unsafe { MapCaptured::with_capacities(&capacities()) };
        let ctx = PlayersCopyContext {
            base: BASE,
            game_state: GS,
        };
        let mut job = PlayersJob::new();
        let report =
        // SAFETY: This is the admitted hour's roster request and its authenticated image.
        unsafe { copy_players_refresh(memory, ctx, &mut job) };
        assert!(report.raw_ok);
        captured.players_raw_mode = true;
        // SAFETY: Decode is settling the matching job into its originating frame.
        unsafe { captured.decode_players_payload(BASE, &job, &mut [0; 2]) };
        (captured, report)
    }

    #[test]
    fn players_split_copy_matches_direct_walk() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let direct = direct(&memory);
        let (split, report) = split(&memory);
        assert!(report.raw_ok);
        assert_eq!(split.players.outcome, direct.players.outcome);
        assert_eq!(
            split.human_controllers.outcome,
            direct.human_controllers.outcome
        );
        assert_eq!(split.players.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.human_controllers.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.players.rows[0].id, 42);
        assert_eq!(split.human_controllers.rows[0].count, 7);
        assert_eq!(&split.player_names[..split.player_names_len], b"abc");
    }

    #[test]
    fn players_cold_and_reusable_storage_match_direct() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        for (players, controllers) in [(0, 0), (1, 1), (0, 1), (1, 0), (64, 64)] {
            let mut caps = capacities();
            caps.rows[0] = players;
            caps.rows[1] = controllers;
            let mut direct =
            // SAFETY: These are session-measured map capacities or capacities owned by an isolated
            // fixture.
            unsafe { MapCaptured::with_capacities(&caps) };
            let _ =
            // SAFETY: The admitted hour or isolated fixture owns this map and the corresponding
            // game image.
            unsafe { direct.sample(&memory, BASE, GS, &mut NoSources) };
            let mut captured =
            // SAFETY: These are session-measured map capacities or capacities owned by an isolated
            // fixture.
            unsafe { MapCaptured::with_capacities(&caps) };
            let ctx = PlayersCopyContext {
                base: BASE,
                game_state: GS,
            };
            let mut first = PlayersJob::new();

            let report =
            // SAFETY: This is the admitted hour's roster request and its authenticated image.
            unsafe { copy_players_refresh(&memory, ctx, &mut first) };

            assert!(report.raw_ok, "players={players} controllers={controllers}");
            captured.players_raw_mode = true;
            let mut ticks = [0u64; 2];
            // SAFETY: Decode is settling the matching job into its originating frame.
            unsafe { captured.decode_players_payload(BASE, &first, &mut ticks) };
            assert_eq!(
                captured.players.outcome, direct.players.outcome,
                "players outcome p={players} c={controllers}"
            );
            assert_eq!(
                captured.human_controllers.outcome, direct.human_controllers.outcome,
                "controllers outcome p={players} c={controllers}"
            );
            assert_eq!(
                &captured.players.rows[..captured.players.outcome.len()],
                &direct.players.rows[..direct.players.outcome.len()],
                "player rows"
            );
        }
        let mut caps = capacities();
        caps.rows[0] = 1;
        caps.rows[1] = 1;
        let mut direct =
        // SAFETY: These are session-measured map capacities or capacities owned by an isolated
        // fixture.
        unsafe { MapCaptured::with_capacities(&caps) };
        let _ =
        // SAFETY: The admitted hour or isolated fixture owns this map and the corresponding game
        // image.
        unsafe { direct.sample(&memory, BASE, GS, &mut NoSources) };
        assert_eq!(direct.players.outcome, Outcome::Rows { len: 1 });
    }

    #[test]
    fn players_bulk_string_pair_serves_enclosed_sub_reads() {
        use engine_native::Memory;
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let mut job = PlayersJob::new();
        let report =
        // SAFETY: This is the admitted hour's roster request and its authenticated image.
        unsafe { copy_players_refresh(
            &memory,
            PlayersCopyContext {
                base: BASE,
                game_state: GS,
            },
            &mut job,
        ) };
        assert!(report.raw_ok);
        assert!(job.log.has_bulk);
        let recorded = collector_raw::raw::Recorded::build(&job.log).unwrap();
        let pair = job
            .log
            .spans
            .iter()
            .find(|span| span.ok && span.bulk && span.len == 64)
            .expect("bulk string pair");
        let mut half = [0u8; 32];
        assert!(recorded.copy(pair.addr, &mut half));
        let mut expected = [0u8; 32];
        assert!(memory.copy(pair.addr, &mut expected));
        assert_eq!(half, expected);
        let mut unaligned = [0u8; 7];
        assert!(recorded.copy(pair.addr + 3, &mut unaligned));
        let mut expected7 = [0u8; 7];
        assert!(memory.copy(pair.addr + 3, &mut expected7));
        assert_eq!(unaligned, expected7);
        let mut straddle = [0u8; 32];
        assert!(!recorded.copy(pair.addr + 48, &mut straddle));
    }

    #[test]
    fn players_split_copy_matches_direct_walk_from_empty_rows() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let mut capacities = capacities();
        capacities.rows[0] = 0;
        let mut direct =
        // SAFETY: These are session-measured map capacities or capacities owned by an isolated
        // fixture.
        unsafe { MapCaptured::with_capacities(&capacities) };
        // SAFETY: The admitted hour or isolated fixture owns this map and the corresponding game
        // image.
        unsafe { direct.sample(&memory, BASE, GS, &mut NoSources) };
        let mut captured =
        // SAFETY: These are session-measured map capacities or capacities owned by an isolated
        // fixture.
        unsafe { MapCaptured::with_capacities(&capacities) };
        let ctx = PlayersCopyContext {
            base: BASE,
            game_state: GS,
        };
        let mut first = PlayersJob::new();
        let report =
        // SAFETY: This is the admitted hour's roster request and its authenticated image.
        unsafe { copy_players_refresh(&memory, ctx, &mut first) };

        assert!(report.raw_ok);
        captured.players_raw_mode = true;
        let mut ticks = [0u64; 2];
        // SAFETY: Decode is settling the matching job into its originating frame.
        unsafe { captured.decode_players_payload(BASE, &first, &mut ticks) };
        assert_eq!(captured.players.outcome, direct.players.outcome);
    }

    #[test]
    fn players_staging_grows_from_cold_storage_before_publication() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let ctx = PlayersCopyContext {
            base: BASE,
            game_state: GS,
        };
        let mut tiny = PlayersJob {
            log: collector_raw::raw::RawLog::new(8, 8),
            game_state: 0,
        };
        let report =
        // SAFETY: This is the admitted hour's roster request and its authenticated image.
        unsafe { copy_players_refresh(&memory, ctx, &mut tiny) };

        assert!(report.raw_ok);
        assert!(!tiny.log.overflowed);
        let mut captured =
        // SAFETY: These are session-measured map capacities or capacities owned by an isolated
        // fixture.
        unsafe { MapCaptured::with_capacities(&capacities()) };
        // SAFETY: Decode is settling the matching job into its originating frame.
        unsafe { captured.decode_players_payload(BASE, &tiny, &mut [0; 2]) };
        assert_eq!(captured.players.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.human_controllers.outcome, Outcome::Rows { len: 1 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ship_name_arenas_charge_capacity_and_clear_on_reset_and_failure() {
        let mut census = ShipCensus::vacant();
        let before = census.allocated_bytes();
        census.ship_names.resize(8193, 0xff);
        census.ship_overrides.resize(32769, 0);
        let arenas = census.ship_names.capacity() + census.ship_overrides.capacity();
        assert_eq!(census.allocated_bytes(), before + arenas);
        census.reset();
        assert!(census.ship_names.is_empty());
        assert!(census.ship_overrides.is_empty());
        assert_eq!(census.allocated_bytes(), before + arenas);
        census.ship_names.extend_from_slice(b"Retained");
        census.ship_overrides.extend_from_slice(b"Override");
        census.fail("synthetic failed ship census");
        assert!(census.ship_names.is_empty());
        assert!(census.ship_overrides.is_empty());
        assert_eq!(census.allocated_bytes(), before + arenas);
        assert!(matches!(census.ships.outcome, Outcome::Unavailable { .. }));
    }

    #[test]
    fn land_visitors_receive_one_census_and_air_comes_only_from_its_worker() {
        #[derive(Default)]
        struct Unreadable(std::cell::Cell<usize>);
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                self.0.set(self.0.get() + 1);
                false
            }
        }
        #[derive(Default)]
        struct Sources {
            begins: usize,
            finished: Vec<Outcome>,
        }
        impl MapSources for Sources {
            fn begin_land(&mut self) -> u64 {
                self.begins += 1;
                0
            }
            fn land(&mut self, _: cland_combat::Source) -> u64 {
                panic!("unreadable census visited combat")
            }
            fn finish_land(&mut self, outcome: Outcome) -> u64 {
                self.finished.push(outcome);
                0
            }
        }
        let mut sources = Sources::default();
        let mut captured =
        // SAFETY: These are session-measured map capacities or capacities owned by an isolated
        // fixture.
        unsafe { MapCaptured::with_capacities(&MapCapacities::default()) };
        let memory = Unreadable::default();
        // SAFETY: The admitted hour or isolated fixture owns this map and the corresponding game
        // image.
        unsafe { captured.sample(&memory, 0, 0x1000, &mut sources) };
        assert_eq!(sources.begins, 1);
        assert!(matches!(
            sources.finished[..],
            [Outcome::Unavailable { .. }]
        ));
        assert_eq!(captured.air_wings.outcome, AIR_NOT_ACQUIRED);
        let ordinary_reads = memory.0.get();
        let ordinary_outcomes = captured.outcomes();
        captured.reset();
        let reason = "accepted wing worker failed";
        captured.fail_air_capture(reason);
        memory.0.set(0);
        let mut sources = Sources::default();
        // SAFETY: The admitted hour or isolated fixture owns this map and the corresponding game
        // image.
        unsafe { captured.sample(&memory, 0, 0x1000, &mut sources) };
        assert_eq!(captured.air_wings.outcome, Outcome::Unavailable { reason });
        assert_eq!(memory.0.get(), ordinary_reads);
        for (index, outcome) in captured.outcomes().into_iter().enumerate() {
            if index != 8 {
                assert_eq!(outcome, ordinary_outcomes[index]);
            }
        }
        captured.reset();
        // SAFETY: The admitted hour or isolated fixture owns this map and the corresponding game
        // image.
        unsafe { captured.sample(&memory, 0, 0x1000, &mut Sources::default()) };
        assert_eq!(captured.air_wings.outcome, AIR_NOT_ACQUIRED);
    }

    #[test]
    fn capacity_pressure_adds_headroom_without_shrinking_or_overflowing() {
        let mut capacity = 64;
        grow_capacity(&mut capacity, 133);
        assert_eq!(capacity, 256);
        for required in [134, 201, 256, 0] {
            grow_capacity(&mut capacity, required);
            assert_eq!(capacity, 256);
        }
        grow_capacity(&mut capacity, 257);
        assert_eq!(capacity, 512);
        grow_capacity(&mut capacity, usize::MAX);
        assert_eq!(capacity, usize::MAX);
        let mut empty = 0;
        grow_capacity(&mut empty, 3);
        assert_eq!(empty, 4);
    }

    #[test]
    fn new_buffers_use_current_primary_capacities() {
        let mut capacity = MapCapacities::default();
        capacity.rows[5] += 31;
        capacity.player_names += 19;
        capacity.land_participants += 13;
        capacity.order_geometry += 17;
        let captured =
        // SAFETY: These are session-measured map capacities or capacities owned by an isolated
        // fixture.
        unsafe { MapCaptured::with_capacities(&capacity) };
        assert_eq!(captured.naval.ships.rows.len(), capacity.rows[5]);
        assert_eq!(captured.player_names.len(), capacity.player_names);
        assert_eq!(captured.land_participants.len(), capacity.land_participants);
        assert_eq!(captured.land_classes.len(), capacity.land_participants);
        assert_eq!(captured.orders.geometry.len(), capacity.order_geometry);
    }

    #[test]
    fn order_failure_keeps_storage_and_growth_demand_but_hides_sources() {
        // SAFETY: The test process has no live attachment and uses fixture-owned capacities.
        let mut orders = unsafe { MapCapacities::default().orders() };
        orders.rows.outcome = Outcome::Rows { len: 1 };
        orders.root_outcome = Outcome::Rows { len: 1 };
        orders.geometry_len = 3;
        orders.geometry_required = 17;
        orders.groups_required = 23;
        orders.visits_required = 41;
        orders.ticks = 99;
        let bytes = orders.allocated_bytes();
        orders.fail("worker failed");
        let unavailable = Outcome::Unavailable {
            reason: "worker failed",
        };
        assert_eq!(orders.rows.outcome, unavailable);
        assert_eq!(orders.root_outcome, unavailable);
        assert!(orders.roots().is_empty());
        assert_eq!((orders.geometry_len, orders.ticks), (0, 0));
        assert_eq!(
            (
                orders.geometry_required,
                orders.groups_required,
                orders.visits_required
            ),
            (17, 23, 41)
        );
        assert_eq!(orders.allocated_bytes(), bytes);
    }

    #[test]
    fn pooled_growth_preserves_complete_rows_and_counts_whole_omissions() {
        let mut capacity = MapCapacities {
            player_names: 4096,
            land_participants: 8192,
            order_geometry: 16384,
            ..MapCapacities::default()
        };
        capacity.rows[5] = 4096;
        let mut captured =
        // SAFETY: These are session-measured map capacities or capacities owned by an isolated
        // fixture.
        unsafe { MapCaptured::with_capacities(&capacity) };
        captured.naval.ships.rows[0].id = u64::MAX;
        captured.naval.ships.outcome = Outcome::Truncated {
            len: capacity.rows[5],
            unread_rows: 7,
        };
        captured.land_combats.outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 1,
        };
        captured.land_participants_required = capacity.land_participants + 11;
        captured.player_names_required = capacity.player_names + 17;
        captured.orders.geometry_required = capacity.order_geometry + 23;
        // SAFETY: This path owns the frame capacity changes and excludes teardown.
        unsafe { captured.grow(&mut capacity) };
        assert_eq!(captured.naval.ships.rows.len(), 8192);

        assert_eq!(captured.naval.ships.rows[0].id, u64::MAX);
        assert_eq!(captured.land_participants.len(), 16384);
        assert_eq!(captured.land_classes.len(), 16384);

        assert_eq!(captured.player_names.len(), 8192);

        assert_eq!(captured.orders.geometry.len(), 16384 + 23);

        let mut idle =
        // SAFETY: This fixture owns the map allocation and runs without a live attachment.
        unsafe { MapCaptured::new() };
        // SAFETY: This path owns the frame capacity changes and excludes teardown.
        unsafe { idle.grow(&mut capacity) };
        assert_eq!(idle.naval.ships.rows.len(), captured.naval.ships.rows.len());
        assert_eq!(
            idle.land_participants.len(),
            captured.land_participants.len()
        );
        assert_eq!(idle.player_names.len(), captured.player_names.len());
        assert_eq!(idle.orders.geometry.len(), captured.orders.geometry.len());
        assert!(idle.allocated_bytes() <= captured.allocated_bytes());
    }

    #[test]
    fn gradually_tables_fit_without_first_losing_an_hour() {
        let mut capacity = 8;
        let mut table = CaptureRows::<u32>::new(capacity);
        for count in 1..=129 {
            assert!(count <= table.rows.len());
            table.rows[count - 1] = count as u32;
            table.outcome = Outcome::Rows { len: count };
            table.grow(&mut capacity);
            assert_eq!(table.rows[count - 1], count as u32);
        }
        assert!(capacity <= 129 * 3);
    }

    #[test]
    fn orders_census_over_walked_roots_matches_its_own_registry_walk() {
        use collector_extended_support::bench_world::{self, Shape};
        let world = bench_world::build(
            40,
            Shape {
                named: true,
                theatre_links: true,
            },
        );
        let memory = world.slice();
        for capacity in [
            MapCapacities::default(),
            MapCapacities {
                order_groups: 3,
                ..MapCapacities::default()
            },
        ] {
            // SAFETY: The test process has no live attachment and uses fixture-owned capacities.
            let mut own = unsafe { capacity.orders() };
            own.walk(&memory, world.base);
            // SAFETY: The test process has no live attachment and uses the same fixture capacities.
            let mut reused = unsafe { capacity.orders() };
            reused.walk_roots(&memory, world.base);
            assert!(reused.roots_walked);
            reused.walk(&memory, world.base);
            assert!(!reused.roots_walked);
            let observed = |orders: &Orders| {
                format!(
                    "{:?}",
                    (
                        orders.rows.outcome,
                        orders.root_outcome,
                        &orders.rows.rows[..orders.rows.outcome.len()],
                        &orders.geometry[..orders.geometry_len],
                        orders.roots(),
                        &orders.visits,
                        &orders.successors,
                        (
                            orders.geometry_required,
                            orders.groups_required,
                            orders.visits_required
                        ),
                    )
                )
            };
            assert_eq!(observed(&reused), observed(&own));
        }
        // SAFETY: The test process has no live attachment and uses fixture-owned capacities.
        let mut orders = unsafe { MapCapacities::default().orders() };
        orders.walk_roots(&memory, world.base);
        orders.reset();
        assert!(!orders.roots_walked);
    }

    #[test]
    fn census_plan_gathers_are_charged_to_the_plans() {
        use collector_extended_support::bench_world::{self, Shape};
        let world = bench_world::build(
            40,
            Shape {
                named: true,
                theatre_links: true,
            },
        );
        // SAFETY: The test process has no live attachment and uses fixture-owned capacities.
        let mut orders = unsafe { MapCapacities::default().orders() };
        let ((), counts) = world.counted(|memory| {
            orders.walk_roots(memory, world.base);
            orders.walk(memory, world.base);
        });
        assert_eq!(
            orders.rows.outcome,
            Outcome::Rows {
                len: world.orders as usize
            }
        );
        let ((), plain) = world.counted(|memory| {
            let _reads = collector_memory::read_accounting::table(TableId::ArmyOrders);
            corder_instance::walk(
                memory,
                world.base,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut vec![corders_group::Root::default(); world.groups as usize],
                &mut Vec::new(),
            );
        });
        let orders_index = TableId::ArmyOrders.index();
        let plans_index = TableId::ArmyOrderPlans.index();
        assert_eq!(counts[orders_index], plain[orders_index]);
        let (gathers, _) = orders.successor_lists().plan_gathers();
        assert_eq!(counts[plans_index], gathers);
        assert!(gathers.record_calls != 0 && gathers.spans >= u64::from(world.orders) * 7);
    }

    #[test]
    fn the_next_hour_takes_the_split_the_last_stitched_hour_planned() {
        use collector_divisions::orders_parts::{MAX_PARTS, Part, Split};
        use collector_extended_support::bench_world::{self, Shape};
        let world = bench_world::build(
            40,
            Shape {
                named: true,
                theatre_links: true,
            },
        );
        let memory = world.slice();
        let frequency = 10_000_000;
        let hour = |orders: &mut Orders, split: Split, millis: u64| {
            orders.reset();
            orders.begin_parts(split, frequency);
            orders.walk_roots(&memory, world.base);
            orders.prepare_parts();
            for index in 0..orders.part_count() {
                let mut part = orders.take_part(index).unwrap();
                part.run(&memory, world.base);
                part.census_ticks = if index == 0 {
                    frequency * millis / 1_000
                } else {
                    0
                };
                part.plan_ticks = 0;
                orders.return_part(index, part);
            }
            orders.stitch();
            assert!(!orders.failed());
            orders.plan_split();
            orders.next_split()
        };
        // SAFETY: The test process has no live attachment and uses fixture-owned capacities.
        let mut frames = unsafe {
            [
                MapCapacities::default().orders(),
                MapCapacities::default().orders(),
            ]
        };
        let split = hour(&mut frames[0], Split::default(), 3);
        assert_eq!(split.parts(), MAX_PARTS);
        let next = hour(&mut frames[1], split, 0);
        assert_eq!(frames[1].part_count(), MAX_PARTS);
        assert!(frames[1].parts_allocated_bytes() >= MAX_PARTS * size_of::<Part>());
        assert_eq!(next.parts(), 1);
        hour(&mut frames[0], next, 0);
        assert_eq!(frames[0].part_count(), 1);
    }

    #[test]
    fn failed_roots_without_parts_refuse_the_census_as_the_single_census_does() {
        struct NoMemory;
        impl Memory for NoMemory {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        let reason = "orders group identity unreadable";
        // SAFETY: The test process has no live attachment and uses fixture-owned capacities.
        let mut single = unsafe { MapCapacities::default().orders() };
        single.fail_roots(reason);
        single.walk(&NoMemory, 0);
        // SAFETY: The test process has no live attachment and uses fixture-owned capacities.
        let mut parted = unsafe { MapCapacities::default().orders() };
        parted.begin_parts(
            collector_divisions::orders_parts::Split::default(),
            10_000_000,
        );
        parted.fail_roots(reason);
        assert_eq!(parted.stitch(), 0);
        assert!(!parted.failed());
        assert_eq!(parted.rows.outcome, Outcome::Unavailable { reason });
        let state = |orders: &Orders| {
            (
                orders.rows.outcome,
                orders.root_outcome,
                orders.geometry_len,
                orders.geometry_required,
                orders.groups_required,
                orders.visits_required,
                orders.successor_lists().census().len(),
            )
        };
        assert_eq!(state(&parted), state(&single));
    }

    #[test]
    fn worker_order_demand_replenishes_reuse_without_allocator_headroom() {
        let mut capacity = MapCapacities::default();
        // SAFETY: The test process has no live attachment and uses fixture-owned capacities.
        let mut orders = unsafe { capacity.orders() };
        orders
            .visits
            .resize(5_000, corder_instance::Visit::default());
        orders.visits.reserve_exact(100_003);
        orders.visits_required = 5_000;
        let worker_capacity = orders.visits.capacity();
        // SAFETY: Isolated unit test owns `orders` and `capacity`.
        unsafe {
            orders.grow(
                &mut capacity.rows[9],
                &mut capacity.order_geometry,
                &mut capacity.order_groups,
                &mut capacity.order_visits,
            );
        }
        assert!(capacity.order_visits >= 5_000);
        assert!(capacity.order_visits < worker_capacity);
        // SAFETY: The test process has no live attachment and only this fixture grew the capacities.
        assert_eq!(unsafe { capacity.orders() }.visits.len(), capacity.order_visits);
    }

    #[test]
    fn unavailable_scratch_walks_grow_without_inventing_truncated_row_counts() {
        let mut capacity = MapCapacities {
            order_groups: 2048,
            ..MapCapacities::default()
        };
        let mut captured =
        // SAFETY: These are session-measured map capacities or capacities owned by an isolated
        // fixture.
        unsafe { MapCaptured::with_capacities(&capacity) };
        captured.orders.rows.outcome = Outcome::Unavailable {
            reason: "orders scratch full",
        };
        let initial_group_capacity = capacity.order_groups;
        captured.orders.groups_required = initial_group_capacity + 1;
        captured.orders.visits_required = 4096;
        captured.naval_combat_participants.outcome = Outcome::Unavailable {
            reason: "naval groups scratch full",
        };
        captured.naval_groups_required = 129;
        // SAFETY: This path owns the frame capacity changes and excludes teardown.
        unsafe { captured.grow(&mut capacity) };
        assert_eq!(captured.orders.groups.len(), initial_group_capacity * 2);
        assert_eq!(captured.orders.visits.len(), 4096);
        assert_eq!(captured.naval_groups.len(), 256);
        let grown = capacity;
        captured.orders.groups_required = initial_group_capacity + 47;
        captured.orders.visits_required = 3500;
        captured.naval_groups_required = 201;
        // SAFETY: This path owns the frame capacity changes and excludes teardown.
        unsafe { captured.grow(&mut capacity) };
        assert!(capacity == grown);
        assert!(matches!(
            captured.orders.rows.outcome,
            Outcome::Unavailable { .. }
        ));
        assert!(matches!(
            captured.naval_combat_participants.outcome,
            Outcome::Unavailable { .. }
        ));
    }
}
