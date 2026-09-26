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
    table_capture::{Arena, CaptureRows},
};
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
use std::collections::BTreeMap;

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
                name: bytes[r.name_start..r.name_start + r.name_len].to_vec(),
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
            name: bytes[r.name_start..r.name_start + r.name_len].to_vec(),
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
                name: bytes[r.name_start..r.name_start + r.name_len].to_vec(),
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
            bytes[range.start..range.start + range.len].to_vec()
        };
        let entries = self.sunk.rows[..outcome.len()]
            .iter()
            .map(|r| rows::SunkShipEntry {
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
            })
            .collect();
        TableView::Complete(TableRows::SunkShips(vec![rows::SunkShipsRow { entries }]))
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
fn bounded(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "walker row count exceeds allocated buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "walker marked truncation without missing rows",
        }
    } else {
        outcome
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
    let mut expected = BTreeMap::new();
    for source in sources {
        let (id, group) = backlink(source);
        if expected.insert(id, group).is_some() {
            return Some("duplicate identity in theatre reverse census");
        }
    }
    let mut seen = BTreeMap::new();
    for row in rows {
        let (id, group) = member(row);
        if seen.insert(id, group).is_some() {
            return Some("unit belongs to multiple theatre membership entries");
        }
        if expected.get(&id) != Some(&group) || group == 0 {
            return Some("theatre membership disagrees with native unit backlink");
        }
    }
    if expected
        .iter()
        .any(|(id, group)| *group != 0 && seen.get(id) != Some(group))
    {
        return Some("native unit backlink is absent from theatre membership");
    }
    None
}

pub(crate) const NAVAL_PACK_N: usize = 9;
/// Fault-attribution masks in closure order: [LandTheatres,
/// LandTheatreGroups, LandTheatreMembers, AirTheatres, AirTheatreGroups,
/// AirTheatreMembers, OrdersGroups, OrdersGroupMembers, ArmyGroupMembers].
/// Shared country roots block the active theatre tables. Land headers, land
/// details and air details have separate fault masks, matching the typed
/// walk's independent outcomes. Shared OOB roots block all three OOB tables.
const THEATRE_MASK: u32 = 0x3F;
const LAND_THEATRE_MASK: u32 = 0x01;
const LAND_DETAILS_MASK: u32 = 0x06;
const LAND_MEMBERS_MASK: u32 = 0x04;
const AIR_DETAILS_MASK: u32 = 0x38;
const OOB_MASK: u32 = 0x1C0;

#[derive(Debug, Default)]
pub(crate) struct NavalJob {
    pub log: RawLog<NAVAL_PACK_N>,
    pub game_state: usize,
    theatres_active: bool,
    wing_sources: Vec<ctheatre::WingMembership>,
    wing_outcome: Option<Outcome>,
    wing_required: usize,
    /// OOB-only memo of freshly authenticated theatre pointers, in root order.
    /// It never shares observations with theatre/member source families. The
    /// root walk pushes every root's theatre; [`dedup_first_seen`] then keeps
    /// each theatre's first occurrence, so demand counts theatre references.
    oob_theatres: Vec<usize>,
    /// Scratch positions for that dedup, with the same capacity as
    /// `oob_theatres`; filling it within that capacity never allocates on the
    /// callback.
    oob_theatre_positions: Vec<usize>,
    oob_theatres_required: usize,
    /// The callback's authenticated country roster as the theatre copy used
    /// it, for this pass's decode only; see [`copy_naval_theatres`].
    roster: engine::ccountry::RosterSnapshot,
    /// The callback's orders-group registry census of this pass, when it was
    /// complete and fitted: `roots[..len]` for `roots_len == Some(len)`. The
    /// order-of-battle copy walks these roots instead of the registry, and
    /// this pass's decode takes them as its census; see [`copy_naval_oob`].
    /// Filled by [`Self::capture_roots`] and dropped by [`Self::clear`].
    roots: Vec<corders_group::Root>,
    roots_len: Option<usize>,
    roots_required: usize,
    capacity_charge: crate::raw_memory::Charge,
}

/// Orders-group theatre references per country slot the order-of-battle
/// walk is sized for up front. A sizing hint only: a larger roster overflows
/// once and grows, as every capture does.
const OOB_THEATRE_REFERENCES_PER_COUNTRY: usize = 4;

/// The order-of-battle theatre scratch for the largest observed country
/// registry ([`crate::country_scale`]), never below the vanilla-sized floor.
fn oob_theatre_rows() -> usize {
    crate::country_scale::census_rows(0)
        .saturating_mul(OOB_THEATRE_REFERENCES_PER_COUNTRY)
        .max(2048)
}

impl NavalJob {
    /// Callback-owned values from the one shared complete wing census.
    pub(crate) fn install_wings(&mut self, job: &super::air::AirWingJob) -> u64 {
        self.theatres_active = job.theatres_active;
        if !job.theatres_active {
            self.wing_sources.clear();
            self.wing_outcome = Some(SKIPPED);
            self.wing_required = 0;
            return 0;
        }
        let begin = qpc();
        self.wing_sources.clear();
        let (source, rows) = job.memberships();
        // Capacity demand is independent of failure: a bad tail leaf in the
        // complete shared census must not hide a smaller consumer's shortage.
        self.wing_required = match source {
            Outcome::Rows { len } => len,
            Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
            Outcome::Unavailable { .. } => rows.len().saturating_add(1),
        };
        let mut failure = None;
        let mut omitted = 0_u32;
        for row in rows {
            if let Some(reason) = row.error {
                failure.get_or_insert(reason);
            } else if self.wing_sources.len() < self.wing_sources.capacity() {
                self.wing_sources.push(row.value);
            } else {
                omitted = omitted.saturating_add(1);
            }
        }
        self.wing_outcome = Some(match source {
            Outcome::Rows { .. } => match failure {
                Some(reason) => Outcome::Unavailable { reason },
                None if omitted != 0 => Outcome::Truncated {
                    len: self.wing_sources.len(),
                    unread_rows: omitted,
                },
                None => Outcome::Rows {
                    len: self.wing_sources.len(),
                },
            },
            Outcome::Truncated { .. } => {
                let len = self.wing_sources.len();
                match u32::try_from(self.wing_required.saturating_sub(len)) {
                    Ok(unread_rows) => Outcome::Truncated { len, unread_rows },
                    Err(_) => Outcome::Unavailable {
                        reason: "air-wing omitted source count overflow",
                    },
                }
            }
            other => other,
        });
        elapsed(begin)
    }

    pub fn new() -> Self {
        let mut job = Self {
            log: RawLog::new(1 << 17, 4 << 20),
            game_state: 0,
            theatres_active: false,
            wing_sources: Vec::with_capacity(4096),
            wing_outcome: None,
            wing_required: 0,
            oob_theatres: Vec::with_capacity(oob_theatre_rows()),
            oob_theatre_positions: Vec::with_capacity(oob_theatre_rows()),
            oob_theatres_required: 0,
            roster: engine::ccountry::RosterSnapshot::with_capacity(
                crate::country_scale::census_rows(1024),
            ),
            roots: Vec::with_capacity(oob_theatre_rows()),
            roots_len: None,
            roots_required: 0,
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.theatres_active = false;
        self.wing_sources.clear();
        self.wing_outcome = None;
        self.wing_required = 0;
        self.oob_theatres.clear();
        self.oob_theatre_positions.clear();
        self.oob_theatres_required = 0;
        self.roster.clear();
        self.roots.clear();
        self.roots_len = None;
        self.roots_required = 0;
    }

    /// On the callback, after its orders-group registry census and before
    /// this job is queued: keeps a complete census for this pass's
    /// order-of-battle copy and decode. Any other outcome, or a census larger
    /// than the snapshot, keeps none, and the copy reads the registry as
    /// before; the demand grows the snapshot off the game thread. Never
    /// allocates.
    pub(crate) fn capture_roots(&mut self, roots: &[corders_group::Root], outcome: Outcome) -> u64 {
        let begin = qpc();
        self.roots.clear();
        self.roots_len = None;
        let required = match outcome {
            Outcome::Rows { len } => len,
            Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
            Outcome::Unavailable { .. } => 0,
        };
        self.roots_required = self.roots_required.max(required);
        if let Outcome::Rows { len } = outcome
            && let Some(roots) = roots.get(..len)
            && len <= self.roots.capacity()
        {
            self.roots.extend_from_slice(roots);
            self.roots_len = Some(len);
        }
        elapsed(begin)
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
            || self.wing_required > self.wing_sources.capacity()
            || self.oob_theatres_required > self.oob_theatres.capacity()
            || self.roster.required() > self.roster.capacity()
            || self.roots_required > self.roots.capacity()
            || matches!(self.wing_outcome, Some(Outcome::Truncated { .. }))
    }

    pub fn grow(&mut self) {
        self.log.grow();
        let required = match self.wing_outcome {
            Some(Outcome::Truncated { len, unread_rows }) => self
                .wing_required
                .max(len.saturating_add(unread_rows as usize)),
            _ => self.wing_required,
        };
        if required > self.wing_sources.capacity() {
            let target = required.max(self.wing_sources.capacity().saturating_mul(2));
            self.wing_sources
                .reserve_exact(target.saturating_sub(self.wing_sources.len()));
        }
        if self.oob_theatres_required > self.oob_theatres.capacity() {
            let target = self
                .oob_theatres_required
                .max(self.oob_theatres.capacity().saturating_mul(2));
            self.oob_theatres
                .reserve_exact(target.saturating_sub(self.oob_theatres.len()));
            self.oob_theatre_positions
                .reserve_exact(target.saturating_sub(self.oob_theatre_positions.len()));
        }
        self.roster.reserve(self.roster.required());
        if self.roots_required > self.roots.capacity() {
            let target = self
                .roots_required
                .max(self.roots.capacity().saturating_mul(2));
            self.roots
                .reserve_exact(target.saturating_sub(self.roots.len()));
        }
        self.update_capacity_charge();
    }

    /// Off the game thread: size the order-of-battle theatre references for
    /// the largest observed country registry, so a large save's first hour does
    /// not overflow. Only grows.
    pub fn presize_countries(&mut self) {
        let target = oob_theatre_rows();
        let roster = crate::country_scale::census_rows(1024);
        if self.oob_theatres.capacity() >= target
            && self.oob_theatre_positions.capacity() >= target
            && self.roots.capacity() >= target
            && self.roster.capacity() >= roster
        {
            return;
        }
        self.roster.reserve(roster);
        self.roots
            .reserve_exact(target.saturating_sub(self.roots.len()));
        self.oob_theatres
            .reserve_exact(target.saturating_sub(self.oob_theatres.len()));
        self.oob_theatre_positions
            .reserve_exact(target.saturating_sub(self.oob_theatre_positions.len()));
        self.update_capacity_charge();
    }

    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            auxiliary_vectors: vector_bytes(&self.wing_sources)
                + vector_bytes(&self.oob_theatres)
                + vector_bytes(&self.oob_theatre_positions)
                + vector_bytes(&self.roots)
                + self.roster.allocated_bytes() as u64,
            ..Default::default()
        });
    }

    pub(crate) fn wing_limit(&self) -> usize {
        self.wing_sources.capacity()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct NavalCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; NAVAL_PACK_N],
    pub raw_bytes: [u64; NAVAL_PACK_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NavalCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

fn npool_u64(log: &RawLog<NAVAL_PACK_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn npool_u32(log: &RawLog<NAVAL_PACK_N>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

fn nspan(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

fn theatre_link(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
    mask: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = nspan(log, memory, links.data.checked_add(offset), 4, mask)?;
    let slot = npool_u32(log, start);
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

fn copy_naval_air_theatres(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine::Memory,
    theatres: Array,
    links: &Array,
) {
    for ordinal in 0..theatres.count {
        let entry_address = theatres
            .data
            .checked_add((ordinal as usize).saturating_mul(8));
        let theatre =
            match nspan(log, memory, entry_address, 8, AIR_DETAILS_MASK).and_then(|start| {
                usize::try_from(npool_u64(log, start))
                    .ok()
                    .filter(|&p| p != 0)
            }) {
                Some(theatre) => theatre,
                None => continue,
            };
        let _ = nspan(log, memory, Some(theatre), 16, AIR_DETAILS_MASK);
        let owner = nspan(
            log,
            memory,
            theatre.checked_add(cair_theatre::COUNTRY),
            4,
            AIR_DETAILS_MASK,
        );
        if let Some(owner) = owner {
            let owner = npool_u32(log, owner);
            if owner != 0 && owner < links.count {
                let address = (owner as usize)
                    .checked_mul(4)
                    .and_then(|offset| links.data.checked_add(offset));
                let _ = nspan(log, memory, address, 4, AIR_DETAILS_MASK);
            }
        }
        copy_theatre_name(
            log,
            memory,
            theatre.checked_add(cair_theatre::NAME),
            AIR_DETAILS_MASK,
        );
        let groups_header = match nspan(
            log,
            memory,
            theatre.checked_add(cair_theatre::GROUPS),
            16,
            AIR_DETAILS_MASK,
        ) {
            Some(start) => start,
            None => continue,
        };
        let mut header = [0; 16];
        header.copy_from_slice(&log.bytes[groups_header..groups_header + 16]);
        let groups = match Array::from_header(header, 8) {
            Some(groups) => groups,
            None => continue,
        };
        for group_ordinal in 0..groups.count {
            let entry_address = groups
                .data
                .checked_add((group_ordinal as usize).saturating_mul(8));
            let group =
                match nspan(log, memory, entry_address, 8, AIR_DETAILS_MASK).and_then(|start| {
                    usize::try_from(npool_u64(log, start))
                        .ok()
                        .filter(|&p| p != 0)
                }) {
                    Some(group) => group,
                    None => continue,
                };
            let _ = nspan(log, memory, Some(group), 16, AIR_DETAILS_MASK);
            let _ = nspan(
                log,
                memory,
                group.checked_add(cair_group::THEATRE_HANDLE),
                8,
                AIR_DETAILS_MASK,
            );
            copy_theatre_name(
                log,
                memory,
                group.checked_add(cair_group::NAME),
                AIR_DETAILS_MASK,
            );
            let wings_header = match nspan(
                log,
                memory,
                group.checked_add(cair_group::WINGS),
                16,
                AIR_DETAILS_MASK,
            ) {
                Some(start) => start,
                None => continue,
            };
            let mut header = [0; 16];
            header.copy_from_slice(&log.bytes[wings_header..wings_header + 16]);
            let wings = match Array::from_header(header, 8) {
                Some(wings) => wings,
                None => continue,
            };
            for wing_ordinal in 0..wings.count {
                let entry_address = wings
                    .data
                    .checked_add((wing_ordinal as usize).saturating_mul(8));
                let _ = nspan(log, memory, entry_address, 8, AIR_DETAILS_MASK);
                if log.overflowed {
                    return;
                }
            }
        }
    }
}

fn copy_naval_theatres(
    log: &mut RawLog<NAVAL_PACK_N>,
    roster_snapshot: &mut engine::ccountry::RosterSnapshot,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    details_active: bool,
) {
    use engine::{carmy_group, ccountry};
    let mask = if details_active {
        THEATRE_MASK
    } else {
        LAND_THEATRE_MASK
    };
    roster_snapshot.clear();
    // Reuse within the pass: the callback authenticated the complete country
    // census this hour before any job ran, and holds the game thread until
    // every job has finished, so authenticating each country again here would
    // read the same values. The copy then takes no census span, and the
    // snapshot hands the same roster to this pass's decode, whose country
    // reader resolves through it exactly as the typed walk does over the
    // callback's memory. Each pass clears the snapshot first; without a
    // roster, or room for it, the census is taken as before.
    if let Some(roster) = memory.authenticated_countries(base, game_state)
        && roster.slots().len() == roster.arrays()[0].count as usize
        && roster_snapshot.capture(roster, base, game_state)
    {
        let [countries, links] = roster.arrays();
        let slots = roster.slots();
        if links.count == 0 {
            return;
        }
        for slot in 1..countries.count {
            let Some(country) = slots[slot as usize].filter(|c| c.canonical_slot == slot) else {
                continue;
            };
            if !copy_country_theatres(
                log,
                memory,
                country.pointer,
                &links,
                Some(slots),
                details_active,
                mask,
            ) {
                return;
            }
        }
        return;
    }
    // Country census exactly like the fleet/wing censuses.
    let countries = match nspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRIES),
        16,
        mask,
    ) {
        Some(start) => start,
        None => return,
    };
    let links = match nspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRY_LINKS),
        16,
        mask,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries..countries + 16]);
    let countries_array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[links..links + 16]);
    let links_array = match Array::from_header(raw, 4) {
        Some(array) => array,
        None => return,
    };
    if countries_array.count < 2 || links_array.count == 0 {
        return;
    }
    let country_vtable = match base.checked_add(ccountry::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    for slot in 1..countries_array.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| countries_array.data.checked_add(o));
        let pointer = match nspan(log, memory, pointer_addr, 8, mask)
            .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0))
        {
            Some(pointer) => pointer,
            None => continue,
        };
        let vtable = match nspan(log, memory, Some(pointer), 8, mask) {
            Some(start) => start,
            None => continue,
        };
        if npool_u64(log, vtable) != country_vtable {
            return;
        }
        let id_start = match nspan(log, memory, pointer.checked_add(ccountry::ID), 4, mask) {
            Some(start) => start,
            None => continue,
        };
        let id = npool_u32(log, id_start);
        let canonical =
            match theatre_link(log, memory, &links_array, countries_array.count, id, mask) {
                Some(canonical) => canonical,
                None => return,
            };
        if theatre_link(log, memory, &links_array, countries_array.count, slot, mask)
            != Some(canonical)
        {
            return;
        }
        if theatre_link(
            log,
            memory,
            &links_array,
            countries_array.count,
            canonical,
            mask,
        ) != Some(canonical)
        {
            return;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|o| countries_array.data.checked_add(o));
        let canonical_pointer = match nspan(log, memory, canonical_addr, 8, mask) {
            Some(start) => start,
            None => return,
        };
        if npool_u64(log, canonical_pointer) != pointer as u64 {
            return;
        }
        if !copy_country_theatres(log, memory, pointer, &links_array, None, details_active, mask) {
            return;
        }
    }
    let _ = carmy_group::VTABLE_RVA;
}

