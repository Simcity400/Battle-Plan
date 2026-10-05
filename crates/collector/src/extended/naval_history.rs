//! Pooled naval/history acquisition; conversion and membership joins run off-thread.
//!
//! Source-fed split (copy/decode): hourly LandTheatres headers, daily
//! LandTheatreGroups/LandTheatreMembers/AirTheatres/AirTheatreGroups/
//! AirTheatreMembers details, and the order-of-battle block (OrdersGroups,
//! OrdersGroupMembers, ArmyGroupMembers) move to the single FIFO decode
//! worker in their own source-fed payload. The four
//! non-source-fed theatres move with the five source-fed tables because
//! they share the `ctheatre::walk` and OOB walks; splitting would duplicate
//! those walks with separate censuses. Each stages its own root refs from
//! its own copies with no cross-family decoded roots; zero-engine-change
//! replay of the existing walks against `Recorded` spans. Choice per table:
//! zero-engine-change (Wars/Divisions precedent). Rail, convoys and battle
//! results use their typed callback paths; supremacy and sunk ships retain
//! independent raw payloads.

use crate::{
    raw::RawLog,
    read_accounting,
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, bounded},
};
use engine::allocation::{collect_vec, copy_vec, default_vec, owned_vec, resize_default};
use engine::{
    Memory, cair_group, cair_theatre, cair_wing, cdominance_values, cgame_date, cgame_state,
    cnaval_combat_results, cnaval_region_dominance, corders_group, cpdx_array::Array, cprovince,
    cprovince_railway_info, crailway_manager, cstrategic_region, csub_unit_definition,
    csunk_convoy_info, csunk_ship_info, ctheater_group, ctheatre, msvc_string,
    naval_dominance_entry, rail_neighbor, walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, TableRows, tables},
};

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

/// Shared traversals are charged once to their first table. The separately
/// acquired reverse censuses are included in these totals.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("railway state", 2, &[2, 3]),
    ("land and air theatres", 4, &[4, 5, 6, 7, 8, 9]),
    ("retained naval results", 10, &[10, 11, 12]),
    ("army order of battle", 14, &[14, 15, 16]),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub theatre_names: usize,
    pub sunk_names: usize,
    pub orders_sources: usize,
    pub wing_sources: usize,
    pub roster_scratch: usize,
}
impl Default for Capacities {
    fn default() -> Self {
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
        }
    }
}

pub struct Captured {
    supremacy: CaptureRows<cnaval_region_dominance::Row>,
    convoys: CaptureRows<csunk_convoy_info::Row>,
    cooldowns: CaptureRows<cprovince_railway_info::Row>,
    edges: CaptureRows<rail_neighbor::Row>,
    land: CaptureRows<ctheatre::Row>,
    land_groups: CaptureRows<ctheater_group::Row>,
    land_members: CaptureRows<ctheater_group::MemberRow>,
    air: CaptureRows<cair_theatre::Row>,
    air_groups: CaptureRows<cair_group::Row>,
    air_members: CaptureRows<cair_group::MemberRow>,
    results: CaptureRows<cnaval_combat_results::Row>,
    result_members: CaptureRows<cnaval_combat_results::MemberRow>,
    result_countries: CaptureRows<cnaval_combat_results::CountryRow>,
    /// One scratch slot per result row: the walk pairs rows with their province buckets in it.
    result_slots: Vec<cnaval_combat_results::ResultSlot>,
    sunk: CaptureRows<csunk_ship_info::Record>,
    groups: CaptureRows<corders_group::GroupRow>,
    group_members: CaptureRows<corders_group::MemberRow>,
    group_children: CaptureRows<corders_group::ChildRow>,
    roster_scratch: Arena<corders_group::RosterEntry>,
    oob_finished: bool,
    theatre_names: Arena<u8>,
    sunk_names: Arena<u8>,
    orders_sources: CaptureRows<ctheatre::OrdersMembership>,
    wing_sources: CaptureRows<ctheatre::WingMembership>,
    orders_finished: bool,
    wings_finished: bool,
    source_ticks: [u64; 3],
    active: [bool; N],
    /// Raw-copy split gate for the hourly source-fed naval block (9 tables).
    /// Set by the game thread every hour with a valid raw payload; `capture`
    /// then skips the theatre/OOB walks and the decode worker replays them
    /// before the ready handoff.
    pub(crate) raw_mode: bool,
    /// Raw-copy split gate for the hourly supremacy payload
    /// (NavalSupremacy). Set by the game thread every hour with a valid raw
    /// payload; `capture` then skips the typed walk and the decode worker
    /// fills the rows before the ready handoff. Preserved across resets,
    /// cleared by the worker after decode.
    pub(crate) supremacy_raw_mode: bool,
    /// Raw-copy split gate for the hourly sunk-ships payload (SunkShips).
    /// Set by the game thread every hour with a valid raw payload; `capture`
    /// then skips the typed walk and the decode worker fills the rows before
    /// the ready handoff. Preserved across resets, cleared by the worker
    /// after decode.
    pub(crate) sunk_raw_mode: bool,
}

