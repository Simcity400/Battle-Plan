use collector_memory::read_accounting;
use collector_primitives::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, bounded, grow_capacity, grow_vec},
};
use collector_projection::projection::{Projection, Witness, project_table, scalar_table};
use collector_projection::table_conversion::unavailable;
use collector_raw::raw::RawLog;
use engine_allocation::{collect_vec, copy_vec, default_vec, owned_vec, resize_default};
use engine_native_memory::{
    Memory,
    cgame_date,
    cpdx_array::Array,
    msvc_string,
    walk::Outcome,
};
use engine_native_parts::{
    cair_group,
    cair_theatre,
    cstrategic_region,
    csub_unit_definition,
    ctheater_group,
};
use engine_native::{cgame_state, corders_group, cprovince, ctheatre};
use engine_native_air::cair_wing;
use engine_naval_history::{
    cdominance_values, cnaval_combat_results, cnaval_region_dominance, csunk_convoy_info,
    csunk_ship_info, naval_dominance_entry,
};
use engine_railways::{cprovince_railway_info, crailway_manager, rail_neighbor};
use replay_data::{rows, rows::RowCodec, rows::TableId, rows::TableRows, rows::tables};
use replay_frame::frame::{TableInput, TableView};

pub const N: usize = 17;
pub const TABLES: [TableId; N] = [
    TableId::NavalSupremacy,
    TableId::SunkConvoys,
    TableId::RailCooldowns,
    TableId::RailEdges,
    TableId::LandTheatres,
    TableId::LandTheatreGroups,
    TableId::LandTheatreMembers,
    TableId::AirTheatres,
    TableId::AirTheatreGroups,
    TableId::AirTheatreMembers,
    TableId::NavalResults,
    TableId::NavalResultMembers,
    TableId::NavalResultCountries,
    TableId::SunkShips,
    TableId::OrdersGroups,
    TableId::OrdersGroupMembers,
    TableId::ArmyGroupMembers,
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub theatre_names: usize,
    pub sunk_names: usize,
    pub orders_sources: usize,
    pub wing_sources: usize,
    pub roster_scratch: usize,
    pub rail_levels: usize,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    theatre_names,
    sunk_names,
    orders_sources,
    wing_sources,
    roster_scratch,
    rail_levels,
});

impl Capacities {
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture() -> Self {
        Self {
            rows: [
                4096, 1024, 16384, 32768, 512, 1024, 4096, 512, 2048, 8192, 512, 8192, 4096, 2048,
                2048, 8192, 2048,
            ],
            theatre_names: 128 * 1024,
            sunk_names: 256 * 1024,
            orders_sources: 4096,
            wing_sources: 8192,
            roster_scratch: 4096,
            rail_levels: 0,
        }
    }

    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        use collector_extended_support::demand::rows;
        use engine_native_memory::walk::Buffer;
        let mut failure = None;
        let mut accept = |slot: usize, outcome: Outcome| match rows(outcome) {
            Ok(required) => self.rows[slot] = self.rows[slot].max(required),
            Err(reason) => {
                failure.get_or_insert(reason);
            }
        };
        accept(0, cnaval_region_dominance::count(memory, base, state));
        accept(1, csunk_convoy_info::count(memory, base, state));
        let (
            mut land,
            mut land_groups,
            mut land_members,
            mut air,
            mut air_groups,
            mut air_members,
            mut names,
        ) = (
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let theatres = ctheatre::walk_buffers(
            memory,
            base,
            state,
            true,
            ctheatre::Storage {
                land: Buffer::Growing(&mut land),
                land_groups: Buffer::Growing(&mut land_groups),
                land_members: Buffer::Growing(&mut land_members),
                air: Buffer::Growing(&mut air),
                air_groups: Buffer::Growing(&mut air_groups),
                air_members: Buffer::Growing(&mut air_members),
                names: Buffer::Growing(&mut names),
            },
        );
        for (slot, outcome) in [
            theatres.land,
            theatres.land_groups,
            theatres.land_members,
            theatres.air,
            theatres.air_groups,
            theatres.air_members,
        ]
        .into_iter()
        .enumerate()
        {
            accept(slot + 4, outcome);
        }
        self.theatre_names = self.theatre_names.max(theatres.names_required);
        let (mut cooldowns, mut edges, mut buildings, mut level_values) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut levels = crailway_manager::selected_inputs::Levels {
            values: &mut level_values,
            len: 0,
            required: 0,
        };
        let railway = crailway_manager::walk_with_selection(
            memory,
            base,
            state,
            &mut cooldowns,
            &mut edges,
            &mut buildings,
            &mut levels,
        );
        accept(2, railway.rows.provinces);
        accept(3, railway.rows.edges);
        self.rail_levels = self.rail_levels.max(levels.required);
        let (mut results, mut slots, mut members, mut countries) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let results_walk = cnaval_combat_results::walk(
            memory,
            base,
            state,
            &mut results,
            &mut slots,
            &mut members,
            &mut countries,
        );
        accept(10, results_walk.records);
        accept(11, results_walk.members);
        accept(12, results_walk.countries);
        let mut roots = Vec::new();
        let root_outcome = corders_group::walk_roots(memory, base, &mut roots);
        if let Outcome::Rows { len } = root_outcome {
            self.orders_sources = self.orders_sources.max(len);
            self.roster_scratch = self.roster_scratch.max(len);
            let (mut groups, mut members, mut children, mut scratch, mut group_names) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let oob = corders_group::walk_order_of_battle_buffers(
                memory,
                base,
                &roots[..len],
                &mut corders_group::OrderOfBattleStorage {
                    groups: Buffer::Growing(&mut groups),
                    members: Buffer::Growing(&mut members),
                    children: Buffer::Growing(&mut children),
                    roster_scratch: Buffer::Growing(&mut scratch),
                },
            );
            accept(14, oob.groups);
            accept(15, oob.members);
            accept(16, oob.children);
            let (mut used, mut required) = (0, 0);
            for root in &roots[..len] {
                if let Err(reason) = corders_group::copy_name_buffer(
                    memory,
                    root.pointer(),
                    Buffer::Growing(&mut group_names),
                    &mut used,
                    &mut required,
                ) {
                    failure.get_or_insert(reason);
                }
            }
            self.theatre_names = self.theatre_names.max(required);
        } else if let Outcome::Unavailable { reason } = root_outcome {
            failure.get_or_insert(reason);
        }
        match csunk_ship_info::storage_demand(memory, base, state) {
            Ok((rows, text)) => {
                self.rows[13] = self.rows[13].max(rows);
                self.sunk_names = self.sunk_names.max(text);
            }
            Err(reason) => {
                failure.get_or_insert(reason);
            }
        }
        match rows(cair_wing::count(memory, base, state)) {
            Ok(required) => self.wing_sources = self.wing_sources.max(required),
            Err(reason) => {
                failure.get_or_insert(reason);
            }
        }
        match failure {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
}

pub struct RailPart {
    cooldowns: CaptureRows<cprovince_railway_info::Row>,
    rail_buildings: CaptureRows<crailway_manager::fresh_inputs::RailwayBuildingRow>,
    rail_buildings_required: usize,
    rail_levels: Arena<i32>,
    rail_hour: Option<i32>,
    edges: CaptureRows<rail_neighbor::Row>,
    rail_clean: Option<std::sync::Arc<rail_store::CleanRails>>,
    rail_witness: Option<Witness>,
    ticks: u64,
}

impl RailPart {
    pub fn vacant() -> Self {
        Self {
            cooldowns: CaptureRows::new(0),
            rail_buildings: CaptureRows::new(0),
            rail_buildings_required: 0,
            rail_levels: Arena::new(0),
            rail_hour: None,
            edges: CaptureRows::new(0),
            rail_clean: None,
            rail_witness: None,
            ticks: 0,
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.cooldowns.allocated_bytes()
            + self.rail_buildings.allocated_bytes()
            + self.rail_levels.allocated_bytes()
            + self.edges.allocated_bytes()
    }

    fn reset(&mut self) {
        self.cooldowns.reset();
        self.rail_buildings.reset();
        self.rail_buildings_required = 0;
        self.rail_levels.reset();
        self.rail_hour = None;
        self.edges.reset();
        self.rail_clean = None;
        self.rail_witness = None;
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        rails: Option<&mut RailStore>,
    ) -> u64 {
        self.reset();
        self.rail_hour = Some(current_hour);
        let _reads = read_accounting::table(TableId::RailCooldowns);
        let begin = qpc();
        match rails {
            Some(store) => {
                let mut reuse = RailReuse::begin(
                    store,
                    &collector_observers::rail_changes::Observer,
                    |list| crailway_manager::cooldown_list(memory, base, game_state, list),
                );
                let clean_hit = crailway_manager::read_header(memory, base, game_state)
                    .ok()
                    .and_then(|header| reuse.clean_hit(header));
                if let Some(clean) = clean_hit {
                    self.cooldowns.outcome = Outcome::Rows {
                        len: clean.cooldowns.len(),
                    };
                    self.edges.outcome = Outcome::Rows {
                        len: clean.edges.len(),
                    };
                    self.rail_buildings.outcome = Outcome::Rows {
                        len: clean.buildings.len(),
                    };
                    self.rail_buildings_required = clean.buildings_required;
                    self.rail_levels.len = clean.levels.len();
                    self.rail_levels.required = clean.levels_required;
                    self.rail_witness = Some(clean.witness);
                    self.rail_clean = Some(clean);
                    reuse.finish_with_clean(true, |_| None);
                } else {
                    let complete = self.walk(memory, base, game_state, &mut reuse);
                    let cooldowns_slice = self.cooldowns.rows.get(..self.cooldowns.outcome.len());
                    let edges_slice = self.edges.rows.get(..self.edges.outcome.len());
                    let buildings_slice = self
                        .rail_buildings
                        .rows
                        .get(..self.rail_buildings.outcome.len());
                    let levels_slice = self.rail_levels.first.get(..self.rail_levels.len);
                    let buildings_required = self.rail_buildings_required;
                    let levels_required = self.rail_levels.required;
                    self.rail_witness = reuse.finish_with_clean(complete, |witness| {
                        let cooldowns = cooldowns_slice?;
                        let edges = edges_slice?;
                        let buildings = buildings_slice?;
                        let levels = levels_slice?;
                        if !cooldowns
                            .windows(2)
                            .all(|pair| pair[0].province < pair[1].province)
                        {
                            return None;
                        }
                        engine_allocation::advisory(|| {
                            let mut c = engine_allocation::vec(cooldowns.len()).ok()?;
                            c.extend_from_slice(cooldowns);
                            let mut e = engine_allocation::vec(edges.len()).ok()?;
                            e.extend_from_slice(edges);
                            let mut b = engine_allocation::vec(buildings.len()).ok()?;
                            b.extend_from_slice(buildings);
                            let mut l = engine_allocation::vec(levels.len()).ok()?;
                            l.extend_from_slice(levels);
                            Some(rail_store::CleanRails {
                                witness,
                                cooldowns: c,
                                edges: e,
                                buildings: b,
                                levels: l,
                                buildings_required,
                                levels_required,
                            })
                        })
                    });
                }
            }
            None => {
                self.walk(memory, base, game_state, &mut crailway_manager::Fresh);
            }
        }
        self.ticks = elapsed(begin);
        self.ticks
    }

    pub fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.reset();
        let active = |table| {
            failed(
                replay_data::cadence::should_walk(table, hour, force_full),
                reason,
            )
        };
        self.cooldowns.fail(active(TABLES[2]));
        self.rail_buildings.fail(active(TABLES[2]));
        self.edges.fail(active(TABLES[3]));
        self.ticks = 0;
    }

    fn walk(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        carry: &mut impl crailway_manager::RailCarry,
    ) -> bool {
        let mut levels = crailway_manager::selected_inputs::Levels {
            values: &mut self.rail_levels.first,
            len: 0,
            required: 0,
        };
        let first = crailway_manager::walk_with_selection_carried(
            memory,
            base,
            game_state,
            &mut self.cooldowns.rows,
            &mut self.edges.rows,
            &mut self.rail_buildings.rows,
            &mut levels,
            carry,
        );
        self.rail_levels.len = levels.len;
        self.rail_levels.required = levels.required;
        self.cooldowns.outcome = bounded(first.rows.provinces, self.cooldowns.rows.len());
        self.edges.outcome = bounded(first.rows.edges, self.edges.rows.len());
        self.rail_buildings.outcome = bounded(first.buildings, self.rail_buildings.rows.len());
        self.rail_buildings_required = first.required;
        [first.rows.provinces, first.rows.edges, first.buildings]
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Rows { .. }))
    }
}