/// One authenticated country's theatre takes: the land theatre headers every
/// hour, and on the daily slot the land details and air theatres. False when
/// the log overflowed and the pass must stop.
///
/// `roster` is the callback's authenticated country roster when the pass
/// resolves countries through it; a theatre owner's link entry is then taken
/// only where the replay's country reader reads it: for an id whose roster
/// slot is null.
#[allow(clippy::too_many_arguments)]
fn copy_country_theatres(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine::Memory,
    pointer: usize,
    links_array: &Array,
    roster: Option<&[Option<engine::ccountry::Country>]>,
    details_active: bool,
    mask: u32,
) -> bool {
    use engine::{ccountry, ctheater_group};
    // Land theatre headers refresh hourly; the five nested/detail tables
    // refresh only on their shared daily slot.
    for theatres_off in [ccountry::THEATRES, ccountry::AIR_THEATRES] {
        if !details_active && theatres_off == ccountry::AIR_THEATRES {
            break;
        }
        let header_mask = if theatres_off == ccountry::THEATRES {
            mask
        } else {
            AIR_DETAILS_MASK
        };
        let theatres_header = match nspan(
            log,
            memory,
            pointer.checked_add(theatres_off),
            16,
            header_mask,
        ) {
            Some(start) => start,
            None => continue,
        };
        let mut header = [0u8; 16];
        header.copy_from_slice(&log.bytes[theatres_header..theatres_header + 16]);
        let theatres = match Array::from_header(header, 8) {
            Some(array) => array,
            None => continue,
        };
        if theatres_off == ccountry::AIR_THEATRES {
            copy_naval_air_theatres(log, memory, theatres, links_array);
            continue;
        }
        for ordinal in 0..theatres.count {
            let entry_addr = theatres
                .data
                .checked_add((ordinal as usize).saturating_mul(8));
            let theatre = match nspan(log, memory, entry_addr, 8, header_mask)
                .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0))
            {
                Some(theatre) => theatre,
                None => continue,
            };
            // Theatre identity (vtable + handle 16), owner and volunteers in
            // one guarded copy, each field its own exact span as the typed
            // walk's gathered record reads them, stopping at the first
            // failure as its per-field reads do; then the owner link resolve
            // and the group vector.
            let [(_, identity), (owner, owner_len), (volunteers, volunteers_len)] =
                ctheatre::HEADER_SPANS;
            let owner_start = match (
                theatre.checked_add(owner),
                theatre.checked_add(volunteers),
            ) {
                (Some(owner), Some(volunteers)) => log
                    .take_scattered(
                        memory,
                        [theatre, owner, volunteers],
                        [identity, owner_len, volunteers_len],
                        [header_mask; 3],
                    )
                    .filter(|&(_, completed)| completed >= 2)
                    .map(|(starts, _)| starts[1]),
                _ => {
                    let _ = nspan(log, memory, Some(theatre), identity, header_mask);
                    nspan(
                        log,
                        memory,
                        theatre.checked_add(owner),
                        owner_len,
                        header_mask,
                    )
                }
            };
            if let Some(owner_start) = owner_start {
                let owner = npool_u32(log, owner_start);
                // The roster resolves an id whose slot it authenticated
                // without reading its link.
                let resolved = roster
                    .and_then(|slots| slots.get(owner as usize))
                    .is_some_and(Option::is_some);
                if owner != 0 && owner < links_array.count && !resolved {
                    let offset = (owner as usize).checked_mul(4);
                    let _ = nspan(
                        log,
                        memory,
                        offset.and_then(|o| links_array.data.checked_add(o)),
                        4,
                        header_mask,
                    );
                }
            }
            if !details_active {
                continue;
            }
            let mask = LAND_DETAILS_MASK;
            let groups_header = match nspan(
                log,
                memory,
                theatre.checked_add(ctheatre::THEATER_GROUP_DATA),
                16,
                mask,
            ) {
                Some(start) => start,
                None => continue,
            };
            let mut groups_raw = [0u8; 16];
            groups_raw.copy_from_slice(&log.bytes[groups_header..groups_header + 16]);
            let groups = match Array::from_header(groups_raw, 8) {
                Some(array) => array,
                None => continue,
            };
            for group_ordinal in 0..groups.count {
                let group_addr = groups
                    .data
                    .checked_add((group_ordinal as usize).saturating_mul(8));
                let group = match nspan(log, memory, group_addr, 8, mask)
                    .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0))
                {
                    Some(group) => group,
                    None => continue,
                };
                // Group identity (vtable + handle 16), theatre backlink,
                // name string, priority.
                let _ = nspan(log, memory, Some(group), 16, mask);
                let _ = nspan(
                    log,
                    memory,
                    group.checked_add(ctheater_group::THEATRE),
                    8,
                    mask,
                );
                // Name string header + payload + terminator (theatre arena).
                copy_theatre_name(log, memory, group.checked_add(ctheater_group::NAME), mask);
                let _ = nspan(
                    log,
                    memory,
                    group.checked_add(ctheater_group::PRIORITY),
                    4,
                    mask,
                );
                let members_header = match nspan(
                    log,
                    memory,
                    group.checked_add(ctheater_group::ORDERS_GROUP_DATA),
                    16,
                    mask,
                ) {
                    Some(start) => start,
                    None => continue,
                };
                let mut members_raw = [0u8; 16];
                members_raw.copy_from_slice(&log.bytes[members_header..members_header + 16]);
                let members = match Array::from_header(members_raw, 8) {
                    Some(array) => array,
                    None => continue,
                };
                for member_ordinal in 0..members.count {
                    let member_addr = members
                        .data
                        .checked_add((member_ordinal as usize).saturating_mul(8));
                    let member = match nspan(log, memory, member_addr, 8, mask).and_then(|s| {
                        usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0)
                    }) {
                        Some(member) => member,
                        None => continue,
                    };
                    // Member vtable (8) for the normal/army-group check plus
                    // identity (vtable + handle 16) plus parents. Both
                    // lengths are recorded because the walk requests both
                    // exact spans (exact-match serving, no sub-ranges).
                    let _ = nspan(log, memory, Some(member), 8, mask);
                    let _ = nspan(log, memory, Some(member), 16, mask);
                    let _ = nspan(
                        log,
                        memory,
                        member.checked_add(corders_group::THEATER_GROUP),
                        8,
                        mask,
                    );
                    let _ = nspan(
                        log,
                        memory,
                        member.checked_add(corders_group::THEATRE),
                        8,
                        mask,
                    );
                    if log.overflowed {
                        return false;
                    }
                }
            }
        }
    }
    true
}

fn copy_theatre_name(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine::Memory,
    name_address: Option<usize>,
    mask: u32,
) {
    use engine::msvc_string;
    let header_addr = match name_address {
        Some(addr) => addr,
        None => return,
    };
    let header_start = match nspan(log, memory, Some(header_addr), 32, mask) {
        Some(start) => start,
        None => return,
    };
    let mut header = [0u8; 32];
    header.copy_from_slice(&log.bytes[header_start..header_start + 32]);
    let len = match usize::try_from(u64::from_le_bytes(header[16..24].try_into().unwrap())) {
        Ok(len) => len,
        Err(_) => return,
    };
    let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
    if len as u64 > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return;
    }
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        header_addr
    } else {
        match usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&s| s != 0)
        {
            Some(source) => source,
            None => return,
        }
    };
    let Some(end) = source.checked_add(len) else {
        return;
    };
    if nspan(log, memory, Some(end), 1, mask).is_none() {
        return;
    }
    if len != 0 {
        let _ = nspan(log, memory, Some(source), len, mask);
    }
}

/// A root's own one-entry order-of-battle spans, as `(offset, len)`, in the
/// order the typed walk reads them: army-group flag, motorization, the five
/// references (theatre, theater group, parent, leader, HQ provider) and the
/// member vector header.
const OOB_ROOT_SPANS: [(usize, usize); 8] = [
    (corders_group::IS_ARMY_GROUP, 1),
    (corders_group::MOTORIZATION_LEVEL, 1),
    (corders_group::THEATRE, 8),
    (corders_group::THEATER_GROUP, 8),
    (corders_group::PARENT, 8),
    (corders_group::LEADER, 8),
    (corders_group::HQ_PROVIDER, 8),
    (corders_group::MEMBER_DATA, 16),
];

/// Logs `pointer`'s [`OOB_ROOT_SPANS`] with one guarded copy, each field its
/// own exact span with its own success or failure, as the per-field takes
/// log them. Those takes are independent, so after a failed field each later
/// one is taken on its own, as before. Returns each field's log start.
fn copy_oob_root_spans(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine::Memory,
    pointer: usize,
) -> [Option<usize>; OOB_ROOT_SPANS.len()] {
    let mut starts = [None; OOB_ROOT_SPANS.len()];
    let mut next = 0;
    let addresses = OOB_ROOT_SPANS.map(|(offset, _)| pointer.checked_add(offset));
    #[cfg(test)]
    let gather = !naval_test_kit::OOB_PER_FIELD.get();
    #[cfg(not(test))]
    let gather = true;
    if gather
        && addresses.iter().all(Option::is_some)
        && let Some((taken, completed)) = log.take_scattered(
            memory,
            addresses.map(|address| address.unwrap_or(0)),
            OOB_ROOT_SPANS.map(|(_, len)| len),
            [OOB_MASK; OOB_ROOT_SPANS.len()],
        )
    {
        for (start, taken) in starts.iter_mut().zip(&taken[..completed]) {
            *start = Some(*taken);
        }
        // The failed field is logged as the failure its own take logs.
        next = (completed + 1).min(OOB_ROOT_SPANS.len());
    }
    for (index, (offset, len)) in OOB_ROOT_SPANS.into_iter().enumerate().skip(next) {
        starts[index] = nspan(log, memory, pointer.checked_add(offset), len, OOB_MASK);
    }
    starts
}

/// One authenticated root's order-of-battle spans, from its army-group flag
/// on; the registry census has read and authenticated its identity and
/// country. Returns whether the copy goes on: false when the log overflowed.
///
/// The root's own one-entry spans are logged first, in one guarded copy
/// ([`copy_oob_root_spans`]), then what they reference. The log holds the
/// same exact spans, bytes and failures as when each field was taken just
/// before its targets: the replay serves each read from its exact key, and
/// every key's occurrences keep their order, because within one pass the
/// game thread is held and two occurrences of a key hold the same bytes.
/// Only a log overflow can come at a different field; the copy then stops,
/// as before.
fn copy_oob_root(
    job: &mut NavalJob,
    memory: &impl engine::Memory,
    base: usize,
    pointer: usize,
) -> bool {
    let log = &mut job.log;
    // OOB identity fields: army-group flag, motorization, country
    // (already above), theatre/theater-group/parent/leader/provider
    // references with target authentication for non-null pointers.
    let own = copy_oob_root_spans(log, memory, pointer);
    let is_army_group = own[0].map(|start| log.bytes[start]);
    for (index, offset) in [
        corders_group::THEATRE,
        corders_group::THEATER_GROUP,
        corders_group::PARENT,
        corders_group::LEADER,
        corders_group::HQ_PROVIDER,
    ]
    .into_iter()
    .enumerate()
    {
        let target = own[index + 2].and_then(|s| usize::try_from(npool_u64(log, s)).ok());
        if let Some(target) = target.filter(|&p| p != 0) {
            // References use a vtable + handle identity, except the
            // HQ provider: it is an embedded member whose host is
            // authenticated below, not a referenced object itself.
            if offset != corders_group::HQ_PROVIDER {
                let identity = nspan(log, memory, Some(target), 16, OOB_MASK);
                if offset == corders_group::THEATRE
                    && identity.is_some_and(|start| {
                        base.checked_add(ctheatre::VTABLE_RVA)
                            .is_some_and(|expected| {
                                npool_u64(log, start) == expected as u64
                            })
                            && npool_u64(log, start + 8) as u32 == ctheatre::REFERENCE_TYPE
                    })
                {
                    job.oob_theatres_required = job.oob_theatres.len().saturating_add(1);
                    if job.oob_theatres.len() == job.oob_theatres.capacity()
                        || job.oob_theatres.len() == job.oob_theatre_positions.capacity()
                    {
                        log.overflowed = true;
                        return false;
                    }
                    job.oob_theatres.push(target);
                }
            }
            if offset == corders_group::PARENT {
                let _ = nspan(
                    log,
                    memory,
                    target.checked_add(corders_group::IS_ARMY_GROUP),
                    1,
                    OOB_MASK,
                );
            }
            if offset == corders_group::HQ_PROVIDER {
                // Provider is a member pointer: host identity span
                // plus group backlink, like member_unit.
                if let Some(unit) =
                    target.checked_sub(engine::corders_group_member::HOST_OFFSET)
                {
                    let _ = nspan(log, memory, Some(unit), 32, OOB_MASK);
                }
                let _ = nspan(
                    log,
                    memory,
                    target.checked_add(engine::corders_group_member::GROUP),
                    8,
                    OOB_MASK,
                );
            }
        }
    }
    // Member vector with per-member host identity (32-byte header)
    // and group backlink, exactly like `member_unit`.
    let Some(members_header) = own[7] else {
        return true;
    };
    let mut members_raw = [0u8; 16];
    members_raw.copy_from_slice(&log.bytes[members_header..members_header + 16]);
    let members = match Array::from_header(members_raw, 8) {
        Some(array) => array,
        None => return true,
    };
    for index in 0..members.count {
        let entry_addr = members.data.checked_add((index as usize).saturating_mul(8));
        let member = match nspan(log, memory, entry_addr, 8, OOB_MASK)
            .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0))
        {
            Some(member) => member,
            None => continue,
        };
        // Host identity span (vtable/class/secondary/handle 32 bytes)
        // at the embedded base, plus group backlink at the member.
        let unit = match member.checked_sub(engine::corders_group_member::HOST_OFFSET) {
            Some(unit) => unit,
            None => continue,
        };
        let _ = nspan(log, memory, Some(unit), 32, OOB_MASK);
        let _ = nspan(
            log,
            memory,
            member.checked_add(engine::corders_group_member::GROUP),
            8,
            OOB_MASK,
        );
        if log.overflowed {
            return false;
        }
    }
    // The typed walk accepts any nonzero discriminator. The child
    // header starts beyond an ordinary COrdersGroup's allocation,
    // so neither it nor its entries may be read for a zero/failed flag.
    if is_army_group.is_some_and(|flag| flag != 0) {
        let children_header = match nspan(
            log,
            memory,
            pointer.checked_add(engine::carmy_group::MEMBER_DATA),
            16,
            OOB_MASK,
        ) {
            Some(start) => start,
            None => return true,
        };
        let mut children_raw = [0u8; 16];
        children_raw.copy_from_slice(&log.bytes[children_header..children_header + 16]);
        if let Some(children) = Array::from_header(children_raw, 8) {
            for index in 0..children.count {
                let entry_addr = children
                    .data
                    .checked_add((index as usize).saturating_mul(8));
                let child =
                    match nspan(log, memory, entry_addr, 8, OOB_MASK).and_then(|s| {
                        usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0)
                    }) {
                        Some(child) => child,
                        None => continue,
                    };
                let _ = nspan(log, memory, Some(child), 16, OOB_MASK);
                if log.overflowed {
                    return false;
                }
            }
        }
    }
    // Order-membership reverse census for this root: theatre-group
    // pointer plus target identity when non-zero.
    if job.theatres_active {
        let _reads = read_accounting::table(TableId::LandTheatreMembers);
        let theater_group = nspan(
            log,
            memory,
            pointer.checked_add(corders_group::THEATER_GROUP),
            8,
            LAND_MEMBERS_MASK,
        )
        .and_then(|s| usize::try_from(npool_u64(log, s)).ok());
        if let Some(theater_group) = theater_group.filter(|&p| p != 0) {
            let _ = nspan(log, memory, Some(theater_group), 16, LAND_MEMBERS_MASK);
        }
    }
    if log.overflowed {
        return false;
    }
    true
}

/// The order-of-battle roots from the orders-group registry itself: its
/// header, buckets and each root's identity and country, then the root's own
/// spans. Returns whether the copy goes on to stage the rosters.
fn copy_oob_registry(job: &mut NavalJob, memory: &impl engine::Memory, base: usize) -> bool {
    use engine::{creference_registry as registry, reference_bucket as bucket};
    let log = &mut job.log;
    let table = match nspan(
        log,
        memory,
        base.checked_add(registry::LOW_ROOTS_RVA)
            .and_then(|p| p.checked_add(corders_group::REFERENCE_TYPE as usize * 8)),
        8,
        OOB_MASK,
    )
    .and_then(|start| {
        usize::try_from(npool_u64(log, start))
            .ok()
            .filter(|&p| p != 0)
    }) {
        Some(table) => table,
        None => return false,
    };
    let header = match nspan(
        log,
        memory,
        table.checked_add(registry::ENTRIES),
        17,
        OOB_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let mut header_bytes = [0u8; 17];
    header_bytes.copy_from_slice(&log.bytes[header..header + 17]);
    let entries = match usize::try_from(u64::from_le_bytes(header_bytes[0..8].try_into().unwrap()))
        .ok()
        .filter(|&e| e != 0)
    {
        Some(entries) => entries,
        None => return false,
    };
    let mask = u32::from_le_bytes(header_bytes[12..16].try_into().unwrap());
    let distance = header_bytes[16];
    let capacity = match mask.checked_add(1) {
        Some(capacity) => capacity,
        None => return false,
    };
    let terminal = match (capacity as usize).checked_add(distance as usize) {
        Some(terminal) => terminal,
        None => return false,
    };
    let terminal_addr = entries
        .checked_add(terminal * bucket::STRIDE)
        .and_then(|p| p.checked_add(bucket::DISTANCE));
    let _ = nspan(log, memory, terminal_addr, 1, OOB_MASK);
    let group_vtable = match base.checked_add(corders_group::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return false,
    };
    let army_group_vtable = match base.checked_add(engine::carmy_group::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return false,
    };
    let mut first: u32 = 0;
    let terminal_u32 = terminal.min(1 << 20) as u32;
    while first < terminal_u32 {
        let log = &mut job.log;
        let slots = (terminal_u32 - first).min(64) as usize;
        let address = match entries.checked_add(first as usize * bucket::STRIDE) {
            Some(address) => address,
            None => break,
        };
        let batch_start = match nspan(log, memory, Some(address), slots * bucket::STRIDE, OOB_MASK)
        {
            Some(start) => start,
            None => break,
        };
        for slot_in_batch in 0..slots {
            let log = &mut job.log;
            let bucket_start = batch_start + slot_in_batch * bucket::STRIDE;
            let distance = log.bytes[bucket_start + bucket::DISTANCE];
            if distance == 0 {
                continue;
            }
            let handle = u64::from_le_bytes(
                log.bytes[bucket_start + bucket::TYPE..bucket_start + bucket::OBJECT]
                    .try_into()
                    .unwrap(),
            );
            if handle as u32 != corders_group::REFERENCE_TYPE {
                continue;
            }
            let pointer = match usize::try_from(u64::from_le_bytes(
                log.bytes[bucket_start + bucket::OBJECT..bucket_start + bucket::STRIDE]
                    .try_into()
                    .unwrap(),
            ))
            .ok()
            .filter(|&p| p != 0)
            {
                Some(pointer) => pointer,
                None => continue,
            };
            let identity = match nspan(log, memory, Some(pointer), 16, OOB_MASK) {
                Some(start) => start,
                None => continue,
            };
            if npool_u64(log, identity) != group_vtable
                && npool_u64(log, identity) != army_group_vtable
            {
                continue;
            }
            if npool_u64(log, identity + 8) != handle {
                continue;
            }
            let country_start = match nspan(
                log,
                memory,
                pointer.checked_add(corders_group::COUNTRY),
                4,
                OOB_MASK,
            ) {
                Some(start) => start,
                None => continue,
            };
            let _country = npool_u32(log, country_start);
            if !copy_oob_root(job, memory, base, pointer) {
                return false;
            }
        }
        first += slots as u32;
    }
    true
}

fn copy_naval_oob(job: &mut NavalJob, memory: &impl engine::Memory, base: usize) {
    job.oob_theatres.clear();
    job.oob_theatre_positions.clear();
    job.oob_theatres_required = 0;
    match job.roots_len {
        // Reuse within the pass: the callback took this registry census from
        // this memory before queueing this job and holds the game thread
        // until every job has finished, so the registry, each root's
        // identity and its country read again would give the same roots.
        // `capture_roots` keeps only a complete census, and `clear` drops it
        // with the pass; this pass's decode takes the same roots as its
        // census instead of replaying the registry, which is not recorded.
        Some(len) => {
            for index in 0..len {
                let pointer = job.roots[index].pointer();
                if !copy_oob_root(job, memory, base, pointer) {
                    return;
                }
            }
        }
        None => {
            if !copy_oob_registry(job, memory, base) {
                return;
            }
        }
    }
    let log = &mut job.log;
    dedup_first_seen(&mut job.oob_theatres, &mut job.oob_theatre_positions);
    // The engine matches these pointers against the complete authenticated
    // root census. Stage each native roster once, including an unused subtype,
    // and retain failures as observations rather than retrying for later roots.
    // This memo belongs only to this OOB pass: identical addresses in another
    // source family never stand in for this roster's own acquisition.
    for &theatre in &job.oob_theatres {
        for offset in [ctheatre::ORDERS_GROUP_DATA, ctheatre::ARMY_GROUP_DATA] {
            let Some(start) = nspan(log, memory, theatre.checked_add(offset), 16, OOB_MASK) else {
                return;
            };
            let mut header = [0; 16];
            header.copy_from_slice(&log.bytes[start..start + 16]);
            let Some(roster) = Array::from_header(header, 8) else {
                return;
            };
            // Up to 64 entries per guarded copy, each recorded as the exact
            // span its per-entry read would record, failure included: the
            // replay walks the theatres in this same first-seen root order
            // and reads each roster entry by entry, so every read is served
            // from the next recorded span.
            for first in (0..roster.count).step_by(64) {
                let count = (roster.count - first).min(64) as usize;
                let Some(address) = roster.element(first, 8) else {
                    return;
                };
                match log.take_array_entries(memory, address, 8, count, OOB_MASK) {
                    Some((_, completed)) if completed == count => {}
                    _ => return,
                }
            }
            if log.overflowed {
                return;
            }
        }
    }
}

/// Keeps the first occurrence of each value in its original order, in
/// O(n log n): positions sorted by `(value, position)` keep their first
/// position per value, then return to position order. A sorted insert per
/// value would move O(n) entries each. `positions` needs `values.len()`
/// capacity to stay allocation-free.
fn dedup_first_seen(values: &mut Vec<usize>, positions: &mut Vec<usize>) {
    positions.clear();
    positions.extend(0..values.len());
    positions.sort_unstable_by_key(|&position| (values[position], position));
    positions.dedup_by_key(|position| values[*position]);
    positions.sort_unstable();
    // Kept positions ascend, so each is at or after its destination.
    for (kept, &position) in positions.iter().enumerate() {
        values[kept] = values[position];
    }
    values.truncate(positions.len());
}

fn copy_naval_pass(
    job: &mut NavalJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; NAVAL_PACK_N],
) {
    // Shared wing values were installed before this independent raw phase.
    job.log.clear();
    job.game_state = game_state;
    let details_active = job.theatres_active;
    let started = qpc();
    read_accounting::with_table(TableId::LandTheatres, || {
        copy_naval_theatres(
            &mut job.log,
            &mut job.roster,
            memory,
            base,
            game_state,
            details_active,
        )
    });
    ticks[0] += qpc().saturating_sub(started).max(0) as u64;
    if job.log.overflowed {
        return;
    }
    let started = qpc();
    read_accounting::with_table(TableId::OrdersGroups, || copy_naval_oob(job, memory, base));
    ticks[6] += qpc().saturating_sub(started).max(0) as u64;
}