const SKIPPED: Outcome = Outcome::Unavailable {
    reason: replay::cadence::SKIPPED_REASON,
};

fn failed(active: bool, reason: &'static str) -> Outcome {
    if active {
        Outcome::Unavailable { reason }
    } else {
        SKIPPED
    }
}

impl Captured {
    /// Late owns the residual typed walks; independent raw payloads keep their
    /// gates and are installed normally after this terminal result.
    pub(crate) fn fail_late_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.prepare_hour(hour, force_full);
        if !self.supremacy_raw_mode {
            self.supremacy.fail(failed(self.active[0], reason));
        }
        self.convoys.fail(failed(self.active[1], reason));
        self.cooldowns.fail(failed(self.active[2], reason));
        self.edges.fail(failed(self.active[3], reason));
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

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            theatre_names: 0,
            sunk_names: 0,
            orders_sources: 0,
            wing_sources: 0,
            roster_scratch: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            supremacy: CaptureRows::new(capacity.rows[0]),
            convoys: CaptureRows::new(capacity.rows[1]),
            cooldowns: CaptureRows::new(capacity.rows[2]),
            edges: CaptureRows::new(capacity.rows[3]),
            land: CaptureRows::new(capacity.rows[4]),
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
            active: [true; N],
            raw_mode: false,
            supremacy_raw_mode: false,
            sunk_raw_mode: false,
        }
    }

    /// Freeze the requested tables before contexts and shared map callbacks.
    pub(crate) fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.active = TABLES.map(|table| replay::cadence::should_walk(table, hour, force_full));
    }

    /// The daily theatre details; LandTheatres headers are walked every hour.
    pub(crate) fn theatres_active(&self) -> bool {
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

    /// Call before shared map acquisition; capture() preserves these source visits.
    pub fn reset(&mut self) {
        self.reset_tables();
        self.orders_sources.reset();
        self.wing_sources.reset();
        self.groups.reset();
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
        self.cooldowns.reset();
        self.edges.reset();
        self.land.reset();
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
        self.cooldowns.grow(&mut capacity.rows[2]);
        self.edges.grow(&mut capacity.rows[3]);
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
        self.group_members.grow(&mut capacity.rows[15]);
        self.group_children.grow(&mut capacity.rows[16]);
        self.roster_scratch.grow(&mut capacity.roster_scratch);
        self.theatre_names.grow(&mut capacity.theatre_names);
        self.sunk_names.grow(&mut capacity.sunk_names);
        self.orders_sources.grow(&mut capacity.orders_sources);
        self.wing_sources.grow(&mut capacity.wing_sources);
    }

    /// Already included in the pack's returned times, in
    /// [theatre membership, wings, order of battle] order. The shared adapter
    /// subtracts these from the enclosing map acquisition times.
    pub fn source_ticks(&self) -> [u64; 3] {
        self.source_ticks
    }

    pub fn set_order_sources(
        &mut self,
        memory: &impl Memory,
        base: usize,
        roots: &[corders_group::Root],
        source: Outcome,
    ) {
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

    /// All three tables use the same authenticated registry census.
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
                let capacity = [groups.len(), members.len(), children.len()];
                let walk = corders_group::walk_order_of_battle(
                    memory,
                    base,
                    &roots[..len],
                    &mut corders_group::OrderOfBattle {
                        groups,
                        members,
                        children,
                        roster_scratch: &mut self.roster_scratch.first,
                    },
                );
                [
                    bounded(walk.groups, capacity[0]),
                    bounded(walk.members, capacity[1]),
                    bounded(walk.children, capacity[2]),
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
    pub fn visit_wing(&mut self, memory: &impl Memory, pointer: usize, wing: &cair_wing::Row) {
        if !self.theatres_active() {
            self.wing_sources.outcome = SKIPPED;
            return;
        }
        let _reads = read_accounting::table(TableId::AirTheatreMembers);
        let begin = qpc();
        let (output, outcome) = (&mut self.wing_sources.rows, &mut self.wing_sources.outcome);
        let row = if outcome.len() < output.len() {
            ctheatre::read_wing_membership(memory, pointer, wing)
        } else {
            Ok(ctheatre::WingMembership::default())
        };
        push(output, outcome, row);
        self.source_ticks[1] = self.source_ticks[1].saturating_add(elapsed(begin));
    }
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
    /// Typed theatre fallback consumes the map-admitted membership prefix.
    pub(crate) fn install_map_wings(&mut self, job: &super::air::AirWingJob) {
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

    pub(crate) fn add_wing_source_ticks(&mut self, ticks: u64) {
        self.source_ticks[1] = self.source_ticks[1].saturating_add(ticks);
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.prepare_hour(current_hour, force_full);
        self.reset_tables();
        let mut ticks = [0; N];
        // Hourly supremacy raw-copy split: on hours with a valid raw payload
        // the game thread stages bytes via `copy_supremacy_refresh` and the
        // decode worker fills the typed rows before the ready handoff, so
        // the typed walk is skipped here.
        if !self.active[0] {
            self.supremacy.outcome = SKIPPED;
        } else if self.supremacy_raw_mode {
            // Refresh payload staged: leave the reset outcome for the decode
            // worker, which installs identical rows and outcomes.
            ticks[0] = 0;
        } else {
            let begin = qpc();
            capture_rows(&mut self.supremacy, |rows| {
                let _reads = read_accounting::table(TableId::NavalSupremacy);
                cnaval_region_dominance::walk(memory, base, game_state, rows)
            });
            ticks[0] = elapsed(begin);
        }
        if self.active[1] {
            let begin = qpc();
            capture_rows(&mut self.convoys, |rows| {
                let _reads = read_accounting::table(TableId::SunkConvoys);
                csunk_convoy_info::walk(memory, base, game_state, rows)
            });
            ticks[1] = elapsed(begin);
        } else {
            self.convoys.outcome = SKIPPED;
        }

        let rail_reads = read_accounting::table(TableId::RailCooldowns);
        let begin = qpc();
        let first = crailway_manager::walk(
            memory,
            base,
            game_state,
            &mut self.cooldowns.rows,
            &mut self.edges.rows,
        );
        self.cooldowns.outcome = bounded(first.provinces, self.cooldowns.rows.len());
        self.edges.outcome = bounded(first.edges, self.edges.rows.len());
        ticks[2] = elapsed(begin);
        drop(rail_reads);

        // Raw-copy split (hourly source-fed naval block): the game thread
        // staged raw bytes and the decode worker replays theatres/OOB before
        // the ready handoff. Skip those walks here.
        if !self.theatres_active() {
            self.skip_theatre_details();
        }
        if !self.raw_mode {
            if self.active[4] {
                let _reads = read_accounting::table(TableId::LandTheatres);
                let begin = qpc();
                self.capture_theatres(memory, base, game_state);
                ticks[4] = elapsed(begin);
                ticks[6] = self.source_ticks[0];
                ticks[9] = self.source_ticks[1];
            }
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

        // Hourly sunk-ships raw-copy split: on hours with a valid raw payload
        // the game thread stages bytes via `copy_sunk_refresh` and the decode
        // worker fills the typed rows before the ready handoff, so the typed
        // walk is skipped here.
        if !self.active[13] {
            self.sunk.outcome = SKIPPED;
        } else if self.sunk_raw_mode {
            // Refresh payload staged: leave the reset outcome for the decode
            // worker, which installs identical rows and outcomes.
            ticks[13] = 0;
        } else {
            let _reads = read_accounting::table(TableId::SunkShips);
            let begin = qpc();
            self.capture_sunk(memory, base, game_state, current_hour);
            ticks[13] = elapsed(begin);
        }
        ticks
    }

    fn capture_theatres(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        let first = ctheatre::walk(
            memory,
            base,
            game_state,
            self.theatres_active(),
            ctheatre::Buffers {
                land: &mut self.land.rows,
                land_groups: &mut self.land_groups.rows,
                land_members: &mut self.land_members.rows,
                air: &mut self.air.rows,
                air_groups: &mut self.air_groups.rows,
                air_members: &mut self.air_members.rows,
                names: &mut self.theatre_names.first,
            },
        );
        self.land.outcome = bounded(first.land, self.land.rows.len());
        if self.theatres_active() {
            self.land_groups.outcome = bounded(first.land_groups, self.land_groups.rows.len());
            self.land_members.outcome = bounded(first.land_members, self.land_members.rows.len());
            self.air.outcome = bounded(first.air, self.air.rows.len());
            self.air_groups.outcome = bounded(first.air_groups, self.air_groups.rows.len());
            self.air_members.outcome = bounded(first.air_members, self.air_members.rows.len());
        }
        self.theatre_names.len = first.names_len;
        self.theatre_names.required = first.names_required;
        let first_names = self
            .theatre_names
            .first
            .get(..first.names_len)
            .filter(|_| first.names_required >= first.names_len);
        validate_named(&mut self.land_groups, first_names, |r| {
            (r.name_start, r.name_len)
        });
        validate_named(&mut self.air, first_names, |r| (r.name_start, r.name_len));
        validate_named(&mut self.air_groups, first_names, |r| {
            (r.name_start, r.name_len)
        });
        if self.theatres_active() && !complete_source(&self.orders_sources, self.orders_finished) {
            self.land_members.outcome = source_unavailable();
        }
        if self.theatres_active() && !complete_source(&self.wing_sources, self.wings_finished) {
            self.air_members.outcome = source_unavailable();
        }
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
        // The list has one replay row. A partial native list omits that whole row.
        result[13] = singleton_outcome(result[13]);
        result
    }

    /// Off-thread only: exact native source joins, byte copies and replay rows.
    pub fn tables(&self) -> [TableView; N] {
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
        [
            table::<tables::NavalSupremacy, _>(outcomes[0], &self.supremacy.rows, |r| {
                rows::NavalSupremacyRow {
                    region: r.region,
                    region_slot: r.region_slot,
                    country: r.country,
                    current: r.current,
                    base_target: r.base_target,
                    target: r.target,
                    individual_ratio: r.individual_ratio,
                    previous: r.previous,
                    decline_from: r.decline_from,
                }
            }),
            table::<tables::SunkConvoys, _>(outcomes[1], &self.convoys.rows, |r| {
                rows::SunkConvoyRow {
                    month: r.month,
                    killer_country: r.killer_country,
                    owner_country: r.owner_country,
                    convoys: r.convoys,
                    ordinal: r.ordinal,
                }
            }),
            table::<tables::RailCooldowns, _>(outcomes[2], &self.cooldowns.rows, |r| {
                rows::RailCooldownRow {
                    province: r.province,
                    cooldown_ticks: r.cooldown_ticks,
                }
            }),
            table::<tables::RailEdges, _>(outcomes[3], &self.edges.rows, |r| rows::RailEdgeRow {
                from: r.from_province,
                to: r.to_province,
                level: r.level,
            }),
            table::<tables::LandTheatres, _>(outcomes[4], &self.land.rows, |r| {
                rows::LandTheatreRow {
                    id: r.id,
                    owner: r.owner,
                    volunteers_theatre: r.volunteers_theatre,
                    ordinal: r.ordinal,
                }
            }),
            self.land_groups_table(outcomes[5]),
            table::<tables::LandTheatreMembers, _>(outcomes[6], &self.land_members.rows, |r| {
                rows::LandTheatreMemberRow {
                    orders_group: r.orders_group,
                    theater_group: r.theater_group,
                    ordinal: r.ordinal,
                }
            }),
            self.air_table(outcomes[7]),
            self.air_groups_table(outcomes[8]),
            table::<tables::AirTheatreMembers, _>(outcomes[9], &self.air_members.rows, |r| {
                rows::AirTheatreMemberRow {
                    wing: r.wing,
                    group: r.group,
                    ordinal: r.ordinal,
                }
            }),
            table::<tables::NavalResults, _>(outcomes[10], &self.results.rows, |r| {
                rows::NavalResultRow {
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
                }
            }),
            table::<tables::NavalResultMembers, _>(outcomes[11], &self.result_members.rows, |r| {
                rows::NavalResultMemberRow {
                    result: r.result,
                    member: r.member,
                    presence_value: r.presence_value,
                    ordinal: r.ordinal,
                    side: r.side,
                }
            }),
            table::<tables::NavalResultCountries, _>(
                outcomes[12],
                &self.result_countries.rows,
                |r| rows::NavalResultCountryRow {
                    result: r.result,
                    country: r.country,
                    ordinal: r.ordinal,
                    list: r.list,
                },
            ),
            self.sunk_table(),
            table::<tables::OrdersGroups, _>(outcomes[14], &self.groups.rows, |r| {
                rows::OrdersGroupRow {
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
                }
            }),
            table::<tables::OrdersGroupMembers, _>(outcomes[15], &self.group_members.rows, |r| {
                rows::OrdersGroupMemberRow {
                    group: r.group,
                    ordinal: r.ordinal,
                    unit: r.unit,
                }
            }),
            table::<tables::ArmyGroupMembers, _>(outcomes[16], &self.group_children.rows, |r| {
                rows::ArmyGroupMemberRow {
                    group: r.group,
                    ordinal: r.ordinal,
                    child: r.child,
                }
            }),
        ]
    }

    fn land_groups_table(&self, outcome: Outcome) -> TableView {
        let Some(bytes) = self.theatre_names.first.get(..self.theatre_names.len) else {
            return unavailable("theatre name arena invalid");
        };
        if !valid_named(outcome, &self.land_groups.rows, Some(bytes), |r| {
            (r.name_start, r.name_len)
        }) {
            return unavailable("land group name range exceeds captured arena");
        }
        table::<tables::LandTheatreGroups, _>(outcome, &self.land_groups.rows, |r| {
            rows::LandTheatreGroupRow {
                id: r.id,
                theatre: r.theatre,
                ordinal: r.ordinal,
                priority: r.priority,
                name: owned_vec(copy_vec(&bytes[r.name_start..r.name_start + r.name_len])),
            }
        })
    }

    fn air_table(&self, outcome: Outcome) -> TableView {
        let Some(bytes) = self.theatre_names.first.get(..self.theatre_names.len) else {
            return unavailable("theatre name arena invalid");
        };
        if !valid_named(outcome, &self.air.rows, Some(bytes), |r| {
            (r.name_start, r.name_len)
        }) {
            return unavailable("air theatre name range exceeds captured arena");
        }
        table::<tables::AirTheatres, _>(outcome, &self.air.rows, |r| rows::AirTheatreRow {
            id: r.id,
            owner: r.owner,
            ordinal: r.ordinal,
            name: owned_vec(copy_vec(&bytes[r.name_start..r.name_start + r.name_len])),
        })
    }

    fn air_groups_table(&self, outcome: Outcome) -> TableView {
        let Some(bytes) = self.theatre_names.first.get(..self.theatre_names.len) else {
            return unavailable("theatre name arena invalid");
        };
        if !valid_named(outcome, &self.air_groups.rows, Some(bytes), |r| {
            (r.name_start, r.name_len)
        }) {
            return unavailable("air group name range exceeds captured arena");
        }
        table::<tables::AirTheatreGroups, _>(outcome, &self.air_groups.rows, |r| {
            rows::AirTheatreGroupRow {
                id: r.id,
                theatre: r.theatre,
                ordinal: r.ordinal,
                name: owned_vec(copy_vec(&bytes[r.name_start..r.name_start + r.name_len])),
            }
        })
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
fn capture_rows<R>(captured: &mut CaptureRows<R>, walk: impl FnOnce(&mut [R]) -> Outcome) {
    captured.outcome = bounded(walk(&mut captured.rows), captured.rows.len());
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
fn validate_named<R>(
    captured: &mut CaptureRows<R>,
    bytes: Option<&[u8]>,
    name: impl Fn(&R) -> (usize, usize),
) {
    if !valid_named(captured.outcome, &captured.rows, bytes, name) {
        captured.outcome = invalid_names();
    }
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

/// Exact join, never a hash/fingerprint acceptance: native full references are
/// the keys. Both censuses must be complete; duplicate membership is rejected.
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
    if crate::allocation::failed() {
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

pub(crate) const NAVAL_PACK_N: usize = 9;

mod theatres;
pub(crate) use theatres::{
    NavalCopyContext, NavalCopyReport, NavalJob, copy_naval_refresh, decode_naval_payload,
};
#[cfg(test)]
use theatres::{
    OOB_THEATRE_REFERENCES_PER_COUNTRY, copy_naval_oob, copy_naval_theatres, dedup_first_seen,
};

/// Hourly payload slots in calibration/report order: [supremacy].
pub(crate) const SUPREMACY_N: usize = 1;

mod supremacy;
pub(crate) use supremacy::{
    SupremacyCopyContext, SupremacyCopyReport, SupremacyJob, copy_supremacy_refresh,
};

/// Hourly payload slots in calibration/report order: [sunk ships]. The
/// singleton table carries one row with the retained entry list.
pub(crate) const SUNK_N: usize = 1;

mod sunk;
pub(crate) use sunk::{SunkCopyContext, SunkCopyReport, SunkJob, copy_sunk_refresh};

#[cfg(test)]
mod tests;

/// Shared synthetic address space for the naval-pack copy/decode harness:
/// one country, one land theatre with a named group and two members, one
/// OOB group with members, and the reference registry. Air theatres are
/// empty (matching the live campaign) with wing handles recorded as a
/// superset.
#[cfg(test)]
pub(crate) mod naval_test_kit;

/// Shared synthetic address space for the supremacy copy/decode harness:
/// two region slots (one null) with dominance containers and value entries.
/// Mirrors the engine supremacy fixture shapes.
#[cfg(test)]
pub(crate) mod supremacy_test_kit;

/// Shared synthetic address space for the sunk-ships copy/decode harness:
/// two retained records (one with an attribution class and province, one
/// minimal) with inline and heap names. Mirrors the engine sunk-ship fixture
/// shapes.
#[cfg(test)]
pub(crate) mod sunk_test_kit;

/// Ignored capture-cost bench; see `bench_world` for the assumed scale. Run
/// with `cargo test -p collector --release -- --ignored l3b_bench --nocapture`.
/// Each case times the copies through the plain slice and through the
/// production guarded source, counts one copy's source requests, and times
/// the decode of the recorded log.
#[cfg(test)]
mod l3b_bench;