pub struct Captured {
    supremacy: CaptureRows<cnaval_region_dominance::Row>,
    convoys: CaptureRows<csunk_convoy_info::Row>,
    cooldowns: CaptureRows<cprovince_railway_info::Row>,
    rail_buildings: CaptureRows<crailway_manager::fresh_inputs::RailwayBuildingRow>,
    rail_buildings_required: usize,
    rail_levels: Arena<i32>,
    rail_hour: Option<i32>,
    edges: CaptureRows<rail_neighbor::Row>,
    rail_clean: Option<std::sync::Arc<rail_store::CleanRails>>,
    rail_witness: Option<Witness>,
    pub land: CaptureRows<ctheatre::Row>,
    reserved_land_count: Option<u32>,
    pub reserved_land_required_rows: usize,
    land_groups: CaptureRows<ctheater_group::Row>,
    land_members: CaptureRows<ctheater_group::MemberRow>,
    air: CaptureRows<cair_theatre::Row>,
    air_groups: CaptureRows<cair_group::Row>,
    air_members: CaptureRows<cair_group::MemberRow>,
    results: CaptureRows<cnaval_combat_results::Row>,
    result_members: CaptureRows<cnaval_combat_results::MemberRow>,
    result_countries: CaptureRows<cnaval_combat_results::CountryRow>,
    result_slots: Vec<cnaval_combat_results::ResultSlot>,
    sunk: CaptureRows<csunk_ship_info::Record>,
    groups: CaptureRows<corders_group::GroupRow>,
    group_names: Arena<u8>,
    group_name_capacity: usize,
    group_members: CaptureRows<corders_group::MemberRow>,
    group_children: CaptureRows<corders_group::ChildRow>,
    roster_scratch: Arena<corders_group::RosterEntry>,
    oob_finished: bool,
    theatre_names: Arena<u8>,
    sunk_names: Arena<u8>,
    orders_sources: CaptureRows<ctheatre::OrdersMembership>,
    wing_sources: CaptureRows<engine_native_air::cair_wing::WingMembership>,
    orders_finished: bool,
    wings_finished: bool,
    source_ticks: [u64; 3],
    active: [bool; N],
    rails_early: Option<u64>,
    pub raw_mode: bool,
    pub supremacy_raw_mode: bool,
    pub sunk_raw_mode: bool,
}

const SKIPPED: Outcome = Outcome::Unavailable {
    reason: replay_data::cadence::SKIPPED_REASON,
};

const SUPREMACY_NOT_STAGED: Outcome = Outcome::Unavailable {
    reason: "naval supremacy payload not staged this hour",
};

const THEATRES_NOT_STAGED: &str = "naval theatre payload not staged this hour";

fn reserved_raw_demand(count: Option<u32>) -> Option<(usize, usize)> {
    let count = count? as usize;
    Some((
        count.checked_mul(32)?.checked_add(16)?,
        count.checked_mul(4)?.checked_add(1)?,
    ))
}

fn failed(active: bool, reason: &'static str) -> Outcome {
    if active {
        Outcome::Unavailable { reason }
    } else {
        SKIPPED
    }
}