pub(crate) fn copy_naval_refresh(
    memory: &impl Memory,
    ctx: NavalCopyContext,
    job: &mut NavalJob,
) -> NavalCopyReport {
    job.theatres_active =
        replay::cadence::should_walk(TableId::LandTheatreGroups, ctx.hour, ctx.force_full);
    if job.theatres_active && job.wing_outcome.is_none() {
        return NavalCopyReport::default();
    }
    let mut ticks = [0u64; NAVAL_PACK_N];
    copy_naval_pass(job, memory, ctx.base, ctx.game_state, &mut ticks);
    NavalCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: ticks,
        raw_bytes: job.log.raw_bytes,
    }
}

impl Captured {
    /// Finish an accepted theatre/OOB acquisition failure after independent
    /// capture. The separately acquired wing census and other naval families
    /// retain their outcomes; no raw payload or typed retry may follow this.
    pub(crate) fn fail_naval_capture(&mut self, reason: &'static str) {
        macro_rules! reject {
            ($($field:ident = $slot:literal),+ $(,)?) => {
                $({
                    self.$field.outcome = failed(
                        $slot >= 6 || ($slot == 0 && self.active[4]) || self.theatres_active(),
                        reason,
                    );
                })+
            };
        }
        reject!(
            land = 0,
            land_groups = 1,
            land_members = 2,
            air = 3,
            air_groups = 4,
            air_members = 5,
            groups = 6,
            group_members = 7,
            group_children = 8,
        );
        self.theatre_names.reset();
        self.raw_mode = false;
    }
}

/// A recorded log replayed with the authenticated country roster its copy
/// resolved countries through; every read still goes to the log.
struct RosterReplay<'a, M> {
    inner: &'a M,
    roster: Option<(usize, usize, engine::ccountry::AuthenticatedRoster<'a>)>,
}

impl<M: Memory> Memory for RosterReplay<'_, M> {
    #[inline(always)]
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.inner.copy(address, destination)
    }

    fn immutable_view(
        &self,
        address: usize,
        len: usize,
    ) -> Option<engine::memory::SliceMemory<'_>> {
        self.inner.immutable_view(address, len)
    }

    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine::ccountry::AuthenticatedRoster<'_>> {
        self.roster
            .filter(|(module, state, _)| (*module, *state) == (base, game_state))
            .map(|(_, _, roster)| roster)
    }
}

pub(crate) fn decode_naval_payload(
    captured: &mut Captured,
    base: usize,
    job: &NavalJob,
    decode_ticks: &mut [u64; NAVAL_PACK_N],
) {
    captured.active[4] = true;
    captured.active[5..10].fill(job.theatres_active);
    let recorded = crate::raw::Recorded::build(&job.log);
    let started = qpc();
    // Order roots replay (worker-side, grows until the registry fits so the
    // census never truncates here; OOB row truncation is handled by the
    // pooled buffers through set_order_sources below).
    let mut walked: Vec<corders_group::Root> = Vec::new();
    let (roots, roots_outcome) = match job.roots_len {
        // The copy walked the callback's complete registry census of this
        // pass instead of recording the registry; that census is what this
        // replay of the registry would give. Never another pass's: the job
        // clears it before it is reused.
        Some(len) => (&job.roots[..len], Outcome::Rows { len }),
        None => {
            let outcome = loop {
                let outcome = corders_group::walk_roots(&recorded, base, &mut walked);
                match outcome {
                    Outcome::Truncated { .. } => {
                        walked.resize(walked.len() * 2 + 16, corders_group::Root::default());
                        continue;
                    }
                    _ => break outcome,
                }
            };
            (&walked[..], outcome)
        }
    };
    // Decoding runs off the game thread. Keep the scratch census complete even
    // when the independent group-row output intentionally truncates.
    if let Outcome::Rows { len } = roots_outcome {
        captured.roster_scratch.required = captured.roster_scratch.required.max(len);
        crate::table_capture::grow_vec(&mut captured.roster_scratch.first, len);
    }
    // Order sources + OOB through the existing setters (identical rows,
    // outcomes and ticks to the game thread's source callbacks).
    captured.set_order_sources(&recorded, base, roots, roots_outcome);
    // The hourly header walk uses the same country and owner checks as the
    // daily detail walk, but never requires wing membership or nested arenas.
    if !job.theatres_active {
        captured.skip_theatre_details();
    }
    // The copy resolved countries through the callback's roster when it had
    // one, so the replay does too: this pass's snapshot, never another's.
    let theatre_source = RosterReplay {
        inner: &recorded,
        roster: job
            .roster
            .roster(base, job.game_state)
            .map(|roster| (base, job.game_state, roster)),
    };
    let first = ctheatre::walk(
        &theatre_source,
        base,
        job.game_state,
        job.theatres_active,
        ctheatre::Buffers {
            land: &mut captured.land.rows,
            land_groups: &mut captured.land_groups.rows,
            land_members: &mut captured.land_members.rows,
            air: &mut captured.air.rows,
            air_groups: &mut captured.air_groups.rows,
            air_members: &mut captured.air_members.rows,
            names: &mut captured.theatre_names.first,
        },
    );
    captured.land.outcome = bounded(first.land, captured.land.rows.len());
    if job.theatres_active {
        captured.land_groups.outcome = bounded(first.land_groups, captured.land_groups.rows.len());
        captured.land_members.outcome =
            bounded(first.land_members, captured.land_members.rows.len());
        captured.air.outcome = bounded(first.air, captured.air.rows.len());
        captured.air_groups.outcome = bounded(first.air_groups, captured.air_groups.rows.len());
        captured.air_members.outcome = bounded(first.air_members, captured.air_members.rows.len());
    }
    captured.theatre_names.len = first.names_len;
    captured.theatre_names.required = first.names_required;
    if !job.theatres_active {
        captured.wing_sources.outcome = SKIPPED;
        captured.wings_finished = true;
        decode_ticks[0] += qpc().saturating_sub(started).max(0) as u64;
        captured.raw_mode = false;
        return;
    }
    // Preserve the complete authoritative reverse census without attempting
    // an unrelated full wing decode from an incomplete mirror.
    let source_len = job.wing_sources.len();
    let copied = source_len.min(captured.wing_sources.rows.len());
    captured.wing_sources.rows[..copied].copy_from_slice(&job.wing_sources[..copied]);
    captured.wing_sources.outcome = match job.wing_outcome {
        Some(Outcome::Rows { .. } | Outcome::Truncated { .. }) => {
            let missing = match job.wing_outcome {
                Some(Outcome::Truncated { unread_rows, .. }) => unread_rows,
                _ => 0,
            };
            match u32::try_from(source_len - copied)
                .ok()
                .and_then(|omitted| missing.checked_add(omitted))
            {
                Some(0) => Outcome::Rows { len: copied },
                Some(unread_rows) => Outcome::Truncated {
                    len: copied,
                    unread_rows,
                },
                None => Outcome::Unavailable {
                    reason: "air-wing omitted source count overflow",
                },
            }
        }
        Some(outcome) => outcome,
        None => source_unavailable(),
    };
    captured.wings_finished = true;
    decode_ticks[0] += qpc().saturating_sub(started).max(0) as u64;
    captured.raw_mode = false;
}

// ===========================================================================
// Hourly supremacy split (NavalSupremacy).
//
// The game thread copies raw bytes only: the strategic-region roster, per
// region identity with its dominance container, and every dominance value
// vector. No row is decoded on the game thread. The single FIFO decode
// worker replays `cnaval_region_dominance::walk` against the recorded spans,
// so rows, ordinals, skips, truncation counts and failure reasons are
// identical by construction (see `crate::raw` for the mirror rules).
//
// The value vectors are cap-dependent (the typed walk reads a
// capacity-truncated prefix in 32-entry chunks), so they are bulk spans:
// the mirror takes each whole value vector once and the replay serves any
// enclosed chunk from it. Every fault fails the whole table in the typed
// walk, so bulk granularity is exact here: rows never publish past a fault.
// ===========================================================================

/// Hourly payload slots in calibration/report order: [supremacy].
pub(crate) const SUPREMACY_N: usize = 1;
const SUPREMACY_MASK: u32 = 0x01;

/// Chunking of the typed walk's value reads (informational: bulk spans
/// serve any enclosed chunk regardless of chunking).
const SUPREMACY_MIRROR_BATCH: usize = 32;

/// One hour of raw supremacy payload. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode
/// side only (see `crate::decode::recycle_supremacy_job`).
#[derive(Debug, Default)]
pub(crate) struct SupremacyJob {
    pub log: RawLog<SUPREMACY_N>,
    pub game_state: usize,
    active: bool,
}

impl SupremacyJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is 49 supremacy rows (appendix
            // census); both pools start above that so steady state never
            // overflows (see the worst-case caps test).
            log: RawLog::new(1 << 12, 1 << 20),
            game_state: 0,
            active: false,
        }
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.active = false;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SupremacyCopyReport {
    /// False when staging overflowed: the caller falls back to the typed
    /// walk (identical rows through the existing path) and enqueues no
    /// payload.
    pub raw_ok: bool,
    pub copy_ticks: [u64; SUPREMACY_N],
    pub raw_bytes: [u64; SUPREMACY_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SupremacyCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

fn supremacy_pool_u64(log: &RawLog<SUPREMACY_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn sspan(
    log: &mut RawLog<SUPREMACY_N>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// One structural copy pass: region roster, per-region identity with its
/// dominance container, and every value vector as one bulk span. The job is
/// cleared on entry so pooled refreshes never accumulate spans across hours.
fn copy_supremacy_pass(
    job: &mut SupremacyJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; SUPREMACY_N],
) {
    let _reads = read_accounting::table(TableId::NavalSupremacy);
    job.clear();
    job.active = true;
    job.game_state = game_state;
    // Value vectors are bulk spans (see above); the flag is set up front so
    // owned replay serves enclosed chunks identically.
    job.log.has_bulk = true;
    let started = qpc();
    copy_supremacy_tables(job, memory, base, game_state);
    ticks[0] += elapsed(started);
}

fn copy_supremacy_tables(
    job: &mut SupremacyJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    let log = &mut job.log;
    let regions_header = match sspan(
        log,
        memory,
        game_state.checked_add(cgame_state::STRATEGIC_REGIONS),
        16,
        SUPREMACY_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[regions_header..regions_header + 16]);
    let regions = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    if regions.count == 0 {
        return;
    }
    let region_vtable = match base.checked_add(cstrategic_region::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    let secondary_vtable = match base.checked_add(cstrategic_region::SECONDARY_VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    let dominance_vtable = match base.checked_add(cnaval_region_dominance::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    let values_vtable = match base.checked_add(cdominance_values::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    for ordinal in 1..regions.count {
        let addr = (ordinal as usize)
            .checked_mul(8)
            .and_then(|offset| regions.data.checked_add(offset));
        let pointer_start = match sspan(log, memory, addr, 8, SUPREMACY_MASK) {
            Some(start) => start,
            None => return,
        };
        let pointer = match usize::try_from(supremacy_pool_u64(log, pointer_start))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(pointer) => pointer,
            None => continue,
        };
        // copy_scattered triple: vtables word, identity words, dominance word.
        let vtables = match sspan(log, memory, Some(pointer), 16, SUPREMACY_MASK) {
            Some(start) => start,
            None => return,
        };
        let identity_len = cstrategic_region::RUNTIME_SLOT - cstrategic_region::ID + 4;
        let identity = match sspan(
            log,
            memory,
            pointer.checked_add(cstrategic_region::ID),
            identity_len,
            SUPREMACY_MASK,
        ) {
            Some(start) => start,
            None => return,
        };
        let dominance_start = match sspan(
            log,
            memory,
            pointer.checked_add(cstrategic_region::NAVAL_DOMINANCE),
            8,
            SUPREMACY_MASK,
        ) {
            Some(start) => start,
            None => return,
        };
        if supremacy_pool_u64(log, vtables) != region_vtable
            || supremacy_pool_u64(log, vtables + 8) != secondary_vtable
        {
            return;
        }
        let region_slot = u32::from_le_bytes(
            log.bytes[identity + identity_len - 4..identity + identity_len]
                .try_into()
                .unwrap(),
        );
        if region_slot != ordinal {
            return;
        }
        let dominance = match usize::try_from(supremacy_pool_u64(log, dominance_start))
            .ok()
            .filter(|&address| address != 0)
        {
            Some(dominance) => dominance,
            None => return,
        };
        let owner = match sspan(log, memory, Some(dominance), 16, SUPREMACY_MASK) {
            Some(start) => start,
            None => return,
        };
        if supremacy_pool_u64(log, owner) != dominance_vtable
            || supremacy_pool_u64(log, owner + cnaval_region_dominance::REGION) != pointer as u64
        {
            return;
        }
        let entries_header = match sspan(
            log,
            memory,
            dominance.checked_add(cnaval_region_dominance::ENTRIES),
            16,
            SUPREMACY_MASK,
        ) {
            Some(start) => start,
            None => return,
        };
        raw.copy_from_slice(&log.bytes[entries_header..entries_header + 16]);
        let entries = match Array::from_header(raw, naval_dominance_entry::STRIDE) {
            Some(array) => array,
            None => return,
        };
        if entries.count != 0 {
            let bulk_start = log.take_bulk(
                memory,
                entries.data,
                entries.count as usize * naval_dominance_entry::STRIDE,
                SUPREMACY_MASK,
            );
            let Some(bulk_start) = bulk_start else {
                return;
            };
            // Validate every staged value locally, mirroring the walk: any
            // invalid value fails the whole table there.
            for index in 0..entries.count as usize {
                let at = bulk_start + index * naval_dominance_entry::STRIDE;
                let bytes = match log.bytes.get(at..at + naval_dominance_entry::STRIDE) {
                    Some(bytes) => bytes,
                    None => return,
                };
                let country = u32::from_le_bytes(
                    bytes[naval_dominance_entry::COUNTRY_KEY
                        ..naval_dominance_entry::COUNTRY_KEY + 4]
                        .try_into()
                        .unwrap(),
                );
                let object = &bytes[naval_dominance_entry::VALUES..];
                if u64::from_le_bytes(object[..8].try_into().unwrap()) != values_vtable
                    || u32::from_le_bytes(
                        object[cdominance_values::COUNTRY..cdominance_values::COUNTRY + 4]
                            .try_into()
                            .unwrap(),
                    ) != country
                    || u64::from_le_bytes(
                        object[cdominance_values::REGION..cdominance_values::REGION + 8]
                            .try_into()
                            .unwrap(),
                    ) != pointer as u64
                {
                    return;
                }
            }
        }
        if log.overflowed {
            return;
        }
        let _ = SUPREMACY_MIRROR_BATCH;
    }
}

/// Acquire one owned hourly supremacy payload.
pub(crate) fn copy_supremacy_refresh(
    memory: &impl Memory,
    ctx: SupremacyCopyContext,
    job: &mut SupremacyJob,
) -> SupremacyCopyReport {
    if !replay::cadence::should_walk(TableId::NavalSupremacy, ctx.hour, ctx.force_full) {
        job.clear();
        return SupremacyCopyReport::default();
    }
    let mut ticks = [0u64; SUPREMACY_N];
    copy_supremacy_pass(job, memory, ctx.base, ctx.game_state, &mut ticks);
    SupremacyCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: ticks,
        raw_bytes: job.log.raw_bytes,
    }
}

impl Captured {
    /// Finish an accepted failure after independent capture without live retry.
    pub(crate) fn fail_supremacy_capture(&mut self, reason: &'static str) {
        self.supremacy.outcome = failed(self.active[0], reason);
        self.supremacy_raw_mode = false;
    }

    /// Decode-worker interpretation of a raw supremacy payload: replays the
    /// supremacy walk against the recorded spans into the pooled buffers,
    /// with the identical bounded/capacity handling the game thread applies.
    /// Installs identical rows, truncation counts and failure reasons.
    pub(crate) fn decode_supremacy_payload(
        &mut self,
        base: usize,
        job: &SupremacyJob,
        decode_ticks: &mut [u64; SUPREMACY_N],
    ) {
        self.active[0] = job.active;
        if !job.active {
            self.fail_supremacy_capture(replay::cadence::SKIPPED_REASON);
            return;
        }
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        let walked = cnaval_region_dominance::walk(
            &recorded,
            base,
            job.game_state,
            &mut self.supremacy.rows,
        );
        self.supremacy.outcome = bounded(walked, self.supremacy.rows.len());
        decode_ticks[0] += elapsed(started);
        self.supremacy_raw_mode = false;
    }
}

// ===========================================================================
// Hourly sunk-ships split (SunkShips).
//
// The game thread copies raw bytes only: the sunk-ship history vector, every
// record with its class headers and name strings, and province links. No
// row is decoded on the game thread. The single FIFO decode worker replays
// `csunk_ship_info::walk` against the recorded spans, so rows, ordinals,
// skips, truncation counts and failure reasons are identical by construction
// (see `crate::raw` for the mirror rules).
//
// Record takes are exact spans with no batching (the typed walk reads one
// record at a time and stops at pooled capacity, which the replay applies
// identically against the real pooled buffers). Name headers parse locally
// from the staged record bytes; only terminator and content bytes are taken.
// ===========================================================================

/// Hourly payload slots in calibration/report order: [sunk ships]. The
/// singleton table carries one row with the retained entry list.
pub(crate) const SUNK_N: usize = 1;
const SUNK_MASK: u32 = 0x01;

/// One hour of raw sunk-ships payload. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode
/// side only (see `crate::decode::recycle_sunk_job`).
#[derive(Debug, Default)]
pub(crate) struct SunkJob {
    pub log: RawLog<SUNK_N>,
    pub game_state: usize,
    pub hour: i32,
    active: bool,
}

impl SunkJob {
    pub fn new() -> Self {
        Self {
            // The retained list is one row with a few hundred entries and
            // names (appendix census: 371 compared); both pools start above
            // that so steady state never overflows (see the caps test).
            log: RawLog::new(1 << 13, 2 << 20),
            game_state: 0,
            hour: 0,
            active: false,
        }
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.hour = 0;
        self.active = false;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SunkCopyReport {
    /// False when staging overflowed: the caller falls back to the typed
    /// walk (identical rows through the existing path) and enqueues no
    /// payload.
    pub raw_ok: bool,
    pub copy_ticks: [u64; SUNK_N],
    pub raw_bytes: [u64; SUNK_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SunkCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

fn sunk_pool_u64(log: &RawLog<SUNK_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn kspan(
    log: &mut RawLog<SUNK_N>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Mirror of one sunk-ship class reference: header take with vtable check
/// plus the class name string takes. Returns false exactly when the typed
/// class read fails.
fn copy_sunk_class(
    log: &mut RawLog<SUNK_N>,
    memory: &impl engine::Memory,
    base: usize,
    pointer: u64,
) -> bool {
    let address = match usize::try_from(pointer)
        .ok()
        .filter(|&pointer| pointer != 0)
    {
        Some(address) => address,
        None => return false,
    };
    let header = match kspan(
        log,
        memory,
        Some(address),
        csub_unit_definition::NAME + msvc_string::STRIDE,
        SUNK_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let vtable = sunk_pool_u64(log, header);
    let expected = match base.checked_add(csub_unit_definition::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_null = match base.checked_add(csub_unit_definition::NULL_VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if vtable != expected && vtable != expected_null {
        return false;
    }
    copy_sunk_string(
        log,
        memory,
        address + csub_unit_definition::NAME,
        header + csub_unit_definition::NAME,
    )
}

/// Mirror of one sunk-ship text: header validation is a local parse over the
/// staged header bytes, heap names take the terminator byte plus the content
/// bytes. Returns false exactly when the typed text read fails.
fn copy_sunk_string(
    log: &mut RawLog<SUNK_N>,
    memory: &impl engine::Memory,
    address: usize,
    head: usize,
) -> bool {
    // Copy the staged header out first so later takes can borrow the log.
    let mut header = [0u8; msvc_string::STRIDE];
    header.copy_from_slice(&log.bytes[head..head + msvc_string::STRIDE]);
    let len = u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::LENGTH + 8]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(
        header[msvc_string::CAPACITY..msvc_string::CAPACITY + 8]
            .try_into()
            .unwrap(),
    );
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return false;
    }
    let len = match usize::try_from(len) {
        Ok(len) => len,
        Err(_) => return false,
    };
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        address
    } else {
        match usize::try_from(u64::from_le_bytes(
            header[msvc_string::DATA..msvc_string::DATA + 8]
                .try_into()
                .unwrap(),
        ))
        .ok()
        .filter(|&source| source != 0)
        {
            Some(source) => source,
            None => return false,
        }
    };
    let terminator = match source.checked_add(len) {
        Some(terminator) => terminator,
        None => return false,
    };
    let term_start = match kspan(log, memory, Some(terminator), 1, SUNK_MASK) {
        Some(start) => start,
        None => return false,
    };
    if log.bytes[term_start] != 0 {
        return false;
    }
    if len != 0 && kspan(log, memory, Some(source), len, SUNK_MASK).is_none() {
        return false;
    }
    true
}

/// Mirror of one sunk-ship record: full record take with local validation
/// plus class and text takes. Returns false exactly when the typed walk
/// fails at or before this record.
fn copy_sunk_record(
    log: &mut RawLog<SUNK_N>,
    memory: &impl engine::Memory,
    base: usize,
    current_hour: i32,
    pointer: usize,
) -> bool {
    let record = match kspan(log, memory, Some(pointer), csunk_ship_info::SIZE, SUNK_MASK) {
        Some(start) => start,
        None => return false,
    };
    // Copy the staged record out first so later takes can borrow the log.
    let mut staged = [0u8; csunk_ship_info::SIZE];
    staged.copy_from_slice(&log.bytes[record..record + csunk_ship_info::SIZE]);
    let u64_at = |offset: usize| u64::from_le_bytes(staged[offset..offset + 8].try_into().unwrap());
    let u32_at = |offset: usize| u32::from_le_bytes(staged[offset..offset + 4].try_into().unwrap());
    let expected = match base.checked_add(csunk_ship_info::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if u64_at(0) != expected {
        return false;
    }
    let expected_date = match base.checked_add(cgame_date::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_persistent = match base.checked_add(cgame_date::PERSISTENT_VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if u64_at(csunk_ship_info::SINK_DATE) != expected_date
        || u64_at(csunk_ship_info::SINK_DATE_PERSISTENT) != expected_persistent
    {
        return false;
    }
    let sink_hour = u32_at(csunk_ship_info::SINK_HOUR) as i32;
    if sink_hour <= 0 || sink_hour > current_hour {
        return false;
    }
    if staged[csunk_ship_info::CAUSE_RECORD] > 1 || staged[csunk_ship_info::ASSISTED_KILL] > 1 {
        return false;
    }
    if !copy_sunk_class(log, memory, base, u64_at(csunk_ship_info::SHIP_CLASS)) {
        return false;
    }
    let attribution = u64_at(csunk_ship_info::ATTRIBUTION_CLASS);
    if attribution != 0 && !copy_sunk_class(log, memory, base, attribution) {
        return false;
    }
    for offset in [
        csunk_ship_info::SHIP_NAME,
        csunk_ship_info::ATTRIBUTION_SOURCE_NAME,
    ] {
        if !copy_sunk_string(log, memory, pointer + offset, record + offset) {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    // Class names parse from the staged class headers above.
    let province_pointer = u64_at(csunk_ship_info::PROVINCE);
    if province_pointer != 0 {
        let province = match usize::try_from(province_pointer).ok() {
            Some(province) => province,
            None => return false,
        };
        let vtable = match kspan(log, memory, Some(province), 8, SUNK_MASK) {
            Some(start) => start,
            None => return false,
        };
        let expected_province = match base.checked_add(cprovince::VTABLE_RVA) {
            Some(expected) => expected as u64,
            None => return false,
        };
        if sunk_pool_u64(log, vtable) != expected_province {
            return false;
        }
        if kspan(
            log,
            memory,
            province.checked_add(cprovince::ID),
            4,
            SUNK_MASK,
        )
        .is_none()
        {
            return false;
        }
    }
    true
}

/// One structural copy pass: history vector plus every record with class,
/// text and province takes. The job is cleared on entry so pooled refreshes
/// never accumulate spans across hours.
fn copy_sunk_pass(
    job: &mut SunkJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    current_hour: i32,
    ticks: &mut [u64; SUNK_N],
) {
    let _reads = read_accounting::table(TableId::SunkShips);
    job.clear();
    job.active = true;
    job.game_state = game_state;
    job.hour = current_hour;
    let started = qpc();
    copy_sunk_tables(job, memory, base, game_state, current_hour);
    ticks[0] += elapsed(started);
}

fn copy_sunk_tables(
    job: &mut SunkJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    current_hour: i32,
) {
    if game_state == 0 || current_hour <= 0 {
        return;
    }
    let log = &mut job.log;
    let header = match kspan(
        log,
        memory,
        game_state.checked_add(cgame_state::SUNK_SHIP_HISTORY),
        16,
        SUNK_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header..header + 16]);
    let array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    for index in 0..array.count {
        let addr = (index as usize)
            .checked_mul(8)
            .and_then(|offset| array.data.checked_add(offset));
        let slot = match kspan(log, memory, addr, 8, SUNK_MASK) {
            Some(start) => start,
            None => return,
        };
        let pointer = match usize::try_from(sunk_pool_u64(log, slot))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(pointer) => pointer,
            None => return,
        };
        if !copy_sunk_record(log, memory, base, current_hour, pointer) {
            return;
        }
        if log.overflowed {
            return;
        }
    }
}

/// Acquire one owned hourly sunk-ships payload.
pub(crate) fn copy_sunk_refresh(
    memory: &impl Memory,
    ctx: SunkCopyContext,
    job: &mut SunkJob,
) -> SunkCopyReport {
    if !replay::cadence::should_walk(TableId::SunkShips, ctx.hour, ctx.force_full) {
        job.clear();
        return SunkCopyReport::default();
    }
    let mut ticks = [0u64; SUNK_N];
    copy_sunk_pass(job, memory, ctx.base, ctx.game_state, ctx.hour, &mut ticks);
    SunkCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: ticks,
        raw_bytes: job.log.raw_bytes,
    }
}

impl Captured {
    /// Finish an accepted failure after independent capture without live retry.
    pub(crate) fn fail_sunk_capture(&mut self, reason: &'static str) {
        self.sunk.outcome = failed(self.active[13], reason);
        self.sunk_names.reset();
        self.sunk_raw_mode = false;
    }

    /// Decode-worker interpretation of a raw sunk-ships payload: replays the
    /// sunk-ship walk against the recorded spans into the pooled buffers,
    /// with the identical bounded/capacity handling the game thread applies.
    /// Installs identical rows, truncation counts and failure reasons.
    pub(crate) fn decode_sunk_payload(
        &mut self,
        base: usize,
        job: &SunkJob,
        decode_ticks: &mut [u64; SUNK_N],
    ) {
        self.active[13] = job.active;
        if !job.active {
            self.fail_sunk_capture(replay::cadence::SKIPPED_REASON);
            return;
        }
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        let first = csunk_ship_info::walk(
            &recorded,
            base,
            job.game_state,
            job.hour,
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
        decode_ticks[0] += elapsed(started);
        self.sunk_raw_mode = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned_failure_fixture() -> Captured {
        Captured::with_capacities(&Capacities {
            rows: [1; N],
            theatre_names: 8,
            sunk_names: 8,
            orders_sources: 1,
            wing_sources: 1,
            roster_scratch: 8,
        })
    }

    /// The OOB theatre memo keeps first-seen root order, whatever the
    /// addresses' order, exactly as the former sorted-insert memo did.
    #[test]
    fn a_large_registry_presizes_the_order_of_battle_theatres() {
        crate::country_scale::note_slots(5_000);
        let mut job = NavalJob::new();
        job.presize_countries();
        let want = crate::country_scale::rows(5_000, 1) * OOB_THEATRE_REFERENCES_PER_COUNTRY;
        assert!(job.oob_theatres.capacity() >= want);
        assert!(job.oob_theatre_positions.capacity() >= want);
    }

    /// The registry census snapshot holds only a complete census that fits,
    /// grows off the callback from its demand, and is dropped with the pass.
    #[test]
    fn census_roots_snapshot_fits_grows_and_never_outlives_its_pass() {
        let roots = [corders_group::Root::default(); 5];
        let mut job = NavalJob::default();
        job.capture_roots(&roots, Outcome::Rows { len: 3 });
        assert_eq!((job.roots_len, job.roots.capacity()), (None, 0));
        assert!(job.grow_for_next());
        job.grow();
        job.capture_roots(&roots, Outcome::Rows { len: 3 });
        assert_eq!(job.roots_len, Some(3));
        assert_eq!(job.roots, roots[..3]);
        job.capture_roots(
            &roots,
            Outcome::Truncated {
                len: 3,
                unread_rows: 4,
            },
        );
        assert_eq!(job.roots_len, None);
        assert_eq!(job.roots_required, 7);
        job.capture_roots(&roots, Outcome::Unavailable { reason: "torn" });
        assert_eq!(job.roots_len, None);
        job.capture_roots(&roots, Outcome::Rows { len: 3 });
        job.clear();
        assert_eq!((job.roots_len, job.roots.len()), (None, 0));
        // A census longer than the roots it names is not kept.
        job.capture_roots(&roots[..2], Outcome::Rows { len: 3 });
        assert_eq!(job.roots_len, None);
    }

    #[test]
    fn oob_theatre_dedup_keeps_first_occurrences_in_root_order() {
        let first_seen = |values: &[usize]| {
            let mut seen = Vec::new();
            for &value in values {
                if !seen.contains(&value) {
                    seen.push(value);
                }
            }
            seen
        };
        for values in [
            vec![],
            vec![7],
            vec![7, 7, 7],
            vec![0x30, 0x10, 0x30, 0x20, 0x10, 0x40, 0x20],
            vec![5, 4, 3, 2, 1, 1, 2, 3, 4, 5],
            (0..500).map(|i| (i * 7919) % 97 * 0x40).collect(),
        ] {
            let mut deduped = values.clone();
            let mut positions = Vec::with_capacity(values.len());
            let capacity = positions.capacity();
            dedup_first_seen(&mut deduped, &mut positions);
            assert_eq!(deduped, first_seen(&values));
            assert_eq!(positions.capacity(), capacity);
        }
    }

    #[test]
    fn owned_supremacy_and_sunk_failures_are_table_local() {
        let mut captured = owned_failure_fixture();
        captured.raw_mode = true;
        captured.supremacy_raw_mode = true;
        captured.sunk_raw_mode = true;
        captured.supremacy.outcome = Outcome::Rows { len: 1 };
        captured.sunk.outcome = Outcome::Rows { len: 1 };
        captured.convoys.outcome = Outcome::Rows { len: 1 };
        captured.sunk_names.len = 4;
        captured.sunk_names.required = 9;
        let bytes = captured.allocated_bytes();
        captured.fail_supremacy_capture("supremacy worker failed");
        assert_eq!(
            captured.supremacy.outcome,
            Outcome::Unavailable {
                reason: "supremacy worker failed"
            }
        );
        assert!(!captured.supremacy_raw_mode);
        assert!(captured.sunk_raw_mode && captured.raw_mode);
        assert_eq!(captured.sunk.outcome, Outcome::Rows { len: 1 });
        captured.fail_sunk_capture("sunk worker failed");
        assert_eq!(
            captured.sunk.outcome,
            Outcome::Unavailable {
                reason: "sunk worker failed"
            }
        );
        assert_eq!(
            (captured.sunk_names.len, captured.sunk_names.required),
            (0, 0)
        );
        assert!(!captured.sunk_raw_mode && captured.raw_mode);
        assert_eq!(captured.convoys.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.allocated_bytes(), bytes);
    }

    #[test]
    fn owned_naval_failure_preserves_independent_sources_and_other_families() {
        let mut captured = owned_failure_fixture();
        captured.raw_mode = true;
        captured.supremacy_raw_mode = true;
        captured.sunk_raw_mode = true;
        captured.supremacy.outcome = Outcome::Rows { len: 1 };
        captured.convoys.outcome = Outcome::Rows { len: 1 };
        captured.cooldowns.outcome = Outcome::Rows { len: 1 };
        captured.edges.outcome = Outcome::Rows { len: 1 };
        captured.results.outcome = Outcome::Rows { len: 1 };
        captured.result_members.outcome = Outcome::Rows { len: 1 };
        captured.result_countries.outcome = Outcome::Rows { len: 1 };
        captured.sunk.outcome = Outcome::Rows { len: 1 };
        captured.orders_sources.outcome = Outcome::Rows { len: 1 };
        captured.wing_sources.outcome = Outcome::Rows { len: 1 };
        captured.orders_finished = true;
        captured.wings_finished = true;
        captured.source_ticks = [17, 23, 29];
        captured.theatre_names.len = 4;
        captured.theatre_names.required = 9;
        let mut outcomes = captured.outcomes();
        let bytes = captured.allocated_bytes();
        captured.fail_naval_capture("naval worker failed");
        for index in [4, 5, 6, 7, 8, 9, 14, 15, 16] {
            outcomes[index] = Outcome::Unavailable {
                reason: "naval worker failed",
            };
        }
        assert_eq!(captured.outcomes(), outcomes);
        assert_eq!(captured.orders_sources.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.wing_sources.outcome, Outcome::Rows { len: 1 });
        assert!(captured.orders_finished);
        assert!(captured.wings_finished);
        assert_eq!(captured.source_ticks, [17, 23, 29]);
        assert_eq!(captured.theatre_names.len, 0);
        assert_eq!(captured.theatre_names.required, 0);
        assert!(!captured.raw_mode);
        assert!(captured.supremacy_raw_mode && captured.sunk_raw_mode);
        assert_eq!(captured.allocated_bytes(), bytes);
    }

    #[test]
    fn naval_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        let mut job = NavalJob::default();
        job.wing_sources.reserve_exact(3);
        job.capacity_charge = Charge::empty_on(&LOCAL);
        job.update_capacity_charge();
        let expected = |job: &NavalJob| vector_bytes(&job.wing_sources);
        let initial = LOCAL.snapshot().current.auxiliary_vectors;
        assert_eq!(initial, expected(&job));
        job.wing_outcome = Some(Outcome::Truncated {
            len: 3,
            unread_rows: 5,
        });
        job.grow();
        let grown = LOCAL.snapshot().current.auxiliary_vectors;
        assert!(grown > initial);
        assert_eq!(grown, expected(&job));
        job.clear();
        assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
        drop(job);
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
    }

    #[test]
    fn shared_naval_projection_preserves_truncated_prefix_and_growth() {
        use crate::extended::air::{AirWingCopyContext, AirWingJob, wing_test_kit as kit};
        let mut bytes = kit::fixture();
        bytes[0x34000..0x34008].fill(0);
        let memory = kit::memory(&bytes);
        let mut naval = NavalJob::default();
        naval.wing_sources.reserve_exact(1);
        let mut wings = AirWingJob::new();
        let context = AirWingCopyContext {
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,

            limits: [2, 3, 2],
            naval_limit: naval.wing_limit(),
        };
        assert!(crate::extended::air::copy_air_refresh(&memory, context, &mut wings).raw_ok);
        naval.install_wings(&wings);
        assert_eq!(naval.wing_sources.len(), 1);
        assert_eq!(
            naval.wing_outcome,
            Some(Outcome::Truncated {
                len: 1,
                unread_rows: 1
            })
        );
        assert_eq!(naval.wing_required, 2);
        assert!(naval.grow_for_next());
        naval.grow();
        assert!(naval.wing_limit() >= 2);
    }

    #[test]
    fn naval_decode_bounds_shared_memberships_to_frame_capacity() {
        use super::naval_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let mut job = NavalJob::new();
        kit::stage_wings(&memory, &mut job);
        let context = NavalCopyContext {
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        assert!(super::copy_naval_refresh(&memory, context, &mut job).raw_ok);
        job.wing_sources.resize(2, Default::default());
        let capacity = Capacities {
            wing_sources: 1,
            ..Default::default()
        };
        let mut captured = Captured::with_capacities(&capacity);
        for missing in [0, 3] {
            job.wing_outcome = Some(if missing == 0 {
                Outcome::Rows { len: 2 }
            } else {
                Outcome::Truncated {
                    len: 2,
                    unread_rows: missing,
                }
            });
            super::decode_naval_payload(&mut captured, kit::BASE, &job, &mut [0; NAVAL_PACK_N]);
            assert_eq!(
                captured.wing_sources.outcome,
                Outcome::Truncated {
                    len: 1,
                    unread_rows: missing + 1
                }
            );
            assert_eq!(captured.wing_sources.rows.len(), 1);
        }
    }

    #[test]
    fn shared_wing_fallback_charges_acquisition_and_installation() {
        use crate::extended::air::{AirWingCopyContext, AirWingJob, wing_test_kit as kit};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let mut wings = AirWingJob::new();
        let context = AirWingCopyContext {
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,

            limits: [2, 3, 2],
            naval_limit: 0,
        };
        assert!(crate::extended::air::copy_air_refresh(&memory, context, &mut wings).raw_ok);
        wings.membership_ticks = 101;
        let mut captured = small();
        captured.add_wing_source_ticks(17);
        assert_eq!(captured.source_ticks[1], 17);
        captured.install_map_wings(&wings);
        assert!(captured.source_ticks[1] >= 118);
        assert_eq!(captured.wing_sources.outcome, Outcome::Rows { len: 2 });
        let first_ticks = captured.source_ticks[1];
        captured.add_wing_source_ticks(23);
        assert_eq!(captured.source_ticks[1], first_ticks + 23);
    }

    use engine::memory::SliceMemory;

    fn small() -> Captured {
        Captured::with_capacities(&Capacities {
            rows: [2; N],
            theatre_names: 32,
            sunk_names: 32,
            orders_sources: 2,
            wing_sources: 2,
            roster_scratch: 8,
        })
    }

    #[test]
    fn primary_membership_rejects_omissions_duplicates_and_incomplete_censuses() {
        let link = |row: &(u64, u64)| *row;
        let first_id = 0x1234_5678_0000_0035;
        let second_id = 0x9abc_def0_0000_0035;
        let group = 0xfedc_ba98_0000_004e;
        let rows = [(second_id, group), (first_id, group)];
        let sources = [(first_id, group), (42, 0), (second_id, group)];
        let full = Outcome::Rows { len: 2 };
        let census = Outcome::Rows { len: 3 };
        assert!(membership_issue(full, &rows, link, census, &sources, link).is_none());
        for bad in [
            vec![(first_id, group)],
            vec![(first_id, group), (first_id, group)],
            vec![(first_id, group), (second_id, group ^ (1_u64 << 32))],
        ] {
            let outcome = Outcome::Rows { len: bad.len() };
            assert!(membership_issue(outcome, &bad, link, census, &sources, link).is_some());
        }
        let partial = Outcome::Truncated {
            len: 2,
            unread_rows: 1,
        };
        assert!(membership_issue(full, &rows, link, partial, &sources, link).is_some());
        assert!(
            membership_issue(full, &rows, link, Outcome::Rows { len: 4 }, &sources, link).is_some()
        );
    }

    #[test]
    fn wing_visitor_requires_a_complete_enclosing_census_and_preserves_full_handles() {
        let mut captured = small();
        captured.reset();
        let mut bytes = [0; 0x48];
        let group = 0x89ab_cdef_0000_004e_u64;
        bytes[cair_wing::GROUP_HANDLE..cair_wing::GROUP_HANDLE + 8]
            .copy_from_slice(&group.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let wing = cair_wing::Row {
            id: 0xfedc_ba98_0000_0009,
            ..Default::default()
        };
        captured.visit_wing(&memory, 0x1000, &wing);
        assert!(!complete_source(
            &captured.wing_sources,
            captured.wings_finished
        ));
        captured.finish_wings(Outcome::Rows { len: 1 });
        assert!(complete_source(
            &captured.wing_sources,
            captured.wings_finished
        ));
        assert_eq!(
            captured.wing_sources.rows[0],
            ctheatre::WingMembership {
                wing: wing.id,
                group
            }
        );
        captured.reset();
        captured.visit_wing(&memory, 0x1000, &wing);
        captured.finish_wings(Outcome::Truncated {
            len: 1,
            unread_rows: 2,
        });
        assert!(!complete_source(
            &captured.wing_sources,
            captured.wings_finished
        ));
        captured.reset();
        captured.visit_wing(&memory, 0x1000, &wing);
        captured.finish_wings(Outcome::Rows { len: 2 });
        assert!(matches!(
            captured.wing_sources.outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn primary_name_ranges_preserve_bytes_and_refuse_invalid_extents() {
        let mut rows = CaptureRows::<ctheater_group::Row>::new(1);
        rows.outcome = Outcome::Rows { len: 1 };
        rows.rows[0] = ctheater_group::Row {
            id: u64::MAX,
            priority: -1,
            name_start: 0,
            name_len: 2,
            ..Default::default()
        };
        let name = |row: &ctheater_group::Row| (row.name_start, row.name_len);
        assert!(valid_named(rows.outcome, &rows.rows, Some(b"ab"), name));
        assert!(!valid_named(rows.outcome, &rows.rows, Some(b"a"), name));
        rows.rows[0].name_start = usize::MAX;
        assert!(!valid_named(rows.outcome, &rows.rows, Some(b"ab"), name));
    }

    #[test]
    fn sunk_history_preserves_duplicate_records_and_never_publishes_a_prefix() {
        let mut captured = small();
        captured.sunk.outcome = Outcome::Rows { len: 2 };
        captured.sunk_names.first[..2].copy_from_slice(&[0xff, 0]);
        captured.sunk_names.len = 2;
        captured.sunk_names.required = 2;
        captured.sunk.rows[0].ship_name = csunk_ship_info::TextRange { start: 0, len: 2 };
        captured.sunk.rows[0].victim_equipment_variant = u64::MAX;
        captured.sunk.rows[1] = captured.sunk.rows[0];
        let TableView::Complete(TableRows::SunkShips(rows)) = captured.sunk_table() else {
            panic!("complete sequence expected");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entries.len(), 2);
        assert_eq!(rows[0].entries[0], rows[0].entries[1]);
        assert_eq!(rows[0].entries[0].ship_name, [0xff, 0]);
        captured.sunk.outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 1,
        };
        assert_eq!(
            captured.outcomes()[13],
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!(
            captured.sunk_table(),
            TableView::Truncated {
                rows: TableRows::SunkShips(Vec::new()),
                missing: 1
            }
        );
        captured.sunk.outcome = Outcome::Rows { len: 2 };
        captured.sunk.rows[0].ship_name.start = usize::MAX;
        assert!(matches!(
            captured.sunk_table(),
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn hourly_land_theatre_headers_need_no_daily_details_or_wing_storage() {
        use super::naval_test_kit as kit;
        struct NoTheatreDetails<'a>(engine::memory::SliceMemory<'a>);
        impl Memory for NoTheatreDetails<'_> {
            fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
                assert_ne!(address, kit::THEATRE + ctheatre::THEATER_GROUP_DATA);
                assert_ne!(address, kit::C0 + engine::ccountry::AIR_THEATRES);
                self.0.copy(address, bytes)
            }
        }
        let bytes = kit::fixture();
        let expected = kit::direct(&kit::memory(&bytes));
        let memory = NoTheatreDetails(kit::memory(&bytes));
        let mut job = NavalJob::new();
        job.wing_sources = Vec::new();
        let mut captured = Captured::with_capacities(&Capacities {
            rows: [8; N],
            theatre_names: 0,
            sunk_names: 0,
            orders_sources: 0,
            wing_sources: 0,
            roster_scratch: 0,
        });
        for hour in (0..24).filter(|hour| *hour != 13) {
            job.clear();
            captured.prepare_hour(hour, false);
            let copy = copy_naval_refresh(
                &memory,
                NavalCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    hour,
                    force_full: false,
                },
                &mut job,
            );
            assert!(copy.raw_ok && !job.grow_for_next());
            assert!(copy.raw_bytes[0] > 0);
            assert_eq!(&copy.raw_bytes[1..6], &[0; 5]);
            let mut ticks = [0; NAVAL_PACK_N];
            decode_naval_payload(&mut captured, kit::BASE, &job, &mut ticks);
            assert_eq!(captured.land.outcome, expected.land_outcome);
            assert_eq!(
                &captured.land.rows[..captured.land.outcome.len()],
                &expected.land[..expected.land_outcome.len()]
            );
            assert_eq!(&captured.outcomes()[5..10], &[SKIPPED; 5]);
            assert_eq!(captured.wing_sources.outcome, SKIPPED);
            assert_eq!(captured.groups.outcome, expected.oob_groups_outcome);
            assert_eq!(captured.group_members.outcome, expected.oob_members_outcome);
            assert_eq!(
                captured.group_children.outcome,
                expected.oob_children_outcome
            );
            let len = captured.groups.outcome.len();
            assert_eq!(&captured.groups.rows[..len], &expected.oob_groups[..len]);
            captured.fail_naval_capture("worker failed");
            assert_eq!(
                captured.land.outcome,
                Outcome::Unavailable {
                    reason: "worker failed"
                }
            );
            assert_eq!(&captured.outcomes()[5..10], &[SKIPPED; 5]);
            assert_eq!(
                captured.groups.outcome,
                Outcome::Unavailable {
                    reason: "worker failed"
                }
            );
        }
    }
    /// The land theatre copy through the callback's authenticated roster
    /// takes no country census span, and its decode gives the rows, ordinals
    /// and outcomes of the census copy and of the typed walk with or without
    /// the roster: clean, with an alias or a null-slot owner (whose link the
    /// replay reads), bad owners, a torn census, and a fault in every header
    /// span, on the daily slot and hourly. A snapshot without room falls back
    /// to the census copy and asks to grow.
    #[test]
    fn roster_land_theatres_match_the_census_copy_and_the_typed_walk() {
        use super::naval_test_kit as kit;
        use engine::{ccountry, cgame_state, country_memory::Cache, memory::SliceMemory};
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            fault: Option<usize>,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if self
                    .fault
                    .is_some_and(|fault| (address..address + output.len()).contains(&fault))
                {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
            bytes[at..at + value.len()].copy_from_slice(value);
        }
        const CTAB: usize = 0x11000;
        const LTAB: usize = 0x12000;
        let owner = kit::THEATRE + ctheatre::OWNER;
        // A fourth, null country slot whose id links to the first country.
        let mut base = kit::fixture();
        put(&mut base, kit::GAME_STATE + cgame_state::COUNTRIES + 8, &[4, 0, 0, 0, 4, 0, 0, 0]);
        put(&mut base, kit::GAME_STATE + cgame_state::COUNTRY_LINKS + 8, &[4, 0, 0, 0, 4, 0, 0, 0]);
        put(&mut base, LTAB + 12, &1_u32.to_le_bytes());
        put(
            &mut base,
            0x40000 + engine::cstrategic_air_manager::COUNTRY_AIR + 8,
            &[4, 0, 0, 0, 4, 0, 0, 0],
        );
        let mut variants: Vec<(&str, Vec<u8>, Option<usize>)> = vec![("clean", base.clone(), None)];
        for (name, value) in [("alias owner", 2_u32), ("null-slot owner", 3), ("zero owner", 0), ("owner past links", 9)] {
            let mut bytes = base.clone();
            put(&mut bytes, owner, &value.to_le_bytes());
            variants.push((name, bytes, None));
        }
        let mut torn = base.clone();
        put(&mut torn, LTAB + 8, &0_u32.to_le_bytes());
        variants.push(("torn census", torn, None));
        let mut vtable = base.clone();
        vtable[kit::THEATRE] ^= 0x10;
        variants.push(("theatre vtable", vtable, None));
        for (offset, len) in ctheatre::HEADER_SPANS {
            for at in [kit::THEATRE + offset, kit::THEATRE + offset + len - 1] {
                variants.push(("header fault", base.clone(), Some(at)));
            }
        }
        for at in [kit::C0 + ccountry::THEATRES, 0x36000, CTAB + 8, kit::C0 + ccountry::ID] {
            variants.push(("country fault", base.clone(), Some(at)));
        }
        let hourly = |memory: &dyn Fn(&mut NavalJob) -> NavalCopyReport, job: &mut NavalJob| {
            let copy = memory(job);
            assert!(copy.raw_ok);
            let mut captured = Captured::with_capacities(&Capacities::default());
            captured.prepare_hour(1, false);
            decode_naval_payload(&mut captured, kit::BASE, job, &mut [0; NAVAL_PACK_N]);
            let len = captured.land.outcome.len();
            (captured.land.outcome, captured.land.rows[..len].to_vec())
        };
        let context = NavalCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            hour: 1,
            force_full: false,
        };
        let mut roster_used = 0;
        for (name, bytes, fault) in &variants {
            let memory = Fault {
                memory: kit::memory(bytes),
                fault: *fault,
            };
            let mut cache = Cache::with_capacity(Cache::cells_for_countries(4, 64));
            let _ = cache.prepare(&memory, kit::BASE, kit::GAME_STATE);
            let roster = cache.roster_view(&memory);
            let has_roster = roster
                .authenticated_countries(kit::BASE, kit::GAME_STATE)
                .is_some();
            roster_used += usize::from(has_roster);
            let case = format!("{name} {fault:x?} roster={has_roster}");

            // Daily: every theatre table and the order of battle.
            let direct = kit::direct(&memory);
            kit::assert_matches_direct(&kit::direct(&roster), &direct);
            let (census, ..) = kit::split(&memory);
            kit::assert_matches_direct(&census, &direct);
            let (split, _, job) = kit::split(&roster);
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(job.roster.roster(kit::BASE, kit::GAME_STATE).is_some(), has_roster, "{case}");

            // Hourly: the land theatre headers alone.
            let mut plain = NavalJob::new();
            let expected = hourly(&|job| copy_naval_refresh(&memory, context, job), &mut plain);
            assert_eq!((expected.0, &expected.1[..]), (direct.land_outcome, &direct.land[..direct.land_outcome.len()]), "{case}");
            let mut job = NavalJob::new();
            let actual = hourly(&|job| copy_naval_refresh(&roster, context, job), &mut job);
            assert_eq!(actual, expected, "{case}");
            if has_roster {
                // No census span: neither header, the country table, a
                // country's identity, nor a link the roster resolves.
                let spans: Vec<_> = job.log.spans.iter().map(|span| span.addr).collect();
                for address in [
                    kit::GAME_STATE + cgame_state::COUNTRIES,
                    kit::GAME_STATE + cgame_state::COUNTRY_LINKS,
                    CTAB + 8,
                    CTAB + 16,
                    kit::C0,
                    kit::C0 + ccountry::ID,
                    LTAB + 4,
                    LTAB + 8,
                ] {
                    assert!(!spans.contains(&address), "{case}: {address:#x}");
                }
                assert!(spans.len() < plain.log.spans.len(), "{case}");
                // The replay reads the null slot's link, so the copy took it.
                if *name == "null-slot owner" {
                    assert!(spans.contains(&(LTAB + 12)), "{case}");
                }
                // Without room for the roster the census copy is taken.
                let mut tight = NavalJob::new();
                tight.roster = ccountry::RosterSnapshot::with_capacity(0);
                let short = hourly(&|job| copy_naval_refresh(&roster, context, job), &mut tight);
                assert_eq!(short, expected, "{case}");
                assert!(tight.roster.roster(kit::BASE, kit::GAME_STATE).is_none());
                assert!(tight.grow_for_next());
                assert_eq!(tight.log.spans.len(), plain.log.spans.len(), "{case}");
            }
        }
        assert!(roster_used >= variants.len() - 3, "{roster_used}");
    }

    #[test]
    fn hourly_land_theatre_owner_fault_does_not_publish_a_header() {
        use super::naval_test_kit as kit;
        let mut bytes = kit::fixture();
        bytes[kit::THEATRE + ctheatre::OWNER..kit::THEATRE + ctheatre::OWNER + 4].fill(0);
        let memory = kit::memory(&bytes);
        let mut job = NavalJob::new();
        let copy = copy_naval_refresh(
            &memory,
            NavalCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour: 1,
                force_full: false,
            },
            &mut job,
        );
        assert!(copy.raw_ok);
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.prepare_hour(1, false);
        decode_naval_payload(&mut captured, kit::BASE, &job, &mut [0; NAVAL_PACK_N]);
        assert!(matches!(captured.land.outcome, Outcome::Unavailable { .. }));
        assert_eq!(&captured.outcomes()[5..10], &[SKIPPED; 5]);
    }
    #[test]
    fn land_theatre_headers_and_order_of_battle_are_hourly() {
        use replay::rows::TableId;
        for (slot, table) in [
            TableId::LandTheatres,
            TableId::LandTheatreGroups,
            TableId::LandTheatreMembers,
            TableId::AirTheatres,
            TableId::AirTheatreGroups,
            TableId::AirTheatreMembers,
            TableId::OrdersGroups,
            TableId::OrdersGroupMembers,
            TableId::ArmyGroupMembers,
        ]
        .into_iter()
        .enumerate()
        {
            let daily = (1..6).contains(&slot);
            assert_eq!(replay::cadence::is_staggered(table), daily, "{table:?}");
            for hour in -24_i32..48 {
                assert_eq!(
                    replay::cadence::is_refresh(table, hour),
                    !daily || hour.rem_euclid(24) == 13,
                    "{table:?} at {hour}"
                );
            }
        }
    }

    #[test]
    fn supremacy_and_sunk_offslot_jobs_skip_reads_and_decoding() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, address: usize, _: &mut [u8]) -> bool {
                panic!("inactive naval read at {address:#x}");
            }
        }
        let mut supremacy = SupremacyJob::default();
        let mut sunk = SunkJob::default();
        let mut captured = owned_failure_fixture();
        for hour in 0..24 {
            captured.prepare_hour(hour, false);
            if hour != 23 {
                let report = copy_supremacy_refresh(
                    &NoReads,
                    SupremacyCopyContext {
                        base: 0,
                        game_state: 1,
                        hour,
                        force_full: false,
                    },
                    &mut supremacy,
                );
                assert!(!report.raw_ok && !supremacy.grow_for_next());
                assert_eq!((report.copy_ticks, report.raw_bytes), ([0], [0]));
                captured.fail_supremacy_capture("worker failed");
                let mut ticks = [0];
                captured.decode_supremacy_payload(0, &supremacy, &mut ticks);
                assert_eq!((captured.supremacy.outcome, ticks), (SKIPPED, [0]));
            }
            if hour != 19 {
                let report = copy_sunk_refresh(
                    &NoReads,
                    SunkCopyContext {
                        base: 0,
                        game_state: 1,
                        hour,
                        force_full: false,
                    },
                    &mut sunk,
                );
                assert!(!report.raw_ok && !sunk.grow_for_next());
                assert_eq!((report.copy_ticks, report.raw_bytes), ([0], [0]));
                captured.fail_sunk_capture("worker failed");
                let mut ticks = [0];
                captured.decode_sunk_payload(0, &sunk, &mut ticks);
                assert_eq!((captured.sunk.outcome, ticks), (SKIPPED, [0]));
            }
        }
    }


    #[test]
    fn naval_split_copy_matches_direct_walk() {
        use super::naval_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.land_outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.members_outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.groups_outcome, Outcome::Rows { len: 1 });
    }

    #[test]
    fn naval_copy_sizes_cover_documented_read_set() {
        // S3: every Memory::copy size the naval copy pass issues, recorded
        // through the real pass. Registry headers (16/32), words (4/8),
        // theatre names (inline/heap) and OOB member rows. Alignment follows
        // the engine layout (headers 8-aligned, strings byte-packed).
        use super::naval_test_kit as kit;
        use super::{NavalCopyContext, NavalJob};
        use engine::Memory;
        use std::collections::BTreeSet;
        struct Recorder<'a> {
            inner: engine::memory::SliceMemory<'a>,
            sizes: std::cell::RefCell<BTreeSet<usize>>,
        }
        impl Memory for Recorder<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.sizes.borrow_mut().insert(output.len());
                self.inner.copy(address, output)
            }
        }
        let bytes = kit::fixture();
        let inner = kit::memory(&bytes);
        let memory = Recorder {
            inner,
            sizes: std::cell::RefCell::new(BTreeSet::new()),
        };
        let ctx = NavalCopyContext {
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut first = NavalJob::new();
        kit::stage_wings(&memory, &mut first);
        let report = super::copy_naval_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        let sizes = memory.sizes.borrow().clone();
        for required in [4, 8, 16] {
            assert!(
                sizes.contains(&required),
                "missing size {required} in {sizes:?}"
            );
        }
    }

    #[test]
    fn naval_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        // Fixture holds 1 land theatre, 1 group, 2 members, 1 OOB group;
        // cap 1 is the overflow-by-one arm for members (total 2 == cap+1),
        // cap 0 the zero/massive arm, cap 2 the members exact-fit arm.
        // u32 unread saturation is not reachable and is covered by the
        // walk's own saturating arithmetic.
        use super::Capacities;
        use super::naval_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for rows in [[0; 17], [1; 17], [2; 17], [64; 17]] {
            let capacity = Capacities {
                rows,
                theatre_names: 1024,
                sunk_names: 1024,
                orders_sources: 64,
                wing_sources: 64,
                roster_scratch: 64,
            };
            let direct = kit::direct_with(&memory, &capacity);
            let (split, report, _) = kit::split_with(&memory, &capacity);
            assert!(report.raw_ok, "rows={rows:?}");
            kit::assert_matches_direct(&split, &direct);
        }
        // Exact-fit arm pins Rows on land/groups with both members present.
        let capacity = Capacities {
            rows: [2; 17],
            theatre_names: 1024,
            sunk_names: 1024,
            orders_sources: 64,
            wing_sources: 64,
            roster_scratch: 64,
        };
        let (split, _, _) = kit::split_with(&memory, &capacity);
        assert_eq!(split.land_outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.members_outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn naval_split_copy_matches_direct_walk_under_truncation() {
        use super::naval_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct_truncated(&memory);
        let (split, report, _) = kit::split_truncated(&memory);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
    }

    #[test]
    fn naval_split_copy_matches_direct_walk_on_fault_matrix() {
        use super::naval_test_kit as kit;
        use engine::memory::SliceMemory;
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            fault: std::ops::Range<usize>,
        }
        impl engine::Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if address < self.fault.end && end > self.fault.start {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        for fault in [
            kit::GAME_STATE + engine::cgame_state::COUNTRIES,
            kit::THEATRE,
            kit::GROUP,
        ] {
            let bytes = kit::fixture();
            let memory = Fault {
                memory: SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
            };
            let direct = kit::direct(&memory);
            let (split, report, _) = kit::split(&memory);
            assert!(report.raw_ok, "fault={fault:x}");
            kit::assert_matches_direct(&split, &direct);
        }
    }

    #[test]
    fn naval_staging_overflow_falls_back_to_typed_walk() {
        use super::naval_test_kit as kit;
        use super::{NavalCopyContext, NavalJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = NavalCopyContext {
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut tiny = NavalJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
            ..Default::default()
        };
        kit::stage_wings(&memory, &mut tiny);
        let report = super::copy_naval_refresh(&memory, ctx, &mut tiny);
        assert!(!report.raw_ok);
    }

    #[test]
    fn rail_tables_are_hourly_tier_h_with_no_refresh_gate() {
        use replay::rows::TableId;
        for table in [TableId::RailCooldowns, TableId::RailEdges] {
            assert!(!replay::cadence::is_staggered(table), "{table:?}");
            for hour in [0, 1, 2, 3, 60_820_531] {
                assert!(
                    replay::cadence::is_refresh(table, hour),
                    "{table:?} at {hour}"
                );
            }
        }
    }

    #[test]
    fn supremacy_uses_slot_23() {
        use replay::rows::TableId;
        assert!(replay::cadence::is_staggered(TableId::NavalSupremacy));
        for hour in -24_i32..48 {
            assert_eq!(
                replay::cadence::is_refresh(TableId::NavalSupremacy, hour),
                hour.rem_euclid(24) == 23
            );
        }
    }

    #[test]
    fn supremacy_split_copy_matches_direct_walk() {
        use super::supremacy_test_kit as kit;
        use engine::memory::{CountedMemory, ReadCounts};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let memory = CountedMemory::new(memory);
        let capture = read_accounting::Capture::new();
        let (split, report, mut job) = kit::split(&memory);
        let counts = capture.finish();
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.rows[0].country, u32::MAX);
        assert_eq!(split.rows[0].current, 1000);
        assert_eq!(split.rows[1].country, 1);
        let anchor = TableId::NavalSupremacy;
        let mut expected = read_accounting::ZERO;
        // Header, two slots, three region fields, owner, entries header,
        // and one 160-byte value vector: nine scalar requests, 260 bytes.
        expected[anchor.index()] = ReadCounts {
            copy_calls: 9,
            spans: 9,
            bytes: 260,
            ..ReadCounts::ZERO
        };
        assert_eq!(counts, expected);
        let mut refreshes = 0;
        for hour in 0..24 {
            if replay::cadence::is_refresh(anchor, hour) {
                refreshes += 1;
                continue;
            }
            let capture = read_accounting::Capture::new();
            let skipped = copy_supremacy_refresh(
                &memory,
                SupremacyCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    hour,
                    force_full: false,
                },
                &mut job,
            );
            assert_eq!(capture.finish(), read_accounting::ZERO);
            assert!(!skipped.raw_ok && !job.grow_for_next());
            assert_eq!((skipped.copy_ticks, skipped.raw_bytes), ([0], [0]));
        }
        assert_eq!(refreshes, 1);
        let measured = counts[anchor.index()];
        let per_refresh = [
            measured.copy_calls,
            measured.scattered_calls,
            measured.record_calls,
            measured.plan_calls,
            measured.spans,
            measured.bytes,
        ];
        println!(
            "daily_fixture cohort=supremacy_split_copy_matches_direct_walk anchor={anchor:?} counters=[copy,scattered,records,plans,spans,bytes] per_refresh={per_refresh:?} before_24h={:?} after_24h={per_refresh:?} basis=unchanged_refresh_x_24_legacy_hourly_vs_1_daily",
            per_refresh.map(|value| value * 24)
        );
    }

    #[test]
    fn supremacy_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit(2)/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        // The bulk value vector serves every truncated prefix identically.
        use super::supremacy_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for cap in [0, 1, 2, 64] {
            let direct = kit::direct_with(&memory, cap);
            let (split, report, _) = kit::split_with(&memory, cap);
            assert!(report.raw_ok, "cap={cap}");
            kit::assert_matches_direct(&split, &direct);
        }
        let direct = kit::direct_with(&memory, 2);
        assert_eq!(direct.outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn supremacy_split_copy_matches_direct_walk_on_fault_matrix() {
        use super::supremacy_test_kit as kit;
        use engine::memory::SliceMemory;
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            fault: std::ops::Range<usize>,
        }
        impl engine::Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if address < self.fault.end && end > self.fault.start {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        for fault in [
            kit::GAME_STATE + engine::cgame_state::STRATEGIC_REGIONS,
            kit::REGION_OBJECT,
            kit::DOMINANCE,
            kit::DOMINANCE + engine::cnaval_region_dominance::ENTRIES,
        ] {
            let bytes = kit::fixture();
            let memory = Fault {
                memory: SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
            };
            let direct = kit::direct(&memory);
            let (split, report, _) = kit::split(&memory);
            assert!(report.raw_ok, "fault={fault:x}");
            kit::assert_matches_direct(&split, &direct);
        }
    }

    #[test]
    fn supremacy_staging_overflow_falls_back_to_typed_walk() {
        use super::supremacy_test_kit as kit;
        use super::{SupremacyCopyContext, SupremacyJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = SupremacyCopyContext {
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut tiny = SupremacyJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
            ..Default::default()
        };
        let report = super::copy_supremacy_refresh(&memory, ctx, &mut tiny);
        assert!(!report.raw_ok);
    }

    #[test]
    fn supremacy_pools_cover_appendix_worst_case() {
        // Worst-case caps proof: the steady-state pools exceed a
        // conservative span/byte estimate built from the appendix census (49
        // supremacy rows over a handful of regions), so live refreshes never
        // take the overflow fallback in steady state.
        use super::SupremacyJob;
        let job = SupremacyJob::new();
        let spans = 1 + 10 * 6 + 49 * 2;
        assert!(
            job.log.spans.capacity() >= spans,
            "span cap {} < worst case {spans}",
            job.log.spans.capacity()
        );
        let bytes = 10 * 64 + 49 * 128;
        assert!(
            job.log.bytes.capacity() >= bytes,
            "byte cap {} < worst case {bytes}",
            job.log.bytes.capacity()
        );
    }

    #[test]
    fn supremacy_copy_sizes_cover_documented_read_set() {
        // S3: every Memory::copy size the supremacy copy pass issues,
        // recorded through the real pass. Roster/identity words (8/12/16)
        // and bulk value vectors (whole-vector takes, served in 32-entry
        // chunks at replay).
        use super::supremacy_test_kit as kit;
        use engine::Memory;
        use std::collections::BTreeSet;
        struct Recorder<'a> {
            inner: engine::memory::SliceMemory<'a>,
            sizes: std::cell::RefCell<BTreeSet<usize>>,
        }
        impl Memory for Recorder<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.sizes.borrow_mut().insert(output.len());
                self.inner.copy(address, output)
            }
        }
        let bytes = kit::fixture();
        let memory = Recorder {
            inner: kit::memory(&bytes),
            sizes: std::cell::RefCell::new(BTreeSet::new()),
        };
        let (_split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        let sizes = memory.sizes.borrow().clone();
        for required in [8, 12, 16] {
            assert!(
                sizes.contains(&required),
                "missing size {required} in {sizes:?}"
            );
        }
    }

    #[test]
    fn sunk_ships_use_slot_19() {
        use replay::rows::TableId;
        assert!(replay::cadence::is_staggered(TableId::SunkShips));
        for hour in -24_i32..48 {
            assert_eq!(
                replay::cadence::is_refresh(TableId::SunkShips, hour),
                hour.rem_euclid(24) == 19
            );
        }
    }

    #[test]
    fn sunk_split_copy_matches_direct_walk() {
        use super::sunk_test_kit as kit;
        use engine::memory::{CountedMemory, ReadCounts};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let memory = CountedMemory::new(memory);
        let capture = read_accounting::Capture::new();
        let (split, report, mut job) = kit::split(&memory);
        let counts = capture.finish();
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        // Two retained records: one with attribution class and province,
        // one minimal with a long inline name and heap attribution name.
        assert_eq!(split.outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.rows[0].victim_country, 3);
        assert_eq!(split.rows[0].province, 77);
        assert_eq!(split.rows[0].attribution_class_present, 1);
        assert_eq!(split.rows[1].victim_country, 5);
        assert_eq!(split.rows[1].province_present, 0);
        assert_eq!(split.rows[1].attribution_class_present, 0);
        let anchor = TableId::SunkShips;
        let mut expected = read_accounting::ZERO;
        // Header + two slots/records + three class headers + seven strings
        // (terminator and content each) + province identity: 24 requests, 585 bytes.
        expected[anchor.index()] = ReadCounts {
            copy_calls: 24,
            spans: 24,
            bytes: 585,
            ..ReadCounts::ZERO
        };
        assert_eq!(counts, expected);
        let mut refreshes = 0;
        for hour in kit::HOUR..kit::HOUR + 24 {
            if replay::cadence::is_refresh(anchor, hour) {
                refreshes += 1;
                continue;
            }
            let capture = read_accounting::Capture::new();
            let skipped = copy_sunk_refresh(
                &memory,
                SunkCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    hour,
                    force_full: false,
                },
                &mut job,
            );
            assert_eq!(capture.finish(), read_accounting::ZERO);
            assert!(!skipped.raw_ok && !job.grow_for_next());
            assert_eq!((skipped.copy_ticks, skipped.raw_bytes), ([0], [0]));
        }
        assert_eq!(refreshes, 1);
        let measured = counts[anchor.index()];
        let per_refresh = [
            measured.copy_calls,
            measured.scattered_calls,
            measured.record_calls,
            measured.plan_calls,
            measured.spans,
            measured.bytes,
        ];
        println!(
            "daily_fixture cohort=sunk_split_copy_matches_direct_walk anchor={anchor:?} counters=[copy,scattered,records,plans,spans,bytes] per_refresh={per_refresh:?} before_24h={:?} after_24h={per_refresh:?} basis=unchanged_refresh_x_24_legacy_hourly_vs_1_daily",
            per_refresh.map(|value| value * 24)
        );
    }

    #[test]
    fn sunk_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit(2)/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        use super::sunk_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for (rows, names) in [(0, 0), (1, 16), (2, 64), (64, 4096)] {
            let direct = kit::direct_with(&memory, rows, names);
            let (split, report, _) = kit::split_with(&memory, rows, names);
            assert!(report.raw_ok, "rows={rows} names={names}");
            kit::assert_matches_direct(&split, &direct);
        }
        let direct = kit::direct_with(&memory, 2, 64);
        assert_eq!(direct.outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn sunk_split_copy_matches_direct_walk_on_fault_matrix() {
        use super::sunk_test_kit as kit;
        use engine::memory::SliceMemory;
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            fault: std::ops::Range<usize>,
        }
        impl engine::Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if address < self.fault.end && end > self.fault.start {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        for fault in [
            kit::GAME_STATE + engine::cgame_state::SUNK_SHIP_HISTORY,
            kit::FIRST,
            kit::FIRST + engine::csunk_ship_info::SINK_HOUR,
            kit::CLASS,
            kit::FIRST + engine::csunk_ship_info::SHIP_NAME,
        ] {
            let bytes = kit::fixture();
            let memory = Fault {
                memory: SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
            };
            let direct = kit::direct(&memory);
            let (split, report, _) = kit::split(&memory);
            assert!(report.raw_ok, "fault={fault:x}");
            kit::assert_matches_direct(&split, &direct);
        }
    }

    #[test]
    fn sunk_staging_overflow_falls_back_to_typed_walk() {
        use super::sunk_test_kit as kit;
        use super::{SunkCopyContext, SunkJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = SunkCopyContext {
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            hour: kit::HOUR,
        };
        let mut tiny = SunkJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
            hour: 0,
            ..Default::default()
        };
        let report = super::copy_sunk_refresh(&memory, ctx, &mut tiny);
        assert!(!report.raw_ok);
    }

    #[test]
    fn sunk_pools_cover_appendix_worst_case() {
        // Worst-case caps proof: the steady-state pools exceed a
        // conservative span/byte estimate built from the appendix census
        // (one row with ~371 retained entries and names), so live refreshes
        // never take the overflow fallback in steady state.
        use super::SunkJob;
        let job = SunkJob::new();
        let spans = 1 + 371 * 12;
        assert!(
            job.log.spans.capacity() >= spans,
            "span cap {} < worst case {spans}",
            job.log.spans.capacity()
        );
        let bytes = 371 * 512;
        assert!(
            job.log.bytes.capacity() >= bytes,
            "byte cap {} < worst case {bytes}",
            job.log.bytes.capacity()
        );
    }

    #[test]
    fn sunk_copy_sizes_cover_documented_read_set() {
        // S3: every Memory::copy size the sunk copy pass issues, recorded
        // through the real pass. Vector words (8/16), records (0xa8),
        // class headers (0x38), name bytes (1 + content) and province
        // words (8/4).
        use super::sunk_test_kit as kit;
        use engine::Memory;
        use std::collections::BTreeSet;
        struct Recorder<'a> {
            inner: engine::memory::SliceMemory<'a>,
            sizes: std::cell::RefCell<BTreeSet<usize>>,
        }
        impl Memory for Recorder<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.sizes.borrow_mut().insert(output.len());
                self.inner.copy(address, output)
            }
        }
        let bytes = kit::fixture();
        let memory = Recorder {
            inner: kit::memory(&bytes),
            sizes: std::cell::RefCell::new(BTreeSet::new()),
        };
        let (_split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        let sizes = memory.sizes.borrow().clone();
        for required in [1, 4, 8, 16, 0xa8] {
            assert!(
                sizes.contains(&required),
                "missing size {required} in {sizes:?}"
            );
        }
    }
}

/// Shared synthetic address space for the naval-pack copy/decode harness:
/// one country, one land theatre with a named group and two members, one
/// OOB group with members, and the reference registry. Air theatres are
/// empty (matching the live campaign) with wing handles recorded as a
/// superset.
#[cfg(test)]
pub(crate) mod naval_test_kit {
    use super::{Capacities, Captured, NavalCopyContext, NavalCopyReport, NavalJob};
    use engine::{Memory, memory::SliceMemory, walk::Outcome};

    pub(crate) fn stage_wings(memory: &impl Memory, job: &mut NavalJob) {
        let mut wings = crate::extended::air::AirWingJob::new();
        let context = crate::extended::air::AirWingCopyContext {
            hour: 0,
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,

            limits: [0; 3],
            naval_limit: job.wing_limit(),
        };
        assert!(crate::extended::air::copy_air_refresh(memory, context, &mut wings).raw_ok);
        job.install_wings(&wings);
    }

    pub(crate) const BASE: usize = 0;
    pub(crate) const GAME_STATE: usize = 0x10000;
    const CTAB: usize = 0x11000;
    const LTAB: usize = 0x12000;
    pub(super) const C0: usize = 0x20000;
    pub(crate) const THEATRE: usize = 0x30000;
    pub(crate) const GROUP: usize = 0x31000;
    const MEMBERS: usize = 0x32000;
    const MEMBER1: usize = 0x33000;
    const MEMBER2: usize = 0x34000;
    const NAMES: usize = 0x35000;
    const OOB_TABLE: usize = 0x60000;
    const OOB_BUCKETS: usize = 0x61000;
    const OOB_GROUP: usize = 0x62000;
    const OOB_MEMBERS: usize = 0x63000;
    const OOB_MEMBER: usize = 0x650b8;
    const OOB_UNIT: usize = 0x65000;

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, count);
        p32(bytes, at + 12, count);
    }

    pub(crate) fn fixture() -> Vec<u8> {
        use engine::{
            carmy, ccountry, corder_instance, corders_group, corders_group_member,
            creference_registry as registry, ctheater_group, ctheatre, cunit,
            reference_bucket as bucket,
        };
        let mut bytes = vec![0; 0x3500000];
        header(
            &mut bytes,
            GAME_STATE + engine::cgame_state::COUNTRIES,
            CTAB,
            3,
        );
        p64(&mut bytes, CTAB, 0);
        p64(&mut bytes, CTAB + 8, C0 as u64);
        p64(&mut bytes, CTAB + 16, C0 as u64);
        header(
            &mut bytes,
            GAME_STATE + engine::cgame_state::COUNTRY_LINKS,
            LTAB,
            3,
        );
        p32(&mut bytes, LTAB, 0);
        p32(&mut bytes, LTAB + 4, 1);
        p32(&mut bytes, LTAB + 8, 1);
        p64(&mut bytes, C0, ccountry::VTABLE_RVA as u64);
        p32(&mut bytes, C0 + ccountry::ID, 1);
        // A valid empty wing census is different from a missing manager.
        p64(
            &mut bytes,
            GAME_STATE + engine::cgame_state::STRATEGIC_AIR_MANAGER,
            0x40000,
        );
        p64(
            &mut bytes,
            0x40000,
            engine::cstrategic_air_manager::VTABLE_RVA as u64,
        );
        header(
            &mut bytes,
            0x40000 + engine::cstrategic_air_manager::COUNTRY_AIR,
            0x41000,
            3,
        );
        // Land theatres: one theatre, one group named "AB", two members.
        header(&mut bytes, C0 + ccountry::THEATRES, 0x36000, 1);
        p64(&mut bytes, 0x36000, THEATRE as u64);
        header(&mut bytes, C0 + ccountry::AIR_THEATRES, 0x36100, 0);
        p64(&mut bytes, THEATRE, ctheatre::VTABLE_RVA as u64);
        p64(
            &mut bytes,
            THEATRE + ctheatre::HANDLE,
            (1u64 << 32) | ctheatre::REFERENCE_TYPE as u64,
        );
        p32(&mut bytes, THEATRE + ctheatre::OWNER, 1);
        p32(&mut bytes, THEATRE + ctheatre::VOLUNTEERS_THEATRE, 0);
        header(
            &mut bytes,
            THEATRE + ctheatre::THEATER_GROUP_DATA,
            0x36200,
            1,
        );
        p64(&mut bytes, 0x36200, GROUP as u64);
        p64(&mut bytes, GROUP, ctheater_group::VTABLE_RVA as u64);
        p64(
            &mut bytes,
            GROUP + ctheater_group::HANDLE,
            (1u64 << 32) | ctheater_group::REFERENCE_TYPE as u64,
        );
        p64(&mut bytes, GROUP + ctheater_group::THEATRE, THEATRE as u64);
        // Inline name "AB" (len 2, capacity 15, terminator).
        bytes[GROUP + ctheater_group::NAME..GROUP + ctheater_group::NAME + 2]
            .copy_from_slice(b"AB");
        p64(&mut bytes, GROUP + ctheater_group::NAME + 16, 2);
        p64(&mut bytes, GROUP + ctheater_group::NAME + 24, 15);
        bytes[GROUP + ctheater_group::NAME + 2] = 0;
        p32(&mut bytes, GROUP + ctheater_group::PRIORITY, 5);
        header(
            &mut bytes,
            GROUP + ctheater_group::ORDERS_GROUP_DATA,
            MEMBERS,
            2,
        );
        p64(&mut bytes, MEMBERS, MEMBER1 as u64);
        p64(&mut bytes, MEMBERS + 8, MEMBER2 as u64);
        for (member, id) in [(MEMBER1, 1u64), (MEMBER2, 2u64)] {
            p64(&mut bytes, member, corders_group::VTABLE_RVA as u64);
            p64(
                &mut bytes,
                member + corders_group::HANDLE,
                (id << 32) | corders_group::REFERENCE_TYPE as u64,
            );
            p64(
                &mut bytes,
                member + corders_group::THEATER_GROUP,
                GROUP as u64,
            );
            p64(&mut bytes, member + corders_group::THEATRE, THEATRE as u64);
        }
        // OOB registry with one group (same MEMBER1 group for simplicity,
        // plus dedicated OOB group with member + child).
        const HANDLE: u64 = (u32::MAX as u64) << 32 | corders_group::REFERENCE_TYPE as u64;
        p64(
            &mut bytes,
            BASE + registry::LOW_ROOTS_RVA + corders_group::REFERENCE_TYPE as usize * 8,
            OOB_TABLE as u64,
        );
        bytes[OOB_TABLE..OOB_TABLE + 32].fill(0);
        p64(
            &mut bytes,
            OOB_TABLE + registry::ENTRIES,
            OOB_BUCKETS as u64,
        );
        p32(&mut bytes, OOB_TABLE + registry::COUNT, 1);
        p32(&mut bytes, OOB_TABLE + registry::MASK, 7);
        bytes[OOB_TABLE + registry::OVERFLOW_DISTANCE] = 5;
        bytes[OOB_BUCKETS..OOB_BUCKETS + 14 * bucket::STRIDE].fill(0);
        bytes[OOB_BUCKETS + bucket::DISTANCE] = 1;
        p64(&mut bytes, OOB_BUCKETS + bucket::TYPE, HANDLE);
        p64(&mut bytes, OOB_BUCKETS + bucket::OBJECT, OOB_GROUP as u64);
        bytes[OOB_BUCKETS + 13 * bucket::STRIDE + bucket::DISTANCE] = bucket::TERMINAL_DISTANCE;
        bytes[OOB_GROUP..OOB_GROUP + 0x240].fill(0);
        p64(
            &mut bytes,
            OOB_GROUP,
            (BASE + corders_group::VTABLE_RVA) as u64,
        );
        p64(&mut bytes, OOB_GROUP + corders_group::HANDLE, HANDLE);
        p32(&mut bytes, OOB_GROUP + corders_group::COUNTRY, 1);
        bytes[OOB_GROUP + corders_group::IS_ARMY_GROUP] = 0;
        bytes[OOB_GROUP + corders_group::MOTORIZATION_LEVEL] = 0;
        p64(&mut bytes, OOB_GROUP + corders_group::THEATRE, 0);
        header(
            &mut bytes,
            OOB_GROUP + corders_group::MEMBER_DATA,
            OOB_MEMBERS,
            1,
        );
        p64(&mut bytes, OOB_MEMBERS, OOB_MEMBER as u64);
        // Member host (division) with backlink.
        p64(&mut bytes, OOB_UNIT, carmy::VTABLE_RVA as u64);
        p64(
            &mut bytes,
            OOB_UNIT + carmy::SECONDARY_BASE,
            carmy::SECONDARY_VTABLE_RVA as u64,
        );
        p64(&mut bytes, OOB_UNIT + cunit::HANDLE, 0x4500_0001u64);
        let member_host = OOB_MEMBER;
        let unit_base = member_host - corders_group_member::HOST_OFFSET;
        assert_eq!(unit_base, OOB_UNIT);
        p64(
            &mut bytes,
            member_host + corders_group_member::GROUP,
            OOB_GROUP as u64,
        );
        header(
            &mut bytes,
            OOB_GROUP + engine::carmy_group::MEMBER_DATA,
            0x66000,
            0,
        );
        let _ = (NAMES, corder_instance::VTABLE_RVA);
        bytes
    }

    pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    #[derive(Debug, PartialEq)]
    pub(crate) struct Split {
        pub land_outcome: Outcome,
        pub land: Vec<engine::ctheatre::Row>,
        pub groups_outcome: Outcome,
        pub groups: Vec<engine::ctheater_group::Row>,
        pub members_outcome: Outcome,
        pub members: Vec<engine::ctheater_group::MemberRow>,
        pub oob_groups_outcome: Outcome,
        pub oob_groups: Vec<engine::corders_group::GroupRow>,
        pub oob_members_outcome: Outcome,
        pub oob_members: Vec<engine::corders_group::MemberRow>,
        pub oob_children_outcome: Outcome,
        pub oob_children: Vec<engine::corders_group::ChildRow>,
    }

    fn collect(memory: &impl Memory, capacity: &Capacities) -> Split {
        let mut captured = Captured::with_capacities(capacity);
        // Typed reference: theatre walk + OOB via roots, single pass (no
        // double-read). Mirrors the game thread's capture order.
        let first = engine::ctheatre::walk(
            memory,
            BASE,
            GAME_STATE,
            true,
            engine::ctheatre::Buffers {
                land: &mut captured.land.rows,
                land_groups: &mut captured.land_groups.rows,
                land_members: &mut captured.land_members.rows,
                air: &mut captured.air.rows,
                air_groups: &mut captured.air_groups.rows,
                air_members: &mut captured.air_members.rows,
                names: &mut captured.theatre_names.first,
            },
        );
        captured.land.outcome = first.land;
        captured.land_groups.outcome = first.land_groups;
        captured.land_members.outcome = first.land_members;
        // OOB via registry roots.
        let mut roots = vec![engine::corders_group::Root::default(); 8];
        let roots_outcome = engine::corders_group::walk_roots(memory, BASE, &mut roots);
        let roots_len = roots_outcome.len();
        if let Outcome::Rows { .. } = roots_outcome {
            let walk = engine::corders_group::walk_order_of_battle(
                memory,
                BASE,
                &roots[..roots_len],
                &mut engine::corders_group::OrderOfBattle {
                    groups: &mut captured.groups.rows,
                    members: &mut captured.group_members.rows,
                    children: &mut captured.group_children.rows,
                    roster_scratch: &mut vec![Default::default(); roots_len],
                },
            );
            captured.groups.outcome = walk.groups;
            captured.group_members.outcome = walk.members;
            captured.group_children.outcome = walk.children;
        }
        Split {
            land_outcome: captured.land.outcome,
            land: captured.land.rows.clone(),
            groups_outcome: captured.land_groups.outcome,
            groups: captured.land_groups.rows.clone(),
            members_outcome: captured.land_members.outcome,
            members: captured.land_members.rows.clone(),
            oob_groups_outcome: captured.groups.outcome,
            oob_groups: captured.groups.rows.clone(),
            oob_members_outcome: captured.group_members.outcome,
            oob_members: captured.group_members.rows.clone(),
            oob_children_outcome: captured.group_children.outcome,
            oob_children: captured.group_children.rows.clone(),
        }
    }

    pub(crate) fn direct(memory: &impl Memory) -> Split {
        collect(memory, &Capacities::default())
    }

    fn decoded(captured: &Captured) -> Split {
        Split {
            land_outcome: captured.land.outcome,
            land: captured.land.rows.clone(),
            groups_outcome: captured.land_groups.outcome,
            groups: captured.land_groups.rows.clone(),
            members_outcome: captured.land_members.outcome,
            members: captured.land_members.rows.clone(),
            oob_groups_outcome: captured.groups.outcome,
            oob_groups: captured.groups.rows.clone(),
            oob_members_outcome: captured.group_members.outcome,
            oob_members: captured.group_members.rows.clone(),
            oob_children_outcome: captured.group_children.outcome,
            oob_children: captured.group_children.rows.clone(),
        }
    }

    /// The same pass given the callback's registry census, as the Naval job
    /// is: the copy walks the census roots, the decode takes them, and the
    /// decoded tables are the registry copy's. A complete census records no
    /// registry, identity or country span; any other census is not kept, and
    /// the copy records the registry exactly as without it.
    fn assert_census_roots_decode_the_same(
        memory: &impl Memory,
        capacity: &Capacities,
        expected: &Split,
        registry: &NavalJob,
    ) {
        let ctx = NavalCopyContext {
            hour: 0,
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,
        };
        let mut roots = vec![engine::corders_group::Root::default(); 64];
        let outcome = engine::corders_group::walk_roots(memory, BASE, &mut roots);
        let mut job = NavalJob::new();
        stage_wings(memory, &mut job);
        job.capture_roots(&roots[..outcome.len().min(roots.len())], outcome);
        assert!(super::copy_naval_refresh(memory, ctx, &mut job).raw_ok);
        let mut captured = Captured::with_capacities(capacity);
        super::decode_naval_payload(&mut captured, BASE, &job, &mut [0u64; 9]);
        assert_eq!(&decoded(&captured), expected, "census {outcome:?}");
        let spans = |job: &NavalJob| job.log.spans.len();
        match outcome {
            Outcome::Rows { len } => {
                assert_eq!(job.roots_len, Some(len));
                assert!(spans(&job) < spans(registry), "census {outcome:?}");
            }
            _ => {
                assert_eq!(job.roots_len, None);
                assert_eq!(spans(&job), spans(registry), "census {outcome:?}");
            }
        }
    }

    pub(crate) fn split(memory: &impl Memory) -> (Split, NavalCopyReport, NavalJob) {
        let split = split_registry(memory);
        assert_census_roots_decode_the_same(memory, &Capacities::default(), &split.0, &split.2);
        split
    }

    /// [`split`] without the census-roots pass, for tests that count the
    /// copy's reads.
    pub(crate) fn split_registry(memory: &impl Memory) -> (Split, NavalCopyReport, NavalJob) {
        let ctx = NavalCopyContext {
            hour: 0,
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,
        };
        let mut first = NavalJob::new();
        stage_wings(memory, &mut first);
        let report = super::copy_naval_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let mut captured = Captured::with_capacities(&Capacities::default());
        let mut ticks = [0u64; 9];
        super::decode_naval_payload(&mut captured, BASE, &first, &mut ticks);
        (
            Split {
                land_outcome: captured.land.outcome,
                land: captured.land.rows.clone(),
                groups_outcome: captured.land_groups.outcome,
                groups: captured.land_groups.rows.clone(),
                members_outcome: captured.land_members.outcome,
                members: captured.land_members.rows.clone(),
                oob_groups_outcome: captured.groups.outcome,
                oob_groups: captured.groups.rows.clone(),
                oob_members_outcome: captured.group_members.outcome,
                oob_members: captured.group_members.rows.clone(),
                oob_children_outcome: captured.group_children.outcome,
                oob_children: captured.group_children.rows.clone(),
            },
            report,
            first,
        )
    }

    pub(crate) fn direct_with(memory: &impl Memory, capacity: &Capacities) -> Split {
        collect(memory, capacity)
    }

    pub(crate) fn split_with(
        memory: &impl Memory,
        capacity: &Capacities,
    ) -> (Split, NavalCopyReport, NavalJob) {
        let ctx = NavalCopyContext {
            hour: 0,
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,
        };
        let mut first = NavalJob::new();
        stage_wings(memory, &mut first);
        let report = super::copy_naval_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let mut captured = Captured::with_capacities(capacity);
        let mut ticks = [0u64; 9];
        super::decode_naval_payload(&mut captured, BASE, &first, &mut ticks);
        assert_census_roots_decode_the_same(memory, capacity, &decoded(&captured), &first);
        (
            Split {
                land_outcome: captured.land.outcome,
                land: captured.land.rows.clone(),
                groups_outcome: captured.land_groups.outcome,
                groups: captured.land_groups.rows.clone(),
                members_outcome: captured.land_members.outcome,
                members: captured.land_members.rows.clone(),
                oob_groups_outcome: captured.groups.outcome,
                oob_groups: captured.groups.rows.clone(),
                oob_members_outcome: captured.group_members.outcome,
                oob_members: captured.group_members.rows.clone(),
                oob_children_outcome: captured.group_children.outcome,
                oob_children: captured.group_children.rows.clone(),
            },
            report,
            first,
        )
    }

    pub(crate) fn direct_truncated(memory: &impl Memory) -> Split {
        let capacity = Capacities {
            rows: [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
            theatre_names: 8,
            sunk_names: 8,
            orders_sources: 1,
            wing_sources: 1,
            roster_scratch: 8,
        };
        collect(memory, &capacity)
    }

    pub(crate) fn split_truncated(memory: &impl Memory) -> (Split, NavalCopyReport, NavalJob) {
        let ctx = NavalCopyContext {
            hour: 0,
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,
        };
        let mut first = NavalJob::new();
        stage_wings(memory, &mut first);
        let report = super::copy_naval_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let capacity = Capacities {
            rows: [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
            theatre_names: 8,
            sunk_names: 8,
            orders_sources: 1,
            wing_sources: 1,
            roster_scratch: 8,
        };
        let mut captured = Captured::with_capacities(&capacity);
        let mut ticks = [0u64; 9];
        super::decode_naval_payload(&mut captured, BASE, &first, &mut ticks);
        assert_census_roots_decode_the_same(memory, &capacity, &decoded(&captured), &first);
        (
            Split {
                land_outcome: captured.land.outcome,
                land: captured.land.rows.clone(),
                groups_outcome: captured.land_groups.outcome,
                groups: captured.land_groups.rows.clone(),
                members_outcome: captured.land_members.outcome,
                members: captured.land_members.rows.clone(),
                oob_groups_outcome: captured.groups.outcome,
                oob_groups: captured.groups.rows.clone(),
                oob_members_outcome: captured.group_members.outcome,
                oob_members: captured.group_members.rows.clone(),
                oob_children_outcome: captured.group_children.outcome,
                oob_children: captured.group_children.rows.clone(),
            },
            report,
            first,
        )
    }

    pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
        assert_eq!(split.land_outcome, direct.land_outcome, "land outcome");
        assert_eq!(split.land, direct.land, "land rows");
        assert_eq!(
            split.groups_outcome, direct.groups_outcome,
            "groups outcome"
        );
        assert_eq!(split.groups, direct.groups, "groups rows");
        assert_eq!(
            split.members_outcome, direct.members_outcome,
            "members outcome"
        );
        assert_eq!(split.members, direct.members, "members rows");
        assert_eq!(
            split.oob_groups_outcome, direct.oob_groups_outcome,
            "oob groups outcome"
        );
        assert_eq!(split.oob_groups, direct.oob_groups, "oob groups rows");
        assert_eq!(
            split.oob_members_outcome, direct.oob_members_outcome,
            "oob members outcome"
        );
        assert_eq!(split.oob_members, direct.oob_members, "oob members rows");
        assert_eq!(
            split.oob_children_outcome, direct.oob_children_outcome,
            "oob children outcome"
        );
        assert_eq!(split.oob_children, direct.oob_children, "oob children rows");
    }

    /// Decode-worker assertion against pooled buffers (same module, so
    /// private rows are visible).
    pub(crate) fn assert_captured_matches_direct(captured: &super::Captured, direct: &Split) {
        assert_eq!(captured.land.outcome, direct.land_outcome, "land outcome");
        assert_eq!(&captured.land.rows, &direct.land, "land rows");
        assert_eq!(
            captured.groups.outcome, direct.oob_groups_outcome,
            "oob groups outcome"
        );
        assert_eq!(&captured.groups.rows, &direct.oob_groups, "oob groups rows");
        assert_eq!(
            captured.group_members.outcome, direct.oob_members_outcome,
            "oob members outcome"
        );
        assert_eq!(
            &captured.group_members.rows, &direct.oob_members,
            "oob members rows"
        );
        assert_eq!(
            captured.group_children.outcome, direct.oob_children_outcome,
            "oob children outcome"
        );
        assert_eq!(
            &captured.group_children.rows, &direct.oob_children,
            "oob children rows"
        );
    }

    #[test]
    fn naval_oob_copy_reads_only_typed_child_and_provider_dependencies() {
        use engine::{carmy_group, corders_group, corders_group_member};
        use std::cell::RefCell;

        struct Fault<'a> {
            memory: SliceMemory<'a>,
            address: Option<usize>,
            reads: RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, output.len()));
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if self
                    .address
                    .is_some_and(|fault| address <= fault && fault < end)
                {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }

        const CHILDREN: usize = 0x66000;
        const CHILD: usize = 0x67000;
        let child_header = OOB_GROUP + carmy_group::MEMBER_DATA;
        let flag_address = OOB_GROUP + corders_group::IS_ARMY_GROUP;
        let backlink = OOB_MEMBER + corders_group_member::GROUP;
        assert_eq!(carmy_group::MEMBER_DATA, corders_group::SIZE);
        let mut bytes = fixture();
        for (name, flag, fault, poison, provider, required_fault) in [
            (
                "ordinary header outside allocation",
                0,
                Some(child_header),
                false,
                false,
                false,
            ),
            (
                "ordinary poisoned child header",
                0,
                None,
                true,
                false,
                false,
            ),
            ("army child", 1, None, false, false, false),
            (
                "army child header fault",
                1,
                Some(child_header),
                false,
                false,
                true,
            ),
            (
                "army child entry fault",
                1,
                Some(CHILDREN),
                false,
                false,
                true,
            ),
            (
                "army child identity fault",
                1,
                Some(CHILD),
                false,
                false,
                true,
            ),
            ("nonzero flag two", 2, None, false, false, false),
            (
                "nonzero flag two header fault",
                2,
                Some(child_header),
                false,
                false,
                true,
            ),
            ("nonzero flag maximum", u8::MAX, None, false, false, false),
            (
                "nonzero flag maximum header fault",
                u8::MAX,
                Some(child_header),
                false,
                false,
                true,
            ),
            (
                "unreadable discriminator",
                1,
                Some(flag_address),
                false,
                false,
                true,
            ),
            (
                "provider unused embedded vtable fault",
                0,
                Some(OOB_MEMBER),
                false,
                true,
                false,
            ),
            (
                "provider host identity fault",
                0,
                Some(OOB_UNIT),
                false,
                true,
                true,
            ),
            (
                "provider backlink fault",
                0,
                Some(backlink),
                false,
                true,
                true,
            ),
        ] {
            let vtable = if flag == 0 {
                corders_group::VTABLE_RVA
            } else {
                carmy_group::VTABLE_RVA
            };
            p64(&mut bytes, OOB_GROUP, (BASE + vtable) as u64);
            bytes[flag_address] = flag;
            header(
                &mut bytes,
                child_header,
                if poison { 0x7fff0000 } else { CHILDREN },
                1,
            );
            p64(&mut bytes, CHILDREN, CHILD as u64);
            p64(&mut bytes, CHILD, (BASE + corders_group::VTABLE_RVA) as u64);
            p64(
                &mut bytes,
                CHILD + corders_group::HANDLE,
                (7u64 << 32) | corders_group::REFERENCE_TYPE as u64,
            );
            p64(
                &mut bytes,
                OOB_GROUP + corders_group::HQ_PROVIDER,
                if provider { OOB_MEMBER as u64 } else { 0 },
            );
            let memory = Fault {
                memory: memory(&bytes),
                address: fault,
                reads: RefCell::new(Vec::new()),
            };
            let direct = direct(&memory);
            memory.reads.borrow_mut().clear();
            let (replayed, report, _) = split_registry(&memory);
            assert_matches_direct(&replayed, &direct);
            assert!(report.raw_ok, "{name}");
            if required_fault {
                assert!(matches!(
                    replayed.oob_groups_outcome,
                    Outcome::Unavailable { .. }
                ));
            }
            let reads = memory.reads.borrow();
            let visits_children = flag != 0 && fault != Some(flag_address);
            assert_eq!(
                reads.contains(&(child_header, 16)),
                visits_children,
                "{name}"
            );
            if !visits_children {
                assert!(!reads.contains(&(CHILDREN, 8)), "{name}");
                assert!(!reads.contains(&(CHILD, 16)), "{name}");
                assert!(!reads.contains(&(0x7fff0000, 8)), "{name}");
                // Skipping the subclass walk must not skip the later reverse census.
                assert_eq!(
                    reads
                        .iter()
                        .filter(|&&(address, len)| address
                            == OOB_GROUP + corders_group::THEATER_GROUP
                            && len == 8)
                        .count(),
                    2,
                    "{name}"
                );
            }
            assert!(!reads.contains(&(OOB_MEMBER, 16)), "{name}");
            if provider {
                // The provider field is taken with the root's other own
                // fields, then its host identity and backlink.
                let field = reads
                    .iter()
                    .position(|&read| read == (OOB_GROUP + corders_group::HQ_PROVIDER, 8));
                let host = reads
                    .windows(2)
                    .position(|window| window == [(OOB_UNIT, 32), (backlink, 8)]);
                assert!(
                    field.is_some() && host.is_some() && field < host,
                    "{name}"
                );
            }
            if !required_fault {
                assert_eq!(
                    replayed.oob_groups_outcome,
                    Outcome::Rows { len: 1 },
                    "{name}"
                );
                assert_eq!(
                    replayed.oob_members_outcome,
                    Outcome::Rows { len: 1 },
                    "{name}"
                );
                assert_eq!(
                    replayed.oob_children_outcome,
                    Outcome::Rows {
                        len: usize::from(flag != 0)
                    },
                    "{name}"
                );
            }
        }
    }
    thread_local! {
        /// Test builds only: take each order-of-battle root's own fields one
        /// by one, as before the gathered copy, for the reference log.
        pub(crate) static OOB_PER_FIELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// Each order-of-battle root's own one-entry spans are taken in one
    /// guarded copy. With every reference in use, and a fault at every byte
    /// of the root's fields and of what they reference, the log holds exactly
    /// the spans, bytes and failures of taking each field on its own, and it
    /// decodes, with the registry and with the census roots, to the typed
    /// walk's tables.
    #[test]
    fn oob_root_fields_in_one_copy_log_the_per_field_spans() {
        use engine::{corders_group, corders_group_member};
        use std::cell::RefCell;
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            address: Option<usize>,
            copies: RefCell<usize>,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                *self.copies.borrow_mut() += 1;
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if self
                    .address
                    .is_some_and(|fault| address <= fault && fault < end)
                {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        const LEADER: usize = 0x68000;
        let mut bytes = fixture();
        // An army group that is its own parent, with no children.
        p64(
            &mut bytes,
            OOB_GROUP,
            (BASE + engine::carmy_group::VTABLE_RVA) as u64,
        );
        bytes[OOB_GROUP + corders_group::IS_ARMY_GROUP] = 1;
        header(&mut bytes, OOB_GROUP + engine::carmy_group::MEMBER_DATA, 0x66000, 0);
        bytes[OOB_GROUP + corders_group::MOTORIZATION_LEVEL] = 3;
        p64(&mut bytes, OOB_GROUP + corders_group::THEATRE, THEATRE as u64);
        p64(&mut bytes, OOB_GROUP + corders_group::THEATER_GROUP, GROUP as u64);
        p64(&mut bytes, OOB_GROUP + corders_group::PARENT, OOB_GROUP as u64);
        p64(&mut bytes, OOB_GROUP + corders_group::LEADER, LEADER as u64);
        p64(&mut bytes, OOB_GROUP + corders_group::HQ_PROVIDER, OOB_MEMBER as u64);
        p64(&mut bytes, LEADER, (BASE + engine::carmy_leader::VTABLE_RVA) as u64);
        p64(&mut bytes, LEADER + 8, (9u64 << 32) | 4713);
        let spans = |job: &NavalJob| {
            let mut spans: Vec<_> = job
                .log
                .spans
                .iter()
                .map(|span| {
                    let bytes = if span.ok {
                        job.log.bytes[span.start as usize..][..span.len as usize].to_vec()
                    } else {
                        Vec::new()
                    };
                    (span.addr, span.len, span.ok, span.mask, bytes)
                })
                .collect();
            spans.sort();
            spans
        };
        let backlink = OOB_MEMBER + corders_group_member::GROUP;
        let objects = [
            (OOB_GROUP, corders_group::SIZE),
            (THEATRE, 0x20),
            (GROUP, 0x20),
            (LEADER, 0x20),
            (OOB_UNIT, 0x20),
            (backlink, 8),
        ];
        let faults = std::iter::once(None).chain(
            objects
                .iter()
                .flat_map(|&(object, size)| (object..object + size).map(Some)),
        );
        let mut copies = [0; 2];
        for fault in faults {
            let memory = Fault {
                memory: memory(&bytes),
                address: fault,
                copies: RefCell::new(0),
            };
            let direct = direct(&memory);
            let (gathered, report, _) = split(&memory);
            assert!(report.raw_ok, "fault {fault:x?}");
            // The typed reference walks the order of battle only over a
            // complete registry census; where that fails, the decode reports
            // the failure instead, with or without the gathered copy.
            let mut roots = [corders_group::Root::default(); 8];
            if matches!(
                corders_group::walk_roots(&memory, BASE, &mut roots),
                Outcome::Rows { .. }
            ) {
                assert_matches_direct(&gathered, &direct);
            }
            OOB_PER_FIELD.set(true);
            *memory.copies.borrow_mut() = 0;
            let (per_field, _, reference) = split_registry(&memory);
            copies[1] += *memory.copies.borrow();
            OOB_PER_FIELD.set(false);
            *memory.copies.borrow_mut() = 0;
            let (_, _, registry) = split_registry(&memory);
            copies[0] += *memory.copies.borrow();
            assert_eq!(gathered, per_field, "fault {fault:x?}");
            assert_eq!(spans(&registry), spans(&reference), "fault {fault:x?}");
            if fault.is_none() {
                assert_eq!(gathered.oob_groups_outcome, Outcome::Rows { len: 1 });
            }
        }
        // Through the plain slice every span is still one copy; the gathered
        // copy's saving is in guarded calls, which the bench counts.
        assert_eq!(copies[0], copies[1]);
    }
}


/// Shared synthetic address space for the supremacy copy/decode harness:
/// two region slots (one null) with dominance containers and value entries.
/// Mirrors the engine supremacy fixture shapes.
#[cfg(test)]
pub(crate) mod supremacy_test_kit {
    use super::{Capacities, Captured, SupremacyCopyContext, SupremacyCopyReport, SupremacyJob};
    use engine::{Memory, memory::SliceMemory, walk::Outcome};

    pub(crate) const BASE: usize = 0;
    pub(crate) const GAME_STATE: usize = 0x1000;
    const REGIONS: usize = 0x1400;
    pub(crate) const REGION_OBJECT: usize = 0x1500;
    pub(crate) const DOMINANCE: usize = 0x1800;
    const ROWS: usize = 0x1900;

    fn put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        put64(bytes, at, data as u64);
        put32(bytes, at + 8, count);
        put32(bytes, at + 12, count);
    }

    pub(crate) fn fixture() -> Vec<u8> {
        use engine::{
            cdominance_values, cgame_state, cnaval_region_dominance, cstrategic_region,
            naval_dominance_entry,
        };
        let mut bytes = vec![0; 0x4000];
        header(
            &mut bytes,
            GAME_STATE + cgame_state::STRATEGIC_REGIONS,
            REGIONS,
            3,
        );
        put64(&mut bytes, REGIONS, 0);
        put64(&mut bytes, REGIONS + 8, REGION_OBJECT as u64);
        put64(&mut bytes, REGIONS + 16, 0);
        put64(
            &mut bytes,
            REGION_OBJECT,
            cstrategic_region::VTABLE_RVA as u64,
        );
        put64(
            &mut bytes,
            REGION_OBJECT + cstrategic_region::SECONDARY,
            cstrategic_region::SECONDARY_VTABLE_RVA as u64,
        );
        put32(
            &mut bytes,
            REGION_OBJECT + cstrategic_region::ID,
            0x8765_4321,
        );
        put32(
            &mut bytes,
            REGION_OBJECT + cstrategic_region::RUNTIME_SLOT,
            1,
        );
        put64(
            &mut bytes,
            REGION_OBJECT + cstrategic_region::NAVAL_DOMINANCE,
            DOMINANCE as u64,
        );
        put64(
            &mut bytes,
            DOMINANCE,
            cnaval_region_dominance::VTABLE_RVA as u64,
        );
        put64(
            &mut bytes,
            DOMINANCE + cnaval_region_dominance::REGION,
            REGION_OBJECT as u64,
        );
        header(
            &mut bytes,
            DOMINANCE + cnaval_region_dominance::ENTRIES,
            ROWS,
            2,
        );
        for (index, country) in [u32::MAX, 1].into_iter().enumerate() {
            let outer = ROWS + index * naval_dominance_entry::STRIDE;
            let object = outer + naval_dominance_entry::VALUES;
            put32(
                &mut bytes,
                outer + naval_dominance_entry::COUNTRY_KEY,
                country,
            );
            put64(&mut bytes, object, cdominance_values::VTABLE_RVA as u64);
            put32(&mut bytes, object + cdominance_values::COUNTRY, country);
            put64(
                &mut bytes,
                object + cdominance_values::REGION,
                REGION_OBJECT as u64,
            );
            put64(
                &mut bytes,
                object + cdominance_values::CURRENT,
                (1000 + index) as u64,
            );
            put64(
                &mut bytes,
                object + cdominance_values::BASE_TARGET,
                (2000 + index) as u64,
            );
        }
        // Spare vector capacity holds poison and must not become live rows.
        put64(
            &mut bytes,
            ROWS + 2 * naval_dominance_entry::STRIDE,
            u64::MAX,
        );
        bytes
    }

    pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    pub(crate) struct Split {
        pub outcome: Outcome,
        pub rows: Vec<engine::cnaval_region_dominance::Row>,
    }

    fn collect_typed(memory: &impl Memory, cap: usize) -> Split {
        use engine::cnaval_region_dominance;
        let mut capacities = Capacities::default();
        capacities.rows[0] = cap;
        let mut captured = Captured::with_capacities(&capacities);
        let walked =
            cnaval_region_dominance::walk(memory, BASE, GAME_STATE, &mut captured.supremacy.rows);
        captured.supremacy.outcome = super::bounded(walked, captured.supremacy.rows.len());
        Split {
            outcome: captured.supremacy.outcome,
            rows: captured.supremacy.rows.clone(),
        }
    }

    fn collect_split(
        memory: &impl Memory,
        cap: usize,
    ) -> (Split, SupremacyCopyReport, SupremacyJob) {
        let ctx = SupremacyCopyContext {
            hour: 0,
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,
        };
        let mut capacities = Capacities::default();
        capacities.rows[0] = cap;
        let mut captured = Captured::with_capacities(&capacities);
        let mut first = SupremacyJob::new();
        let report = super::copy_supremacy_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let mut ticks = [0u64; 1];
        captured.decode_supremacy_payload(BASE, &first, &mut ticks);
        (
            Split {
                outcome: captured.supremacy.outcome,
                rows: captured.supremacy.rows.clone(),
            },
            report,
            first,
        )
    }

    pub(crate) fn direct(memory: &impl Memory) -> Split {
        collect_typed(memory, 4096)
    }

    pub(crate) fn split(memory: &impl Memory) -> (Split, SupremacyCopyReport, SupremacyJob) {
        collect_split(memory, 4096)
    }

    pub(crate) fn direct_with(memory: &impl Memory, cap: usize) -> Split {
        collect_typed(memory, cap)
    }

    pub(crate) fn split_with(
        memory: &impl Memory,
        cap: usize,
    ) -> (Split, SupremacyCopyReport, SupremacyJob) {
        collect_split(memory, cap)
    }

    pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
        assert_eq!(split.outcome, direct.outcome, "supremacy outcome");
        assert_eq!(split.rows, direct.rows, "supremacy rows");
    }
}

/// Shared synthetic address space for the sunk-ships copy/decode harness:
/// two retained records (one with an attribution class and province, one
/// minimal) with inline and heap names. Mirrors the engine sunk-ship fixture
/// shapes.
#[cfg(test)]
pub(crate) mod sunk_test_kit {
    use super::{Capacities, Captured, SunkCopyContext, SunkCopyReport, SunkJob};
    use engine::{Memory, memory::SliceMemory, walk::Outcome};

    pub(crate) const BASE: usize = 0;
    pub(crate) const GAME_STATE: usize = 0x100;
    pub(crate) const HOUR: i32 = 100;
    const VECTOR: usize = 0x800;
    pub(crate) const FIRST: usize = 0x900;
    const SECOND: usize = 0xb00;
    pub(crate) const CLASS: usize = 0xd00;
    const NULL_CLASS: usize = 0xe00;
    const MAP_PROVINCE: usize = 0xf00;
    const HEAP_NAMES: usize = 0x1000;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn short(bytes: &mut [u8], at: usize, value: &[u8]) {
        put(bytes, at, value);
        put(
            bytes,
            at + msvc_string::LENGTH,
            &(value.len() as u64).to_le_bytes(),
        );
        put(bytes, at + msvc_string::CAPACITY, &15_u64.to_le_bytes());
    }

    use engine::msvc_string;

    pub(crate) fn fixture() -> Vec<u8> {
        use engine::{cgame_date, cgame_state, cprovince, csub_unit_definition, csunk_ship_info};
        let mut bytes = vec![0_u8; 0x2000];
        put(
            &mut bytes,
            GAME_STATE + cgame_state::SUNK_SHIP_HISTORY,
            &(VECTOR as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            GAME_STATE + cgame_state::SUNK_SHIP_HISTORY_CAPACITY,
            &2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            GAME_STATE + cgame_state::SUNK_SHIP_HISTORY_COUNT,
            &2_u32.to_le_bytes(),
        );
        put(&mut bytes, VECTOR, &(FIRST as u64).to_le_bytes());
        put(&mut bytes, VECTOR + 8, &(SECOND as u64).to_le_bytes());
        for (record, hour, class, attribution, province, victim, attacker) in [
            (FIRST, 90u32, CLASS, NULL_CLASS, MAP_PROVINCE, 3u32, 4u32),
            (SECOND, 100u32, CLASS, 0usize, 0usize, 5u32, 6u32),
        ] {
            put(
                &mut bytes,
                record,
                &(csunk_ship_info::VTABLE_RVA as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                record + csunk_ship_info::SINK_DATE,
                &(cgame_date::VTABLE_RVA as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                record + csunk_ship_info::SINK_DATE_PERSISTENT,
                &(cgame_date::PERSISTENT_VTABLE_RVA as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                record + csunk_ship_info::SINK_HOUR,
                &hour.to_le_bytes(),
            );
            put(
                &mut bytes,
                record + csunk_ship_info::SHIP_CLASS,
                &(class as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                record + csunk_ship_info::ATTRIBUTION_CLASS,
                &(attribution as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                record + csunk_ship_info::VICTIM_COUNTRY,
                &victim.to_le_bytes(),
            );
            put(
                &mut bytes,
                record + csunk_ship_info::ATTACKER_COUNTRY,
                &attacker.to_le_bytes(),
            );
            put(
                &mut bytes,
                record + csunk_ship_info::MODEL_LEVEL,
                &2_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                record + csunk_ship_info::VICTIM_EQUIPMENT_VARIANT,
                &0x1234_5678_9abc_def0u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                record + csunk_ship_info::DAMAGE_SOURCE_REFERENCE,
                &0x0fed_cba9_8765_4321u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                record + csunk_ship_info::PROVINCE,
                &(province as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                record + csunk_ship_info::BATTLE,
                &0x5555_6666_7777_8888u64.to_le_bytes(),
            );
            bytes[record + csunk_ship_info::CAUSE_RECORD] = 1;
            bytes[record + csunk_ship_info::ASSISTED_KILL] = 0;
        }
        // Inline ship/attribution names on both records.
        short(&mut bytes, FIRST + csunk_ship_info::SHIP_NAME, b"Ship");
        short(
            &mut bytes,
            FIRST + csunk_ship_info::ATTRIBUTION_SOURCE_NAME,
            b"Src",
        );
        short(
            &mut bytes,
            SECOND + csunk_ship_info::SHIP_NAME,
            b"SecondShip",
        );
        // Heap attribution name on the second record.
        put(
            &mut bytes,
            SECOND + csunk_ship_info::ATTRIBUTION_SOURCE_NAME + msvc_string::DATA,
            &(HEAP_NAMES as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            SECOND + csunk_ship_info::ATTRIBUTION_SOURCE_NAME + msvc_string::LENGTH,
            &4_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            SECOND + csunk_ship_info::ATTRIBUTION_SOURCE_NAME + msvc_string::CAPACITY,
            &16_u64.to_le_bytes(),
        );
        put(&mut bytes, HEAP_NAMES, b"Heap");
        bytes[HEAP_NAMES + 4] = 0;
        // Class definitions with inline names and tokens.
        for (class, token, vtable) in [
            (CLASS, 11u32, csub_unit_definition::VTABLE_RVA),
            (NULL_CLASS, 0u32, csub_unit_definition::NULL_VTABLE_RVA),
        ] {
            put(&mut bytes, class, &(vtable as u64).to_le_bytes());
            put(
                &mut bytes,
                class + csub_unit_definition::TOKEN,
                &token.to_le_bytes(),
            );
            short(&mut bytes, class + csub_unit_definition::NAME, b"Cls");
        }
        put(
            &mut bytes,
            MAP_PROVINCE,
            &(cprovince::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MAP_PROVINCE + cprovince::ID,
            &77_u32.to_le_bytes(),
        );
        bytes
    }

    pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    pub(crate) struct Split {
        pub outcome: Outcome,
        pub rows: Vec<engine::csunk_ship_info::Record>,
        pub bytes_len: usize,
        pub required: usize,
    }

    fn collect_typed(memory: &impl Memory, rows_cap: usize, names_cap: usize) -> Split {
        use engine::csunk_ship_info;
        let mut capacities = Capacities::default();
        capacities.rows[13] = rows_cap;
        capacities.sunk_names = names_cap;
        let mut captured = Captured::with_capacities(&capacities);
        let walked = csunk_ship_info::walk(
            memory,
            BASE,
            GAME_STATE,
            HOUR,
            &mut captured.sunk.rows,
            &mut captured.sunk_names.first,
        );
        captured.sunk.outcome = super::bounded(walked.outcome, captured.sunk.rows.len());
        captured.sunk_names.len = walked.bytes_len;
        captured.sunk_names.required = walked.required_bytes;
        if !super::valid_sunk(
            captured.sunk.outcome,
            &captured.sunk.rows,
            &captured.sunk_names.first,
            walked.bytes_len,
            walked.required_bytes,
        ) {
            captured.sunk.outcome = super::invalid_names();
        }
        Split {
            outcome: captured.sunk.outcome,
            rows: captured.sunk.rows.clone(),
            bytes_len: captured.sunk_names.len,
            required: captured.sunk_names.required,
        }
    }

    fn collect_split(
        memory: &impl Memory,
        rows_cap: usize,
        names_cap: usize,
    ) -> (Split, SunkCopyReport, SunkJob) {
        let ctx = SunkCopyContext {
            force_full: true,
            base: BASE,
            game_state: GAME_STATE,
            hour: HOUR,
        };
        let mut capacities = Capacities::default();
        capacities.rows[13] = rows_cap;
        capacities.sunk_names = names_cap;
        let mut captured = Captured::with_capacities(&capacities);
        let mut first = SunkJob::new();
        let report = super::copy_sunk_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let mut ticks = [0u64; 1];
        captured.decode_sunk_payload(BASE, &first, &mut ticks);
        (
            Split {
                outcome: captured.sunk.outcome,
                rows: captured.sunk.rows.clone(),
                bytes_len: captured.sunk_names.len,
                required: captured.sunk_names.required,
            },
            report,
            first,
        )
    }

    pub(crate) fn direct(memory: &impl Memory) -> Split {
        collect_typed(memory, 2048, 256 * 1024)
    }

    pub(crate) fn split(memory: &impl Memory) -> (Split, SunkCopyReport, SunkJob) {
        collect_split(memory, 2048, 256 * 1024)
    }

    pub(crate) fn direct_with(memory: &impl Memory, rows: usize, names: usize) -> Split {
        collect_typed(memory, rows, names)
    }

    pub(crate) fn split_with(
        memory: &impl Memory,
        rows: usize,
        names: usize,
    ) -> (Split, SunkCopyReport, SunkJob) {
        collect_split(memory, rows, names)
    }

    pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
        assert_eq!(split.outcome, direct.outcome, "sunk outcome");
        assert_eq!(split.rows, direct.rows, "sunk rows");
        assert_eq!(split.bytes_len, direct.bytes_len, "sunk bytes len");
        assert_eq!(split.required, direct.required, "sunk required");
    }
}

/// Ignored capture-cost bench; see `bench_world` for the assumed scale. Run
/// with `cargo test -p collector --release -- --ignored l3b_bench --nocapture`.
/// Each case times the copies through the plain slice and through the
/// production guarded source, counts one copy's source requests, and times
/// the decode of the recorded log.
#[cfg(test)]
mod l3b_bench {
    use super::*;
    use crate::extended::bench_world::{self, Shape, World};

    /// The median and p95 of the LandTheatres header copy, of the order of
    /// battle copy and of the callback's registry census it walks, in µs.
    fn time_copies(
        memory: &impl engine::Memory,
        world: &World,
        job: &mut NavalJob,
    ) -> [(f64, f64); 3] {
        let theatres = bench_world::time(50, || {
            job.log.clear();
            copy_naval_theatres(
                &mut job.log,
                &mut job.roster,
                memory,
                world.base,
                world.game_state,
                false,
            );
        });
        // The pass clears the log first; so does each timed copy.
        let oob = bench_world::time(50, || {
            job.log.clear();
            copy_naval_oob(job, memory, world.base);
            assert!(!job.log.overflowed);
        });
        let mut roots = vec![corders_group::Root::default(); world.groups as usize];
        let census = bench_world::time(50, || {
            let outcome = corders_group::walk_roots(memory, world.base, &mut roots);
            job.capture_roots(&roots, outcome);
        });
        assert!(job.roots_len.is_some());
        [theatres, oob, census]
    }

    /// The callback's registry census of the pass into the job's snapshot.
    fn census_roots(memory: &impl engine::Memory, world: &World, job: &mut NavalJob) {
        let mut roots = vec![corders_group::Root::default(); world.groups as usize];
        let outcome = corders_group::walk_roots(memory, world.base, &mut roots);
        job.capture_roots(&roots, outcome);
    }

    #[test]
    #[ignore = "timing bench"]
    fn l3b_bench_hourly_land_theatres_and_order_of_battle_ordinals() {
        for countries in [1_500, 5_000] {
            for theatre_links in [true, false] {
                let world = bench_world::build(
                    countries,
                    Shape {
                        named: true,
                        theatre_links,
                    },
                );
                let slice = world.slice();
                // The callback's country census of the hour, as the Naval job
                // sees it: every copy reads through its roster view.
                let mut countries_cache = engine::country_memory::Cache::with_capacity(
                    engine::country_memory::Cache::cells_for_countries(
                        countries as usize + 1,
                        crate::country_scale::MIN_COUNTRY_CELLS,
                    ),
                );
                let _ = countries_cache.prepare(&slice, world.base, world.game_state);
                let live = world.live();
                let slice_roster = countries_cache.roster_view(&slice);
                let live_roster = countries_cache.roster_view(&live);
                let mut job = NavalJob::new();
                // An off-slot hour: the land theatre headers and the order of
                // battle are the hourly work; theatre details stay daily.
                let ctx = NavalCopyContext {
                    hour: 1,
                    force_full: false,
                    base: world.base,
                    game_state: world.game_state,
                };
                // Grow the raw log and the roster snapshot off the clock, as
                // the spool thread does, so every case times the steady path.
                while {
                    census_roots(&slice, &world, &mut job);
                    !copy_naval_refresh(&slice_roster, ctx, &mut job).raw_ok
                        || job.grow_for_next()
                } {
                    job.grow();
                }
                assert!(job.roots_len.is_some());
                let ((), census_work) = world.counted(|memory| {
                    census_roots(memory, &world, &mut job);
                });
                let ((), theatres_work) = world.counted(|memory| {
                    job.log.clear();
                    copy_naval_theatres(
                        &mut job.log,
                        &mut job.roster,
                        &countries_cache.roster_view(memory),
                        world.base,
                        world.game_state,
                        false,
                    );
                });
                let ((), oob_work) = world.counted(|memory| {
                    job.log.clear();
                    copy_naval_oob(&mut job, &countries_cache.roster_view(memory), world.base);
                });
                let all = |counts| {
                    bench_world::work(bench_world::total(&counts, &replay::rows::TableId::ALL))
                };
                println!(
                    "naval-work countries={countries} theatre_links={theatre_links}: LandTheatres copy {}; order-of-battle copy {}; roots census {}",
                    all(theatres_work),
                    all(oob_work),
                    all(census_work)
                );
                for source in ["slice", "live"] {
                    let [(theatres, theatres_p95), (oob, oob_p95), (census, census_p95)] =
                        if source == "slice" {
                            time_copies(&slice_roster, &world, &mut job)
                        } else {
                            time_copies(&live_roster, &world, &mut job)
                        };
                    println!(
                        "naval countries={countries} theatres={countries} groups={} theatre_links={theatre_links} source={source}: LandTheatres copy {theatres:.1} us (p95 {theatres_p95:.1}), order-of-battle copy {oob:.1} us (p95 {oob_p95:.1}), roots census {census:.1} us (p95 {census_p95:.1})",
                        world.groups,
                    );
                }
                assert!(copy_naval_refresh(&slice_roster, ctx, &mut job).raw_ok);
                let rows = 4 * world.groups as usize;
                let capacity = Capacities {
                    rows: [rows; N],
                    ..Capacities::default()
                };
                let mut captured = Captured::with_capacities(&capacity);
                let mut ticks = [0; NAVAL_PACK_N];
                // The raw replay's decode of every hourly naval table: the
                // registry census, the order of battle and the theatre headers.
                let (decode, decode_p95) = bench_world::time(20, || {
                    decode_naval_payload(&mut captured, world.base, &job, &mut ticks);
                });
                assert_eq!(
                    captured.land.outcome,
                    Outcome::Rows {
                        len: countries as usize
                    }
                );
                assert_eq!(
                    captured.groups.outcome,
                    Outcome::Rows {
                        len: world.groups as usize
                    }
                );
                assert_eq!(
                    captured.groups.rows[..world.groups as usize]
                        .iter()
                        .all(|row| row.theatre_ordinal.is_some()),
                    theatre_links
                );
                println!(
                    "naval-decode countries={countries} theatre_links={theatre_links}: hourly naval decode {decode:.1} us (p95 {decode_p95:.1}), raw bytes {}, spans {}, bulk {}",
                    job.log.raw_bytes.iter().sum::<u64>(),
                    job.log.spans.len(),
                    job.log.has_bulk
                );
            }
        }
    }
}