impl Captured {
    pub fn fail_late_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.prepare_hour(hour, force_full);
        self.reserved_land_count = None;
        self.reserved_land_required_rows = 0;
        if !self.supremacy_raw_mode {
            self.supremacy.fail(failed(self.active[0], reason));
        }
        self.convoys.fail(failed(self.active[1], reason));
        if self.rails_early.take().is_none() {
            self.cooldowns.fail(failed(self.active[2], reason));
            self.rail_buildings.fail(failed(self.active[2], reason));
            self.edges.fail(failed(self.active[3], reason));
            self.rail_clean = None;
            self.rail_witness = None;
        }
        if !self.raw_mode {
            self.land.fail(failed(self.active[4], reason));
            self.land_groups.fail(failed(self.active[5], reason));
            self.land_members.fail(failed(self.active[6], reason));
            self.air.fail(failed(self.active[7], reason));
            self.air_groups.fail(failed(self.active[8], reason));
            self.air_members.fail(failed(self.active[9], reason));
            self.groups.fail(failed(self.active[14], reason));
            self.group_members.fail(failed(self.active[15], reason));
            self.group_children.fail(failed(self.active[16], reason));
        }
        self.results.fail(failed(self.active[10], reason));
        self.result_members.fail(failed(self.active[11], reason));
        self.result_countries.fail(failed(self.active[12], reason));
        if !self.sunk_raw_mode {
            self.sunk.fail(failed(self.active[13], reason));
        }
    }

    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            theatre_names: 0,
            sunk_names: 0,
            orders_sources: 0,
            wing_sources: 0,
            roster_scratch: 0,
            rail_levels: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            supremacy: CaptureRows::new(capacity.rows[0]),
            convoys: CaptureRows::new(capacity.rows[1]),
            cooldowns: CaptureRows::new(capacity.rows[2]),
            rail_buildings: CaptureRows::new(capacity.rows[2]),
            rail_buildings_required: 0,
            rail_levels: Arena::new(capacity.rail_levels),
            rail_hour: None,
            edges: CaptureRows::new(capacity.rows[3]),
            rail_clean: None,
            rail_witness: None,
            land: CaptureRows::new(capacity.rows[4]),
            reserved_land_count: None,
            reserved_land_required_rows: 0,
            land_groups: CaptureRows::new(capacity.rows[5]),
            land_members: CaptureRows::new(capacity.rows[6]),
            air: CaptureRows::new(capacity.rows[7]),
            air_groups: CaptureRows::new(capacity.rows[8]),
            air_members: CaptureRows::new(capacity.rows[9]),
            results: CaptureRows::new(capacity.rows[10]),
            result_members: CaptureRows::new(capacity.rows[11]),
            result_countries: CaptureRows::new(capacity.rows[12]),
            result_slots: owned_vec(default_vec(capacity.rows[10])),
            sunk: CaptureRows::new(capacity.rows[13]),
            groups: CaptureRows::new(capacity.rows[14]),
            group_names: Arena::new(capacity.theatre_names),
            group_name_capacity: capacity.theatre_names,
            group_members: CaptureRows::new(capacity.rows[15]),
            group_children: CaptureRows::new(capacity.rows[16]),
            roster_scratch: Arena::new(capacity.roster_scratch),
            oob_finished: false,
            theatre_names: Arena::new(capacity.theatre_names),
            sunk_names: Arena::new(capacity.sunk_names),
            orders_sources: CaptureRows::new(capacity.orders_sources),
            wing_sources: CaptureRows::new(capacity.wing_sources),
            orders_finished: false,
            wings_finished: false,
            source_ticks: [0; 3],
            rails_early: None,
            active: [true; N],
            raw_mode: false,
            supremacy_raw_mode: false,
            sunk_raw_mode: false,
        }
    }

    pub fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.active =
            TABLES.map(|table| replay_data::cadence::should_walk(table, hour, force_full));
    }

    pub fn theatres_active(&self) -> bool {
        self.active[5]
    }

    fn skip_theatre_details(&mut self) {
        self.land_groups.outcome = SKIPPED;
        self.land_members.outcome = SKIPPED;
        self.air.outcome = SKIPPED;
        self.air_groups.outcome = SKIPPED;
        self.air_members.outcome = SKIPPED;
        self.theatre_names.reset();
    }

    pub fn reset(&mut self) {
        self.reset_tables();
        self.orders_sources.reset();
        self.wing_sources.reset();
        self.groups.reset();
        self.group_names.reset();
        self.group_members.reset();
        self.group_children.reset();
        self.roster_scratch.reset();
        self.oob_finished = false;
        self.orders_sources.outcome = Outcome::Rows { len: 0 };
        self.wing_sources.outcome = Outcome::Rows { len: 0 };
        self.orders_finished = false;
        self.wings_finished = false;
        self.source_ticks = [0; 3];
        self.raw_mode = false;
    }

    fn reset_tables(&mut self) {
        self.supremacy.reset();
        self.convoys.reset();
        if self.rails_early.is_none() {
            self.cooldowns.reset();
            self.rail_buildings.reset();
            self.rail_buildings_required = 0;
            self.rail_levels.reset();
            self.rail_hour = None;
            self.edges.reset();
            self.rail_clean = None;
            self.rail_witness = None;
        }
        self.land.reset();
        self.reserved_land_count = None;
        self.reserved_land_required_rows = 0;
        self.land_groups.reset();
        self.land_members.reset();
        self.air.reset();
        self.air_groups.reset();
        self.air_members.reset();
        self.results.reset();
        self.result_members.reset();
        self.result_countries.reset();
        self.sunk.reset();
        self.theatre_names.reset();
        self.sunk_names.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.supremacy.allocated_bytes()
            + self.convoys.allocated_bytes()
            + self.cooldowns.allocated_bytes()
            + self.rail_buildings.allocated_bytes()
            + self.rail_levels.allocated_bytes()
            + self.edges.allocated_bytes()
            + self.land.allocated_bytes()
            + self.land_groups.allocated_bytes()
            + self.land_members.allocated_bytes()
            + self.air.allocated_bytes()
            + self.air_groups.allocated_bytes()
            + self.air_members.allocated_bytes()
            + self.results.allocated_bytes()
            + self.result_members.allocated_bytes()
            + self.result_countries.allocated_bytes()
            + self.result_slots.capacity() * size_of::<cnaval_combat_results::ResultSlot>()
            + self.sunk.allocated_bytes()
            + self.groups.allocated_bytes()
            + self.group_names.allocated_bytes()
            + self.group_members.allocated_bytes()
            + self.group_children.allocated_bytes()
            + self.roster_scratch.allocated_bytes()
            + self.theatre_names.allocated_bytes()
            + self.sunk_names.allocated_bytes()
            + self.orders_sources.allocated_bytes()
            + self.wing_sources.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.supremacy.grow(&mut capacity.rows[0]);
        self.convoys.grow(&mut capacity.rows[1]);
        grow_capacity(&mut capacity.rows[2], self.rail_buildings_required);
        self.cooldowns.grow(&mut capacity.rows[2]);
        self.rail_buildings.grow(&mut capacity.rows[2]);
        grow_vec(&mut self.cooldowns.rows, capacity.rows[2]);
        self.rail_levels.grow(&mut capacity.rail_levels);
        self.edges.grow(&mut capacity.rows[3]);
        let positive_demand = match self.land.outcome {
            Outcome::Rows { len } => len,
            Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
            Outcome::Unavailable { .. } => 0,
        };
        let reserved_demand = if self.reserved_land_count.is_some() {
            0
        } else {
            self.reserved_land_required_rows
        };
        if let Some(required) = positive_demand.checked_add(reserved_demand) {
            grow_capacity(&mut capacity.rows[4], required);
        }
        self.land.grow(&mut capacity.rows[4]);
        self.land_groups.grow(&mut capacity.rows[5]);
        self.land_members.grow(&mut capacity.rows[6]);
        self.air.grow(&mut capacity.rows[7]);
        self.air_groups.grow(&mut capacity.rows[8]);
        self.air_members.grow(&mut capacity.rows[9]);
        self.results.grow(&mut capacity.rows[10]);
        if self.result_slots.len() < self.results.rows.len()
            && resize_default(&mut self.result_slots, self.results.rows.len()).is_err()
        {
            return;
        }
        self.result_members.grow(&mut capacity.rows[11]);
        self.result_countries.grow(&mut capacity.rows[12]);
        self.sunk.grow(&mut capacity.rows[13]);
        self.groups.grow(&mut capacity.rows[14]);
        self.group_names.grow(&mut self.group_name_capacity);
        self.group_members.grow(&mut capacity.rows[15]);
        self.group_children.grow(&mut capacity.rows[16]);
        self.roster_scratch.grow(&mut capacity.roster_scratch);
        self.theatre_names.grow(&mut capacity.theatre_names);
        self.sunk_names.grow(&mut capacity.sunk_names);
        self.orders_sources.grow(&mut capacity.orders_sources);
        self.wing_sources.grow(&mut capacity.wing_sources);
    }

    #[cfg(test)]
    fn railway_row_at(
        &self,
        hint: usize,
        province: u32,
    ) -> Option<&crailway_manager::fresh_inputs::RailwayBuildingRow> {
        let captured = match self.rail_clean.as_deref() {
            Some(clean) => clean.buildings.as_slice(),
            None => self
                .rail_buildings
                .rows
                .get(..self.rail_buildings.outcome.len())?,
        };
        if let Some(row) = captured.get(hint)
            && row.province == province
        {
            return Some(row);
        }
        let index = captured
            .binary_search_by_key(&province, |row| row.province)
            .ok()?;
        captured.get(index)
    }

    #[cfg(test)]
    fn railway_building(&self, province: u32) -> Option<rows::RailwayBuildingState> {
        self.railway_building_at(usize::MAX, province)
    }

    #[cfg(test)]
    fn railway_selection(&self, province: u32) -> Option<rows::RailwaySelectionInputs> {
        self.railway_selection_at(usize::MAX, province)
    }

    #[cfg(test)]
    fn railway_building_at(&self, hint: usize, province: u32) -> Option<rows::RailwayBuildingState> {
        let state = self.railway_row_at(hint, province)?.building.ok()??;
        Some(rows::RailwayBuildingState {
            building: state.building.map(|health| rows::RailwayBuildingHealth {
                level: health.level,
                healthy: health.healthy,
                partial: health.partial,
            }),
        })
    }

    #[cfg(test)]
    fn railway_selection_at(
        &self,
        hint: usize,
        province: u32,
    ) -> Option<rows::RailwaySelectionInputs> {
        self.rail_hour?;
        let selected = self.railway_row_at(hint, province)?.selection?;
        let levels_slice = match self.rail_clean.as_deref() {
            Some(clean) => clean.levels.as_slice(),
            None => self.rail_levels.first.get(..self.rail_levels.len)?,
        };
        let levels = selected.levels.ok().and_then(|span| {
            span.first
                .checked_add(span.len)
                .and_then(|end| levels_slice.get(span.first..end))
                .map(|values| owned_vec(copy_vec(values)))
        });
        Some(rows::RailwaySelectionInputs {
            levels,
            pair_count: selected.pair_count.ok(),
            building_level: selected.building_level.ok().flatten(),
        })
    }

    fn rail_cooldowns_input(
        &self,
        mut projection: Option<&mut Projection<'_>>,
        outcome: Outcome,
    ) -> TableInput {
        let retain_witness = self.rail_witness.filter(|_| self.rail_hour.is_some());
        if let Some(projection) = projection.as_deref()
            && matches!(outcome, Outcome::Rows { .. })
            && !replay_data::cadence::is_staggered(TableId::RailCooldowns)
            && let Some(input) = projection.republish_uniform_optional_i32::<tables::RailCooldowns>(
                retain_witness,
                3,
                self.rail_hour,
            )
        {
            return input;
        }
        let (cooldown_rows, captured, rail_levels) = match self.rail_clean.as_deref() {
            Some(clean) => (
                clean.cooldowns.as_slice(),
                Some(clean.buildings.as_slice()),
                Some(clean.levels.as_slice()),
            ),
            None => (
                self.cooldowns.rows.as_slice(),
                self.rail_buildings
                    .rows
                    .get(..self.rail_buildings.outcome.len()),
                self.rail_levels.first.get(..self.rail_levels.len),
            ),
        };
        let base = cooldown_rows.as_ptr() as usize;
        let stride = size_of::<cprovince_railway_info::Row>();
        let rail_hour = self.rail_hour;
        let entry_of = |r: &cprovince_railway_info::Row| {
            let hint = (r as *const cprovince_railway_info::Row as usize - base) / stride;
            captured.and_then(|captured| {
                if let Some(row) = captured.get(hint)
                    && row.province == r.province
                {
                    return Some(row);
                }
                captured
                    .binary_search_by_key(&r.province, |row| row.province)
                    .ok()
                    .and_then(|index| captured.get(index))
            })
        };
        let building_of = |entry: Option<&crailway_manager::fresh_inputs::RailwayBuildingRow>| {
            entry.and_then(|row| row.building.ok()?).map(|state| {
                rows::RailwayBuildingState {
                    building: state.building.map(|health| rows::RailwayBuildingHealth {
                        level: health.level,
                        healthy: health.healthy,
                        partial: health.partial,
                    }),
                }
            })
        };
        let non_hour_equal = |r: &cprovince_railway_info::Row, previous: &rows::RailCooldownRow| {
            if r.cooldown_ticks != previous.cooldown_ticks
                || previous.building_bound != rows::Recorded::Value(Some(r.building_bound))
            {
                return false;
            }
            let entry = entry_of(r);
            if previous.building != rows::Recorded::Value(building_of(entry)) {
                return false;
            }
            let selected = rail_hour.and_then(|_| entry?.selection);
            match (selected, &previous.selection) {
                (None, rows::Recorded::Value(None)) => true,
                (Some(selected), rows::Recorded::Value(Some(prev))) => {
                    let levels = selected.levels.ok().and_then(|span| {
                        let end = span.first.checked_add(span.len)?;
                        rail_levels?.get(span.first..end)
                    });
                    prev.levels.as_deref() == levels
                        && prev.pair_count == selected.pair_count.ok()
                        && prev.building_level == selected.building_level.ok().flatten()
                }
                _ => false,
            }
        };
        let convert = |r: &cprovince_railway_info::Row| {
            let entry = entry_of(r);
            let building = building_of(entry);
            let selection = rail_hour.and_then(|_| {
                let selected = entry?.selection?;
                let levels = selected.levels.ok().and_then(|span| {
                    let end = span.first.checked_add(span.len)?;
                    rail_levels?
                        .get(span.first..end)
                        .map(|values| owned_vec(copy_vec(values)))
                });
                Some(rows::RailwaySelectionInputs {
                    levels,
                    pair_count: selected.pair_count.ok(),
                    building_level: selected.building_level.ok().flatten(),
                })
            });
            rows::RailCooldownRow {
                province: r.province,
                cooldown_ticks: r.cooldown_ticks,
                building: rows::Recorded::Value(building),
                selection: rows::Recorded::Value(selection),
                selection_hour: rows::Recorded::Value(rail_hour),
                building_bound: rows::Recorded::Value(Some(r.building_bound)),
            }
        };
        let input = if let Some(projection) = projection.as_deref()
            && matches!(outcome, Outcome::Rows { .. })
            && !replay_data::cadence::is_staggered(TableId::RailCooldowns)
            && let Some(source) = cooldown_rows.get(..outcome.len())
            && source
                .windows(2)
                .all(|pair| pair[0].province < pair[1].province)
            && let Some((token, previous)) =
                projection.slice_baseline_rows::<tables::RailCooldowns>()
        {
            projection.timed(TableId::RailCooldowns, || {
                let target_hour = rows::Recorded::Value(rail_hour);
                let mut all_same_hour = true;
                let mut keys = Vec::new();
                let mut changed = Vec::new();
                let mut edit = |key: u128, row: Option<rows::RailCooldownRow>| {
                    if keys.len() == keys.capacity() {
                        engine_allocation::reserve(&mut keys, 1)?;
                    }
                    keys.push(key);
                    if let Some(mut row) = row {
                        row.round_to_display();
                        if changed.len() == changed.capacity() {
                            engine_allocation::reserve(&mut changed, 1)?;
                        }
                        changed.push(row);
                    }
                    Ok::<_, std::io::Error>(())
                };
                let (mut current, mut held) = (0, 0);
                let merged = loop {
                    let row = source.get(current);
                    let result = match (row, previous.get(held)) {
                        (None, None) => break Ok(()),
                        (Some(row), Some(baseline)) if row.province == baseline.province => {
                            current += 1;
                            held += 1;
                            all_same_hour &= baseline.selection_hour == target_hour;
                            if non_hour_equal(row, baseline) {
                                continue;
                            }
                            edit(u128::from(row.province), Some(convert(row)))
                        }
                        (Some(row), baseline)
                            if baseline.is_none_or(|b| row.province < b.province) =>
                        {
                            current += 1;
                            all_same_hour = false;
                            edit(u128::from(row.province), Some(convert(row)))
                        }
                        (_, Some(baseline)) => {
                            held += 1;
                            edit(baseline.key(), None)
                        }
                        (Some(_), None) => unreachable!("absent baseline takes insert arm"),
                    };
                    if result.is_err() {
                        break result;
                    }
                };
                if merged.is_err() {
                    return unavailable("collector allocation failed").into();
                }
                if keys.is_empty() && all_same_hour {
                    projection.note_compared(TableId::RailCooldowns, None);
                    return TableInput::ComparedEqual(token);
                }
                projection.note_compared(TableId::RailCooldowns, Some(changed.len()));
                TableInput::UniformOptionalI32 {
                    token,
                    column: 3,
                    value: rail_hour,
                    update: std::sync::Arc::new(replay_data::sliced::SliceUpdate::keyed(
                        keys,
                        TableRows::RailCooldowns(changed),
                    )),
                }
            })
        } else {
            project_table::<tables::RailCooldowns, _>(
                projection.as_deref_mut(),
                outcome,
                cooldown_rows,
                |r| u128::from(r.province),
                |r, previous| {
                    previous.selection_hour == rows::Recorded::Value(rail_hour)
                        && non_hour_equal(r, previous)
                },
                convert,
            )
        };
        if let Some(projection) = projection.as_deref() {
            projection.retain(TableId::RailCooldowns, retain_witness, &input);
        }
        input
    }

    fn rail_edges_input(
        &self,
        mut projection: Option<&mut Projection<'_>>,
        outcome: Outcome,
    ) -> TableInput {
        if let Some(projection) = projection.as_deref()
            && matches!(outcome, Outcome::Rows { .. })
            && !replay_data::cadence::is_staggered(TableId::RailEdges)
            && let Some(input) = projection.republish::<tables::RailEdges>(self.rail_witness)
        {
            return input;
        }
        let edge = |r: &rail_neighbor::Row| rows::RailEdgeRow {
            from: r.from_province,
            to: r.to_province,
            level: r.level,
        };
        let rows = match self.rail_clean.as_deref() {
            Some(clean) => clean.edges.as_slice(),
            None => self.edges.rows.as_slice(),
        };
        let input = scalar_table::<tables::RailEdges, _>(
            projection.as_deref_mut(),
            outcome,
            rows,
            |r| RowCodec::key(&edge(r)),
            edge,
        );
        if let Some(projection) = projection.as_deref() {
            projection.retain(TableId::RailEdges, self.rail_witness, &input);
        }
        input
    }

    pub fn set_order_sources(
        &mut self,
        memory: &impl Memory,
        base: usize,
        roots: &[corders_group::Root],
        source: Outcome,
    ) {
        self.set_order_sources_inner(memory, base, roots, source);
    }

    fn set_order_sources_inner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        roots: &[corders_group::Root],
        source: Outcome,
    ) {
        grow_vec(&mut self.orders_sources.rows, source.len());

        if !self.theatres_active() {
            self.orders_sources.outcome = SKIPPED;
            self.orders_finished = true;
            self.set_order_of_battle(memory, base, roots, source);
            return;
        }
        let reads = read_accounting::table(TableId::LandTheatreMembers);
        let begin = qpc();
        let output = &mut self.orders_sources.rows;
        self.orders_sources.outcome = match source {
            Outcome::Rows { len } if len <= roots.len() => {
                let mut result = Outcome::Rows { len: 0 };
                for root in &roots[..len] {
                    let row = if result.len() < output.len() {
                        ctheatre::read_orders_membership(memory, base, root)
                    } else {
                        Ok(ctheatre::OrdersMembership::default())
                    };
                    push(output, &mut result, row);
                    if matches!(result, Outcome::Unavailable { .. }) {
                        break;
                    }
                }
                result
            }
            Outcome::Unavailable { reason } => Outcome::Unavailable { reason },
            Outcome::Truncated { .. } => Outcome::Unavailable {
                reason: "orders registry census truncated",
            },
            _ => Outcome::Unavailable {
                reason: "orders registry census exceeds captured roots",
            },
        };
        self.orders_finished = true;
        self.source_ticks[0] = self.source_ticks[0].saturating_add(elapsed(begin));
        drop(reads);
        self.set_order_of_battle(memory, base, roots, source);
    }

    fn set_order_of_battle(
        &mut self,
        memory: &impl Memory,
        base: usize,
        roots: &[corders_group::Root],
        source: Outcome,
    ) {
        let _reads = read_accounting::table(TableId::OrdersGroups);
        let begin = qpc();
        let outcomes = match source {
            Outcome::Rows { len } if len <= roots.len() => {
                self.roster_scratch.required = self.roster_scratch.required.max(len);
                let (groups, members, children) = (
                    &mut self.groups.rows,
                    &mut self.group_members.rows,
                    &mut self.group_children.rows,
                );
                let mut storage = corders_group::OrderOfBattleStorage {
                    groups: storage_buffer(groups),
                    members: storage_buffer(members),
                    children: storage_buffer(children),
                    roster_scratch: storage_buffer(&mut self.roster_scratch.first),
                };
                let walk = corders_group::walk_order_of_battle_buffers(
                    memory,
                    base,
                    &roots[..len],
                    &mut storage,
                );
                [
                    bounded(walk.groups, storage.groups.len()),
                    bounded(walk.members, storage.members.len()),
                    bounded(walk.children, storage.children.len()),
                ]
            }
            Outcome::Unavailable { reason } => [Outcome::Unavailable { reason }; 3],
            Outcome::Truncated { .. } => {
                [Outcome::Unavailable {
                    reason: "orders registry census truncated",
                }; 3]
            }
            _ => {
                [Outcome::Unavailable {
                    reason: "orders registry census exceeds captured roots",
                }; 3]
            }
        };
        self.group_names.reset();
        for row in &mut self.groups.rows[..outcomes[0].len()] {
            if let Ok((start, len)) = corders_group::copy_name_buffer(
                memory,
                row.pointer,
                storage_buffer(&mut self.group_names.first),
                &mut self.group_names.len,
                &mut self.group_names.required,
            ) {
                row.name_start = start;
                row.name_len = len;
                row.name_known = true;
            }
        }
        self.groups.outcome = outcomes[0];
        self.group_members.outcome = outcomes[1];
        self.group_children.outcome = outcomes[2];
        self.oob_finished = true;
        self.source_ticks[2] = self.source_ticks[2].saturating_add(elapsed(begin));
    }
    fn finish_order_of_battle(&mut self) {
        if !self.oob_finished {
            self.groups.outcome = source_unavailable();
            self.group_members.outcome = source_unavailable();
            self.group_children.outcome = source_unavailable();
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn finish_wings(&mut self, source: Outcome) {
        if !self.theatres_active() {
            self.wing_sources.outcome = SKIPPED;
            self.wings_finished = true;
            return;
        }
        let begin = qpc();
        let outcome = &mut self.wing_sources.outcome;
        let read_count = match *outcome {
            Outcome::Rows { len } => Some(len),
            Outcome::Truncated { len, unread_rows } => len.checked_add(unread_rows as usize),
            _ => None,
        };
        match source {
            Outcome::Unavailable { reason } => *outcome = Outcome::Unavailable { reason },
            Outcome::Truncated { .. } => {
                *outcome = Outcome::Unavailable {
                    reason: "air-wing source census truncated",
                }
            }
            Outcome::Rows { len }
                if Some(len) != read_count && !matches!(*outcome, Outcome::Unavailable { .. }) =>
            {
                *outcome = Outcome::Unavailable {
                    reason: "air-wing visitor count differs from source census",
                };
            }
            _ => {}
        }
        self.wings_finished = true;
        self.source_ticks[1] = self.source_ticks[1].saturating_add(elapsed(begin));
    }
    pub fn install_owned_wings(&mut self, job: &mut NavalJob) {
        let begin = qpc();
        self.wing_sources.reset();
        if !job.theatres_active {
            self.wing_sources.outcome = SKIPPED;
        } else {
            let outcome = job.wing_outcome.unwrap_or(Outcome::Unavailable {
                reason: "registered missions lack the complete wing census",
            });
            let len = outcome.len();
            if len > self.wing_sources.rows.len() {
                std::mem::swap(&mut self.wing_sources.rows, &mut job.wing_sources);
            } else {
                self.wing_sources.rows[..len].copy_from_slice(&job.wing_sources[..len]);
            }
            self.wing_sources.outcome = outcome;
        }
        self.wings_finished = true;
        self.add_wing_source_ticks(elapsed(begin));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn install_map_wings(&mut self, job: &collector_air::extended::air::AirWingJob) {
        if !job.theatres_active {
            self.wing_sources.outcome = SKIPPED;
            self.wings_finished = true;
            return;
        }
        let begin = qpc();
        let sources = job.map_memberships();
        let (output, outcome) = (&mut self.wing_sources.rows, &mut self.wing_sources.outcome);
        *outcome = Outcome::Rows { len: 0 };
        for source in sources.iter().take(job.map_outcome.len()) {
            let row = match source.error {
                Some(reason) if outcome.len() < output.len() => Err(reason),
                _ => Ok(source.value),
            };
            push(output, outcome, row);
        }
        self.add_wing_source_ticks(job.membership_ticks.saturating_add(elapsed(begin)));
        self.finish_wings(job.map_outcome);
    }

    pub fn add_wing_source_ticks(&mut self, ticks: u64) {
        self.source_ticks[1] = self.source_ticks[1].saturating_add(ticks);
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        force_full: bool,
        rails: Option<&mut RailStore>,
    ) -> [u64; N] {
        self.prepare_hour(current_hour, force_full);
        self.reset_tables();
        let mut ticks = [0; N];
        if !self.active[0] {
            self.supremacy.outcome = SKIPPED;
        } else if !self.supremacy_raw_mode {
            self.supremacy.outcome = SUPREMACY_NOT_STAGED;
        }
        if self.active[1] {
            let begin = qpc();
            let _reads = read_accounting::table(TableId::SunkConvoys);
            self.convoys.outcome =
                csunk_convoy_info::walk(memory, base, game_state, &mut self.convoys.rows);
            ticks[1] = elapsed(begin);
        } else {
            self.convoys.outcome = SKIPPED;
        }

        ticks[2] = match self.rails_early.take() {
            Some(ticks) => ticks,
            None => {
                let mut part = RailPart::vacant();
                self.swap_rails(&mut part);
                let ticks = part.capture(memory, base, game_state, current_hour, rails);
                self.swap_rails(&mut part);
                ticks
            }
        };

        if !self.theatres_active() {
            self.skip_theatre_details();
        }
        if !self.raw_mode {
            let active = self.active;
            self.land.fail(failed(active[4], THEATRES_NOT_STAGED));
            self.land_groups.fail(failed(active[5], THEATRES_NOT_STAGED));
            self.land_members.fail(failed(active[6], THEATRES_NOT_STAGED));
            self.air.fail(failed(active[7], THEATRES_NOT_STAGED));
            self.air_groups.fail(failed(active[8], THEATRES_NOT_STAGED));
            self.air_members.fail(failed(active[9], THEATRES_NOT_STAGED));
            ticks[14] = self.source_ticks[2];
            self.finish_order_of_battle();
        }

        let result_reads = read_accounting::table(TableId::NavalResults);
        let begin = qpc();
        let first = cnaval_combat_results::walk(
            memory,
            base,
            game_state,
            &mut self.results.rows,
            &mut self.result_slots,
            &mut self.result_members.rows,
            &mut self.result_countries.rows,
        );
        self.results.outcome = bounded(first.records, self.results.rows.len());
        self.result_members.outcome = bounded(first.members, self.result_members.rows.len());
        self.result_countries.outcome = bounded(first.countries, self.result_countries.rows.len());
        ticks[10] = elapsed(begin);
        drop(result_reads);

        if !self.active[13] {
            self.sunk.outcome = SKIPPED;
        } else if self.sunk_raw_mode {
            ticks[13] = 0;
        } else {
            let _reads = read_accounting::table(TableId::SunkShips);
            let begin = qpc();
            self.capture_sunk(memory, base, game_state, current_hour);
            ticks[13] = elapsed(begin);
        }
        ticks
    }

    fn swap_rails(&mut self, part: &mut RailPart) {
        std::mem::swap(&mut self.cooldowns, &mut part.cooldowns);
        std::mem::swap(&mut self.rail_buildings, &mut part.rail_buildings);
        std::mem::swap(
            &mut self.rail_buildings_required,
            &mut part.rail_buildings_required,
        );
        std::mem::swap(&mut self.rail_levels, &mut part.rail_levels);
        std::mem::swap(&mut self.rail_hour, &mut part.rail_hour);
        std::mem::swap(&mut self.edges, &mut part.edges);
        std::mem::swap(&mut self.rail_clean, &mut part.rail_clean);
        std::mem::swap(&mut self.rail_witness, &mut part.rail_witness);
    }

    pub fn install_rails(&mut self, part: &mut RailPart) {
        let ticks = part.ticks;
        self.swap_rails(part);
        self.rails_early = Some(ticks);
    }

    pub fn reserved_land_count(&self) -> Option<u32> {
        self.reserved_land_count
    }

    pub fn withhold_reserved_land_certificate(&mut self) {
        let Some(count) = self.reserved_land_count.take() else {
            return;
        };
        if let Outcome::Rows { len } = self.land.outcome
            && len <= self.land.rows.len()
            && let Some(start) = len.checked_sub(count as usize)
        {
            self.land.rows[start..len].fill(ctheatre::Row::default());
            self.land.outcome = Outcome::Rows { len: start };
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn capture_reserved_land(
        &mut self,
        memory: &impl Memory,
        base: usize,
        root: Result<engine_native_memory::country_identity::Country, &'static str>,
        prior_rosters: Option<(&RawLog<NAVAL_PACK_N>, usize)>,
        staging_refusal: Option<&'static str>,
    ) {
        self.capture_reserved_land_inner(memory, base, root, prior_rosters, staging_refusal);
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_reserved_land_inner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        root: Result<engine_native_memory::country_identity::Country, &'static str>,
        prior_rosters: Option<(&RawLog<NAVAL_PACK_N>, usize)>,
        staging_refusal: Option<&'static str>,
    ) {
        self.reserved_land_count = None;
        let positive_len = match self.land.outcome {
            Outcome::Rows { len } if len <= self.land.rows.len() => len,
            _ => {
                let walk = ctheatre::walk_reserved_land_headers_buffer(
                    memory,
                    base,
                    root,
                    engine_native_memory::walk::Buffer::Count,
                    |_, _| Ok(()),
                );
                self.reserved_land_required_rows = walk.required_rows;
                return;
            }
        };
        let oob_finished = self.oob_finished;
        let groups_outcome = self.groups.outcome;
        let groups = &self.groups.rows;
        let scratch = &self.roster_scratch.first;
        let walk = ctheatre::walk_reserved_land_headers_buffer(
            memory,
            base,
            root,
            engine_native_memory::walk::Buffer::GrowingFrom(&mut self.land.rows, positive_len),
            |theatre, handle| {
                if !oob_finished {
                    return Err("reserved land OOB census not captured");
                }
                let len = match groups_outcome {
                    Outcome::Rows { len } if len <= groups.len() => len,
                    Outcome::Unavailable { reason } => return Err(reason),
                    _ => return Err("reserved land OOB census incomplete"),
                };
                let scratch = scratch
                    .get(..len)
                    .ok_or("reserved land OOB proof scratch incomplete")?;
                if corders_group::captured_theatre_rosters(
                    &groups[..len],
                    scratch,
                    theatre,
                    handle,
                )? {
                    return Ok(());
                }
                for offset in [ctheatre::ORDERS_GROUP_DATA, ctheatre::ARMY_GROUP_DATA] {
                    let address = theatre
                        .checked_add(offset)
                        .ok_or("reserved land group roster address overflow")?;
                    let prior = match prior_rosters {
                        Some((log, end)) => theatres::prior_roster_header(log, end, address)?,
                        None => None,
                    };
                    let bytes = match prior {
                        Some(bytes) => bytes,
                        None => {
                            let mut bytes = [0; 16];
                            if !memory.copy(address, &mut bytes) {
                                return Err("reserved land group roster header unreadable");
                            }
                            bytes
                        }
                    };
                    let roster = Array::from_header(bytes, 8)
                        .ok_or("reserved land group roster header invalid")?;
                    if roster.count != 0 {
                        return Err("reserved land nonempty group roster absent from OOB proof");
                    }
                }
                Ok(())
            },
        );
        self.reserved_land_required_rows = walk.required_rows;
        let (Ok(len), Some(count)) = (walk.len, walk.native_count) else {
            return;
        };
        if len != count as usize {
            return;
        }
        let (positive, spare) = self.land.rows.split_at_mut(positive_len);
        let zero = &mut spare[..len];
        zero.sort_unstable_by_key(|row| row.id);
        let duplicate = zero.windows(2).any(|rows| rows[0].id == rows[1].id)
            || positive
                .iter()
                .any(|row| zero.binary_search_by_key(&row.id, |row| row.id).is_ok());
        if duplicate {
            zero.fill(ctheatre::Row::default());
            return;
        }
        zero.sort_unstable_by_key(|row| row.ordinal);
        if zero
            .iter()
            .enumerate()
            .any(|(ordinal, row)| row.owner != 0 || row.ordinal as usize != ordinal)
        {
            zero.fill(ctheatre::Row::default());
            return;
        }
        let Some(total) = positive_len.checked_add(len) else {
            zero.fill(ctheatre::Row::default());
            return;
        };
        if staging_refusal.is_some() {
            zero.fill(ctheatre::Row::default());
            return;
        }
        self.land.outcome = Outcome::Rows { len: total };
        self.reserved_land_count = Some(count);
    }

    fn capture_sunk(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
    ) {
        let first = csunk_ship_info::walk(
            memory,
            base,
            game_state,
            current_hour,
            &mut self.sunk.rows,
            &mut self.sunk_names.first,
        );
        self.sunk.outcome = bounded(first.outcome, self.sunk.rows.len());
        self.sunk_names.len = first.bytes_len;
        self.sunk_names.required = first.required_bytes;
        if !valid_sunk(
            self.sunk.outcome,
            &self.sunk.rows,
            &self.sunk_names.first,
            first.bytes_len,
            first.required_bytes,
        ) {
            self.sunk.outcome = invalid_names();
        }
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        let mut result = [
            self.supremacy.outcome,
            self.convoys.outcome,
            self.cooldowns.outcome,
            self.edges.outcome,
            self.land.outcome,
            self.land_groups.outcome,
            self.land_members.outcome,
            self.air.outcome,
            self.air_groups.outcome,
            self.air_members.outcome,
            self.results.outcome,
            self.result_members.outcome,
            self.result_countries.outcome,
            self.sunk.outcome,
            self.groups.outcome,
            self.group_members.outcome,
            self.group_children.outcome,
        ];
        if self.theatres_active()
            && !matches!(result[6], Outcome::Unavailable { .. })
            && !complete_source(&self.orders_sources, self.orders_finished)
        {
            result[6] = source_unavailable();
        }
        if self.theatres_active()
            && !matches!(result[9], Outcome::Unavailable { .. })
            && !complete_source(&self.wing_sources, self.wings_finished)
        {
            result[9] = source_unavailable();
        }
        result[13] = singleton_outcome(result[13]);
        result
    }

    fn checked_outcomes(&self) -> [Outcome; N] {
        let mut outcomes = self.outcomes();
        if let Some(reason) = membership_issue(
            outcomes[6],
            &self.land_members.rows,
            |r| (r.orders_group, r.theater_group),
            self.orders_sources.outcome,
            &self.orders_sources.rows,
            |r| (r.orders_group, r.theater_group),
        ) {
            outcomes[6] = Outcome::Unavailable { reason };
        }
        if let Some(reason) = membership_issue(
            outcomes[9],
            &self.air_members.rows,
            |r| (r.wing, r.group),
            self.wing_sources.outcome,
            &self.wing_sources.rows,
            |r| (r.wing, r.group),
        ) {
            outcomes[9] = Outcome::Unavailable { reason };
        }
        outcomes
    }

    pub fn table_inputs(&self, mut projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
        let outcomes = self.checked_outcomes();
        let supremacy = |r: &cnaval_region_dominance::Row| rows::NavalSupremacyRow {
            region: r.region,
            region_slot: r.region_slot,
            country: r.country,
            current: r.current,
            base_target: r.base_target,
            target: r.target,
            individual_ratio: r.individual_ratio,
            previous: r.previous,
            decline_from: r.decline_from,
        };
        let convoy = |r: &csunk_convoy_info::Row| rows::SunkConvoyRow {
            month: r.month,
            killer_country: r.killer_country,
            owner_country: r.owner_country,
            convoys: r.convoys,
            ordinal: r.ordinal,
        };
        let land = |r: &ctheatre::Row| rows::LandTheatreRow {
            id: r.id,
            owner: r.owner,
            volunteers_theatre: r.volunteers_theatre,
            ordinal: r.ordinal,
        };
        let land_member = |r: &ctheater_group::MemberRow| rows::LandTheatreMemberRow {
            orders_group: r.orders_group,
            theater_group: r.theater_group,
            ordinal: r.ordinal,
        };
        let air_member = |r: &cair_group::MemberRow| rows::AirTheatreMemberRow {
            wing: r.wing,
            group: r.group,
            ordinal: r.ordinal,
        };
        let result_row = |r: &cnaval_combat_results::Row| rows::NavalResultRow {
            id: r.id,
            province: r.province,
            result_hour: r.result_hour,
            retire_hour: r.retire_hour,
            category: r.category,
            flag_a: r.flag_a,
            flag_b: r.flag_b,
            subtype: r.subtype,
            readiness: r.readiness,
            winning_side: r.winning_side,
            losing_side: r.losing_side,
            province_ordinal: r.province_ordinal,
        };
        let result_member = |r: &cnaval_combat_results::MemberRow| rows::NavalResultMemberRow {
            result: r.result,
            member: r.member,
            presence_value: r.presence_value,
            ordinal: r.ordinal,
            side: r.side,
        };
        let result_country = |r: &cnaval_combat_results::CountryRow| rows::NavalResultCountryRow {
            result: r.result,
            country: r.country,
            ordinal: r.ordinal,
            list: r.list,
        };
        let group_member = |r: &corders_group::MemberRow| rows::OrdersGroupMemberRow {
            group: r.group,
            ordinal: r.ordinal,
            unit: r.unit,
        };
        let group_child = |r: &corders_group::ChildRow| rows::ArmyGroupMemberRow {
            group: r.group,
            ordinal: r.ordinal,
            child: r.child,
        };
        [
            scalar_table::<tables::NavalSupremacy, _>(
                projection.as_deref_mut(),
                outcomes[0],
                &self.supremacy.rows,
                |r| RowCodec::key(&supremacy(r)),
                supremacy,
            ),
            scalar_table::<tables::SunkConvoys, _>(
                projection.as_deref_mut(),
                outcomes[1],
                &self.convoys.rows,
                |r| RowCodec::key(&convoy(r)),
                convoy,
            ),
            self.rail_cooldowns_input(projection.as_deref_mut(), outcomes[2]),
            self.rail_edges_input(projection.as_deref_mut(), outcomes[3]),
            scalar_table::<tables::LandTheatres, _>(
                projection.as_deref_mut(),
                outcomes[4],
                &self.land.rows,
                |r| RowCodec::key(&land(r)),
                land,
            ),
            self.land_groups_input(projection.as_deref_mut(), outcomes[5]),
            scalar_table::<tables::LandTheatreMembers, _>(
                projection.as_deref_mut(),
                outcomes[6],
                &self.land_members.rows,
                |r| RowCodec::key(&land_member(r)),
                land_member,
            ),
            self.air_input(projection.as_deref_mut(), outcomes[7]),
            self.air_groups_input(projection.as_deref_mut(), outcomes[8]),
            scalar_table::<tables::AirTheatreMembers, _>(
                projection.as_deref_mut(),
                outcomes[9],
                &self.air_members.rows,
                |r| RowCodec::key(&air_member(r)),
                air_member,
            ),
            scalar_table::<tables::NavalResults, _>(
                projection.as_deref_mut(),
                outcomes[10],
                &self.results.rows,
                |r| RowCodec::key(&result_row(r)),
                result_row,
            ),
            scalar_table::<tables::NavalResultMembers, _>(
                projection.as_deref_mut(),
                outcomes[11],
                &self.result_members.rows,
                |r| RowCodec::key(&result_member(r)),
                result_member,
            ),
            scalar_table::<tables::NavalResultCountries, _>(
                projection.as_deref_mut(),
                outcomes[12],
                &self.result_countries.rows,
                |r| RowCodec::key(&result_country(r)),
                result_country,
            ),
            self.sunk_table().into(),
            self.orders_groups_input(projection.as_deref_mut(), outcomes[14]),
            scalar_table::<tables::OrdersGroupMembers, _>(
                projection.as_deref_mut(),
                outcomes[15],
                &self.group_members.rows,
                |r| RowCodec::key(&group_member(r)),
                group_member,
            ),
            scalar_table::<tables::ArmyGroupMembers, _>(
                projection,
                outcomes[16],
                &self.group_children.rows,
                |r| RowCodec::key(&group_child(r)),
                group_child,
            ),
        ]
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection without a baseline always samples"),
        })
    }

    fn orders_groups_input(
        &self,
        projection: Option<&mut Projection<'_>>,
        outcome: Outcome,
    ) -> TableInput {
        let name_slice = |r: &corders_group::GroupRow| {
            r.name_known
                .then(|| {
                    self.group_names
                        .first
                        .get(r.name_start..r.name_start + r.name_len)
                })
                .flatten()
        };
        project_table::<tables::OrdersGroups, _>(
            projection,
            outcome,
            &self.groups.rows,
            |r| u128::from(r.handle),
            |r, prev| {
                r.handle == prev.handle
                    && r.country == prev.country
                    && r.is_army_group == prev.is_army_group
                    && r.motorization_level == prev.motorization_level
                    && r.theatre == prev.theatre
                    && r.theatre_ordinal == prev.theatre_ordinal
                    && r.theater_group == prev.theater_group
                    && r.parent == prev.parent
                    && r.leader == prev.leader
                    && r.hq_provider == prev.hq_provider
                    && matches!(
                        &prev.name,
                        rows::Recorded::Value(name) if name.as_deref() == name_slice(r)
                    )
            },
            |r| rows::OrdersGroupRow {
                handle: r.handle,
                country: r.country,
                is_army_group: r.is_army_group,
                motorization_level: r.motorization_level,
                theatre: r.theatre,
                theatre_ordinal: r.theatre_ordinal,
                theater_group: r.theater_group,
                parent: r.parent,
                leader: r.leader,
                hq_provider: r.hq_provider,
                name: name_slice(r)
                    .map(|bytes| owned_vec(copy_vec(bytes)))
                    .into(),
            },
        )
    }

    fn land_groups_input(
        &self,
        projection: Option<&mut Projection<'_>>,
        outcome: Outcome,
    ) -> TableInput {
        let Some(bytes) = self.theatre_names.first.get(..self.theatre_names.len) else {
            return unavailable("theatre name arena invalid").into();
        };
        if !valid_named(outcome, &self.land_groups.rows, Some(bytes), |r| {
            (r.name_start, r.name_len)
        }) {
            return unavailable("land group name range exceeds captured arena").into();
        }
        project_table::<tables::LandTheatreGroups, _>(
            projection,
            outcome,
            &self.land_groups.rows,
            |r| u128::from(r.id),
            |r, prev| {
                r.id == prev.id
                    && r.theatre == prev.theatre
                    && r.ordinal == prev.ordinal
                    && r.priority == prev.priority
                    && prev.name.as_slice() == &bytes[r.name_start..r.name_start + r.name_len]
            },
            |r| rows::LandTheatreGroupRow {
                id: r.id,
                theatre: r.theatre,
                ordinal: r.ordinal,
                priority: r.priority,
                name: owned_vec(copy_vec(&bytes[r.name_start..r.name_start + r.name_len])),
            },
        )
    }

    fn air_input(
        &self,
        projection: Option<&mut Projection<'_>>,
        outcome: Outcome,
    ) -> TableInput {
        let Some(bytes) = self.theatre_names.first.get(..self.theatre_names.len) else {
            return unavailable("theatre name arena invalid").into();
        };
        if !valid_named(outcome, &self.air.rows, Some(bytes), |r| {
            (r.name_start, r.name_len)
        }) {
            return unavailable("air theatre name range exceeds captured arena").into();
        }
        project_table::<tables::AirTheatres, _>(
            projection,
            outcome,
            &self.air.rows,
            |r| u128::from(r.id),
            |r, prev| {
                r.id == prev.id
                    && r.owner == prev.owner
                    && r.ordinal == prev.ordinal
                    && prev.name.as_slice() == &bytes[r.name_start..r.name_start + r.name_len]
            },
            |r| rows::AirTheatreRow {
                id: r.id,
                owner: r.owner,
                ordinal: r.ordinal,
                name: owned_vec(copy_vec(&bytes[r.name_start..r.name_start + r.name_len])),
            },
        )
    }

    fn air_groups_input(
        &self,
        projection: Option<&mut Projection<'_>>,
        outcome: Outcome,
    ) -> TableInput {
        let Some(bytes) = self.theatre_names.first.get(..self.theatre_names.len) else {
            return unavailable("theatre name arena invalid").into();
        };
        if !valid_named(outcome, &self.air_groups.rows, Some(bytes), |r| {
            (r.name_start, r.name_len)
        }) {
            return unavailable("air group name range exceeds captured arena").into();
        }
        project_table::<tables::AirTheatreGroups, _>(
            projection,
            outcome,
            &self.air_groups.rows,
            |r| u128::from(r.id),
            |r, prev| {
                r.id == prev.id
                    && r.theatre == prev.theatre
                    && r.ordinal == prev.ordinal
                    && prev.name.as_slice() == &bytes[r.name_start..r.name_start + r.name_len]
            },
            |r| rows::AirTheatreGroupRow {
                id: r.id,
                theatre: r.theatre,
                ordinal: r.ordinal,
                name: owned_vec(copy_vec(&bytes[r.name_start..r.name_start + r.name_len])),
            },
        )
    }

    fn sunk_table(&self) -> TableView {
        let outcome = self.sunk.outcome;
        match singleton_outcome(outcome) {
            Outcome::Unavailable { reason } => return unavailable(reason),
            Outcome::Truncated { .. } => {
                return TableView::Truncated {
                    rows: TableRows::SunkShips(Vec::new()),
                    missing: 1,
                };
            }
            _ => {}
        }
        if !valid_sunk(
            outcome,
            &self.sunk.rows,
            &self.sunk_names.first,
            self.sunk_names.len,
            self.sunk_names.required,
        ) {
            return unavailable("sunk-ship name range exceeds captured arena");
        }
        let bytes = &self.sunk_names.first[..self.sunk_names.len];
        let text = |range: csunk_ship_info::TextRange| {
            owned_vec(copy_vec(&bytes[range.start..range.start + range.len]))
        };
        let entries = owned_vec(collect_vec(self.sunk.rows[..outcome.len()].iter().map(
            |r| rows::SunkShipEntry {
                ship_name: text(r.ship_name),
                attribution_name: text(r.attribution_name),
                class_name: text(r.class_name),
                attribution_class_name: text(r.attribution_class_name),
                victim_country: r.victim_country,
                attacker_country: r.attacker_country,
                sink_hour: r.sink_hour,
                class_token: r.class_token,
                attribution_class_token: r.attribution_class_token,
                attribution_class_present: r.attribution_class_present,
                model_level: r.model_level,
                victim_equipment_variant: r.victim_equipment_variant,
                damage_source_reference: r.damage_source_reference,
                province: r.province,
                province_present: r.province_present,
                battle: r.battle,
                cause_record: r.cause_record,
                assisted_kill: r.assisted_kill,
            },
        )));
        TableView::Complete(TableRows::SunkShips(owned_vec(collect_vec([
            rows::SunkShipsRow { entries },
        ]))))
    }
}

fn storage_buffer<T>(values: &mut Vec<T>) -> engine_native_memory::walk::Buffer<'_, T> {
    engine_native_memory::walk::Buffer::Growing(values)
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn source_unavailable() -> Outcome {
    Outcome::Unavailable {
        reason: "complete theatre reverse census unavailable",
    }
}
fn invalid_names() -> Outcome {
    Outcome::Unavailable {
        reason: "name range exceeds captured arena",
    }
}
fn complete_source<R>(captured: &CaptureRows<R>, finished: bool) -> bool {
    finished && matches!(captured.outcome, Outcome::Rows { len } if len <= captured.rows.len())
}
fn singleton_outcome(outcome: Outcome) -> Outcome {
    match outcome {
        Outcome::Rows { .. } => Outcome::Rows { len: 1 },
        Outcome::Truncated { .. } => Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        },
        other => other,
    }
}
fn push<R>(rows: &mut [R], outcome: &mut Outcome, row: Result<R, &'static str>) {
    if matches!(*outcome, Outcome::Unavailable { .. }) {
        return;
    }
    let row = match row {
        Ok(row) => row,
        Err(reason) => {
            *outcome = Outcome::Unavailable { reason };
            return;
        }
    };
    let len = outcome.len();
    if let Some(destination) = rows.get_mut(len) {
        *destination = row;
        *outcome = Outcome::Rows { len: len + 1 };
    } else {
        let omitted = match *outcome {
            Outcome::Truncated { unread_rows, .. } => unread_rows,
            _ => 0,
        };
        *outcome = match omitted.checked_add(1) {
            Some(unread_rows) => Outcome::Truncated { len, unread_rows },
            None => Outcome::Unavailable {
                reason: "theatre reverse census row count overflow",
            },
        };
    }
}
fn range(bytes: &[u8], start: usize, len: usize) -> Option<&[u8]> {
    bytes.get(start..start.checked_add(len)?)
}
fn valid_named<R>(
    outcome: Outcome,
    rows: &[R],
    bytes: Option<&[u8]>,
    name: impl Fn(&R) -> (usize, usize),
) -> bool {
    if matches!(outcome, Outcome::Unavailable { .. }) {
        return true;
    }
    let Some(bytes) = bytes else {
        return false;
    };
    rows.get(..outcome.len()).is_some_and(|rows| {
        rows.iter().all(|row| {
            let (start, len) = name(row);
            range(bytes, start, len).is_some()
        })
    })
}
fn valid_sunk(
    outcome: Outcome,
    rows: &[csunk_ship_info::Record],
    bytes: &[u8],
    len: usize,
    required: usize,
) -> bool {
    if matches!(outcome, Outcome::Unavailable { .. }) {
        return true;
    }
    let Some(bytes) = bytes.get(..len).filter(|_| required >= len) else {
        return false;
    };
    rows.get(..outcome.len()).is_some_and(|rows| {
        rows.iter().all(|r| {
            [
                r.ship_name,
                r.attribution_name,
                r.class_name,
                r.attribution_class_name,
            ]
            .iter()
            .all(|r| range(bytes, r.start, r.len).is_some())
        })
    })
}

fn membership_issue<A, B>(
    forward: Outcome,
    rows: &[A],
    member: impl Fn(&A) -> (u64, u64),
    reverse: Outcome,
    sources: &[B],
    backlink: impl Fn(&B) -> (u64, u64),
) -> Option<&'static str> {
    let Outcome::Rows { len } = forward else {
        return None;
    };
    let Outcome::Rows { len: source_len } = reverse else {
        return Some("complete theatre reverse census unavailable");
    };
    let Some(rows) = rows.get(..len) else {
        return Some("theatre membership exceeds captured rows");
    };
    let Some(sources) = sources.get(..source_len) else {
        return Some("theatre reverse census exceeds captured rows");
    };
    let Ok(mut expected) = collect_vec(sources.iter().map(backlink)) else {
        return Some("collector allocation failed");
    };
    let Ok(mut seen) = collect_vec(rows.iter().map(member)) else {
        return Some("collector allocation failed");
    };
    if collector_memory::terminal::failed() {
        return Some("collector allocation failed");
    }
    expected.sort_unstable_by_key(|&(id, _)| id);
    seen.sort_unstable_by_key(|&(id, _)| id);
    if expected.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Some("duplicate identity in theatre reverse census");
    }
    if seen.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Some("unit belongs to multiple theatre membership entries");
    }
    for &(id, group) in &seen {
        if group == 0
            || expected
                .binary_search_by_key(&id, |&(id, _)| id)
                .map_or(true, |index| expected[index].1 != group)
        {
            return Some("theatre membership disagrees with native unit backlink");
        }
    }
    if expected.iter().any(|&(id, group)| {
        group != 0
            && seen
                .binary_search_by_key(&id, |&(id, _)| id)
                .map_or(true, |index| seen[index].1 != group)
    }) {
        return Some("native unit backlink is absent from theatre membership");
    }
    None
}

pub const NAVAL_PACK_N: usize = 9;

mod theatres;
pub use theatres::{
    NavalCopyContext, NavalCopyReport, NavalJob, OrderGroupSource, copy_naval_refresh,
    decode_naval_payload,
};
#[cfg(test)]
pub use theatres::{copy_naval_oob, copy_naval_theatres, dedup_first_seen};

pub const SUPREMACY_N: usize = 1;

mod supremacy;
pub use supremacy::{
    SupremacyCopyContext, SupremacyCopyReport, SupremacyJob, copy_supremacy_refresh,
};

pub const SUNK_N: usize = 1;

mod rail_store;
mod sunk;
pub use rail_store::{RailReuse, RailStore};
#[cfg(test)]
mod rail_store_tests;
pub use sunk::{SunkCopyContext, SunkCopyReport, SunkJob, copy_sunk_refresh};

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
pub mod naval_test_kit;

#[cfg(any(test, feature = "test-support"))]
pub mod reserved_land_tests;

#[cfg(any(test, feature = "test-support"))]
pub mod supremacy_test_kit;

#[cfg(any(test, feature = "test-support"))]
pub mod sunk_test_kit;

